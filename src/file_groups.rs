use anyhow::{Context, Result, bail};
use gpui_kit::component::diff::DiffFile;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

// Keep the on-disk identity stable so Gander preserves existing file groups.
pub const CONFIG_FILE: &str = "butler-diff.toml";
pub const TEST_PATTERN: &str = r"\.test\.tsx$|(^|/)__tests__/|(^|/)tests/.*\.py$";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GroupDefinition {
    pub name: String,
    pub pattern: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GroupConfig {
    pub enabled: bool,
    pub groups: Vec<GroupDefinition>,
}

impl Default for GroupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            groups: vec![],
        }
    }
}

pub struct CompiledGroup {
    pub name: String,
    regex: Regex,
}

impl GroupConfig {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).context("Couldn't parse file groups"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).context("Couldn't read file groups"),
        }
    }

    pub fn compile(&self) -> Result<Vec<CompiledGroup>> {
        let mut names = HashSet::new();
        self.groups
            .iter()
            .map(|group| {
                if group.name.trim().is_empty() {
                    bail!("Every group needs a name.");
                }
                if !names.insert(group.name.trim().to_lowercase()) {
                    bail!("Group names must be unique: {}", group.name);
                }
                if group.pattern.is_empty() {
                    bail!("{} needs a regex. Use .* to match every path.", group.name);
                }
                let regex = Regex::new(&group.pattern)
                    .with_context(|| format!("Invalid regex for {}", group.name))?;
                Ok(CompiledGroup {
                    name: group.name.clone(),
                    regex,
                })
            })
            .collect()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.compile()?;
        let text = toml::to_string_pretty(self)?;
        // Atomic replacement keeps the watcher from observing a partial config.
        let temporary = path.with_file_name(format!(
            ".gander-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let result = (|| -> Result<()> {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.with_context(|| format!("Couldn't save {}", path.display()))
    }
}

// First matching definition wins, so overlapping regexes never double-count LOC.
pub fn group_key(path: &str, groups: &[CompiledGroup]) -> String {
    groups
        .iter()
        .find(|group| group.regex.is_match(path))
        .map(|group| format!("group:{}", group.name))
        .unwrap_or_else(|| "ungrouped".into())
}

pub fn matches_filter(file: &DiffFile, query: &str) -> bool {
    query.is_empty() || file.path().to_lowercase().contains(query)
}

pub fn hidden(file: &DiffFile, groups: &[CompiledGroup], collapsed: &HashSet<String>) -> bool {
    collapsed.contains(&group_key(file.path(), groups))
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileTotals {
    pub files: usize,
    pub additions: usize,
    pub deletions: usize,
}

impl FileTotals {
    pub fn from_files<'a>(files: impl IntoIterator<Item = &'a DiffFile>) -> Self {
        files.into_iter().fold(Self::default(), |mut total, file| {
            total.files += 1;
            total.additions += file.additions();
            total.deletions += file.deletions();
            total
        })
    }

    pub fn label(self) -> String {
        format!(
            "{} files  +{}  −{}",
            self.files, self.additions, self.deletions
        )
    }
}

pub struct FileBucket<'a> {
    pub key: String,
    pub name: String,
    pub files: Vec<&'a DiffFile>,
}

pub fn buckets<'a>(
    files: &'a [DiffFile],
    query: &str,
    groups: &[CompiledGroup],
) -> Vec<FileBucket<'a>> {
    let mut buckets: Vec<_> = groups
        .iter()
        .map(|group| FileBucket {
            key: format!("group:{}", group.name),
            name: group.name.clone(),
            files: vec![],
        })
        .chain(std::iter::once(FileBucket {
            key: "ungrouped".into(),
            name: "Ungrouped".into(),
            files: vec![],
        }))
        .collect();
    for file in files.iter().filter(|file| matches_filter(file, query)) {
        let index = groups
            .iter()
            .position(|group| group.regex.is_match(file.path()))
            .unwrap_or(groups.len());
        buckets[index].files.push(file);
    }
    buckets.retain(|bucket| !bucket.files.is_empty());
    buckets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<DiffFile> {
        DiffFile::parse("diff --git a/src/widget.test.tsx b/src/widget.test.tsx\n--- a/src/widget.test.tsx\n+++ b/src/widget.test.tsx\n@@ -1 +1,2 @@\n-old\n+new\n+extra\ndiff --git a/src/__tests__/helper.ts b/src/__tests__/helper.ts\n--- a/src/__tests__/helper.ts\n+++ b/src/__tests__/helper.ts\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/api/tests/nested/test_api.py b/api/tests/nested/test_api.py\n--- a/api/tests/nested/test_api.py\n+++ b/api/tests/nested/test_api.py\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/src/widget.tsx b/src/widget.tsx\n--- a/src/widget.tsx\n+++ b/src/widget.tsx\n@@ -1 +1 @@\n-a\n+b\n").unwrap()
    }

    fn config() -> GroupConfig {
        GroupConfig {
            enabled: true,
            groups: vec![GroupDefinition {
                name: "Tests".into(),
                pattern: TEST_PATTERN.into(),
            }],
        }
    }

    #[test]
    fn groups_paths_and_sums_real_diff_lines_without_duplicates() {
        let mut config = config();
        config.groups.push(GroupDefinition {
            name: "TypeScript".into(),
            pattern: r"\.tsx?$".into(),
        });
        let groups = config.compile().unwrap();
        let files = files();
        let buckets = buckets(&files, "", &groups);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].name, "Tests");
        assert_eq!(
            FileTotals::from_files(buckets[0].files.iter().copied()),
            FileTotals {
                files: 3,
                additions: 4,
                deletions: 3
            }
        );
        assert_eq!(buckets[1].files.len(), 1);
        assert_eq!(
            FileTotals::from_files(&files),
            FileTotals {
                files: 4,
                additions: 5,
                deletions: 4
            }
        );
    }

    #[test]
    fn filters_group_members_and_totals_and_omits_empty_headers() {
        let groups = config().compile().unwrap();
        let files = files();
        let filtered = buckets(&files, "widget", &groups);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[1].name, "Ungrouped");
        assert_eq!(
            FileTotals::from_files(files.iter().filter(|f| matches_filter(f, "widget"))),
            FileTotals {
                files: 2,
                additions: 3,
                deletions: 2
            }
        );
        assert!(buckets(&files, "no match", &groups).is_empty());
    }

    #[test]
    fn collapsed_group_hides_its_files_but_keeps_counts() {
        let groups = config().compile().unwrap();
        let files = files();
        let collapsed = HashSet::from(["group:Tests".into()]);
        assert_eq!(
            files
                .iter()
                .filter(|f| !hidden(f, &groups, &collapsed))
                .count(),
            1
        );
        assert_eq!(buckets(&files, "", &groups)[0].files.len(), 3);
        assert_eq!(files.len(), 4);
    }

    #[test]
    fn validates_patterns_and_round_trips_repo_settings() {
        let config = config();
        let text = toml::to_string_pretty(&config).unwrap();
        let loaded: GroupConfig = toml::from_str(&text).unwrap();
        assert_eq!(loaded, config);
        loaded.compile().unwrap();
        let mut invalid = config.clone();
        invalid.groups[0].pattern = "[".into();
        assert!(invalid.compile().is_err());
        invalid = config;
        invalid.groups.push(invalid.groups[0].clone());
        assert!(invalid.compile().is_err());
    }
}
