use super::*;

impl ReviewApp {
    pub(super) fn reload_comments(&mut self, cx: &mut Context<Self>) {
        self.comment_generation += 1;
        let generation = self.comment_generation;
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let common_dir = repo.common_dir;
        let expected_dir = common_dir.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { comments::list(&common_dir.join(comments::STATE_FILE)) },
                )
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.comment_generation != generation
                    || view
                        .repo
                        .as_ref()
                        .is_none_or(|repo| repo.common_dir != expected_dir)
                {
                    return;
                }
                match result {
                    Ok(threads) => {
                        view.comments = threads;
                        view.sync_comment_anchors();
                    }
                    Err(error) => {
                        view.comment_error = Some(format!("Couldn't load comments: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn sync_comment_anchors(&mut self) {
        self.current_comment_ids.clear();
        let Some((_, base, tip)) = &self.loaded else {
            return;
        };
        let files: HashMap<_, _> = self
            .files
            .iter()
            .map(|file| (file.path().as_str(), file))
            .collect();
        for thread in &self.comments {
            if files
                .get(thread.anchor.location.path.as_str())
                .is_some_and(|file| comments::matches(thread, base, tip, file))
            {
                self.current_comment_ids.insert(thread.id.clone());
            }
        }
    }

    fn thread_current(&self, thread: &comments::Thread) -> bool {
        self.current_comment_ids.contains(&thread.id)
    }

    pub(super) fn begin_comment(
        &mut self,
        range: &DiffLineRange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.comment_pending {
            return;
        }
        let Some((_, base, tip)) = self.loaded.clone() else {
            return;
        };
        if range.side() != range.end_side() {
            self.comment_error = Some("Select lines on one side of the diff to comment.".into());
            cx.notify();
            return;
        }
        let target = CommentTarget::New(comments::AnchorRequest {
            base,
            tip,
            path: range.path().to_string(),
            side: range.side().into(),
            start_line: range.start().min(range.end()),
            end_line: range.start().max(range.end()),
        });
        self.start_comment_draft(target, window, cx);
    }

    fn start_comment_draft(
        &mut self,
        target: CommentTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.comment_pending {
            return;
        }
        let Some(repo) = &self.repo else { return };
        let key = CommentDraftKey {
            repository: repo.common_dir.clone(),
            target,
        };
        if let Some(draft) = &self.comment_draft
            && draft.key == key
        {
            draft.input.update(cx, |state, cx| state.focus(window, cx));
            return;
        }
        self.stash_comment_draft(cx);
        let saved = self
            .comment_drafts
            .get(&key)
            .cloned()
            .or_else(|| match &key.target {
                CommentTarget::Edit(id) => self
                    .comments
                    .iter()
                    .flat_map(|thread| &thread.messages)
                    .find(|message| &message.id == id)
                    .map(|message| message.body.clone()),
                _ => None,
            });
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(3)
                .placeholder("Leave a review comment…")
        });
        input.update(cx, |state, cx| {
            if let Some(saved) = saved {
                state.set_value(saved, window, cx);
            }
            state.focus(window, cx);
        });
        self.comment_draft = Some(CommentDraft { key, input });
        self.comment_error = None;
        cx.notify();
    }

    pub(super) fn stash_comment_draft(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = self.comment_draft.take() {
            let body = draft.input.read(cx).value().to_string();
            if body.trim().is_empty() {
                self.comment_drafts.remove(&draft.key);
            } else {
                self.comment_drafts.insert(draft.key, body);
            }
        }
    }

    fn dismiss_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stash_comment_draft(cx);
        self.comment_error = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn post_comment(&mut self, cx: &mut Context<Self>) {
        if self.comment_pending {
            return;
        }
        let (Some(draft), Some(repo)) = (&self.comment_draft, self.repo.clone()) else {
            return;
        };
        let key = draft.key.clone();
        let target = key.target.clone();
        let body = draft.input.read(cx).value().to_string();
        if body.trim().is_empty() {
            self.comment_error = Some("Write a comment first.".into());
            cx.notify();
            return;
        }
        self.comment_pending = true;
        self.comment_error = None;
        let common_dir = repo.common_dir.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let author = repository::git(&repo.root, &["config", "user.name"])
                        .ok()
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or_else(|| "You".into());
                    let path = repo.common_dir.join(comments::STATE_FILE);
                    match target {
                        CommentTarget::New(request) => comments::create(
                            &path,
                            &comments::anchor(&repo, request)?,
                            author.trim(),
                            &body,
                        ),
                        CommentTarget::Reply(id) => {
                            comments::reply(&path, &id, author.trim(), &body)
                        }
                        CommentTarget::Edit(id) => comments::edit(&path, &id, &body),
                    }
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.comment_pending = false;
                if result.is_ok() {
                    view.comment_drafts.remove(&key);
                    if view
                        .comment_draft
                        .as_ref()
                        .is_some_and(|draft| draft.key == key)
                    {
                        view.comment_draft = None;
                    }
                }
                if view
                    .repo
                    .as_ref()
                    .is_none_or(|repo| repo.common_dir != common_dir)
                {
                    return;
                }
                match result {
                    Ok(_) => {
                        view.reload_comments(cx);
                    }
                    Err(error) => {
                        view.comment_error = Some(format!("Couldn't post comment: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn resolve_comment(&mut self, id: String, resolved: bool, cx: &mut Context<Self>) {
        if self.comment_pending {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let common_dir = repo.common_dir;
        let expected_dir = common_dir.clone();
        self.comment_pending = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    comments::set_resolved(&common_dir.join(comments::STATE_FILE), &id, resolved)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.comment_pending = false;
                if view
                    .repo
                    .as_ref()
                    .is_none_or(|repo| repo.common_dir != expected_dir)
                {
                    return;
                }
                match result {
                    Ok(_) => view.reload_comments(cx),
                    Err(error) => {
                        view.comment_error = Some(format!("Couldn't update thread: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn delete_comment_message(&mut self, id: String, deleted: bool, cx: &mut Context<Self>) {
        if self.comment_pending {
            return;
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let common_dir = repo.common_dir;
        let expected_dir = common_dir.clone();
        self.comment_pending = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    comments::set_deleted(&common_dir.join(comments::STATE_FILE), &id, deleted)
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.comment_pending = false;
                if view
                    .repo
                    .as_ref()
                    .is_none_or(|repo| repo.common_dir != expected_dir)
                {
                    return;
                }
                match result {
                    Ok(_) => view.reload_comments(cx),
                    Err(error) => {
                        view.comment_error = Some(format!("Couldn't update message: {error:#}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn comment_annotations(&self) -> Vec<DiffAnnotation> {
        let mut seen = HashSet::new();
        self.comments
            .iter()
            .filter(|thread| !thread.resolved && self.thread_current(thread))
            .filter_map(|thread| {
                let location = &thread.anchor.location;
                let position = DiffLinePosition::new(
                    location.path.clone(),
                    location.side.into(),
                    location.end_line,
                );
                seen.insert(position.clone()).then(|| {
                    DiffAnnotation::line(
                        SharedString::from(format!("thread:{}", thread.id)),
                        position,
                    )
                })
            })
            .collect()
    }

    pub(super) fn inline_comments(
        &mut self,
        annotation: &DiffAnnotation,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(position) = annotation.position() else {
            return div().into_any_element();
        };
        let threads: Vec<_> = self
            .comments
            .iter()
            .filter(|thread| {
                let loc = &thread.anchor.location;
                !thread.resolved
                    && self.thread_current(thread)
                    && loc.path == position.path().as_str()
                    && loc.side == position.side().into()
                    && loc.end_line == position.line()
            })
            .cloned()
            .collect();
        div()
            .w_full()
            .min_w_0()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .children(
                threads
                    .iter()
                    .map(|thread| self.thread_card(thread, false, cx)),
            )
            .into_any_element()
    }

    fn jump_to_comment(&mut self, thread: &comments::Thread, cx: &mut Context<Self>) {
        if !self.thread_current(thread) {
            self.comment_error = Some("This thread belongs to an older or different file comparison. Its original source is preserved in the thread.".into());
            cx.notify();
            return;
        }
        self.comment_error = None;
        let loc = &thread.anchor.location;
        self.visit_file(&loc.path, cx);
        let position = DiffLinePosition::new(loc.path.clone(), loc.side.into(), loc.end_line);
        self.diff
            .update(cx, |state, cx| state.scroll_to_line(position, cx));
        cx.notify();
    }

    fn thread_card(
        &self,
        thread: &comments::Thread,
        in_panel: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reply_id = thread.id.clone();
        let resolve_id = thread.id.clone();
        let resolved = thread.resolved;
        let current = self.thread_current(thread);
        let loc = &thread.anchor.location;
        let line_label = format!(
            "{} L{}{}",
            if loc.side == comments::Side::Original {
                "old"
            } else {
                "new"
            },
            loc.start_line,
            if loc.end_line == loc.start_line {
                String::new()
            } else {
                format!("–{}", loc.end_line)
            }
        );
        let label = format!("{} · {line_label}", loc.path);
        let jump_thread = thread.clone();
        let (directory, filename) = loc.path.rsplit_once('/').unwrap_or(("", &loc.path));
        div()
            .id(SharedString::from(format!(
                "comment:{}:{in_panel}",
                thread.id
            )))
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .whitespace_normal()
            .when(in_panel, |card| {
                card.child(
                    Button::new("jump")
                        .ghost()
                        .small()
                        .w_full()
                        .min_w_0()
                        .h_auto()
                        .py_1()
                        .tooltip(label)
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .text_left()
                                .whitespace_normal()
                                .child(format!("{filename} · {line_label}"))
                                .when(!directory.is_empty(), |heading| {
                                    heading.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(directory.replace('/', "/\u{200b}")),
                                    )
                                }),
                        )
                        .accessibility_label(format!(
                            "Show comment on {} line {}",
                            loc.path, loc.end_line
                        ))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.jump_to_comment(&jump_thread, cx)
                        })),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{}{} · thread {}",
                        if resolved { "Resolved" } else { "Open" },
                        if !current {
                            " · older/different comparison"
                        } else {
                            ""
                        },
                        &thread.id[..8]
                    )),
            )
            .when(in_panel && !current, |card| {
                card.child(
                    div()
                        .id("source-excerpt")
                        .w_full()
                        .min_w_0()
                        .overflow_x_scroll()
                        .p_2()
                        .text_xs()
                        .font_family("monospace")
                        .whitespace_nowrap()
                        .child(thread.anchor.excerpt.clone()),
                )
            })
            .children(thread.messages.iter().map(|message| {
                let edit_id = message.id.clone();
                let delete_id = message.id.clone();
                let deleted = message.deleted_at.is_some();
                div()
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .child(format!(
                                        "{}{}",
                                        message.author,
                                        if message.edited_at.is_some() {
                                            " · edited"
                                        } else {
                                            ""
                                        }
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_shrink_0()
                                    .gap_1()
                                    .when(!deleted, |controls| {
                                        controls.child(
                                            Button::new(SharedString::from(format!(
                                                "edit:{}",
                                                message.id
                                            )))
                                            .ghost()
                                            .xsmall()
                                            .label("Edit")
                                            .disabled(self.comment_pending)
                                            .accessibility_label(format!(
                                                "Edit message by {}",
                                                message.author
                                            ))
                                            .on_click(cx.listener(move |view, _, window, cx| {
                                                view.start_comment_draft(
                                                    CommentTarget::Edit(edit_id.clone()),
                                                    window,
                                                    cx,
                                                )
                                            })),
                                        )
                                    })
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "delete:{}",
                                            message.id
                                        )))
                                        .ghost()
                                        .xsmall()
                                        .label(if deleted { "Undo delete" } else { "Delete" })
                                        .disabled(self.comment_pending)
                                        .accessibility_label(format!(
                                            "{} message by {}",
                                            if deleted { "Restore" } else { "Delete" },
                                            message.author
                                        ))
                                        .on_click(
                                            cx.listener(move |view, _, _, cx| {
                                                view.delete_comment_message(
                                                    delete_id.clone(),
                                                    !deleted,
                                                    cx,
                                                )
                                            }),
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .whitespace_normal()
                            .text_sm()
                            .child(if deleted {
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Message deleted")
                                    .into_any_element()
                            } else {
                                TextView::markdown(
                                    SharedString::from(format!("message-body:{}", message.id)),
                                    message.body.clone(),
                                )
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
                                .into_any_element()
                            }),
                    )
            }))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("reply")
                            .small()
                            .label("Reply")
                            .disabled(self.comment_pending)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.start_comment_draft(
                                    CommentTarget::Reply(reply_id.clone()),
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("resolve")
                            .ghost()
                            .small()
                            .label(if resolved { "Reopen" } else { "Resolve" })
                            .disabled(self.comment_pending)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.resolve_comment(resolve_id.clone(), !resolved, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn comments_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w_full()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child("Repository comments")
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Hover a diff line and click + to start a thread."),
                    ),
            )
            .child(
                div()
                    .id("review-threads")
                    .w_full()
                    .min_w_0()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(self.comments.is_empty(), |panel| {
                        panel.child(div().p_3().child("No comments yet."))
                    })
                    .children(
                        self.comments
                            .iter()
                            .rev()
                            .map(|thread| self.thread_card(thread, true, cx)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn comment_composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(draft) = &self.comment_draft else {
            return div().into_any_element();
        };
        let label = match &draft.key.target {
            CommentTarget::New(loc) => format!(
                "Comment · {} · {:?} L{}–{}",
                loc.path, loc.side, loc.start_line, loc.end_line
            ),
            CommentTarget::Reply(id) => format!("Reply to thread {}", &id[..8]),
            CommentTarget::Edit(id) => format!("Edit message {}", &id[..8]),
        };
        div()
            .id("comment-composer")
            .key_context("CommentComposer")
            .capture_action(cx.listener(|view, enter: &InputEnter, _, cx| {
                if enter.secondary {
                    cx.stop_propagation();
                    view.post_comment(cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(
                cx.listener(|view, _: &InputEscape, window, cx| view.dismiss_comment(window, cx)),
            )
            .on_action(cx.listener(|view, _: &SubmitComment, _, cx| view.post_comment(cx)))
            .p_3()
            .flex_shrink_0()
            .border_t_1()
            .border_color(cx.theme().border)
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_xs().child(label))
            .child(
                Textarea::new(&draft.input)
                    .disabled(self.comment_pending)
                    .aria_label("Review comment"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("post-comment")
                            .primary()
                            .small()
                            .disabled(self.comment_pending)
                            .label(if self.comment_pending {
                                "Posting…"
                            } else {
                                if matches!(draft.key.target, CommentTarget::Edit(_)) {
                                    "Save edit ⌘↵"
                                } else {
                                    "Post comment ⌘↵"
                                }
                            })
                            .on_click(cx.listener(|view, _, _, cx| view.post_comment(cx))),
                    )
                    .child(
                        Button::new("cancel-comment")
                            .ghost()
                            .small()
                            .label("Close · Esc")
                            .tooltip("Close and keep a temporary draft for this line or message")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.dismiss_comment(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }
}
