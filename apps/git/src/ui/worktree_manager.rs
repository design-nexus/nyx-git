use super::{window::current, workflow::confirm};
use gtk::{gio, prelude::*};
use nyx_git::core::{
    advanced,
    operations::{self, Operation},
    worktrees::{self, Action, State},
};
use nyx_ui::widgets;
use std::path::PathBuf;
fn error(e: impl std::fmt::Display) {
    if let Some(ui) = current() {
        ui.toast(&e.to_string());
    }
}
/// A failure from Git or a provider: shown until dismissed.
fn fail(e: impl std::fmt::Display) {
    if let Some(ui) = current() {
        ui.error(&e.to_string());
    }
}
fn run(root: PathBuf, action: Action) {
    if let Some(ui) = current() {
        let removed = if let Action::Remove { expected, .. } = &action { Some(expected.path.clone()) } else { None };
        ui.execute_at_then(root, Operation::Worktree(action), move |result| {
            if result.is_ok() {
                if let Some(path) = removed
                    && let Some(ui) = current()
                {
                    ui.close_repository(&path);
                }
                show();
            }
        });
    }
}
pub fn show() {
    let Some(root) = current().and_then(|ui| ui.root()) else {
        error("Open a repository first.");
        return;
    };
    nyx_core::cmd::background(
        move || worktrees::overview(&root).map(|entries| (root, entries)),
        move |result| {
            let (root, entries) = match result {
                Ok(v) => v,
                Err(e) => {
                    fail(format!("{e:#}"));
                    return;
                }
            };
            if current().and_then(|ui| ui.root()).as_ref() != Some(&root) {
                return;
            }
            let (dialog, body) = widgets::dialog("Git worktrees", 740);
            let controls = widgets::hbox(6);
            let create = gtk::Button::with_label("Create worktree…");
            let r = root.clone();
            let d = dialog.clone();
            create.connect_clicked(move |_| {
                d.close();
                create_dialog(r.clone());
            });
            controls.append(&create);
            let inventory = entries.iter().map(|e| e.worktree.clone()).collect::<Vec<_>>();
            let candidates = inventory.iter().filter(|w| w.prunable.is_some() && w.locked.is_none()).collect::<Vec<_>>();
            let prune = gtk::Button::with_label("Prune stale…");
            prune.set_sensitive(!candidates.is_empty());
            let description = format!(
                "Remove stale Git registration metadata for these missing worktrees:\n{}\nNo checkout directories or branch refs are deleted. Locked worktrees are preserved. Verify disconnected drives are not simply unmounted.",
                candidates.iter().map(|w| w.path.to_string_lossy()).collect::<Vec<_>>().join("\n")
            );
            let r = root.clone();
            let d = dialog.clone();
            prune.connect_clicked(move |_| {
                d.close();
                let r = r.clone();
                let expected = inventory.clone();
                confirm("Prune stale worktrees?", &description, "Prune metadata", move || {
                    run(r.clone(), Action::Prune { expected: expected.clone() })
                });
            });
            controls.append(&prune);
            body.append(&controls);
            let rows = widgets::vbox(0);
            rows.add_css_class("group-list");
            for entry in entries {
                let w = entry.worktree;
                let status = match &entry.state {
                    State::Clean => "Clean".into(),
                    State::Dirty(n) => format!("Dirty · {n} changed files"),
                    State::Ignored => "Ignored files present · removal blocked".into(),
                    State::Missing => "Missing directory".into(),
                    State::Unavailable(e) => format!("Status unavailable: {e}"),
                    State::Bare => "Bare repository".into(),
                };
                let description = format!(
                    "{}\n{status}{}{}{}",
                    w.path.display(),
                    if w.main { " · main worktree" } else { "" },
                    w.locked.as_ref().map(|r| format!("\nLocked: {r}")).unwrap_or_default(),
                    w.prunable.as_ref().map(|r| format!("\nStale: {r}")).unwrap_or_default()
                );
                let actions = gtk::FlowBox::new();
                actions.set_selection_mode(gtk::SelectionMode::None);
                actions.set_max_children_per_line(2);
                if !matches!(entry.state, State::Missing | State::Bare | State::Unavailable(_)) {
                    let open = gtk::Button::with_label("Open tab");
                    let path = w.path.clone();
                    let d = dialog.clone();
                    open.connect_clicked(move |_| {
                        d.close();
                        if let Some(ui) = current() {
                            ui.open(path.clone());
                        }
                    });
                    actions.insert(&open, -1);
                    let folder = gtk::Button::with_label("Folder");
                    let path = w.path.clone();
                    folder.connect_clicked(move |_| super::workspace::external(path.clone(), "folder"));
                    actions.insert(&folder, -1);
                }
                if !w.main {
                    let lock = gtk::Button::with_label(if w.locked.is_some() { "Unlock…" } else { "Lock…" });
                    let r = root.clone();
                    let expected = w.clone();
                    let d = dialog.clone();
                    lock.connect_clicked(move |_|{
                    d.close();let r=r.clone();let expected=expected.clone();
                    if expected.locked.is_some(){confirm("Unlock worktree?","Allow this checkout's registration to be pruned or removed again. Only unlock a disconnected worktree when its location is understood.","Unlock",move||run(r.clone(),Action::Unlock{expected:expected.clone()}));}
                    else{widgets::ask_text("Lock worktree","Prevent removal and pruning while this checkout is locked. Enter a reason.","","Lock",move|reason|run(r.clone(),Action::Lock{expected:expected.clone(),reason}));}
                });
                    actions.insert(&lock, -1);
                    let remove = gtk::Button::with_label("Remove…");
                    remove.set_sensitive(matches!(entry.state, State::Clean) && w.locked.is_none() && w.path != root);
                    let r = root.clone();
                    let expected = w.clone();
                    let d = dialog.clone();
                    remove.connect_clicked(move |_|{
                    d.close();let r=r.clone();let expected=expected.clone();
                    nyx_core::cmd::background(move||worktrees::removal_guard(&r,&expected).map(|guard|(r,expected,guard)),move|result|match result{Ok((r,expected,guard))=>super::preview::confirm(r.clone(),nyx_git::core::preview::Subject::RemoveWorktree(expected.path.clone()),"Remove clean worktree?".into(),format!("Delete the checkout directory {} and its Git worktree registration. The associated branch remains. Dirty, ignored, locked and externally changed worktrees are refused.",expected.path.display()),"Remove worktree".into(),true,move||run(r.clone(),Action::Remove{expected:expected.clone(),guard:Box::new(guard.clone())})),Err(e)=>fail(format!("{e:#}"))});
                });
                    actions.insert(&remove, -1);
                }
                let name = w.branch.as_deref().map(|b| b.trim_start_matches("refs/heads/")).unwrap_or(if w.bare {
                    "Bare repository"
                } else {
                    "Detached HEAD"
                });
                let title = name.chars().take(48).collect::<String>();
                rows.append(&widgets::row(&title, &gtk::glib::markup_escape_text(&description), Some(actions.upcast_ref())));
            }
            let scroll = gtk::ScrolledWindow::builder()
                .child(&rows)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vscrollbar_policy(super::scrollbars::policy())
                .min_content_height(280)
                .max_content_height(500)
                .propagate_natural_height(true)
                .build();
            body.append(&scroll);
            let close = gtk::Button::with_label("Close");
            close.set_halign(gtk::Align::End);
            let d = dialog.clone();
            close.connect_clicked(move |_| d.close());
            body.append(&close);
            dialog.present();
        },
    );
}
/// Remove the worktree at `path` after the usual guards and a preview, from outside the manager.
pub fn remove_at(root: PathBuf, path: PathBuf) {
    nyx_core::cmd::background(
        move || -> anyhow::Result<_> {
            let expected = worktrees::list(&root)?.into_iter().find(|w| w.path == path).ok_or_else(|| anyhow::anyhow!("The worktree is no longer registered"))?;
            let guard = worktrees::removal_guard(&root, &expected)?;
            Ok((root, expected, guard))
        },
        move |result| match result {
            Ok((root, expected, guard)) => super::preview::confirm(
                root.clone(),
                nyx_git::core::preview::Subject::RemoveWorktree(expected.path.clone()),
                "Remove clean worktree?".into(),
                format!("Delete the checkout directory {} and its Git worktree registration. The associated branch remains.", expected.path.display()),
                "Remove worktree".into(),
                true,
                move || {
                    let Some(ui) = current() else { return };
                    let path = expected.path.clone();
                    ui.execute_at_then(
                        root.clone(),
                        Operation::Worktree(Action::Remove { expected: expected.clone(), guard: Box::new(guard.clone()) }),
                        move |result| {
                            if result.is_ok()
                                && let Some(ui) = current()
                            {
                                ui.close_repository(&path);
                            }
                        },
                    );
                },
            ),
            Err(e) => fail(format!("{e:#}")),
        },
    );
}
fn create_dialog(root: PathBuf) {
    create_for(root, None, None);
}
/// Create a worktree: for an existing `branch`, or a new branch from `base`.
pub fn create_for(root: PathBuf, branch_name: Option<String>, base_target: Option<String>) {
    nyx_core::cmd::background(
        move || operations::run(&root, Operation::Branches).map(|branches| (root, branches)),
        move |result| {
            let (root, branches) = match result {
                Ok(v) => v,
                Err(e) => {
                    fail(format!("{e:#}"));
                    return;
                }
            };
            let (dialog, body) = widgets::dialog("Create worktree", 620);
            let path = gtk::Entry::new();
            path.set_placeholder_text(Some("Absolute destination path · must not exist"));
            let location = widgets::hbox(6);
            path.set_hexpand(true);
            location.append(&path);
            let browse = gtk::Button::with_label("Choose parent…");
            let d = dialog.clone();
            let p = path.clone();
            browse.connect_clicked(move |_| {
                let picker = gtk::FileDialog::builder().title("Parent directory for the new worktree").modal(true).build();
                let p = p.clone();
                picker.select_folder(Some(&d), gio::Cancellable::NONE, move |result| {
                    if let Ok(folder) = result
                        && let Some(parent) = folder.path()
                    {
                        p.set_text(&parent.join("new-worktree").to_string_lossy());
                    }
                });
            });
            location.append(&browse);
            body.append(&location);
            let new_branch = gtk::CheckButton::with_label("Create a new branch");
            new_branch.set_active(branch_name.is_none());
            // Suggest a sibling folder of the main checkout, named after the branch.
            let slug = |b: &str| b.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect::<String>();
            if let Some(b) = &branch_name
                && let Some(parent) = root.parent()
            {
                let name = root.file_name().unwrap_or_default().to_string_lossy().into_owned();
                path.set_text(&parent.join(format!("{name}-{}", slug(b))).to_string_lossy());
            }
            body.append(&new_branch);
            let branch = gtk::Entry::new();
            branch.set_placeholder_text(Some("New branch name"));
            body.append(&branch);
            let names = String::from_utf8_lossy(&branches).lines().map(String::from).collect::<Vec<_>>();
            let existing = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
            super::compact_dropdown(&existing);
            existing.set_visible(branch_name.is_some());
            if let Some(i) = branch_name.as_ref().and_then(|b| names.iter().position(|n| n == b)) {
                existing.set_selected(i as u32);
            }
            body.append(&existing);
            let base = gtk::Entry::new();
            base.set_placeholder_text(Some("Start from branch, tag, full hash or HEAD"));
            base.set_text(base_target.as_deref().unwrap_or("HEAD"));
            body.append(&base);
            branch.set_visible(branch_name.is_none());
            base.set_visible(branch_name.is_none());
            let b = branch.clone();
            let e = existing.clone();
            let start = base.clone();
            new_branch.connect_toggled(move |c| {
                b.set_visible(c.is_active());
                start.set_visible(c.is_active());
                e.set_visible(!c.is_active());
            });
            let note = widgets::label(
                "Creates a separate checkout and opens it in a tab. Existing branches cannot be checked out twice. Choose a destination outside existing worktrees.",
                "dim-label",
            );
            note.set_wrap(true);
            body.append(&note);
            let buttons = widgets::hbox(6);
            buttons.set_halign(gtk::Align::End);
            let cancel = gtk::Button::with_label("Not now");
            let d = dialog.clone();
            cancel.connect_clicked(move |_| d.close());
            buttons.append(&cancel);
            let create = gtk::Button::with_label("Create and open");
            create.add_css_class("suggested-action");
            let d = dialog.clone();
            create.connect_clicked(move |_| {
                let destination = PathBuf::from(path.text().trim());
                let is_new = new_branch.is_active();
                let branch = if is_new {
                    branch.text().trim().to_string()
                } else {
                    names.get(existing.selected() as usize).cloned().unwrap_or_default()
                };
                let target = if is_new { base.text().to_string() } else { format!("refs/heads/{branch}") };
                let root = root.clone();
                let d = d.clone();
                nyx_core::cmd::background(
                    move || {
                        advanced::resolve_target(&root, &target).map(|base| {
                            (root, Action::Create { path: destination.clone(), branch, new_branch: is_new, base }, destination)
                        })
                    },
                    move |result| match result {
                        Ok((root, action, destination)) => {
                            if let Some(ui) = current() {
                                d.close();
                                ui.execute_at_then(root, Operation::Worktree(action), move |result| {
                                    if result.is_ok()
                                        && let Some(ui) = current()
                                    {
                                        ui.open(destination);
                                    }
                                });
                            }
                        }
                        Err(e) => fail(format!("{e:#}")),
                    },
                );
            });
            buttons.append(&create);
            body.append(&buttons);
            dialog.present();
        },
    );
}
