use crate::repository::Repository;
use anyhow::Result;
use async_channel::{Receiver, Sender};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

pub const QUIET_PERIOD: Duration = Duration::from_millis(600);
pub const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
pub const MAX_BURST_WAIT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, Default)]
pub struct Changes(u8);
impl Changes {
    const METADATA: Self = Self(1);
    const GIT: Self = Self(2);
    const GROUPING: Self = Self(4);
    const COMMENTS: Self = Self(8);
    pub fn merge(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn metadata_only(self) -> bool {
        self.0 == Self::METADATA.0
    }
    pub fn grouping_changed(self) -> bool {
        self.0 & Self::GROUPING.0 != 0
    }
    pub fn comments_changed(self) -> bool {
        self.0 & Self::COMMENTS.0 != 0
    }
    pub fn without_local_settings(self) -> Self {
        Self(self.0 & !(Self::GROUPING.0 | Self::COMMENTS.0))
    }
}

#[derive(Clone)]
pub struct WatchEvents {
    receiver: Receiver<()>,
    changed_at: Arc<Mutex<Instant>>,
    changes: Arc<AtomicU8>,
}
impl WatchEvents {
    pub async fn recv(&self) -> bool {
        self.receiver.recv().await.is_ok()
    }
    pub fn remaining(&self, first: Instant, previous_refresh: Option<Instant>) -> Duration {
        refresh_deadline(first, *self.changed_at.lock().unwrap(), previous_refresh)
            .saturating_duration_since(Instant::now())
    }
    pub fn take_changes(&self) -> Changes {
        while self.receiver.try_recv().is_ok() {}
        Changes(self.changes.swap(0, Ordering::AcqRel))
    }
    #[cfg(test)]
    pub fn is_pending(&self) -> bool {
        self.changes.load(Ordering::Acquire) != 0
    }
}

// A quiet-period debounce, a hard bound under continuous writes, and a throttle
// on refresh starts. Keeping the clock calculation pure makes boundaries testable.
pub fn refresh_deadline(first: Instant, last: Instant, previous: Option<Instant>) -> Instant {
    let settled = (last + QUIET_PERIOD).min(first + MAX_BURST_WAIT);
    previous.map_or(settled, |p| settled.max(p + MIN_REFRESH_INTERVAL))
}

pub struct RepoWatcher {
    _watcher: RecommendedWatcher,
    pub events: WatchEvents,
}

fn classify(path: &Path) -> Changes {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if name.ends_with(".lock") {
        return Changes::default();
    }
    if name == crate::comments::STATE_FILE || name == format!("{}-wal", crate::comments::STATE_FILE)
    {
        Changes::COMMENTS
    } else if name == crate::file_groups::CONFIG_FILE {
        Changes::GROUPING
    } else if path.components().any(|c| c.as_os_str() == "refs")
        || matches!(name, "HEAD" | "config" | "config.worktree" | "packed-refs")
    {
        Changes::GIT
    } else if matches!(
        name,
        "virtual_branches.toml" | "but.sqlite" | "but.sqlite-wal" | "REFRESH" | "INVALIDATE"
    ) {
        Changes::METADATA
    } else {
        Changes::default()
    }
}

fn record(event: Event, sender: &Sender<()>, events: &WatchEvents) {
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    let mut changes = Changes::default();
    for path in &event.paths {
        changes.merge(classify(path));
    }
    if changes.is_empty() {
        return;
    }
    *events.changed_at.lock().unwrap() = Instant::now();
    events.changes.fetch_or(changes.0, Ordering::Release);
    // One wakeup per burst; never queue unbounded events or block notify's thread.
    let _ = sender.try_send(());
}

impl RepoWatcher {
    pub fn new(repo: &Repository) -> Result<Self> {
        let (sender, receiver) = async_channel::bounded(1);
        let events = WatchEvents {
            receiver,
            changed_at: Arc::new(Mutex::new(Instant::now())),
            changes: Arc::new(AtomicU8::new(0)),
        };
        let signal = events.clone();
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            match result {
                Ok(event) => record(event, &sender, &signal),
                Err(_) => {
                    // Reconcile once after an overflow or lost event.
                    *signal.changed_at.lock().unwrap() = Instant::now();
                    signal.changes.fetch_or(Changes::GIT.0, Ordering::Release);
                    let _ = sender.try_send(());
                }
            }
        })?;
        watcher.watch(&repo.common_dir, RecursiveMode::NonRecursive)?;
        if repo.git_dir != repo.common_dir {
            watcher.watch(&repo.git_dir, RecursiveMode::NonRecursive)?;
        }
        for (path, mode) in [
            (repo.common_dir.join("refs"), RecursiveMode::Recursive),
            (
                repo.common_dir.join("gitbutler"),
                RecursiveMode::NonRecursive,
            ),
        ] {
            if path.exists() {
                watcher.watch(&path, mode)?;
            }
        }
        Ok(Self {
            _watcher: watcher,
            events,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Changes, MAX_BURST_WAIT, MIN_REFRESH_INTERVAL, QUIET_PERIOD, classify, refresh_deadline,
    };
    use std::{
        path::Path,
        time::{Duration, Instant},
    };

    #[test]
    fn burst_waits_for_the_last_write_but_cannot_starve() {
        let first = Instant::now();
        let last = first + Duration::from_millis(300);
        assert_eq!(refresh_deadline(first, last, None), last + QUIET_PERIOD);
        assert_eq!(
            refresh_deadline(first, first + Duration::from_secs(10), None),
            first + MAX_BURST_WAIT
        );
    }

    #[test]
    fn subsequent_bursts_are_throttled() {
        let previous = Instant::now();
        let first = previous + Duration::from_millis(100);
        assert_eq!(
            refresh_deadline(first, first, Some(previous)),
            previous + MIN_REFRESH_INTERVAL
        );
    }

    #[test]
    fn ignores_lock_accessory_files_and_distinguishes_database_writes() {
        assert!(classify(Path::new("/repo/.git/refs/heads/a.lock")).is_empty());
        assert!(classify(Path::new("/repo/.git/gitbutler/but.sqlite-shm")).is_empty());
        assert!(classify(Path::new("/repo/.git/gitbutler/but_cache.sqlite")).is_empty());
        let config = classify(Path::new("/repo/.git/butler-diff.toml"));
        assert!(config.grouping_changed());
        assert!(config.without_local_settings().is_empty());
        let comments = classify(Path::new("/repo/.git/butler-diff-comments.sqlite-wal"));
        assert!(comments.comments_changed());
        assert!(comments.without_local_settings().is_empty());
        assert!(classify(Path::new("/repo/.git/butler-diff-comments.sqlite-shm")).is_empty());
        assert!(classify(Path::new("/repo/.git/gitbutler/but.sqlite-wal")).metadata_only());
        assert!(!classify(Path::new("/repo/.git/refs/heads/config")).metadata_only());
        let mut mixed = Changes::default();
        mixed.merge(classify(Path::new("/repo/.git/gitbutler/but.sqlite")));
        mixed.merge(classify(Path::new("/repo/.git/packed-refs")));
        assert!(!mixed.metadata_only());
    }

    #[test]
    fn event_bursts_are_bounded_and_reads_do_not_trigger_refreshes() {
        use super::{WatchEvents, record};
        use notify::{
            Event, EventKind,
            event::{AccessKind, ModifyKind},
        };
        use std::sync::{Arc, Mutex, atomic::AtomicU8};
        let (sender, receiver) = async_channel::bounded(1);
        let events = WatchEvents {
            receiver,
            changed_at: Arc::new(Mutex::new(Instant::now())),
            changes: Arc::new(AtomicU8::new(0)),
        };
        let path = Path::new("/repo/.git/gitbutler/but.sqlite").to_owned();
        record(
            Event::new(EventKind::Access(AccessKind::Read)).add_path(path.clone()),
            &sender,
            &events,
        );
        assert!(!events.is_pending());
        for _ in 0..1000 {
            record(
                Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path.clone()),
                &sender,
                &events,
            );
        }
        assert_eq!(events.receiver.len(), 1);
        assert!(events.take_changes().metadata_only());
        assert!(!events.is_pending());
        assert!(events.receiver.is_empty());
    }
}
