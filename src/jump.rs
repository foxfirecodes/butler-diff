use super::*;
use gpui_kit::component::{
    WindowExt,
    command::{Command, CommandGroup, CommandItem, CommandState},
};

#[derive(Clone)]
enum Target {
    File(String),
    Branch(String, String),
    Stack(Selection),
}

impl ReviewApp {
    pub(super) fn open_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let mut sections: Vec<(String, Vec<(String, Target)>)> = vec![
            (
                "Files in current selection".into(),
                self.files
                    .iter()
                    .map(|file| {
                        let path = file.path().to_string();
                        let (directory, filename) = path.rsplit_once('/').unwrap_or(("", &path));
                        let label = if directory.is_empty() {
                            filename.to_owned()
                        } else {
                            format!("{filename} — {directory}")
                        };
                        (label, Target::File(path))
                    })
                    .collect(),
            ),
            ("Branches".into(), vec![]),
            ("Stacks".into(), vec![]),
        ];
        if let Some(repo) = &self.repo {
            for (index, stack) in repo.stacks.iter().enumerate() {
                for branch in &stack.branches {
                    let review = branch.review.as_deref().unwrap_or("");
                    sections[1].1.push((
                        format!("{} · stack {} {review}", branch.name, index + 1),
                        Target::Branch(stack.id.clone(), branch.name.clone()),
                    ));
                }
                if let (Some(first), Some(last)) = (stack.branches.first(), stack.branches.last()) {
                    sections[2].1.push((
                        format!(
                            "Stack {} · {} → {} · {} branches",
                            index + 1,
                            first.name,
                            last.name,
                            stack.branches.len()
                        ),
                        Target::Stack(Selection {
                            stack: stack.id.clone(),
                            anchor: first.name.clone(),
                            end: last.name.clone(),
                        }),
                    ));
                }
            }
        }
        let state = cx.new(|cx| CommandState::new(window, cx));
        let owner = cx.weak_entity();
        let focus_on_mount = Rc::new(std::cell::Cell::new(true));
        window.open_dialog(cx, move |dialog, _, _| {
            let state = state.clone();
            let owner = owner.clone();
            let sections = sections.clone();
            let focus_on_mount = focus_on_mount.clone();
            dialog
                .close_button(true)
                .p_0()
                .max_w(px(760.))
                .content(move |content, window, cx| {
                    if focus_on_mount.replace(false) {
                        let state = state.clone();
                        window.defer(cx, move |window, cx| {
                            state.read(cx).focus_handle(cx).focus(window, cx)
                        });
                    }
                    let targets = sections.clone();
                    let confirm_owner = owner.clone();
                    let mut command = Command::new(&state)
                        .bordered(false)
                        .placeholder("Jump to a file, branch, or stack…")
                        .max_h(px(460.))
                        .on_confirm(move |index, window, cx| {
                            let Some((_, target)) = targets
                                .get(index.section)
                                .and_then(|(_, entries)| entries.get(index.row))
                            else {
                                return;
                            };
                            let target = target.clone();
                            window.close_dialog(cx);
                            let _ = confirm_owner.update(cx, |view, cx| {
                                match target {
                                    Target::File(path) => {
                                        if !view.query.is_empty()
                                            && !path.to_lowercase().contains(&view.query)
                                        {
                                            view.query.clear();
                                            view.filter.update(cx, |input, cx| {
                                                input.set_value("", window, cx)
                                            });
                                        }
                                        view.visit_file(&path, cx);
                                    }
                                    Target::Branch(stack, branch) => {
                                        view.select(stack, branch, false, cx)
                                    }
                                    Target::Stack(selection) => {
                                        view.remember_comparison(cx);
                                        view.selection = Some(selection);
                                        view.load_diff(cx);
                                    }
                                }
                                window.focus(&view.focus, cx);
                                cx.notify();
                            });
                        });
                    for (heading, entries) in &sections {
                        command = command.group(CommandGroup::new().label(heading.clone()).items(
                            entries.iter().map(|(label, target)| {
                                let keywords = match target {
                                    Target::File(path) => path.clone(),
                                    _ => label.clone(),
                                };
                                CommandItem::new().label(label.clone()).keywords([keywords])
                            }),
                        ));
                    }
                    content.child(command)
                })
        });
    }
}
