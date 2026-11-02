mod comments;
mod file_groups;
mod jump;
mod launch;
mod mcp;
mod navigation;
mod repository;
mod review_ui;
mod session;
#[cfg(test)]
mod tests;
mod viewed;
mod watch;
mod workspace_ui;

use file_groups::{CompiledGroup, FileTotals, GroupConfig, GroupDefinition};
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    diff::{
        Diff, DiffAnnotation, DiffEvent, DiffFile, DiffLinePosition, DiffLineRange, DiffMode,
        DiffState,
    },
    h_resizable,
    input::{
        Enter as InputEnter, Escape as InputEscape, Input, InputEvent, InputState, Textarea,
        TextareaState,
    },
    resizable::{HandleEdge, ResizablePanelEvent, ResizableState, resize_handle},
    resizable_panel,
    text::TextView,
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use repository::Repository;
use session::Selection;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Instant,
};
use watch::{Changes, RepoWatcher};

actions!(
    gander,
    [
        Refresh,
        OpenRepository,
        ToggleStats,
        SubmitComment,
        NavigateBack,
        NavigateForward,
        OpenJump,
        Quit
    ]
);

struct GroupDraft {
    name: Entity<InputState>,
    pattern: Entity<InputState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CommentTarget {
    New(comments::AnchorRequest),
    Reply(String),
    Edit(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CommentDraftKey {
    repository: PathBuf,
    target: CommentTarget,
}

struct CommentDraft {
    key: CommentDraftKey,
    input: Entity<TextareaState>,
}

#[derive(Clone)]
struct BranchSidebarDrag;

impl Render for BranchSidebarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

fn viewed_button(
    path: &str,
    viewed: bool,
    pending: bool,
    ready: bool,
    app: WeakEntity<ReviewApp>,
) -> Button {
    let path = path.to_owned();
    Button::new(SharedString::from(format!("viewed:{path}")))
        .ghost()
        .xsmall()
        .selected(viewed)
        .disabled(pending || !ready)
        .label(if pending {
            "Saving…"
        } else if viewed {
            "✓ Viewed"
        } else {
            "Mark viewed"
        })
        .accessibility_label(format!(
            "Mark {path} as {}viewed",
            if viewed { "not " } else { "" }
        ))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            let _ = app.update(cx, |view, cx| view.toggle_viewed(&path, cx));
        })
}

fn viewed_checkbox(
    path: &str,
    viewed: bool,
    pending: bool,
    ready: bool,
    app: WeakEntity<ReviewApp>,
) -> Checkbox {
    let path = path.to_owned();
    Checkbox::new(SharedString::from(format!("viewed:{path}")))
        .small()
        .p_1()
        .flex_shrink_0()
        .checked(viewed)
        .disabled(pending || !ready)
        .tooltip(if pending {
            "Saving viewed state…"
        } else if viewed {
            "Unmark as viewed (expand file)"
        } else {
            "Mark as viewed (collapse file)"
        })
        .accessibility_label(format!(
            "Mark {path} as {}viewed",
            if viewed { "not " } else { "" }
        ))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            let _ = app.update(cx, |view, cx| view.toggle_viewed(&path, cx));
        })
}

#[derive(Clone)]
struct GroupHeader {
    key: String,
    name: String,
    totals: FileTotals,
    viewed: usize,
    pending: bool,
}

fn sticky_offset(top: Pixels, height: Pixels, next_top: Option<Pixels>) -> Pixels {
    next_top
        .map(|next| (next - top - height).min(px(0.)))
        .unwrap_or(px(0.))
}

fn sticky_layer(
    render: impl FnOnce(Bounds<Pixels>, &mut Window, &mut App) -> Option<(AnyElement, Option<Pixels>)>
    + 'static,
) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let (mut header, next_top) = render(bounds, window, cx)?;
            let measured = header.layout_as_root(
                size(
                    AvailableSpace::Definite(bounds.size.width),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            let offset = sticky_offset(bounds.top(), measured.height, next_top);
            if offset + measured.height <= px(0.) {
                return None;
            }
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                header.prepaint_at(bounds.origin + point(px(0.), offset), window, cx);
            });
            Some(header)
        },
        |bounds, header, window, cx| {
            if let Some(mut header) = header {
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    header.paint(window, cx)
                });
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

struct ReviewApp {
    development: bool,
    history: navigation::History,
    pending_navigation: Option<navigation::Visit>,
    session: session::Session,
    session_store: Option<session::Store>,
    session_task: Option<Task<()>>,
    focused_file: Option<String>,
    loaded_selection: Option<Selection>,
    panels: Entity<ResizableState>,
    file_panel_width: f32,
    comment_panel_width: f32,
    path: PathBuf,
    base_override: Option<String>,
    repo: Option<Repository>,
    selection: Option<Selection>,
    diff: Entity<DiffState>,
    files: Vec<DiffFile>,
    viewed_files: Arc<HashSet<String>>,
    pending_views: HashSet<String>,
    comments: Vec<comments::Thread>,
    comment_views: HashMap<String, review_ui::ThreadView>,
    current_comment_ids: HashSet<String>,
    comment_draft: Option<CommentDraft>,
    comment_drafts: HashMap<CommentDraftKey, String>,
    comment_error: Option<String>,
    comment_pending: bool,
    comment_generation: u64,
    show_comments: bool,
    show_branches: bool,
    sidebars_swapped: bool,
    branch_sidebar_width: Pixels,
    viewed_error: Option<String>,
    viewed_epoch: u64,
    collapsed_files: HashSet<String>,
    file_scroll: ScrollHandle,
    filter: Entity<InputState>,
    query: String,
    group_config: GroupConfig,
    groups: Vec<CompiledGroup>,
    group_path: Option<PathBuf>,
    collapsed_groups: HashSet<String>,
    group_editor: Option<Vec<GroupDraft>>,
    group_error: Option<String>,
    mode: DiffMode,
    wrap: bool,
    show_stats: bool,
    loading_repo: bool,
    loading_diff: bool,
    error: Option<String>,
    watch_error: Option<String>,
    watcher: Option<RepoWatcher>,
    loaded: Option<(PathBuf, String, String)>,
    pending_diff: Option<(PathBuf, String, String)>,
    refresh_visible: bool,
    queued_manual_refresh: bool,
    queued_changes: Changes,
    repo_task: Option<Task<()>>,
    diff_task: Option<Task<()>>,
    watch_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    focus: FocusHandle,
}

impl ReviewApp {
    fn new(
        path: PathBuf,
        base_override: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state_path = session::state_path();
        let (session, session_store) = match state_path {
            Some(path) => match session::Session::load(&path) {
                Ok(session) => (session, Some(session::Store::new(path))),
                Err(error) => {
                    eprintln!("UI state unavailable: {error:#}");
                    (session::Session::default(), None)
                }
            },
            None => (session::Session::default(), None),
        };
        let diff = cx.new(|cx| DiffState::new([], cx));
        let panels = cx.new(|_| ResizableState::default());
        let panel_sub = cx.subscribe(&panels, |view, panels, _: &ResizablePanelEvent, cx| {
            view.remember_panel_widths(panels.read(cx).sizes());
            cx.notify();
        });
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter changed files…"));
        let sub = cx.subscribe(&filter, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = input.read(cx).value().to_lowercase();
                cx.notify();
            }
        });
        let collapse_sub = cx.subscribe(&diff, |this, _, event, cx| {
            match event {
                DiffEvent::FileCollapsed(path) => {
                    this.collapsed_files.insert(path.to_string());
                }
                DiffEvent::FileExpanded(path) => {
                    this.collapsed_files.remove(path.as_str());
                }
                _ => return,
            }
            cx.notify();
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut view = Self {
            development: launch::is_development(),
            history: navigation::History::default(),
            pending_navigation: None,
            session,
            session_store,
            session_task: None,
            focused_file: None,
            loaded_selection: None,
            panels,
            file_panel_width: 340.,
            comment_panel_width: 360.,
            path,
            base_override: None,
            repo: None,
            selection: None,
            diff,
            files: vec![],
            viewed_files: Arc::default(),
            pending_views: HashSet::new(),
            comments: vec![],
            comment_views: HashMap::new(),
            current_comment_ids: HashSet::new(),
            comment_draft: None,
            comment_drafts: HashMap::new(),
            comment_error: None,
            comment_pending: false,
            comment_generation: 0,
            show_comments: false,
            show_branches: true,
            sidebars_swapped: false,
            branch_sidebar_width: px(300.),
            viewed_error: None,
            viewed_epoch: 0,
            collapsed_files: HashSet::new(),
            file_scroll: ScrollHandle::new(),
            filter,
            query: String::new(),
            group_config: GroupConfig::default(),
            groups: vec![],
            group_path: None,
            collapsed_groups: HashSet::new(),
            group_editor: None,
            group_error: None,
            mode: DiffMode::Unified,
            wrap: false,
            show_stats: false,
            loading_repo: false,
            loading_diff: false,
            error: None,
            watch_error: None,
            watcher: None,
            loaded: None,
            pending_diff: None,
            refresh_visible: true,
            queued_manual_refresh: false,
            queued_changes: Changes::default(),
            repo_task: None,
            diff_task: None,
            watch_task: None,
            _subscriptions: vec![sub, collapse_sub, panel_sub],
            focus,
        };
        view.restore_repo_ui(window, cx);
        view.base_override = base_override.or(view.base_override);
        let observer = cx.observe(&cx.entity(), |view, _, cx| view.schedule_session_save(cx));
        let on_quit = cx.on_app_quit(|view, cx| {
            view.save_session_now(cx);
            async {}
        });
        let on_release = cx.on_release(|view, cx| view.save_session_now(cx));
        view._subscriptions.extend([observer, on_quit, on_release]);
        view.refresh(cx);
        view
    }

    fn start_watching(&mut self, repo: &Repository, cx: &mut Context<Self>) {
        match RepoWatcher::new(repo) {
            Ok(watcher) => {
                let events = watcher.events.clone();
                self.watcher = Some(watcher);
                self.watch_error = None;
                self.watch_task = Some(cx.spawn(async move |this, cx| {
                    let mut previous_refresh = None;
                    while events.recv().await {
                        let first = Instant::now();
                        loop {
                            let wait = events.remaining(first, previous_refresh);
                            if wait.is_zero() {
                                break;
                            }
                            cx.background_executor().timer(wait).await;
                        }
                        let changes = events.take_changes();
                        if changes.is_empty() {
                            continue;
                        }
                        previous_refresh = Some(Instant::now());
                        if this
                            .update(cx, |view, cx| {
                                if changes.grouping_changed() {
                                    view.reload_groups(cx);
                                }
                                if changes.comments_changed() {
                                    view.reload_comments(cx);
                                }
                                let changes = changes.without_local_settings();
                                if !changes.is_empty() {
                                    view.start_refresh(Some(changes), cx);
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
            }
            Err(e) => {
                self.watch_error = Some(format!("Auto-refresh unavailable: {e}. Use Refresh."))
            }
        }
    }

    fn clear_viewed(&mut self) {
        self.focused_file = None;
        self.current_comment_ids.clear();
        self.viewed_files = Arc::default();
        self.viewed_error = None;
        self.collapsed_files.clear();
        self.file_scroll.set_offset(point(px(0.), px(0.)));
        // Writes already requested by the user finish even if they switch
        // comparisons. Keep their buttons disabled until those writes finish.
    }

    fn reload_viewed(&mut self, cx: &mut Context<Self>) {
        let (Some(key), Some(repo)) = (self.loaded.clone(), self.repo.clone()) else {
            return;
        };
        let files = self.files.clone();
        let epoch = self.viewed_epoch;
        let (_, base, tip) = key.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { viewed::load(&repo, &base, &tip, &files) })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.loaded.as_ref() != Some(&key) || view.viewed_epoch != epoch {
                    return;
                }
                match result {
                    Ok(files) => {
                        if *view.viewed_files == files && view.viewed_error.is_none() {
                            return;
                        }
                        // Only changed viewed bits have collapse side effects.
                        // Manually expanded viewed files stay expanded on refresh.
                        for path in view.viewed_files.symmetric_difference(&files) {
                            if files.contains(path) {
                                view.collapsed_files.insert(path.clone());
                            } else {
                                view.collapsed_files.remove(path);
                            }
                        }
                        view.viewed_files = Arc::new(files);
                        view.viewed_error = None;
                        view.sync_collapsed_files(cx);
                    }
                    Err(error) => {
                        view.viewed_error = Some(format!("Couldn't load viewed files: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_viewed(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(file) = self
            .files
            .iter()
            .find(|file| file.path().as_str() == path)
            .cloned()
        else {
            return;
        };
        self.save_viewed(vec![file], !self.viewed_files.contains(path), cx);
    }

    fn toggle_group_viewed(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.group_config.enabled || self.groups.is_empty() {
            return;
        }
        let Some(bucket) = file_groups::buckets(&self.files, &self.query, &self.groups)
            .into_iter()
            .find(|bucket| bucket.key == key)
        else {
            return;
        };
        let viewed = !bucket
            .files
            .iter()
            .all(|file| self.viewed_files.contains(file.path().as_str()));
        let files = bucket.files.into_iter().cloned().collect();
        self.save_viewed(files, viewed, cx);
    }

    fn save_viewed(&mut self, files: Vec<DiffFile>, viewed: bool, cx: &mut Context<Self>) {
        if files.is_empty()
            || files
                .iter()
                .any(|file| self.pending_views.contains(file.path().as_str()))
        {
            return;
        }
        let (Some(key), Some(repo)) = (self.loaded.clone(), self.repo.clone()) else {
            return;
        };
        let paths: Vec<_> = files.iter().map(|file| file.path().to_string()).collect();
        self.pending_views.extend(paths.iter().cloned());
        let common_dir = repo.common_dir.clone();
        let (_, base, tip) = key.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    // Resolve only the requested files, in the background. One
                    // transaction commits the entire group, with no partial marks.
                    let versions = files
                        .iter()
                        .map(|file| {
                            let version = viewed::FileVersion::resolve(&repo, &base, &tip, file)?;
                            Ok((file.path().to_string(), version))
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    viewed::set_many(&repo.common_dir.join(viewed::STATE_FILE), &versions, viewed)?;
                    Ok::<_, anyhow::Error>(versions)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                for path in &paths {
                    view.pending_views.remove(path);
                }
                if view
                    .repo
                    .as_ref()
                    .is_some_and(|repo| repo.common_dir == common_dir)
                {
                    match result {
                        Ok(versions) => {
                            view.viewed_epoch = view.viewed_epoch.wrapping_add(1);
                            let same_comparison = view.loaded.as_ref() == Some(&key);
                            let current: HashMap<_, _> = view
                                .files
                                .iter()
                                .map(|file| (file.path().as_str(), file))
                                .collect();
                            let viewed_files = Arc::make_mut(&mut view.viewed_files);
                            for (path, version) in versions {
                                // A completed write must never mark a different
                                // revision after switching branches or ranges.
                                let matches = current.get(path.as_str()).is_some_and(|file| {
                                    same_comparison
                                        || viewed::FileVersion::from_patch(file).as_ref()
                                            == Some(&version)
                                });
                                if !matches {
                                    continue;
                                }
                                if viewed {
                                    view.collapsed_files.insert(path.clone());
                                    viewed_files.insert(path);
                                } else {
                                    view.collapsed_files.remove(&path);
                                    viewed_files.remove(&path);
                                }
                            }
                            view.viewed_error = None;
                            // Apply all collapse side effects with one rebuild.
                            view.sync_collapsed_files(cx);
                            if !same_comparison {
                                view.reload_viewed(cx);
                            }
                        }
                        Err(error) => {
                            view.viewed_error =
                                Some(format!("Couldn't save viewed files: {error:#}"))
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.reload_viewed(cx);
        self.reload_comments(cx);
        self.start_refresh(None, cx);
    }

    fn start_refresh(&mut self, changes: Option<Changes>, cx: &mut Context<Self>) {
        if self.loading_repo {
            if let Some(changes) = changes {
                self.queued_changes.merge(changes);
            } else {
                self.queued_manual_refresh = true;
            }
            return;
        }
        let automatic = changes.is_some();
        let path = self.path.clone();
        let base = self.base_override.clone();
        let previous = self.repo.clone();
        self.loading_repo = true;
        self.refresh_visible = !automatic;
        self.repo_task = Some(cx.spawn(async move |this, cx| {
            let result: anyhow::Result<Option<Repository>> = cx
                .background_spawn(async move {
                    if changes.is_some_and(Changes::metadata_only)
                        && let Some(previous) = &previous
                        && !previous.is_local()
                        && previous.current_metadata_signature()? == previous.metadata_signature
                    {
                        return Ok(None);
                    }
                    Repository::load(&path, base.as_deref()).map(Some)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.loading_repo = false;
                let mut changed = !automatic;
                match result {
                    Ok(Some(repo)) => {
                        changed |= view.repo.as_ref() != Some(&repo) || view.error.is_some();
                        view.error = None;
                        let path = repo.common_dir.join(file_groups::CONFIG_FILE);
                        if view.group_path.as_ref() != Some(&path) {
                            view.group_path = Some(path);
                            view.reload_groups(cx);
                        }
                        if view.watcher.is_none() {
                            view.start_watching(&repo, cx);
                        }
                        let valid = view.selection.as_ref().is_some_and(|sel| {
                            repo.stacks.iter().any(|s| {
                                s.id == sel.stack
                                    && s.branches.iter().any(|b| b.name == sel.anchor)
                                    && s.branches.iter().any(|b| b.name == sel.end)
                            })
                        });
                        if !valid {
                            view.selection = repo.stacks.first().and_then(|s| {
                                s.branches.first().map(|b| Selection {
                                    stack: s.id.clone(),
                                    anchor: b.name.clone(),
                                    end: b.name.clone(),
                                })
                            });
                        }
                        view.repo = Some(repo);
                        view.reload_comments(cx);
                        view.load_diff(cx);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let error = format!("{e:#}");
                        changed |= view.error.as_deref() != Some(&error);
                        view.error = Some(error);
                        if !automatic {
                            view.repo = None;
                            view.selection = None;
                            view.loaded = None;
                            view.pending_diff = None;
                            view.diff_task = None;
                            view.loading_diff = false;
                            view.files.clear();
                            view.clear_viewed();
                            view.diff.update(cx, |state, cx| state.set_files([], cx));
                        }
                    }
                }
                if changed {
                    cx.notify();
                }
                if std::mem::take(&mut view.queued_manual_refresh) {
                    view.queued_changes = Changes::default();
                    view.refresh(cx);
                } else {
                    let changes = std::mem::take(&mut view.queued_changes);
                    if !changes.is_empty() {
                        view.start_refresh(Some(changes), cx);
                    }
                }
            });
        }));
        if !automatic {
            cx.notify();
        }
    }

    fn endpoints(&self) -> anyhow::Result<Option<(String, String, String)>> {
        let Some((stack, a, b)) = self.selection.as_ref().and_then(|sel| {
            let stack = self
                .repo
                .as_ref()?
                .stacks
                .iter()
                .find(|s| s.id == sel.stack)?;
            Some((
                stack,
                stack.branches.iter().position(|b| b.name == sel.anchor)?,
                stack.branches.iter().position(|b| b.name == sel.end)?,
            ))
        }) else {
            return Ok(None);
        };
        let (lo, hi) = (a.min(b), a.max(b));
        let label = if lo == hi {
            stack.branches[lo].name.clone()
        } else {
            format!(
                "{} → {} · {} branches",
                stack.branches[lo].name,
                stack.branches[hi].name,
                hi - lo + 1
            )
        };
        Ok(Some((
            stack.branches[lo].base()?.to_owned(),
            stack.branches[hi].tip.clone(),
            label,
        )))
    }

    fn load_diff(&mut self, cx: &mut Context<Self>) {
        let endpoints = self.endpoints();
        let endpoint_error = endpoints.as_ref().err().map(|error| format!("{error:#}"));
        let Some((base, tip, _)) = endpoints.ok().flatten() else {
            self.diff_task = None;
            self.loading_diff = false;
            self.loaded = None;
            self.pending_diff = None;
            self.files.clear();
            self.clear_viewed();
            self.diff.update(cx, |state, cx| state.set_files([], cx));
            self.error = endpoint_error;
            return;
        };
        let repo = self.repo.as_ref().unwrap().clone();
        let key = (repo.root.clone(), base.clone(), tip.clone());
        if self.loaded.as_ref() == Some(&key) {
            self.loaded_selection = self.selection.clone();
            return;
        }
        if self.pending_diff.as_ref() == Some(&key) {
            return;
        }
        self.remember_comparison(cx);
        self.loaded = None;
        self.pending_diff = Some(key.clone());
        self.loading_diff = true;
        self.error = None;
        self.files.clear();
        self.clear_viewed();
        self.diff.update(cx, |state, cx| state.set_files([], cx));
        let viewed_epoch = self.viewed_epoch;
        self.diff_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let patch = repo.patch(&base, &tip)?;
                    let files = DiffFile::parse(&patch)
                        .map_err(|e| anyhow::anyhow!("Couldn't parse git diff: {e}"))?;
                    let viewed = viewed::load(&repo, &base, &tip, &files);
                    Ok::<_, anyhow::Error>((files, viewed))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.loading_diff = false;
                view.pending_diff = None;
                match result {
                    Ok((files, viewed)) => {
                        view.loaded = Some(key);
                        view.loaded_selection = view.selection.clone();
                        view.files = files;
                        view.sync_comment_anchors();
                        match viewed {
                            Ok(files) => view.viewed_files = Arc::new(files),
                            Err(error) => {
                                view.viewed_error =
                                    Some(format!("Couldn't load viewed files: {error:#}"))
                            }
                        }
                        view.collapsed_files = (*view.viewed_files).clone();
                        let saved = view.saved_comparison().cloned();
                        if let Some(saved) = &saved {
                            view.collapsed_files = saved.collapsed_for(
                                view.files.iter().map(|file| file.path().as_str()),
                                &view.viewed_files,
                            );
                        }
                        view.sync_diff_files(cx);
                        if let Some(saved) = saved {
                            view.restore_comparison_position(saved, cx);
                        }
                        if let Some(visit) = view.pending_navigation.take()
                            && view.selection.as_ref() == Some(&visit.selection)
                        {
                            view.reveal_file(&visit.file, cx);
                        }
                        // Reconcile a mark completed while this diff was loading.
                        if view.viewed_epoch != viewed_epoch {
                            view.reload_viewed(cx);
                        }
                    }
                    Err(e) => view.error = Some(format!("{e:#}")),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn select(&mut self, stack: String, name: String, extend: bool, cx: &mut Context<Self>) {
        self.remember_comparison(cx);
        let anchor = self
            .selection
            .as_ref()
            .filter(|s| extend && s.stack == stack)
            .map(|s| s.anchor.clone())
            .unwrap_or_else(|| name.clone());
        self.selection = Some(Selection {
            stack,
            anchor,
            end: name,
        });
        self.load_diff(cx);
        cx.notify();
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Git repository".into()),
        });
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.snapshot_session(cx);
                        view.stash_comment_draft(cx);
                        view.path = path;
                        view.history = navigation::History::default();
                        view.pending_navigation = None;
                        view.base_override = None;
                        view.repo = None;
                        view.comments.clear();
                        view.comment_views.clear();
                        view.current_comment_ids.clear();
                        view.comment_generation += 1;
                        view.selection = None;
                        view.watcher = None;
                        view.watch_task = None;
                        view.repo_task = None;
                        view.loading_repo = false;
                        view.queued_manual_refresh = false;
                        view.queued_changes = Changes::default();
                        view.pending_diff = None;
                        view.group_path = None;
                        view.group_config = GroupConfig::default();
                        view.groups.clear();
                        view.collapsed_groups.clear();
                        view.group_editor = None;
                        view.group_error = None;
                        view.watch_error = None;
                        view.loaded = None;
                        view.diff_task = None;
                        view.loading_diff = false;
                        view.files.clear();
                        view.clear_viewed();
                        view.restore_repo_ui(window, cx);
                        view.reset_panel_layout(cx);
                        view.diff.update(cx, |state, cx| state.set_files([], cx));
                        view.refresh(cx);
                    });
                }
            }
            Ok(Err(e)) => {
                let _ = this.update(cx, |view, cx| {
                    view.error = Some(e.to_string());
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }

    fn remember_panel_widths(&mut self, sizes: &[Pixels]) {
        let file_index = if self.sidebars_swapped {
            if self.show_comments { 2 } else { 1 }
        } else {
            0
        };
        if let Some(width) = sizes.get(file_index) {
            self.file_panel_width = (*width).into();
        }
        if self.show_comments {
            let comment_index = if self.sidebars_swapped { 0 } else { 2 };
            if let Some(width) = sizes.get(comment_index) {
                self.comment_panel_width = (*width).into();
            }
        }
    }

    fn capture_panel_widths(&mut self, cx: &App) {
        let panels = self.panels.clone();
        self.remember_panel_widths(panels.read(cx).sizes());
    }

    fn reset_panel_layout(&mut self, cx: &mut Context<Self>) {
        // The resize library caches sizes by slot. Recreate those slots after
        // changing their order or visibility, using each pane's saved width.
        self.panels
            .update(cx, |panels, _| *panels = ResizableState::default());
    }

    fn review_panels(&self, body: AnyElement, cx: &mut Context<Self>) -> impl IntoElement {
        let files = Some(
            resizable_panel()
                .size(px(self.file_panel_width))
                .fixed()
                .size_range(px(220.)..px(750.))
                .child(self.file_sidebar(cx)),
        );
        let comments = self.show_comments.then(|| {
            resizable_panel()
                .size(px(self.comment_panel_width))
                .fixed()
                .size_range(px(260.)..px(650.))
                .child(self.comments_panel(cx))
        });
        let (left, right) = if self.sidebars_swapped {
            (comments, files)
        } else {
            (files, comments)
        };
        h_resizable("file-diff-panels")
            .with_state(&self.panels)
            .when_some(left, |panels, left| panels.child(left))
            .child(
                resizable_panel()
                    .size_range(px(300.)..Pixels::MAX)
                    .child(body),
            )
            .when_some(right, |panels, right| panels.child(right))
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let local = self.repo.as_ref().is_some_and(Repository::is_local);
        let mut sidebar = div()
            .id("stacks")
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(cx.theme().border)
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if local {
                        "LOCAL BRANCHES"
                    } else {
                        "STACKS · BASE → TIP"
                    }),
            );
        if let Some(repo) = &self.repo {
            for (si, stack) in repo.stacks.iter().enumerate() {
                let id = stack.id.clone();
                let first = stack.branches.first().unwrap().name.clone();
                let last = stack.branches.last().unwrap().name.clone();
                let full_id = id.clone();
                let mut card =
                    div()
                        .flex()
                        .flex_col()
                        .flex_shrink_0()
                        .gap_1()
                        .when(!local, |card| {
                            card.pb_3().child(
                                Button::new(("stack", si))
                                    .ghost()
                                    .small()
                                    .label(format!(
                                        "stack {} · {} · view all",
                                        si + 1,
                                        if stack.applied {
                                            "applied"
                                        } else {
                                            "unapplied"
                                        }
                                    ))
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.remember_comparison(cx);
                                        view.selection = Some(Selection {
                                            stack: full_id.clone(),
                                            anchor: first.clone(),
                                            end: last.clone(),
                                        });
                                        view.load_diff(cx);
                                        cx.notify();
                                    })),
                            )
                        });
                let range = self
                    .selection
                    .as_ref()
                    .filter(|s| s.stack == id)
                    .and_then(|s| {
                        Some((
                            stack.branches.iter().position(|b| b.name == s.anchor)?,
                            stack.branches.iter().position(|b| b.name == s.end)?,
                        ))
                    });
                for (bi, branch) in stack.branches.iter().enumerate() {
                    let selected = range.is_some_and(|(a, b)| (a.min(b)..=a.max(b)).contains(&bi));
                    let branch_id = id.clone();
                    let name = branch.name.clone();
                    let mut row = div()
                        .id(SharedString::from(format!("{id}-{bi}")))
                        .role(Role::Button)
                        .aria_label(format!("Review branch {}", branch.name))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .cursor_pointer()
                        .text_sm()
                        .border_l_2()
                        .border_color(if selected {
                            cx.theme().primary
                        } else {
                            cx.theme().border
                        })
                        .bg(if selected {
                            cx.theme().accent
                        } else {
                            cx.theme().background
                        })
                        .hover(|s| s.bg(cx.theme().accent))
                        .child(
                            div()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(branch.name.clone()),
                        )
                        .on_click(cx.listener(move |view, event: &ClickEvent, _, cx| {
                            view.select(
                                branch_id.clone(),
                                name.clone(),
                                event.modifiers().shift,
                                cx,
                            );
                        }));
                    if let Some(review) = &branch.review {
                        row = row.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(review.clone()),
                        );
                    }
                    card = card.child(row);
                }
                sidebar = sidebar.child(card);
            }
        }
        sidebar
    }

    fn file_hidden(&self, file: &DiffFile) -> bool {
        self.group_config.enabled
            && !self.groups.is_empty()
            && file_groups::hidden(file, &self.groups, &self.collapsed_groups)
    }

    fn sync_diff_files(&mut self, cx: &mut Context<Self>) {
        let files: Vec<_> = self
            .files
            .iter()
            .filter(|file| !self.file_hidden(file))
            .cloned()
            .collect();
        self.diff.update(cx, |state, cx| state.set_files(files, cx));
        self.sync_collapsed_files(cx);
    }

    fn sync_collapsed_files(&self, cx: &mut Context<Self>) {
        let paths = self.collapsed_files.iter().cloned().map(SharedString::from);
        self.diff
            .update(cx, |state, cx| state.set_collapsed_files(paths, cx));
    }

    fn apply_groups(
        &mut self,
        config: GroupConfig,
        groups: Vec<CompiledGroup>,
        cx: &mut Context<Self>,
    ) {
        let keys: HashSet<_> = groups
            .iter()
            .map(|group| format!("group:{}", group.name))
            .chain(std::iter::once("ungrouped".into()))
            .collect();
        self.collapsed_groups.retain(|key| keys.contains(key));
        self.group_config = config;
        self.groups = groups;
        self.group_error = None;
        self.sync_diff_files(cx);
        cx.notify();
    }

    fn reload_groups(&mut self, cx: &mut Context<Self>) {
        let Some(path) = &self.group_path else { return };
        let result = GroupConfig::load(path).and_then(|config| {
            let groups = config.compile()?;
            Ok((config, groups))
        });
        match result {
            Ok((config, groups)) => {
                if config != self.group_config || self.group_error.is_some() {
                    self.apply_groups(config, groups, cx);
                }
            }
            Err(error) => {
                let error = format!("{error:#}");
                if self.group_error.as_ref() != Some(&error) {
                    self.group_error = Some(error);
                    cx.notify();
                }
            }
        }
    }

    fn save_groups(&mut self, config: GroupConfig, cx: &mut Context<Self>) -> bool {
        let result = (|| -> anyhow::Result<_> {
            let groups = config.compile()?;
            let path = self
                .group_path
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Open a repository before saving file groups."))?;
            config.save(path)?;
            Ok(groups)
        })();
        match result {
            Ok(groups) => {
                self.apply_groups(config, groups, cx);
                true
            }
            Err(error) => {
                self.group_error = Some(format!("{error:#}"));
                cx.notify();
                false
            }
        }
    }

    fn new_group_draft(
        name: &str,
        pattern: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> GroupDraft {
        GroupDraft {
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Group name")
                    .default_value(name.to_owned())
            }),
            pattern: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Full-path regex")
                    .default_value(pattern.to_owned())
            }),
        }
    }

    fn edit_groups(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut drafts: Vec<_> = self
            .group_config
            .groups
            .iter()
            .map(|group| Self::new_group_draft(&group.name, &group.pattern, window, cx))
            .collect();
        if drafts.is_empty() {
            drafts.push(Self::new_group_draft(
                "Tests",
                file_groups::TEST_PATTERN,
                window,
                cx,
            ));
        }
        self.group_editor = Some(drafts);
        cx.notify();
    }

    fn group_editor_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut rows = div()
            .id("group-rules")
            .max_h(px(185.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        if let Some(drafts) = &self.group_editor {
            for (index, draft) in drafts.iter().enumerate() {
                rows = rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(160.)).child(Input::new(&draft.name).small()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&draft.pattern).small()),
                        )
                        .child(
                            Button::new(("remove-group", index))
                                .ghost()
                                .small()
                                .label("Remove")
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    if let Some(drafts) = &mut view.group_editor {
                                        drafts.remove(index);
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            }
        }
        div().flex_shrink_0().p_3().flex().flex_col().gap_2().border_b_1().border_color(cx.theme().border)
            .child(div().font_weight(FontWeight::MEDIUM).child("Repository file groups"))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Regexes match full repo-relative paths. First matching group wins. Unmatched files stay in Ungrouped."))
            .child(rows)
            .when_some(self.group_error.clone(), |view, error| view.child(div().text_xs().text_color(rgb(0xe89090)).child(error)))
            .child(div().flex().items_center().gap_2()
                .child(Button::new("add-group").small().label("Add group").on_click(cx.listener(|view, _, window, cx| {
                    let draft = Self::new_group_draft("", "", window, cx);
                    if let Some(drafts) = &mut view.group_editor { drafts.push(draft); }
                    cx.notify();
                })))
                .child(div().flex_1().text_xs().text_color(cx.theme().muted_foreground).child("Saved for this repository across stacks and worktrees."))
                .child(Button::new("cancel-groups").small().label("Cancel").on_click(cx.listener(|view, _, window, cx| {
                    view.group_editor = None;
                    window.focus(&view.focus, cx);
                    cx.notify();
                })))
                .child(Button::new("save-groups").primary().small().label("Save groups").on_click(cx.listener(|view, _, window, cx| {
                    let groups = view.group_editor.as_ref().map(|drafts| drafts.iter().map(|draft| GroupDefinition {
                        name: draft.name.read(cx).value().trim().to_owned(),
                        pattern: draft.pattern.read(cx).value().to_string(),
                    }).collect()).unwrap_or_default();
                    let config = GroupConfig { enabled: view.group_config.enabled, groups };
                    if view.save_groups(config, cx) {
                        view.group_editor = None;
                        window.focus(&view.focus, cx);
                    }
                }))))
    }

    fn file_row(&self, file: &DiffFile, cx: &mut Context<Self>) -> impl IntoElement {
        let path = file.path().to_string();
        let viewed = self.viewed_files.contains(&path);
        let button = viewed_button(
            &path,
            viewed,
            self.pending_views.contains(&path),
            self.loaded.is_some(),
            cx.entity().downgrade(),
        );
        let (directory, filename) = path.rsplit_once('/').unwrap_or(("", &path));
        let filename = filename.to_owned();
        let directory = directory.replace('/', "/\u{200b}");
        let tooltip_path = path.clone();
        div()
            .id(SharedString::from(path.clone()))
            .role(Role::Button)
            .aria_label(format!("Show file {path}"))
            .p_2()
            .rounded_md()
            .cursor_pointer()
            .text_xs()
            .hover(|s| s.bg(cx.theme().accent))
            .tooltip(move |window, cx| {
                let path = tooltip_path.replace('/', "/\u{200b}");
                Tooltip::element(move |_, _| {
                    div()
                        .max_w(px(800.))
                        .whitespace_normal()
                        .child(path.clone())
                })
                .build(window, cx)
            })
            .child(
                div()
                    .text_sm()
                    .text_color(if viewed {
                        cx.theme().muted_foreground
                    } else {
                        cx.theme().foreground
                    })
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_normal()
                    .child(filename),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .whitespace_normal()
                    .child(directory),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_color(rgb(0x72cc91))
                            .child(format!("+{}", file.additions())),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xe89090))
                            .child(format!("−{}", file.deletions())),
                    )
                    .child(div().flex_1())
                    .child(button),
            )
            .on_click(cx.listener(move |view, _, _, cx| {
                view.visit_file(&path, cx);
            }))
    }

    fn group_header(
        &self,
        bucket: &GroupHeader,
        sticky: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let key = bucket.key.clone();
        let collapsed = self.collapsed_groups.contains(&key);
        let totals = bucket.totals;
        let viewed = bucket.viewed;
        let checked = viewed == totals.files;
        let group_key = key.clone();
        let action = if checked { "Unmark" } else { "Mark" };
        let scope = if self.query.is_empty() {
            "all"
        } else {
            "filtered"
        };
        let checkbox = Checkbox::new("group-viewed")
            .small()
            .p_1()
            .flex_shrink_0()
            .checked(checked)
            .disabled(bucket.pending || self.loaded.is_none())
            .tooltip(if bucket.pending {
                format!("Saving viewed state for {}…", bucket.name)
            } else {
                format!(
                    "{action} {scope} {} files in {} as viewed",
                    totals.files, bucket.name
                )
            })
            .accessibility_label(format!(
                "Mark group {} as {}viewed, {} files",
                bucket.name,
                if checked { "not " } else { "" },
                totals.files
            ))
            .on_click(cx.listener(move |view, _, _, cx| {
                cx.stop_propagation();
                view.toggle_group_viewed(&group_key, cx);
            }));
        div()
            .id(SharedString::from(format!(
                "{}header:{key}",
                if sticky { "sticky:" } else { "" }
            )))
            .role(Role::Button)
            .aria_label(format!(
                "{}{} group {}, {}, {viewed}/{} viewed",
                if sticky { "Pinned: " } else { "" },
                if collapsed { "Expand" } else { "Collapse" },
                bucket.name,
                totals.label(),
                totals.files
            ))
            .flex_shrink_0()
            .p_2()
            .when(!sticky, |view| view.mt_2().rounded_md())
            .when(sticky, |view| view.block_mouse_except_scroll())
            .cursor_pointer()
            .bg(cx.theme().muted)
            .hover(|s| s.bg(cx.theme().accent))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(if collapsed { "▸" } else { "▾" })
                    .child(checkbox)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .font_weight(FontWeight::MEDIUM)
                            .child(bucket.name.clone()),
                    )
                    .child(div().text_xs().child(format!("{} files", totals.files))),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .text_color(rgb(0x72cc91))
                            .child(format!("+{}", totals.additions)),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xe89090))
                            .child(format!("−{}", totals.deletions)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{viewed}/{} viewed", totals.files)),
                    ),
            )
            .on_click(cx.listener(move |view, _, _, cx| {
                if !view.collapsed_groups.remove(&key) {
                    view.collapsed_groups.insert(key.clone());
                }
                view.sync_diff_files(cx);
                cx.notify();
            }))
    }

    fn pinned_file_header(&self, file: &DiffFile, cx: &mut Context<Self>) -> impl IntoElement {
        let path = file.path().to_string();
        let collapsed = self.collapsed_files.contains(&path);
        let (directory, filename) = path.rsplit_once('/').unwrap_or(("", &path));
        let button = viewed_checkbox(
            &path,
            self.viewed_files.contains(&path),
            self.pending_views.contains(&path),
            self.loaded.is_some(),
            cx.entity().downgrade(),
        );
        let collapse_path = path.clone();
        div()
            .id("pinned-file")
            .block_mouse_except_scroll()
            .flex_shrink_0()
            .min_w_0()
            .w_full()
            .p_2()
            .flex()
            .items_center()
            .gap_2()
            .bg(cx.theme().muted)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("pinned-file-collapse")
                    .ghost()
                    .xsmall()
                    .label(if collapsed { "▸" } else { "▾" })
                    .accessibility_label(format!(
                        "{} pinned file {path}",
                        if collapsed { "Expand" } else { "Collapse" }
                    ))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if collapsed {
                            view.collapsed_files.remove(&collapse_path);
                        } else {
                            view.collapsed_files.insert(collapse_path.clone());
                        }
                        view.diff.update(cx, |state, cx| {
                            state.set_file_collapsed(&collapse_path, !collapsed, cx)
                        });
                        cx.notify();
                    })),
            )
            .child(button)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .whitespace_normal()
                            .child(filename.to_owned()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .whitespace_normal()
                            .child(directory.replace('/', "/\u{200b}")),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x72cc91))
                    .child(format!("+{}", file.additions())),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0xe89090))
                    .child(format!("−{}", file.deletions())),
            )
    }

    fn file_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let grouped = self.group_config.enabled && !self.groups.is_empty();
        let groups = if grouped { self.groups.as_slice() } else { &[] };
        let buckets = file_groups::buckets(&self.files, &self.query, groups);
        let all_groups_collapsed = !buckets.is_empty()
            && buckets
                .iter()
                .all(|bucket| self.collapsed_groups.contains(&bucket.key));
        let matching_files: Vec<_> = self
            .files
            .iter()
            .filter(|file| file_groups::matches_filter(file, &self.query))
            .collect();
        let all_files_collapsed = !matching_files.is_empty()
            && matching_files
                .iter()
                .all(|file| self.collapsed_files.contains(file.path().as_str()));
        let mut list = div()
            .id("files")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.file_scroll)
            .h_full()
            .p_2();
        if buckets.is_empty() && !self.query.is_empty() && !self.loading_diff {
            list = list.child(
                div()
                    .p_2()
                    .text_color(cx.theme().muted_foreground)
                    .child("No matching files."),
            );
        }
        let headers: Vec<_> = buckets
            .iter()
            .map(|bucket| GroupHeader {
                key: bucket.key.clone(),
                name: bucket.name.clone(),
                totals: FileTotals::from_files(bucket.files.iter().copied()),
                viewed: bucket
                    .files
                    .iter()
                    .filter(|file| self.viewed_files.contains(file.path().as_str()))
                    .count(),
                pending: bucket
                    .files
                    .iter()
                    .any(|file| self.pending_views.contains(file.path().as_str())),
            })
            .collect();
        // Capture each header after scrolling has been applied. ScrollHandle's
        // child bounds are from layout, before the scroller's paint offset.
        let header_bounds = Rc::new(RefCell::new(vec![None; buckets.len()]));
        for (index, bucket) in buckets.iter().enumerate() {
            let collapsed = grouped && self.collapsed_groups.contains(&bucket.key);
            if grouped {
                let captured = header_bounds.clone();
                list = list.child(
                    self.group_header(&headers[index], false, cx)
                        .relative()
                        .child(
                            canvas(
                                move |bounds, _, _| {
                                    captured.borrow_mut()[index] = Some(bounds);
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        ),
                );
            }
            if !collapsed {
                for file in &bucket.files {
                    list = list.child(self.file_row(file, cx));
                }
            }
        }
        let app = cx.entity().downgrade();
        let list = div()
            .relative()
            .flex_1()
            .min_h_0()
            .child(list)
            .child(sticky_layer(move |viewport, _, cx| {
                if !grouped {
                    return None;
                }
                let bounds = header_bounds.borrow();
                let group = bounds.iter().rposition(|bounds| {
                    bounds.is_some_and(|bounds| bounds.top() < viewport.top())
                })?;
                let next_top = bounds
                    .get(group + 1)
                    .copied()
                    .flatten()
                    .map(|bounds| bounds.top());
                let header = app
                    .update(cx, |view, cx| {
                        view.group_header(&headers[group], true, cx)
                            .into_any_element()
                    })
                    .ok()?;
                Some((header, next_top))
            }));
        let hidden = self
            .files
            .iter()
            .filter(|file| self.file_hidden(file))
            .count();
        div()
            .w_full()
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .when(self.sidebars_swapped, |sidebar| sidebar.border_l_1())
            .when(!self.sidebars_swapped, |sidebar| sidebar.border_r_1())
            .border_color(cx.theme().border)
            .child(
                div()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Input::new(&self.filter).small())
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .when(grouped, |row| {
                                row.child(
                                    Button::new("toggle-all-groups")
                                        .ghost()
                                        .xsmall()
                                        .label(if all_groups_collapsed {
                                            "Expand groups"
                                        } else {
                                            "Collapse groups"
                                        })
                                        .accessibility_label(if all_groups_collapsed {
                                            "Expand all matching groups"
                                        } else {
                                            "Collapse all matching groups"
                                        })
                                        .disabled(buckets.is_empty())
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            for bucket in file_groups::buckets(
                                                &view.files,
                                                &view.query,
                                                &view.groups,
                                            ) {
                                                if all_groups_collapsed {
                                                    view.collapsed_groups.remove(&bucket.key);
                                                } else {
                                                    view.collapsed_groups.insert(bucket.key);
                                                }
                                            }
                                            view.sync_diff_files(cx);
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("toggle-all-files")
                                    .ghost()
                                    .xsmall()
                                    .label(if all_files_collapsed {
                                        "Expand files"
                                    } else {
                                        "Collapse files"
                                    })
                                    .accessibility_label(if all_files_collapsed {
                                        "Expand all matching file diffs"
                                    } else {
                                        "Collapse all matching file diffs"
                                    })
                                    .disabled(matching_files.is_empty())
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        for file in view.files.iter().filter(|file| {
                                            file_groups::matches_filter(file, &view.query)
                                        }) {
                                            if all_files_collapsed {
                                                view.collapsed_files.remove(file.path().as_str());
                                            } else {
                                                view.collapsed_files
                                                    .insert(file.path().to_string());
                                            }
                                        }
                                        view.sync_collapsed_files(cx);
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("toggle-grouping")
                                    .small()
                                    .selected(self.group_config.enabled)
                                    .disabled(self.group_path.is_none())
                                    .label(if self.group_config.enabled {
                                        "Grouping on"
                                    } else {
                                        "Grouping off"
                                    })
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        let mut config = view.group_config.clone();
                                        config.enabled = !config.enabled;
                                        view.save_groups(config, cx);
                                    })),
                            )
                            .child(
                                Button::new("edit-groups")
                                    .small()
                                    .label("Groups…")
                                    .disabled(self.group_path.is_none())
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.edit_groups(window, cx)
                                    })),
                            )
                            .when(hidden > 0, |view| {
                                view.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("{hidden} hidden")),
                                )
                            }),
                    ),
            )
            .when(self.group_editor.is_none(), |view| {
                view.when_some(self.group_error.clone(), |view, error| {
                    view.child(
                        div()
                            .px_2()
                            .text_xs()
                            .text_color(rgb(0xe89090))
                            .child(error),
                    )
                })
            })
            .when_some(self.viewed_error.clone(), |view, error| {
                view.child(
                    div()
                        .px_2()
                        .text_xs()
                        .text_color(rgb(0xe89090))
                        .child(error),
                )
            })
            .child(list)
    }
}

impl Render for ReviewApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let endpoints = self.endpoints().ok().flatten();
        let title = endpoints
            .as_ref()
            .map(|(_, _, label)| label.clone())
            .or_else(|| {
                self.selection
                    .as_ref()
                    .map(|selection| selection.end.clone())
            })
            .unwrap_or_else(|| "Select a branch".into());
        let subtitle = endpoints
            .as_ref()
            .map(|(base, tip, _)| {
                format!(
                    "{} → {}",
                    &base[..8.min(base.len())],
                    &tip[..8.min(tip.len())]
                )
            })
            .unwrap_or_default();
        let totals = FileTotals::from_files(&self.files).label();
        let filtered = FileTotals::from_files(
            self.files
                .iter()
                .filter(|file| file_groups::matches_filter(file, &self.query)),
        );
        let viewed = self.viewed_files.len();
        let filtered_viewed = self
            .files
            .iter()
            .filter(|file| {
                file_groups::matches_filter(file, &self.query)
                    && self.viewed_files.contains(file.path().as_str())
            })
            .count();
        let mut body = div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col();
        if self.loading_diff || (self.loading_repo && self.repo.is_none()) {
            body = body.child(div().p_6().child("Loading…"));
        } else if let Some(error) = &self.error {
            body = body.child(div().p_6().text_color(rgb(0xe89090)).child(error.clone()));
        } else if self.files.is_empty() {
            body = body.child(div().p_6().text_color(cx.theme().muted_foreground).child(
                if self.repo.as_ref().is_some_and(|r| r.stacks.is_empty()) {
                    if self.repo.as_ref().is_some_and(Repository::is_local) {
                        "No local branches with commits yet. Create a commit to start reviewing."
                    } else {
                        "No GitButler stacks found in this repository."
                    }
                } else if self.selection.is_some() {
                    "No changes in this comparison."
                } else {
                    "Open a Git repository to start reviewing."
                },
            ));
        } else if self.files.iter().all(|file| self.file_hidden(file)) {
            body = body.child(div().p_6().text_color(cx.theme().muted_foreground).child(
                "All files are in collapsed groups. Expand a group or turn grouping off to see their diffs.",
            ));
        } else {
            let viewed = self.viewed_files.clone();
            let pending = self.pending_views.clone();
            let ready = self.loaded.is_some();
            let app = cx.entity().downgrade();
            let comment_app = cx.entity().downgrade();
            let annotation_app = cx.entity().downgrade();
            let annotations = self.comment_annotations();
            body = body.child(
                Diff::new(&self.diff)
                    .soft_wrap(self.wrap)
                    .size_full()
                    .annotations(annotations)
                    .on_add_annotation(move |range, window, cx| {
                        let _ = comment_app
                            .update(cx, |view, cx| view.begin_comment(range, window, cx));
                    })
                    .render_annotation(move |annotation, _, cx| {
                        annotation_app
                            .update(cx, |view, cx| view.inline_comments(annotation, cx))
                            .ok()
                            .unwrap_or_else(|| div().into_any_element())
                    })
                    .render_header_prefix(move |file, _, _| {
                        viewed_checkbox(
                            file.path(),
                            viewed.contains(file.path().as_str()),
                            pending.contains(file.path().as_str()),
                            ready,
                            app.clone(),
                        )
                    }),
            );
            let diff = self.diff.clone();
            let app = cx.entity().downgrade();
            body = body.child(sticky_layer(move |_, _, cx| {
                let (file, next_top) = diff.read(cx).sticky_file_header()?;
                let file = file.clone();
                let header = app
                    .update(cx, |view, cx| {
                        view.pinned_file_header(&file, cx).into_any_element()
                    })
                    .ok()?;
                Some((header, next_top))
            }));
        }
        if self.comment_draft.is_some() {
            body = body.child(self.comment_composer(cx));
        }
        body = body.when_some(self.comment_error.clone(), |body, error| {
            body.child(
                div()
                    .px_3()
                    .py_1()
                    .min_w_0()
                    .flex()
                    .flex_shrink_0()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .text_color(rgb(0xe89090))
                            .child(error),
                    )
                    .child(
                        Button::new("dismiss-comment-error")
                            .ghost()
                            .xsmall()
                            .label("Dismiss")
                            .accessibility_label("Dismiss comment error")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.comment_error = None;
                                cx.notify();
                            })),
                    ),
            )
        });
        let status = self.watch_error.clone().unwrap_or_else(|| {
            if self.watcher.is_some() {
                if self.repo.as_ref().is_some_and(Repository::is_local) {
                    "watching git metadata · branch changes since the merge-base".into()
                } else {
                    "watching git metadata · shift-click branches to review a range".into()
                }
            } else {
                "read-only · open a Git repository to start".into()
            }
        });
        div()
            .relative()
            .key_context("Review")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .on_action(cx.listener(|view, _: &Refresh, _, cx| view.refresh(cx)))
            .on_action(cx.listener(|view, _: &OpenJump, window, cx| view.open_jump(window, cx)))
            .on_action(cx.listener(|view, _: &NavigateBack, _, cx| view.navigate_history(false, cx)))
            .on_action(cx.listener(|view, _: &NavigateForward, _, cx| view.navigate_history(true, cx)))
            .on_action(cx.listener(|view, _: &OpenRepository, _, cx| view.open(cx)))
            .on_action(cx.listener(|view, _: &ToggleStats, _, cx| {
                view.show_stats = !view.show_stats;
                cx.notify();
            }))
            .child(
                div()
                    .h(px(52.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(Button::new("history-back").ghost().small().label("←")
                        .accessibility_label("Back to previous file")
                        .tooltip("Back · ⌥← / ⌥[")
                        .disabled(!self.history.can_back())
                        .on_click(cx.listener(|view, _, _, cx| view.navigate_history(false, cx))))
                    .child(Button::new("history-forward").ghost().small().label("→")
                        .accessibility_label("Forward to next file")
                        .tooltip("Forward · ⌥→ / ⌥]")
                        .disabled(!self.history.can_forward())
                        .on_click(cx.listener(|view, _, _, cx| view.navigate_history(true, cx))))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("toggle-branches")
                            .small()
                            .label(if self.show_branches { "Hide branches" } else { "Show branches" })
                            .selected(self.show_branches)
                            .accessibility_label(if self.show_branches { "Hide branch sidebar" } else { "Show branch sidebar" })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.show_branches = !view.show_branches;
                                cx.notify();
                            })),
                    )
                    .child(div().font_weight(FontWeight::BOLD).child(if self.development { "gander dev" } else { "gander" }))
                    .child(Button::new("jump").small().label("Jump… ⌘K")
                        .accessibility_label("Jump to file, branch, or stack")
                        .on_click(cx.listener(|view, _, window, cx| view.open_jump(window, cx))))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.path.display().to_string()),
                    )
                    .child(
                        Button::new("base-branch")
                            .small()
                            .child(div().max_w(px(180.)).overflow_hidden().text_ellipsis().child(format!("Base: {}…", self.repo.as_ref()
                                .map(|repo| repo.target_ref.as_str())
                                .or(self.base_override.as_deref())
                                .filter(|name| !name.is_empty())
                                .map(repository::short_ref)
                                .unwrap_or("auto"))))
                            .accessibility_label("Choose base branch")
                            .disabled(self.loading_repo)
                            .on_click(cx.listener(|view, _, window, cx| view.open_base_picker(window, cx))),
                    )
                    .child(
                        Button::new("open")
                            .small()
                            .label("Open repo…")
                            .on_click(cx.listener(|view, _, _, cx| view.open(cx))),
                    )
                    .child(
                        Button::new("refresh")
                            .small()
                            .label(if self.loading_repo && self.refresh_visible {
                                "Refreshing…"
                            } else {
                                "Refresh ⌘R"
                            })
                            .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .on_drag_move::<BranchSidebarDrag>(cx.listener(|view, event: &DragMoveEvent<BranchSidebarDrag>, window, cx| {
                        let maximum = (window.viewport_size().width - px(540.)).clamp(px(200.), px(600.));
                        view.branch_sidebar_width = event.event.position.x.clamp(px(200.), maximum);
                        cx.notify();
                    }))
                    .when(self.show_branches, |row| {
                        let maximum = (window.viewport_size().width - px(540.)).clamp(px(200.), px(600.));
                        row.child(
                            div().relative().h_full().w(self.branch_sidebar_width.min(maximum)).flex_shrink_0()
                                .child(self.sidebar(cx))
                                .child(resize_handle("branch-sidebar-resize", Axis::Horizontal)
                                    .inside(HandleEdge::Trailing)
                                    .on_drag(BranchSidebarDrag, |drag, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.new(|_| drag.clone())
                                    })),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .child(title),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .flex()
                                                    .flex_wrap()
                                                    .gap_x_3()
                                                    .child(format!("{subtitle}  ·  {totals}"))
                                                    .child(div().text_color(rgb(0x72cc91)).child(format!("{viewed}/{} viewed", self.files.len())))
                                                    .when(!self.query.is_empty(), |view| {
                                                        view.child(
                                                            div().text_color(rgb(0x81b9ed)).child(
                                                                format!(
                                                                    "Filtered: {} · {filtered_viewed}/{} viewed",
                                                                    filtered.label(), filtered.files
                                                                ),
                                                            ),
                                                        )
                                                    }),
                                            ),
                                    )
                                    .child(
                                        Button::new("mode")
                                            .small()
                                            .label(if self.mode == DiffMode::Unified {
                                                "Split view"
                                            } else {
                                                "Unified view"
                                            })
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.mode = if view.mode == DiffMode::Unified {
                                                    DiffMode::Split
                                                } else {
                                                    DiffMode::Unified
                                                };
                                                view.diff
                                                    .update(cx, |s, cx| s.set_mode(view.mode, cx));
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("comments")
                                            .small()
                                            .selected(self.show_comments)
                                            .label(format!("Comments ({})", self.comments.iter().filter(|t| !t.resolved).count()))
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.capture_panel_widths(cx);
                                                view.show_comments = !view.show_comments;
                                                view.reset_panel_layout(cx);
                                                view.reload_comments(cx);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("swap-sidebars")
                                            .small()
                                            .label("Swap sidebars")
                                            .selected(self.sidebars_swapped)
                                            .accessibility_label("Swap file and comment sidebars")
                                            .tooltip(if self.sidebars_swapped { "Move files left and comments right" } else { "Move comments left and files right" })
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.capture_panel_widths(cx);
                                                view.sidebars_swapped = !view.sidebars_swapped;
                                                view.reset_panel_layout(cx);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("wrap")
                                            .small()
                                            .label(if self.wrap { "Unwrap" } else { "Wrap lines" })
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.wrap = !view.wrap;
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .when(self.group_editor.is_some(), |view| {
                                view.child(self.group_editor_panel(cx))
                            })
                            .child(
                                div().flex_1().min_h_0().child(
                                    self.review_panels(body.into_any_element(), cx),
                                ),
                            ),
                    ),
            )
            .when(self.show_stats, |view| {
                view.child(gpui_fps::fps_monitor(window, cx))
            })
            .child(
                div()
                    .h(px(28.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(status)
                    .child(
                        self.repo
                            .as_ref()
                            .map(|r| r.source.clone())
                            .unwrap_or_default(),
                    ),
            )
    }
}

fn main() -> anyhow::Result<()> {
    let mut path = None;
    let mut base = None;
    let mut inspect = false;
    let mut serve_mcp = false;
    let mut author = None;
    let mut background = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--base" => {
                base = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--base requires a git ref"))?,
                )
            }
            "--inspect" => inspect = true,
            "--background" => background = true,
            "--mcp" => serve_mcp = true,
            "--author" => {
                author = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--author requires a name"))?,
                )
            }
            "--help" | "-h" => {
                println!(
                    "gander [REPOSITORY] [--base REF] [--background | --inspect | --mcp [--author NAME]]\n\nNative Git branch and GitButler stack reviews.\nClick a branch; in GitButler repos, shift-click another for a range or click a stack header for the whole stack.\n--base selects the comparison base; the GUI also has a base-branch picker.\n--background opens a window without activating the app.\n--inspect prints repository/branch endpoints as JSON without opening a window.\n--mcp runs the local review-comment MCP server over stdio without opening a window.\n--author labels MCP comments; defaults to the connected client's name."
                );
                return Ok(());
            }
            _ if arg.starts_with('-') => anyhow::bail!("Unknown option: {arg}"),
            _ => {
                if path.is_some() {
                    anyhow::bail!("Only one repository path is accepted");
                }
                path = Some(PathBuf::from(arg));
            }
        }
    }
    let path = path
        .or_else(|| {
            if inspect || serve_mcp {
                return None;
            }
            session::state_path().and_then(|path| session::Session::load(&path).ok()?.last_repo)
        })
        .unwrap_or(std::env::current_dir()?);
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    if inspect && serve_mcp {
        anyhow::bail!("Choose either --inspect or --mcp");
    }
    if author.is_some() && !serve_mcp {
        anyhow::bail!("--author is only valid with --mcp");
    }
    if background && (serve_mcp || inspect) {
        anyhow::bail!("--background is only valid for the GUI");
    }
    if serve_mcp {
        Repository::load(&path, base.as_deref())?;
        return mcp::Server::new(path, base, author).run();
    }
    if inspect {
        let repo = Repository::load(&path, base.as_deref())?;
        println!("{}", serde_json::to_string_pretty(&repo)?);
        return Ok(());
    }
    let development = launch::is_development();
    let background = background || development;
    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.bind_keys([
            KeyBinding::new("cmd-r", Refresh, Some("Review")),
            KeyBinding::new("cmd-k", OpenJump, Some("Review")),
            KeyBinding::new("alt-left", NavigateBack, Some("Review")),
            KeyBinding::new("alt-right", NavigateForward, Some("Review")),
            KeyBinding::new("alt-[", NavigateBack, Some("Review")),
            KeyBinding::new("alt-]", NavigateForward, Some("Review")),
            KeyBinding::new("cmd-[", NavigateBack, Some("Review")),
            KeyBinding::new("cmd-]", NavigateForward, Some("Review")),
            KeyBinding::new("ctrl-r", Refresh, Some("Review")),
            KeyBinding::new("cmd-o", OpenRepository, Some("Review")),
            KeyBinding::new("cmd-shift-p", ToggleStats, Some("Review")),
            KeyBinding::new("ctrl-shift-p", ToggleStats, Some("Review")),
            KeyBinding::new("escape", InputEscape, Some("CommentComposer")),
            KeyBinding::new("cmd-enter", SubmitComment, Some("CommentComposer")),
            KeyBinding::new("ctrl-enter", SubmitComment, Some("CommentComposer")),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        open_window(
            WindowOptions {
                focus: !background,
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(1000.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(if development { "gander dev" } else { "gander" }.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            move |window, cx| cx.new(|cx| ReviewApp::new(path, base, window, cx)),
        )
        .expect("Couldn't open app window");
        if !background {
            cx.activate(true);
        }
    });
    Ok(())
}
