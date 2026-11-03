use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub stack: String,
    pub anchor: String,
    pub end: String,
}
impl Selection {
    pub fn key(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Comparison {
    pub known_files: HashSet<String>,
    pub collapsed_files: HashSet<String>,
    pub viewed_files: HashSet<String>,
    pub focused_file: Option<String>,
    pub file_scroll: [f32; 2],
}
impl Comparison {
    pub fn collapsed_for<'a>(
        &self,
        paths: impl IntoIterator<Item = &'a str>,
        viewed: &HashSet<String>,
    ) -> HashSet<String> {
        paths
            .into_iter()
            .filter(|path| {
                if self.viewed_files.contains(*path) != viewed.contains(*path) {
                    viewed.contains(*path)
                } else if self.known_files.contains(*path) {
                    self.collapsed_files.contains(*path)
                } else {
                    viewed.contains(*path)
                }
            })
            .map(str::to_owned)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoState {
    pub selection: Option<Selection>,
    pub base_override: Option<String>,
    pub wrap: bool,
    pub split: bool,
    pub show_comments: bool,
    pub show_semantic: bool,
    pub show_branches: bool,
    pub sidebars_swapped: bool,
    pub branch_width: f32,
    pub file_width: f32,
    pub comment_width: f32,
    pub filter: String,
    pub collapsed_groups: HashSet<String>,
    pub comparisons: BTreeMap<String, Comparison>,
}
impl Default for RepoState {
    fn default() -> Self {
        Self {
            selection: None,
            base_override: None,
            wrap: false,
            split: false,
            show_comments: false,
            show_semantic: true,
            show_branches: true,
            sidebars_swapped: false,
            branch_width: 300.,
            file_width: 340.,
            comment_width: 360.,
            filter: String::new(),
            collapsed_groups: HashSet::new(),
            comparisons: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub version: u32,
    pub last_repo: Option<PathBuf>,
    pub repositories: BTreeMap<String, RepoState>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            version: 1,
            last_repo: None,
            repositories: BTreeMap::new(),
        }
    }
}
impl Session {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        let session: Self =
            serde_json::from_slice(&bytes).context("Couldn't read saved UI state")?;
        if session.version != 1 {
            bail!("Unsupported UI state version {}", session.version);
        }
        Ok(session)
    }
}

pub fn state_path() -> Option<PathBuf> {
    if let Some(dir) =
        std::env::var_os("GANDER_STATE_DIR").or_else(|| std::env::var_os("BUTLER_DIFF_STATE_DIR"))
    {
        return Some(PathBuf::from(dir).join("session.json"));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let development = crate::launch::is_development();
    #[cfg(target_os = "macos")]
    let parent = home.join("Library/Application Support");
    #[cfg(not(target_os = "macos"))]
    let parent = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"));
    Some(default_state_path(&parent, development))
}

fn default_state_path(parent: &Path, development: bool) -> PathBuf {
    #[cfg(target_os = "macos")]
    let (current, legacy) = if development {
        ("Gander Dev", "Butler Diff Dev")
    } else {
        ("Gander", "Butler Diff")
    };
    #[cfg(not(target_os = "macos"))]
    let (current, legacy) = if development {
        ("gander-dev", "butler-diff-dev")
    } else {
        ("gander", "butler-diff")
    };
    let current = parent.join(current).join("session.json");
    let legacy = parent.join(legacy).join("session.json");
    // Existing installations keep their saved state through the Gander rename.
    if !current.exists() && legacy.is_file() {
        legacy
    } else {
        current
    }
}

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
    sequence: Arc<AtomicU64>,
    lock: Arc<Mutex<()>>,
}
impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            sequence: Arc::default(),
            lock: Arc::default(),
        }
    }
    pub fn next(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst) + 1
    }
    pub fn save(&self, session: &Session, sequence: u64) -> Result<()> {
        let _lock = self.lock.lock().unwrap();
        if sequence != self.sequence.load(Ordering::SeqCst) {
            return Ok(());
        }
        let parent = self.path.parent().context("Missing UI state directory")?;
        std::fs::create_dir_all(parent)?;
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec(session)?)?;
        std::fs::rename(temporary, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renamed_app_preserves_legacy_state_and_prefers_its_own_state() {
        let parent = std::env::temp_dir().join(format!("gander-state-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        for development in [false, true] {
            let current = default_state_path(&parent, development);
            #[cfg(target_os = "macos")]
            let legacy = parent
                .join(if development {
                    "Butler Diff Dev"
                } else {
                    "Butler Diff"
                })
                .join("session.json");
            #[cfg(not(target_os = "macos"))]
            let legacy = parent
                .join(if development {
                    "butler-diff-dev"
                } else {
                    "butler-diff"
                })
                .join("session.json");
            assert_ne!(current, legacy);
            std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
            std::fs::write(&legacy, serde_json::to_vec(&Session::default()).unwrap()).unwrap();
            assert_eq!(default_state_path(&parent, development), legacy);
            std::fs::create_dir_all(current.parent().unwrap()).unwrap();
            std::fs::write(&current, serde_json::to_vec(&Session::default()).unwrap()).unwrap();
            assert_eq!(default_state_path(&parent, development), current);
        }
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn repo_and_comparison_state_roundtrip_and_stale_writes_are_ignored() {
        let path = std::env::temp_dir()
            .join(format!("gander-session-{}", std::process::id()))
            .join("session.json");
        let store = Store::new(path.clone());
        let mut session = Session::default();
        let selection = Selection {
            stack: "stack".into(),
            anchor: "lower".into(),
            end: "upper".into(),
        };
        let comparison = Comparison {
            known_files: HashSet::from(["a.rs".into()]),
            collapsed_files: HashSet::from(["a.rs".into()]),
            viewed_files: HashSet::new(),
            focused_file: Some("a.rs".into()),
            file_scroll: [0., -200.],
        };
        session.last_repo = Some("/repo".into());
        session.repositories.insert(
            "/repo".into(),
            RepoState {
                selection: Some(selection.clone()),
                base_override: Some("refs/heads/main".into()),
                wrap: true,
                split: true,
                show_branches: false,
                sidebars_swapped: true,
                file_width: 280.,
                comment_width: 440.,
                filter: "test".into(),
                comparisons: BTreeMap::from([(selection.key(), comparison)]),
                ..Default::default()
            },
        );
        session
            .repositories
            .insert("/other".into(), RepoState::default());
        let stale = store.next();
        let latest = store.next();
        store.save(&session, latest).unwrap();
        store.save(&Session::default(), stale).unwrap();
        assert_eq!(Session::load(&path).unwrap(), session);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn manual_expansion_survives_but_new_viewed_files_start_collapsed() {
        let saved = Comparison {
            known_files: HashSet::from(["expanded".into(), "collapsed".into(), "removed".into()]),
            collapsed_files: HashSet::from(["collapsed".into(), "removed".into()]),
            viewed_files: HashSet::from(["expanded".into()]),
            ..Default::default()
        };
        let viewed = HashSet::from(["expanded".into(), "new".into()]);
        assert_eq!(
            saved.collapsed_for(["expanded", "collapsed", "new", "unviewed"], &viewed),
            HashSet::from(["collapsed".into(), "new".into()])
        );
    }
    #[test]
    fn reset_viewed_marks_expand_changed_files_and_new_marks_collapse_them() {
        let saved = Comparison {
            known_files: HashSet::from(["changed".into(), "newly-marked".into()]),
            collapsed_files: HashSet::from(["changed".into()]),
            viewed_files: HashSet::from(["changed".into()]),
            ..Default::default()
        };
        assert_eq!(
            saved.collapsed_for(
                ["changed", "newly-marked"],
                &HashSet::from(["newly-marked".into()])
            ),
            HashSet::from(["newly-marked".into()])
        );
    }
    #[test]
    fn missing_state_defaults_but_malformed_and_future_state_are_rejected() {
        let path = std::env::temp_dir().join(format!(
            "gander-session-invalid-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        assert_eq!(Session::load(&path).unwrap(), Session::default());
        std::fs::write(&path, "{").unwrap();
        assert!(Session::load(&path).is_err());
        std::fs::write(&path, r#"{"version":99}"#).unwrap();
        assert!(Session::load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
