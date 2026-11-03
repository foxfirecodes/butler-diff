//! Paged, read-only access to the two committed trees in a semantic review.
use crate::{
    acp::Cancellation,
    semantic::{Evidence, Side},
};
use anyhow::{Context, Result, bail, ensure};
use gpui_kit::component::diff::DiffFile;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    path::{Component, Path},
    process::{Child, Command, Stdio},
};

const PAGE_BYTES: usize = 24 * 1024;
const MAX_LINE: usize = 16 * 1024;

#[derive(Clone, Debug, Default)]
pub struct Coverage {
    pub read_lines: usize,
    pub total_lines: usize,
    pub partial_files: usize,
}

#[derive(Default, Serialize, Deserialize)]
pub struct ReadScope {
    lines: Vec<Evidence>,
}
impl ReadScope {
    pub fn supplied_ranges(&self, evidence: &Evidence) -> String {
        let ranges: Vec<_> = self
            .lines
            .iter()
            .filter(|range| range.path == evidence.path && range.side == evidence.side)
            .take(8)
            .map(|range| format!("{}–{}", range.start_line, range.end_line))
            .collect();
        if ranges.is_empty() {
            "none".into()
        } else {
            ranges.join(", ")
        }
    }
    fn record(&mut self, path: &str, side: Side, line: usize) {
        if let Some(range) = self.lines.iter_mut().rev().find(|e| {
            e.path == path
                && e.side == side
                && line >= e.start_line.saturating_sub(1)
                && line <= e.end_line.saturating_add(1)
        }) {
            range.start_line = range.start_line.min(line);
            range.end_line = range.end_line.max(line);
        } else {
            self.lines.push(Evidence {
                path: path.into(),
                side,
                start_line: line,
                end_line: line,
            });
        }
    }
    pub fn contains(&self, evidence: &Evidence) -> bool {
        (evidence.start_line..=evidence.end_line)
            .all(|line| self.contains_line(&evidence.path, evidence.side, line))
    }
    fn contains_line(&self, path: &str, side: Side, line: usize) -> bool {
        self.lines
            .iter()
            .any(|e| e.path == path && e.side == side && line >= e.start_line && line <= e.end_line)
    }
}

pub fn tools() -> Value {
    let string = json!({"type":"string"});
    let side = json!({"type":"string","enum":["original","modified"]});
    let offset = json!({"type":"integer","minimum":0});
    let count = json!({"type":"integer","minimum":1,"maximum":200});
    let mut tools = Vec::new();
    let mut add = |name, description, properties, required| {
        tools.push(json!({
        "type":"function","name":name,"description":description,
        "inputSchema":{"type":"object","additionalProperties":false,"properties":properties,"required":required}
    }))
    };
    add(
        "gander_report_progress",
        "Keep the reviewer informed with one concise sentence about the behavior you are investigating or an established observation. Call immediately, when focus changes, and before final synthesis. This is a progress update, not the final review. Do not include private reasoning, code, generic waiting text or invented percentages.",
        json!({"message":{"type":"string","minLength":1,"maxLength":240}}),
        json!(["message"]),
    );
    add(
        "gander_list_changes",
        "Page the changed-file inventory, including before/after paths and counts. Does not supply source evidence. Follow next_offset until null.",
        json!({"offset":offset}),
        json!(["offset"]),
    );
    add(
        "gander_read_diff",
        "Read a page of one file's immutable diff. start_line is a zero-based DIFF line offset; use next_line for continuation. Each returned line has exact original/modified SOURCE coordinates for citations. Read all relevant pages; unread regions are unknown. Oversized lines are marked omitted and cannot be cited.",
        json!({"path":string,"start_line":offset,"line_count":count}),
        json!(["path", "start_line", "line_count"]),
    );
    add(
        "gander_read_source",
        "Read numbered source from the committed original or modified tree, including unchanged context. start_line is one-based SOURCE line, not a diff offset. Use next_line for continuation. For renamed changed files use the displayed comparison path on both sides. Never reads working files. Oversized lines are marked omitted and cannot be cited.",
        json!({"path":string,"side":side,"start_line":{"type":"integer","minimum":1},"line_count":count}),
        json!(["path", "side", "start_line", "line_count"]),
    );
    add(
        "gander_search_source",
        "Find literal text in a committed tree to discover callers, renderers, tests and behavior. Returns bounded numbered matches, not a complete call graph. path_prefix restricts to a repository directory or file; empty string searches the tree. Page with next_offset or narrow the query; read surrounding source before inferring a flow.",
        json!({"query":string,"side":side,"path_prefix":string,"offset":offset}),
        json!(["query", "side", "path_prefix", "offset"]),
    );
    json!(tools)
}

struct Change<'a> {
    file: DiffFile,
    patch: &'a str,
}
pub struct Snapshot<'a> {
    root: &'a Path,
    base: &'a str,
    tip: &'a str,
    changes: Vec<Change<'a>>,
    pub scope: ReadScope,
    cancel: Cancellation,
    hunk: regex::Regex,
}
impl<'a> Snapshot<'a> {
    pub fn new(
        root: &'a Path,
        base: &'a str,
        tip: &'a str,
        patch: &'a str,
        cancel: Cancellation,
    ) -> Result<Self> {
        for revision in [base, tip] {
            ensure!(
                [40, 64].contains(&revision.len())
                    && revision.bytes().all(|b| b.is_ascii_hexdigit()),
                "Source reads require exact commit IDs"
            );
        }
        let mut changes = Vec::new();
        for text in crate::semantic::patches(patch) {
            let file = DiffFile::parse(text)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .into_iter()
                .next()
                .context("Missing changed file")?;
            changes.push(Change { file, patch: text });
        }
        Ok(Self {
            root,
            base,
            tip,
            changes,
            scope: ReadScope::default(),
            cancel,
            hunk: regex::Regex::new(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@")?,
        })
    }
    pub fn changed_paths(&self) -> Vec<&str> {
        self.changes
            .iter()
            .map(|c| c.file.path().as_str())
            .collect()
    }
    pub fn coverage(&self, scope: &ReadScope) -> Coverage {
        let mut coverage = Coverage::default();
        for change in &self.changes {
            let (mut old, mut new, mut in_hunk, mut read, mut total) = (0, 0, false, 0, 0);
            for text in change.patch.lines() {
                if let Some(c) = self.hunk.captures(text) {
                    old = c[1].parse::<usize>().unwrap();
                    new = c[2].parse::<usize>().unwrap();
                    in_hunk = true;
                } else if in_hunk {
                    let changed = match text.as_bytes().first() {
                        Some(b' ') => {
                            old += 1;
                            new += 1;
                            None
                        }
                        Some(b'-') => {
                            let line = old;
                            old += 1;
                            Some((Side::Original, line))
                        }
                        Some(b'+') => {
                            let line = new;
                            new += 1;
                            Some((Side::Modified, line))
                        }
                        _ => None,
                    };
                    if let Some((side, line)) = changed {
                        total += 1;
                        read += usize::from(scope.contains_line(
                            change.file.path().as_str(),
                            side,
                            line,
                        ));
                    }
                }
            }
            coverage.read_lines += read;
            coverage.total_lines += total;
            coverage.partial_files += usize::from(read < total);
        }
        coverage
    }
    pub fn call(&mut self, tool: &str, args: &Value) -> Result<Value> {
        self.cancel.check()?;
        let text = |key: &str| args[key].as_str().with_context(|| format!("Missing {key}"));
        match tool {
            "gander_list_changes" => self.inventory(number(args, "offset", 0)?),
            "gander_read_diff" => {
                self.diff(text("path")?, number(args, "start_line", 0)?, count(args)?)
            }
            "gander_read_source" => self.source(
                text("path")?,
                serde_json::from_value(args["side"].clone())?,
                number(args, "start_line", 1)?,
                count(args)?,
            ),
            "gander_search_source" => self.search(
                text("query")?,
                serde_json::from_value(args["side"].clone())?,
                text("path_prefix")?,
                number(args, "offset", 0)?,
            ),
            _ => bail!("Unknown committed-source tool"),
        }
    }
    pub fn inventory(&self, offset: usize) -> Result<Value> {
        ensure!(
            offset <= self.changes.len(),
            "Inventory offset is past the end"
        );
        let mut files = Vec::new();
        let mut used = 0;
        for change in self.changes.iter().skip(offset).take(50) {
            let file = &change.file;
            let entry = json!({"path":file.path().as_str(),"original_path":file.original_path().map(|p|p.as_str()),
                "modified_path":file.modified_path().map(|p|p.as_str()),"additions":file.additions(),"deletions":file.deletions(),"binary":file.is_binary(),"diff_lines":change.patch.lines().count()});
            let size = serde_json::to_vec(&entry)?.len();
            ensure!(
                size <= PAGE_BYTES,
                "A changed path exceeds the inventory page size"
            );
            if used + size > PAGE_BYTES {
                break;
            }
            used += size;
            files.push(entry);
        }
        let next = offset + files.len();
        Ok(
            json!({"files":files,"total_files":self.changes.len(),"next_offset":(next < self.changes.len()).then_some(next)}),
        )
    }
    fn diff(&mut self, path: &str, start: usize, count: usize) -> Result<Value> {
        let change = self
            .changes
            .iter()
            .find(|c| c.file.path().as_str() == path)
            .context("Not a changed file")?;
        let mut lines = Vec::new();
        let (mut old, mut new, mut in_hunk, mut used) = (0, 0, false, 0);
        let mut next = None;
        let mut total = 0;
        for (index, text) in change.patch.lines().enumerate() {
            self.cancel.check()?;
            total = index + 1;
            let (mut original, mut modified) = (None, None);
            if let Some(c) = self.hunk.captures(text) {
                old = c[1].parse::<usize>()?;
                new = c[2].parse::<usize>()?;
                in_hunk = true;
            } else if in_hunk {
                match text.as_bytes().first() {
                    Some(b' ') => {
                        original = Some(old);
                        modified = Some(new);
                        old += 1;
                        new += 1;
                    }
                    Some(b'-') => {
                        original = Some(old);
                        old += 1;
                    }
                    Some(b'+') => {
                        modified = Some(new);
                        new += 1;
                    }
                    _ => {}
                }
            }
            if index < start {
                continue;
            }
            let omission = json!({"diff_line":index,"omitted":"Source line exceeds the page size; cannot cite this line"});
            let (entry, supplied) = if text.len() > MAX_LINE {
                (omission, false)
            } else {
                fit_entry(
                    json!({"diff_line":index,"text":text,"original_line":original,"modified_line":modified}),
                    omission,
                )?
            };
            let size = serde_json::to_vec(&entry)?.len();
            if lines.len() == count || used + size > PAGE_BYTES {
                next = Some(index);
                break;
            }
            used += size;
            if supplied {
                if let Some(line) = original {
                    self.scope.record(path, Side::Original, line);
                }
                if let Some(line) = modified {
                    self.scope.record(path, Side::Modified, line);
                }
            }
            lines.push(entry);
        }
        ensure!(start <= total, "Diff offset is past the end");
        Ok(json!({"path":path,"lines":lines,"next_line":next}))
    }
    fn revision(&self, side: Side) -> &'a str {
        match side {
            Side::Original => self.base,
            Side::Modified => self.tip,
        }
    }
    fn source_path(&self, path: &str, side: Side) -> Result<String> {
        valid_path(path)?;
        if let Some(change) = self.changes.iter().find(|c| c.file.path().as_str() == path) {
            return match side {
                Side::Original => change.file.original_path(),
                Side::Modified => change.file.modified_path(),
            }
            .map(|p| p.to_string())
            .context("File does not exist on this side of the comparison");
        }
        Ok(path.into())
    }
    fn display_path(&self, path: &str, side: Side) -> String {
        self.changes
            .iter()
            .find(|c| {
                match side {
                    Side::Original => c.file.original_path(),
                    Side::Modified => c.file.modified_path(),
                }
                .is_some_and(|p| p.as_str() == path)
            })
            .map_or_else(|| path.into(), |c| c.file.path().to_string())
    }
    fn source(&mut self, path: &str, side: Side, start: usize, count: usize) -> Result<Value> {
        ensure!(start > 0, "Source lines are one-based");
        let source_path = self.source_path(path, side)?;
        let revision = self.revision(side);
        let object = format!("{revision}:{source_path}");
        let mut process = GitRead::spawn(self.root, &["cat-file", "blob", &object])?;
        let mut lines = Vec::new();
        let (mut line, mut used, mut next) = (0, 0, None);
        while let Some(text) = process.line(b'\n', &self.cancel)? {
            line += 1;
            if line < start {
                continue;
            }
            let omission = json!({"line":line,"omitted":"Source line exceeds the page size or is not UTF-8 text; cannot cite this line"});
            let (entry, supplied) = match &text {
                Some(text) => fit_entry(json!({"line":line,"text":text}), omission)?,
                None => (omission, false),
            };
            let size = serde_json::to_vec(&entry)?.len();
            if lines.len() == count || used + size > PAGE_BYTES {
                next = Some(line);
                break;
            }
            used += size;
            if supplied {
                self.scope.record(path, side, line);
            }
            lines.push(entry);
        }
        if next.is_none() {
            process
                .success()
                .context("File is not an available committed blob")?;
        }
        Ok(
            json!({"path":path,"snapshot_path":source_path,"side":side,"revision":revision,"lines":lines,"next_line":next}),
        )
    }
    fn search(&mut self, query: &str, side: Side, prefix: &str, offset: usize) -> Result<Value> {
        ensure!(
            (2..=200).contains(&query.len()) && !query.contains(['\n', '\0']),
            "Search needs 2–200 bytes of literal text on one line"
        );
        if !prefix.is_empty() {
            valid_path(prefix)?;
        }
        let revision = self.revision(side);
        let mut args = vec![
            "grep",
            "--null",
            "-n",
            "-I",
            "-F",
            "--no-color",
            "--no-textconv",
            "-e",
            query,
            revision,
            "--",
        ];
        let pathspec = format!(":(literal){prefix}");
        if !prefix.is_empty() {
            args.push(&pathspec);
        }
        let mut process = GitRead::spawn(self.root, &args)?;
        let (mut hits, mut matched, mut used, mut next) = (Vec::new(), 0, 0, None);
        while let Some(path) = process.line(0, &self.cancel)? {
            let path = path.context("Search path is too long or not UTF-8")?;
            let path = path
                .strip_prefix(&format!("{revision}:"))
                .context("Unexpected Git search path")?;
            let number = process
                .line(0, &self.cancel)?
                .flatten()
                .context("Missing search line number")?
                .parse::<usize>()?;
            let text = process
                .line(b'\n', &self.cancel)?
                .context("Missing search match")?;
            if matched < offset {
                matched += 1;
                continue;
            }
            let display = self.display_path(path, side);
            let omission = json!({"path":display,"line":number,"omitted":"Match line exceeds the page size or is not UTF-8 text; cannot cite this line"});
            let (hit, supplied) = match &text {
                Some(text) => {
                    fit_entry(json!({"path":display,"line":number,"text":text}), omission)?
                }
                None => (omission, false),
            };
            let size = serde_json::to_vec(&hit)?.len();
            if hits.len() == 40 || used + size > PAGE_BYTES {
                next = Some(matched);
                break;
            }
            matched += 1;
            used += size;
            if supplied {
                self.scope.record(&display, side, number);
            }
            hits.push(hit);
        }
        if next.is_none() {
            let status = process.child.wait()?;
            ensure!(
                status.success() || status.code() == Some(1),
                "Committed-source search failed ({status})"
            );
        }
        Ok(json!({"side":side,"revision":revision,"hits":hits,"next_offset":next}))
    }
}

fn number(args: &Value, key: &str, minimum: usize) -> Result<usize> {
    let value = args[key]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .with_context(|| format!("{key} must be an integer"))?;
    ensure!(value >= minimum, "{key} is out of range");
    Ok(value)
}
fn fit_entry(entry: Value, omission: Value) -> Result<(Value, bool)> {
    // JSON escaping can make a short source line much larger on the wire.
    Ok(if serde_json::to_vec(&entry)?.len() > PAGE_BYTES - 512 {
        (omission, false)
    } else {
        (entry, true)
    })
}
fn count(args: &Value) -> Result<usize> {
    let count = number(args, "line_count", 1)?;
    ensure!(count <= 200, "Request at most 200 lines per page");
    Ok(count)
}
fn valid_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && path.len() <= 4096
            && !path.contains('\0')
            && Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "Use a repository-relative source path"
    );
    Ok(())
}

struct GitRead {
    child: Child,
    output: BufReader<std::process::ChildStdout>,
}
impl GitRead {
    fn spawn(root: &Path, args: &[&str]) -> Result<Self> {
        let mut child = Command::new("git")
            .arg("--no-pager")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let output = BufReader::new(child.stdout.take().unwrap());
        Ok(Self { child, output })
    }
    /// None means EOF; Some(None) is an omitted oversized or non-UTF-8 line.
    fn line(&mut self, delimiter: u8, cancel: &Cancellation) -> Result<Option<Option<String>>> {
        let mut bytes = Vec::new();
        let mut oversized = false;
        let mut consumed = false;
        loop {
            cancel.check()?;
            let buffer = self.output.fill_buf()?;
            if buffer.is_empty() {
                if !consumed {
                    return Ok(None);
                }
                break;
            }
            let end = buffer.iter().position(|b| *b == delimiter);
            let count = end.map_or(buffer.len(), |i| i + 1);
            consumed = true;
            if !oversized && bytes.len() + count <= MAX_LINE {
                bytes.extend_from_slice(&buffer[..count]);
            } else {
                oversized = true;
                bytes.clear();
            }
            self.output.consume(count);
            if end.is_some() {
                break;
            }
        }
        if oversized {
            return Ok(Some(None));
        }
        if bytes.last() == Some(&delimiter) {
            bytes.pop();
        }
        Ok(Some(
            String::from_utf8(bytes)
                .ok()
                .filter(|text| !text.contains('\0')),
        ))
    }
    fn success(&mut self) -> Result<()> {
        ensure!(
            self.child.wait()?.success(),
            "Git couldn't read this snapshot"
        );
        Ok(())
    }
}
impl Drop for GitRead {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
