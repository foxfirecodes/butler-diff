use super::*;
use gpui_kit::component::WindowExt;

impl ReviewApp {
    pub(super) fn new_window(&mut self, cx: &mut Context<Self>) {
        self.snapshot_session(cx);
        if let Err(error) =
            open_review_window(self.path.clone(), self.base_override.clone(), false, cx)
        {
            self.error = Some(format!("Couldn't open a window: {error:#}"));
            cx.notify();
        }
    }

    pub(super) fn open_mcp_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let executable = match std::env::current_exe() {
            Ok(path) => path,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let project = self
            .repo
            .as_ref()
            .map(|repo| &repo.root)
            .unwrap_or(&self.path)
            .display()
            .to_string();
        let commands: anyhow::Result<Vec<_>> =
            [mcp_setup::Client::Codex, mcp_setup::Client::Claude]
                .into_iter()
                .map(|client| Ok((client, mcp_setup::command(&executable, client)?)))
                .collect();
        let commands = match commands {
            Ok(commands) => commands,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        window.open_dialog(cx, move |dialog, _, _| {
            let commands = commands.clone();
            let project = project.clone();
            dialog.title("Set up Gander MCP").max_w(px(720.)).content(move |content, _, cx| {
                let mut content = content.gap_3().child("Run a command from this project's directory:")
                    .child(div().text_color(cx.theme().muted_foreground).child(project.clone()));
                for (client, command) in &commands {
                    let command = command.clone();
                    let copy = command.clone();
                    content = content.child(div().flex().flex_col().gap_2().p_3().rounded_md().border_1().border_color(cx.theme().border)
                        .child(div().flex().items_center().justify_between().child(client.name())
                            .child(Button::new(SharedString::from(format!("copy-mcp-{}", client.flag())))
                                .small().label("Copy command").accessibility_label(format!("Copy {} MCP setup command", client.name()))
                                .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())))))
                        .child(div().text_xs().whitespace_normal().child(command)));
                }
                content.child("Adds Gander to this project's configuration. Other servers stay intact. Restart the MCP connection afterward; Codex also requires a trusted project.")
            })
        });
    }

    pub(super) fn current_visit(&self, cx: &App) -> Option<navigation::Visit> {
        let selection = self.loaded_selection.clone()?;
        let file = self
            .diff
            .read(cx)
            .sticky_file_header()
            .map(|(file, _)| file.path().to_string())
            .or_else(|| self.focused_file.clone())
            .or_else(|| {
                self.files
                    .iter()
                    .find(|file| !self.file_hidden(file))
                    .map(|file| file.path().to_string())
            })?;
        Some(navigation::Visit { selection, file })
    }

    pub(super) fn reveal_file(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.files.iter().any(|file| file.path().as_str() == path) {
            return;
        }
        let group = file_groups::group_key(path, &self.groups);
        let group_changed = self.collapsed_groups.remove(&group);
        self.collapsed_files.remove(path);
        if group_changed {
            self.sync_diff_files(cx);
        } else {
            self.sync_collapsed_files(cx);
        }
        self.focused_file = Some(path.to_owned());
        self.diff
            .update(cx, |diff, cx| diff.scroll_to_file(path, cx));
        cx.notify();
    }

    pub(super) fn visit_file(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(visit) = self.current_visit(cx) {
            self.history.record(visit);
        }
        self.reveal_file(path, cx);
        if let Some(selection) = self.selection.clone() {
            self.history.record(navigation::Visit {
                selection,
                file: path.to_owned(),
            });
        }
    }

    pub(super) fn navigate_history(&mut self, forward: bool, cx: &mut Context<Self>) {
        while let Some(visit) = self.history.step(forward) {
            let valid = self.repo.as_ref().is_some_and(|repo| {
                repo.stacks.iter().any(|stack| {
                    stack.id == visit.selection.stack
                        && stack
                            .branches
                            .iter()
                            .any(|b| b.name == visit.selection.anchor)
                        && stack.branches.iter().any(|b| b.name == visit.selection.end)
                })
            });
            if !valid {
                continue;
            }
            if self.selection.as_ref() == Some(&visit.selection) && self.loaded.is_some() {
                if !self
                    .files
                    .iter()
                    .any(|file| file.path().as_str() == visit.file)
                {
                    continue;
                }
                self.reveal_file(&visit.file, cx);
            } else {
                self.remember_comparison(cx);
                self.selection = Some(visit.selection.clone());
                self.pending_navigation = Some(visit.clone());
                self.load_diff(cx);
                if self.loaded.is_some() {
                    self.pending_navigation = None;
                    self.reveal_file(&visit.file, cx);
                }
            }
            cx.notify();
            break;
        }
    }

    fn repo_ui_key(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    pub(super) fn saved_comparison(&self) -> Option<session::Comparison> {
        self.session
            .borrow()
            .repositories
            .get(&self.repo_ui_key())?
            .comparisons
            .get(&self.selection.as_ref()?.key())
            .cloned()
    }

    pub(super) fn remember_comparison(&mut self, cx: &App) {
        if self.loaded.is_none() {
            return;
        }
        let Some(selection) = &self.loaded_selection else {
            return;
        };
        let focused_file = self
            .diff
            .read(cx)
            .sticky_file_header()
            .map(|(file, _)| file.path().to_string())
            .or_else(|| self.focused_file.clone());
        let offset = self.file_scroll.offset();
        let comparison = session::Comparison {
            known_files: self
                .files
                .iter()
                .map(|file| file.path().to_string())
                .collect(),
            collapsed_files: self.collapsed_files.clone(),
            viewed_files: (*self.viewed_files).clone(),
            focused_file,
            file_scroll: [offset.x.into(), offset.y.into()],
        };
        let key = selection.key();
        let repo_key = self.repo_ui_key();
        self.session
            .borrow_mut()
            .repositories
            .entry(repo_key)
            .or_default()
            .comparisons
            .insert(key, comparison);
    }

    pub(super) fn snapshot_session(&mut self, cx: &App) -> session::Session {
        self.remember_comparison(cx);
        let repo_key = self.repo_ui_key();
        let mut session = self.session.borrow_mut();
        let state = session.repositories.entry(repo_key).or_default();
        state.selection = self.selection.clone();
        state.base_override = self.base_override.clone();
        state.wrap = self.wrap;
        state.split = self.mode == DiffMode::Split;
        state.show_comments = self.show_comments;
        state.show_branches = self.show_branches;
        state.sidebars_swapped = self.sidebars_swapped;
        state.branch_width = self.branch_sidebar_width.into();
        state.file_width = self.file_panel_width;
        state.comment_width = self.comment_panel_width;
        state.filter = self.filter.read(cx).value().to_string();
        state.collapsed_groups = self.collapsed_groups.clone();
        session.last_repo = Some(self.path.clone());
        session.clone()
    }

    pub(super) fn restore_repo_ui(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self
            .session
            .borrow()
            .repositories
            .get(&self.repo_ui_key())
            .cloned()
            .unwrap_or_default();
        self.selection = state.selection;
        self.base_override = state.base_override;
        self.wrap = state.wrap;
        self.mode = if state.split {
            DiffMode::Split
        } else {
            DiffMode::Unified
        };
        self.show_comments = state.show_comments;
        self.show_branches = state.show_branches;
        self.sidebars_swapped = state.sidebars_swapped;
        self.branch_sidebar_width = px(state.branch_width.clamp(200., 600.));
        self.file_panel_width = state.file_width.clamp(220., 750.);
        self.comment_panel_width = state.comment_width.clamp(260., 650.);
        self.collapsed_groups = state.collapsed_groups;
        self.query = state.filter.to_lowercase();
        self.filter
            .update(cx, |input, cx| input.set_value(state.filter, window, cx));
        self.diff
            .update(cx, |diff, cx| diff.set_mode(self.mode, cx));
        self.focused_file = None;
    }

    pub(super) fn restore_comparison_position(
        &mut self,
        saved: session::Comparison,
        cx: &mut Context<Self>,
    ) {
        self.file_scroll
            .set_offset(point(px(saved.file_scroll[0]), px(saved.file_scroll[1])));
        self.focused_file = saved
            .focused_file
            .filter(|path| self.files.iter().any(|file| file.path().as_str() == path));
        if let Some(path) = self.focused_file.clone() {
            // Preserve the saved collapse state; restoring a position isn't a review action.
            self.diff
                .update(cx, |diff, cx| diff.scroll_to_file(&path, cx));
        }
    }

    pub(super) fn schedule_session_save(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.session_store.clone() else {
            return;
        };
        self.session_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;
            let Ok(session) = this.update(cx, |view, cx| view.snapshot_session(cx)) else {
                return;
            };
            let sequence = store.next();
            cx.background_spawn(async move {
                if let Err(error) = store.save(&session, sequence) {
                    eprintln!("Couldn't save UI state: {error:#}");
                }
            })
            .await;
        }));
    }

    pub(super) fn save_session_now(&mut self, cx: &App) {
        self.session_task = None;
        if let Some(store) = self.session_store.clone() {
            let session = self.snapshot_session(cx);
            if let Err(error) = store.save(&session, store.next()) {
                eprintln!("Couldn't save UI state: {error:#}");
            }
        }
    }
}
