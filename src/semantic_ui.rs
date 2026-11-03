use super::*;
use crate::{acp, semantic};
use gpui_kit::component::{WindowExt, text::TextViewStyle};

fn markdown(id: impl Into<ElementId>, text: impl Into<SharedString>) -> TextView {
    TextView::markdown(id, text)
        .w_full()
        .min_w_0()
        .on_link_click(|url, _, _, cx| {
            if url.starts_with("https://")
                || url.starts_with("http://")
                || url.starts_with("mailto:")
            {
                cx.open_url(url);
            }
        })
}

fn fenced_source(path: &str, source: &str) -> String {
    let language = std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| extension.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("text");
    let fence = "`".repeat(
        source
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    format!("{fence}{language}\n{source}\n{fence}")
}

fn source_preview(path: &str, lines: &[(u64, String)], cx: &App) -> impl IntoElement {
    let source = lines
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let code_style = StyleRefinement::default()
        .p_0()
        .border_0()
        .bg(cx.theme().background)
        .text_size(px(13.))
        .line_height(px(20.))
        .whitespace_nowrap();
    div()
        .id("semantic-source-context")
        .max_h(px(480.))
        .min_w_0()
        .overflow_scroll()
        .child(
            div()
                .flex()
                .items_start()
                .gap_3()
                .py_2()
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_col()
                        .items_end()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(px(13.))
                        .line_height(px(20.))
                        .text_color(cx.theme().muted_foreground)
                        .children(lines.iter().map(|(number, _)| {
                            div().h(px(20.)).text_right().child(number.to_string())
                        })),
                )
                .child(
                    TextView::markdown("committed-source-code", fenced_source(path, &source))
                        .min_w_0()
                        .flex_1()
                        .code_block_actions(|_, _, _| div())
                        .style(TextViewStyle::default().code_block(code_style)),
                ),
        )
}

#[derive(Default)]
pub(super) struct State {
    pub review: Option<Arc<semantic::Review>>,
    pub selected: usize,
    pub inspected: HashSet<usize>,
    pub sources: HashMap<String, semantic::SourceFile>,
    pub expanded_imports: HashSet<String>,
    pub patch: Arc<str>,
    pub error: Option<String>,
    pub job: Option<Job>,
    progress: ProgressState,
    pub show_basis: bool,
    pub show_trail: bool,
    pub undo: Option<Undo>,
    scroll: ScrollHandle,
}
pub(super) struct Job {
    cancel: acp::Cancellation,
    _task: Task<()>,
}
enum AnalysisEvent {
    Progress(semantic::Progress),
    Complete(anyhow::Result<semantic::Review>),
}
#[derive(Default)]
struct ProgressState {
    updates: Vec<String>,
    source: Option<(String, usize)>,
    expanded: bool,
}
impl ProgressState {
    fn update(&mut self, event: semantic::Progress) {
        match event {
            semantic::Progress::Status(message) => {
                if self.updates.last() != Some(&message) {
                    if self.updates.len() == 120 {
                        self.updates.remove(0);
                    }
                    self.updates.push(message);
                }
            }
            semantic::Progress::Source { target, requests } => {
                self.source = Some((target, requests))
            }
        }
    }
    fn message(&self) -> &str {
        self.updates
            .last()
            .map_or("Preparing committed source…", String::as_str)
    }
}
pub(super) enum Undo {
    Decision(usize),
    File {
        comparison: (PathBuf, String, String),
        path: String,
    },
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn score(value: Option<u8>) -> String {
    value
        .map(|n| format!("{n}/100"))
        .unwrap_or_else(|| "unknown".into())
}

impl ReviewApp {
    pub(super) fn sync_semantic_selection(&mut self, range: &DiffLineRange) {
        let Some(review) = &self.semantic.review else {
            return;
        };
        let matches = |d: &semantic::Decision| {
            d.evidence.iter().any(|e| {
                let same_side = matches!(
                    (&e.side, range.side()),
                    (
                        semantic::Side::Original,
                        gpui_kit::component::diff::DiffSide::Original
                    ) | (
                        semantic::Side::Modified,
                        gpui_kit::component::diff::DiffSide::Modified
                    )
                );
                e.path == range.path().as_str()
                    && same_side
                    && range.start() >= e.start_line
                    && range.start() <= e.end_line
            })
        };
        if review
            .decisions
            .get(self.semantic.selected)
            .is_some_and(matches)
        {
            return;
        }
        if let Some(index) = review.decisions.iter().position(matches) {
            self.semantic.selected = index;
            self.semantic.scroll.set_offset(point(px(0.), px(0.)));
        }
    }
    pub(super) fn analyze_semantics(&mut self, cx: &mut Context<Self>) {
        if self.semantic.job.is_some() || self.files.is_empty() {
            return;
        }
        let (Some(key), Some(repo)) = (self.loaded.clone(), self.repo.clone()) else {
            return;
        };
        let patch = self.semantic.patch.clone();
        self.semantic.error = None;
        self.semantic.progress = ProgressState::default();
        let cancel = acp::Cancellation::default();
        let token = cancel.clone();
        let task = cx.spawn(async move |this, cx| {
            let expected = key.clone();
            let (_, base, tip) = key;
            let (tx, rx) = async_channel::bounded(32);
            let _work = cx.background_spawn(async move {
                let progress = tx.clone();
                let result = semantic::analyze(&repo, &base, &tip, &patch, token, move |event| {
                    match &event {
                        semantic::Progress::Status(message) => {
                            eprintln!("Semantic review: {message}")
                        }
                        semantic::Progress::Source { target, requests } => {
                            eprintln!("Semantic review: source request {requests}: {target}")
                        }
                    }
                    if matches!(event, semantic::Progress::Status(_)) {
                        let _ = progress.send_blocking(AnalysisEvent::Progress(event));
                    } else {
                        let _ = progress.try_send(AnalysisEvent::Progress(event));
                    }
                });
                if let Err(error) = &result {
                    eprintln!("Semantic review failed: {error:#}");
                }
                let _ = tx.send_blocking(AnalysisEvent::Complete(result));
            });
            while let Ok(event) = rx.recv().await {
                let complete = matches!(event, AnalysisEvent::Complete(_));
                if this
                    .update(cx, |view, cx| {
                        if view.loaded.as_ref() != Some(&expected) {
                            return;
                        }
                        match event {
                            AnalysisEvent::Progress(event) => view.semantic.progress.update(event),
                            AnalysisEvent::Complete(result) => {
                                view.semantic.job = None;
                                match result {
                                    Ok(review) => {
                                        view.semantic.progress.update(semantic::Progress::Status(
                                            if review.omitted_decisions > 0 {
                                                "Partial review ready; some choices need more evidence."
                                            } else {
                                                "Review ready."
                                            }.into(),
                                        ));
                                        view.semantic.review = Some(Arc::new(review));
                                        view.semantic.selected = 0;
                                        view.semantic.inspected.clear();
                                        view.semantic.undo = None;
                                    }
                                    Err(error) => {
                                        view.semantic.progress.update(semantic::Progress::Status(
                                            "Review stopped with an error.".into(),
                                        ));
                                        view.semantic.error = Some(format!("{error:#}"));
                                    }
                                }
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                    || complete
                {
                    break;
                }
            }
        });
        self.semantic.job = Some(Job {
            cancel,
            _task: task,
        });
        cx.notify();
    }

    pub(super) fn toggle_semantic(&mut self, cx: &mut Context<Self>) {
        self.show_semantic = !self.show_semantic;
        self.reset_panel_layout(cx);
        if !self.show_semantic {
            self.semantic.job = None;
        }
        cx.notify();
    }

    pub(super) fn select_decision(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self
            .semantic
            .review
            .as_ref()
            .is_some_and(|r| index < r.decisions.len())
        {
            return;
        }
        self.semantic.selected = index;
        self.semantic.scroll.set_offset(point(px(0.), px(0.)));
        let evidence = self
            .semantic
            .review
            .as_ref()
            .and_then(|r| r.decisions.get(index))
            .and_then(|d| {
                d.evidence.iter().find(|e| {
                    self.semantic
                        .sources
                        .get(&e.path)
                        .is_some_and(|f| f.contains(e))
                })
            })
            .cloned();
        if let Some(evidence) = evidence {
            self.reveal_semantic_evidence(&evidence, cx);
        }
        cx.notify();
    }

    pub(super) fn step_review(&mut self, forward: bool, unviewed: bool, cx: &mut Context<Self>) {
        if self.show_semantic
            && let Some(review) = &self.semantic.review
        {
            let index = self.semantic.selected;
            let target = if forward {
                (index + 1..review.decisions.len())
                    .find(|i| !unviewed || !self.semantic.inspected.contains(i))
            } else {
                (0..index)
                    .rev()
                    .find(|i| !unviewed || !self.semantic.inspected.contains(i))
            };
            if let Some(target) = target {
                self.select_decision(target, cx);
            }
            return;
        }
        let current = self.current_visit(cx).map(|v| v.file);
        let files: Vec<_> = self
            .files
            .iter()
            .filter(|f| file_groups::matches_filter(f, &self.query) && !self.file_hidden(f))
            .map(|f| f.path().to_string())
            .collect();
        if let Some(index) = navigation::next_index(&files, current.as_deref(), forward, |path| {
            !unviewed || !self.viewed_files.contains(path)
        }) {
            self.visit_file(&files[index], cx);
        }
    }

    pub(super) fn inspect_and_advance(&mut self, cx: &mut Context<Self>) {
        if self.show_semantic && self.semantic.review.is_some() {
            if self.semantic.inspected.insert(self.semantic.selected) {
                self.semantic.undo = Some(Undo::Decision(self.semantic.selected));
            }
            self.step_review(true, false, cx);
            cx.notify();
            return;
        }
        if let Some(visit) = self.current_visit(cx) {
            if self.viewed_files.contains(&visit.file) {
                self.step_review(true, false, cx);
                return;
            }
            if self.pending_views.contains(&visit.file) {
                return;
            }
            let file = self
                .files
                .iter()
                .find(|f| f.path().as_str() == visit.file)
                .cloned();
            if let Some(file) = file {
                self.save_viewed(vec![file], true, true, cx);
            }
        }
    }

    pub(super) fn undo_inspection(&mut self, cx: &mut Context<Self>) {
        match self.semantic.undo.take() {
            Some(Undo::Decision(index)) => {
                self.semantic.inspected.remove(&index);
                self.select_decision(index, cx);
            }
            Some(Undo::File { comparison, path }) if self.loaded.as_ref() == Some(&comparison) => {
                if let Some(file) = self
                    .files
                    .iter()
                    .find(|f| f.path().as_str() == path)
                    .cloned()
                {
                    self.save_viewed(vec![file], false, false, cx);
                    self.visit_file(&path, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn reveal_semantic_evidence(
        &mut self,
        e: &semantic::Evidence,
        cx: &mut Context<Self>,
    ) {
        if !self
            .semantic
            .sources
            .get(&e.path)
            .is_some_and(|f| f.contains(e))
        {
            return;
        }
        if self.semantic.expanded_imports.insert(e.path.clone()) {
            self.sync_diff_files(cx);
        }
        self.visit_file(&e.path, cx);
        let side = match e.side {
            semantic::Side::Original => gpui_kit::component::diff::DiffSide::Original,
            semantic::Side::Modified => gpui_kit::component::diff::DiffSide::Modified,
        };
        self.diff.update(cx, |diff, cx| {
            diff.set_selected_lines(
                Some(DiffLineRange::new(
                    e.path.clone(),
                    side,
                    e.start_line,
                    e.end_line,
                )),
                cx,
            );
            diff.scroll_to_line(
                DiffLinePosition::new(e.path.clone(), side, e.start_line),
                cx,
            );
        });
    }

    fn open_semantic_evidence(&mut self, e: &semantic::Evidence, cx: &mut Context<Self>) {
        if self
            .semantic
            .sources
            .get(&e.path)
            .is_some_and(|f| f.contains(e))
        {
            self.reveal_semantic_evidence(e, cx);
            return;
        }
        let (Some(expected), Some(repo)) = (self.loaded.clone(), self.repo.clone()) else {
            return;
        };
        let patch = self.semantic.patch.clone();
        let evidence = e.clone();
        cx.spawn(async move |this, cx| {
            let (_, base, tip) = expected.clone();
            let path = evidence.path.clone();
            let title = format!("{} · {:?} · {}–{}", evidence.path, evidence.side, evidence.start_line, evidence.end_line);
            let result = cx.background_spawn(async move {
                let mut source = crate::semantic_source::Snapshot::new(&repo.root,&base,&tip,&patch,acp::Cancellation::default())?;
                source.call("gander_read_source", &serde_json::json!({"path":evidence.path,"side":evidence.side,"start_line":evidence.start_line,"line_count":(evidence.end_line-evidence.start_line+1).min(200)}))
            }).await;
            let _ = this.update_in(cx, |view, window, cx| {
                if view.loaded.as_ref() != Some(&expected) || window.has_active_dialog(cx) {
                    return;
                }
                window.open_dialog(cx, move |dialog, _, _| {
                    let result = result.as_ref();
                    let path = path.clone();
                    let (revision, lines, note) = match result {
                        Ok(page) => {
                            let lines: Vec<_> = page["lines"].as_array().into_iter().flatten()
                                .filter_map(|line| Some((line["line"].as_u64()?, line["text"].as_str()
                                    .or_else(|| line["omitted"].as_str()).unwrap_or("").to_owned())))
                                .collect();
                            (page["revision"].as_str().unwrap_or("").to_owned(), lines,
                                (!page["next_line"].is_null()).then(|| "Additional source is outside this preview.".to_owned()))
                        }
                        Err(error) => (String::new(), Vec::new(), Some(format!("Couldn't read committed source: {error:#}"))),
                    };
                    dialog.title(title.clone()).close_button(true).max_w(px(920.)).content(move |content, _, cx| {
                        content
                            .when(!revision.is_empty(), |content| content.child(div().text_color(cx.theme().muted_foreground).child(format!("Committed source · {revision}"))))
                            .when(!lines.is_empty(), |content| content.child(source_preview(&path, &lines, cx)))
                            .children(note.as_ref().map(|note| div().text_xs().text_color(cx.theme().muted_foreground).whitespace_normal().child(note.clone())))
                    })
                });
            });
        }).detach();
    }

    pub(super) fn toggle_imports(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.semantic.expanded_imports.remove(path) {
            self.semantic.expanded_imports.insert(path.to_owned());
        }
        self.sync_diff_files(cx);
        self.reveal_file(path, cx);
    }

    pub(super) fn import_button(&self, path: &str, cx: &mut Context<Self>) -> Button {
        let count = self
            .semantic
            .sources
            .get(path)
            .map_or(0, |s| s.folded_lines);
        let expanded = self.semantic.expanded_imports.contains(path);
        let path = path.to_owned();
        Button::new(SharedString::from(format!("imports:{path}")))
            .ghost()
            .xsmall()
            .label(if count == 0 {
                String::new()
            } else if expanded {
                format!("Fold {count} import lines")
            } else {
                format!("{count} import lines folded · reveal")
            })
            .disabled(count == 0)
            .tooltip(
                "Import-only hunks. Expand to inspect dependencies and initialization effects.",
            )
            .on_click(cx.listener(move |view, _, _, cx| view.toggle_imports(&path, cx)))
    }

    fn evidence_links(&self, evidence: &[semantic::Evidence], cx: &mut Context<Self>) -> Div {
        let mut row = div().flex().flex_wrap().gap_1().min_w_0();
        for (i, e) in evidence.iter().enumerate() {
            let e = e.clone();
            let label = format!(
                "{}:{} · {}",
                e.path.rsplit('/').next().unwrap_or(&e.path),
                e.start_line,
                match e.side {
                    semantic::Side::Original => "before",
                    semantic::Side::Modified => "after",
                }
            );
            row = row.child(
                Button::new(SharedString::from(format!(
                    "source:{i}:{}:{}",
                    e.path, e.start_line
                )))
                .ghost()
                .xsmall()
                .label(label)
                .tooltip(format!(
                    "{} · {:?} · {}–{}",
                    e.path, e.side, e.start_line, e.end_line
                ))
                .on_click(cx.listener(move |view, _, _, cx| view.open_semantic_evidence(&e, cx))),
            );
        }
        row
    }

    pub(super) fn semantic_panel(&self, cx: &mut Context<Self>) -> Div {
        let busy = self.semantic.job.is_some();
        let mut panel = div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Semantic review"),
                    )
                    .child(
                        Button::new("analyze-semantic")
                            .ghost()
                            .small()
                            .disabled(self.loaded.is_none() || self.files.is_empty())
                            .label(if busy {
                                "Cancel"
                            } else if self.semantic.review.is_some() {
                                "Reload"
                            } else {
                                "Analyze"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                if view.semantic.job.is_some() {
                                    view.semantic.job = None;
                                    view.semantic.progress.update(semantic::Progress::Status(
                                        "Analysis cancelled.".into(),
                                    ));
                                    cx.notify();
                                } else {
                                    view.analyze_semantics(cx);
                                }
                            })),
                    ),
            );
        let mut content = div()
            .id("semantic-content")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(&self.semantic.scroll)
            .p_4()
            .flex()
            .flex_col()
            .gap_4();
        if busy || (self.semantic.review.is_none() && !self.semantic.progress.updates.is_empty()) {
            let message = self.semantic.progress.message().to_owned();
            content = content.child(
                div()
                    .id("semantic-progress")
                    .role(gpui_kit::accesskit::Role::Status)
                    .aria_label(message.clone())
                    .text_color(cx.theme().muted_foreground)
                    .whitespace_normal()
                    .child(markdown("semantic-progress-body", message)),
            );
            if busy && let Some((target, requests)) = &self.semantic.progress.source {
                content = content.child(
                    div()
                        .id("semantic-source-activity")
                        .role(gpui_kit::accesskit::Role::Label)
                        .aria_label(format!("{requests} source reads · latest: {target}"))
                        .text_xs()
                        .whitespace_normal()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{requests} source reads · latest: {target}")),
                );
            }
        }
        let earlier = self.semantic.progress.updates.len().saturating_sub(1);
        if earlier > 0 {
            let expanded = self.semantic.progress.expanded;
            content = content.child(
                Button::new("semantic-progress-trail")
                    .ghost()
                    .xsmall()
                    .label(format!(
                        "{} earlier updates ({earlier})",
                        if expanded { "Hide" } else { "Show" }
                    ))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.semantic.progress.expanded = !view.semantic.progress.expanded;
                        cx.notify();
                    })),
            );
            if expanded {
                content = content.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .pl_3()
                        .children(
                            self.semantic
                                .progress
                                .updates
                                .iter()
                                .take(earlier)
                                .enumerate()
                                .map(|(i, message)| {
                                    div()
                                        .id(("progress-update", i))
                                        .role(gpui_kit::accesskit::Role::Label)
                                        .aria_label(message.clone())
                                        .text_xs()
                                        .whitespace_normal()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(markdown(
                                            ("progress-update-body", i),
                                            message.clone(),
                                        ))
                                }),
                        ),
                );
            }
        }
        if let Some(error) = &self.semantic.error {
            content = content.child(
                div()
                    .id("semantic-error")
                    .role(gpui_kit::accesskit::Role::Alert)
                    .aria_label(error.clone())
                    .whitespace_normal()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        if let Some(review) = &self.semantic.review {
            if review.omitted_decisions > 0 {
                let notice = format!(
                    "Partial review · {} choices omitted because their source evidence couldn't be verified. Uncovered files are listed below. Analyze to retry.",
                    review.omitted_decisions
                );
                content = content.child(
                    div()
                        .id("semantic-partial-review")
                        .role(gpui_kit::accesskit::Role::Status)
                        .aria_label(notice.clone())
                        .whitespace_normal()
                        .text_color(cx.theme().muted_foreground)
                        .child(notice),
                );
            }
            let coverage = &review.coverage;
            if coverage.total_lines > 0 {
                let scope = format!(
                    "Evidence scope · {} / {} changed lines read{}",
                    coverage.read_lines,
                    coverage.total_lines,
                    if coverage.partial_files > 0 {
                        format!(" · {} files partially read", coverage.partial_files)
                    } else {
                        String::new()
                    }
                );
                content = content.child(
                    div()
                        .id("semantic-evidence-scope")
                        .role(gpui_kit::accesskit::Role::Label)
                        .aria_label(scope.clone())
                        .text_xs()
                        .whitespace_normal()
                        .text_color(cx.theme().muted_foreground)
                        .child(scope),
                );
            }
            let count = review.decisions.len();
            if let Some(d) = review.decisions.get(self.semantic.selected) {
                content = content
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("previous-decision")
                                    .ghost()
                                    .small()
                                    .label("←")
                                    .tooltip("Previous item · ⌘⇧[")
                                    .disabled(self.semantic.selected == 0)
                                    .on_click(
                                        cx.listener(|v, _, _, cx| v.step_review(false, false, cx)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} / {count} choices · {} inspected",
                                        self.semantic.selected + 1,
                                        self.semantic.inspected.len()
                                    )),
                            )
                            .child(
                                Button::new("next-decision")
                                    .ghost()
                                    .small()
                                    .label("→")
                                    .tooltip("Next item · ⌘⇧]")
                                    .disabled(self.semantic.selected + 1 >= count)
                                    .on_click(
                                        cx.listener(|v, _, _, cx| v.step_review(true, false, cx)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .whitespace_normal()
                            .child(markdown("semantic-title", d.title.clone())),
                    );
                let why_label = match d.why_origin {
                    semantic::RationaleOrigin::Inferred => "Why · inferred",
                    semantic::RationaleOrigin::Unknown => "Why · unknown",
                };
                for (i, (label, claim)) in [("What", &d.what), ("How", &d.how), (why_label, &d.why)]
                    .into_iter()
                    .enumerate()
                {
                    let mut section = div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .gap_2()
                                .child(div().font_weight(FontWeight::MEDIUM).child(label))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(score(claim.confidence)),
                                ),
                        )
                        .child(markdown(("semantic-claim", i), claim.text.clone()));
                    if self.semantic.show_basis {
                        section = section.child(
                            div()
                                .text_xs()
                                .whitespace_normal()
                                .text_color(cx.theme().muted_foreground)
                                .child(markdown(("semantic-claim-basis", i), claim.basis.clone())),
                        );
                    }
                    content = content.child(section);
                }
                content = content.child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Before → after"),
                );
                for (i, case) in d.cases.iter().enumerate() {
                    let mut row = div()
                        .id(("semantic-case", i))
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .pb_3()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .whitespace_normal()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(markdown(
                                            ("semantic-situation", i),
                                            case.situation.clone(),
                                        )),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(score(case.confidence)),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_3()
                                .min_w_0()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("Before"),
                                        )
                                        .child(markdown(
                                            ("semantic-before", i),
                                            case.before.clone(),
                                        )),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child("After"),
                                        )
                                        .child(markdown(("semantic-after", i), case.after.clone())),
                                ),
                        );
                    if self.semantic.show_basis {
                        row = row.child(
                            div()
                                .text_xs()
                                .whitespace_normal()
                                .text_color(cx.theme().muted_foreground)
                                .child(markdown(("semantic-case-basis", i), case.basis.clone())),
                        );
                    }
                    content = content.child(row);
                }
                content = content.child(self.evidence_links(&d.evidence, cx)).child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(
                            Button::new("semantic-confidence")
                                .ghost()
                                .xsmall()
                                .selected(self.semantic.show_basis)
                                .label("Confidence details")
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.semantic.show_basis = !v.semantic.show_basis;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("semantic-trail")
                                .ghost()
                                .xsmall()
                                .selected(self.semantic.show_trail)
                                .label(format!("Behavior trail ({})", d.trail.len()))
                                .disabled(d.trail.is_empty())
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.semantic.show_trail = !v.semantic.show_trail;
                                    cx.notify();
                                })),
                        ),
                );
                if self.semantic.show_basis {
                    content=content.child(div().text_xs().whitespace_normal().text_color(cx.theme().muted_foreground)
                    .child("Model estimates of how well each description matches this revision. Unknown means insufficient evidence. Tests have not been run by this analysis."));
                }
                if self.semantic.show_trail {
                    for (i, step) in d.trail.iter().enumerate() {
                        let mut row = div()
                            .id(("semantic-step", i))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .min_w_0()
                            .pl_3()
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{} · {}", i + 1, score(step.confidence))),
                            )
                            .child(markdown(
                                ("semantic-step-body", i),
                                step.description.clone(),
                            ))
                            .child(self.evidence_links(&step.evidence, cx));
                        if self.semantic.show_basis {
                            row = row.child(
                                div()
                                    .text_xs()
                                    .whitespace_normal()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(markdown(
                                        ("semantic-step-basis", i),
                                        step.basis.clone(),
                                    )),
                            );
                        }
                        content = content.child(row);
                    }
                }
                content = content.child(
                    Button::new("inspect-next")
                        .small()
                        .label(
                            if self.semantic.inspected.contains(&self.semantic.selected) {
                                "Inspected · next ⌘⇧↵"
                            } else {
                                "Inspect & next ⌘⇧↵"
                            },
                        )
                        .on_click(cx.listener(|v, _, _, cx| v.inspect_and_advance(cx))),
                );
                if self.semantic.undo.is_some() {
                    content = content.child(
                        Button::new("undo-inspection")
                            .ghost()
                            .xsmall()
                            .label("Undo inspection · ⌘⇧Z")
                            .on_click(cx.listener(|v, _, _, cx| v.undo_inspection(cx))),
                    );
                }
            } else {
                content = content.child("No decisions inferred for this comparison.");
            }
            if !review.unexplained_files.is_empty() {
                content = content.child(div().font_weight(FontWeight::MEDIUM).child(format!(
                    "{} files need a closer look",
                    review.unexplained_files.len()
                )));
                for path in &review.unexplained_files {
                    let path = path.clone();
                    content = content.child(
                        Button::new(SharedString::from(format!("unexplained:{path}")))
                            .ghost()
                            .small()
                            .label(path.clone())
                            .on_click(cx.listener(move |v, _, _, cx| v.visit_file(&path, cx))),
                    );
                }
            }
            content=content.child(div().text_xs().whitespace_normal().text_color(cx.theme().muted_foreground)
                .child("Descriptions are inferred from this revision. Unexplained changes still need review."));
        } else if !busy && self.semantic.error.is_none() {
            content=content.child(div().whitespace_normal().text_color(cx.theme().muted_foreground)
                .child("Choose Analyze to review the decisions, behavior, and design in this comparison. Analysis starts only when requested and uses a transient Codex session."));
        }
        panel = panel.child(content);
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::{ProgressState, fenced_source};
    use crate::semantic;
    use core::prelude::v1::test;

    #[test]
    fn committed_source_cannot_close_its_own_code_fence() {
        let source = "const example = `value`;\n```\n# still source\n````";
        let rendered = fenced_source("preview.ts", source);
        assert_eq!(rendered, format!("`````ts\n{source}\n`````"));
        assert_eq!(
            fenced_source("file.unknown\nlanguage", "x"),
            "```text\nx\n```"
        );
    }
    #[test]
    fn source_activity_preserves_the_latest_status_and_earlier_updates() {
        let mut progress = ProgressState::default();
        progress.update(semantic::Progress::Status("Starting review…".into()));
        progress.update(semantic::Progress::Status(
            "Checking failure recovery.".into(),
        ));
        progress.update(semantic::Progress::Source {
            target: "save.ts".into(),
            requests: 3,
        });
        assert_eq!(progress.message(), "Checking failure recovery.");
        assert_eq!(progress.source, Some(("save.ts".into(), 3)));
        assert_eq!(
            progress.updates,
            ["Starting review…", "Checking failure recovery."]
        );
        progress.update(semantic::Progress::Status("Review ready.".into()));
        assert_eq!(progress.updates[1], "Checking failure recovery.");
        assert_eq!(progress.message(), "Review ready.");
        assert!(!progress.expanded);
    }
}
