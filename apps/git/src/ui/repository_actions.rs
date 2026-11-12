use super::{window::current, workflow::confirm};
use gtk::prelude::*;
use nyx_git::core::operations::{self, Operation};
use nyx_ui::widgets;
use std::{path::PathBuf, rc::Rc};

fn run(root: &std::path::Path, operation: Operation) {
    let root = root.to_path_buf();
    if let Some(ui) = current() {
        ui.execute_at(root, operation);
    }
}
fn close_button(dialog: &gtk::Window, body: &gtk::Box) {
    let b = gtk::Button::with_label("Close");
    b.set_halign(gtk::Align::End);
    let d = dialog.clone();
    b.connect_clicked(move |_| d.close());
    body.append(&b);
}
fn list_scroll(rows: &gtk::Box) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .min_content_height(120)
        .max_content_height(400)
        .propagate_natural_height(true)
        .child(rows)
        .build()
}
pub fn branches() {
    let Some(ui) = current() else { return };
    let Some(root) = ui.root() else { return };
    let branch = ui.snapshot.borrow().as_ref().map(|s| s.repository.branch.clone()).unwrap_or_default();
    nyx_core::cmd::background(
        move || operations::run(&root, Operation::Branches).map(|bytes| (root, bytes)),
        move |result| {
            let (root, bytes) = match result {
                Ok(v) => v,
                Err(e) => {
                    if let Some(ui) = current() {
                        ui.error(&format!("{e:#}"));
                    }
                    return;
                }
            };
            let Some(ui) = current() else { return };
            if ui.root().as_ref() != Some(&root) {
                return;
            }
            let (dialog, body) = widgets::dialog("Local branches", 620);
            crate::dialog_dismissal::add(&dialog, &body, "Close");
            let create = gtk::Button::with_label("Create and switch…");
            create.set_halign(gtk::Align::Start);
            let r = root.clone();
            let d = dialog.clone();
            create.connect_clicked(move |_| {
                d.close();
                let root = r.clone();
                widgets::ask_text(
                    "Create branch",
                    "Create a branch at HEAD and switch to it. Git preserves local changes or refuses an unsafe switch.",
                    "",
                    "Create and switch",
                    move |name| run(&root, Operation::CreateBranch(name)),
                );
            });
            body.append(&create);
            let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
            rows.add_css_class("group-list");
            for name in String::from_utf8_lossy(&bytes).lines() {
                let name = name.to_string();
                let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                if name != branch {
                    let b = gtk::Button::with_label("Switch");
                    let root = root.clone();
                    let name2 = name.clone();
                    let d = dialog.clone();
                    b.connect_clicked(move |_| {
                        d.close();
                        run(&root, Operation::SwitchBranch(name2.clone()));
                    });
                    actions.append(&b);
                }
                let push = gtk::Button::with_label("Push…");
                let push_root = root.clone();
                let push_name = name.clone();
                let d = dialog.clone();
                push.connect_clicked(move |_| {
                    d.close();
                    push_branch(push_root.clone(), push_name.clone());
                });
                actions.append(&push);
                let rename = gtk::Button::with_label("Rename…");
                let root2 = root.clone();
                let name2 = name.clone();
                let d = dialog.clone();
                rename.connect_clicked(move |_| {
                    d.close();
                    let root = root2.clone();
                    let old = name2.clone();
                    widgets::ask_text(
                        "Rename branch",
                        "Rename this local branch. Remote branches are unchanged.",
                        &name2,
                        "Rename",
                        move |new| run(&root, Operation::RenameBranch { old: old.clone(), new }),
                    );
                });
                actions.append(&rename);
                if name != branch {
                    let delete = gtk::Button::with_label("Delete…");
                    let root = root.clone();
                    let name2 = name.clone();
                    let d = dialog.clone();
                    delete.connect_clicked(move |_| {
                    d.close(); let root = root.clone(); let name = name2.clone();
                    super::preview::confirm(root.clone(), nyx_git::core::preview::Subject::DeleteMergedBranch(name.clone()), "Delete local branch?".into(), format!("Delete {name}. Git refuses branches with unmerged commits. Working files and remote branches are unchanged."), "Delete branch".into(), true, move || run(&root, Operation::DeleteBranch(name.clone())));
                });
                    actions.append(&delete);
                }
                rows.append(&widgets::row(&name, if name == branch { "Current branch" } else { "" }, Some(actions.upcast_ref())));
            }
            if bytes.is_empty() {
                rows.append(&widgets::row("No local branches yet", "Create the first commit to establish this branch.", None));
            }
            body.append(&list_scroll(&rows));
            close_button(&dialog, &body);
            dialog.present();
        },
    );
}
pub fn stashes() {
    let Some(ui) = current() else { return };
    let Some(root) = ui.root() else { return };
    nyx_core::cmd::background(
        move || operations::run(&root, Operation::Stashes).map(|bytes| (root, bytes)),
        move |result| {
            let (root, bytes) = match result {
                Ok(v) => v,
                Err(e) => {
                    if let Some(ui) = current() {
                        ui.error(&format!("{e:#}"));
                    }
                    return;
                }
            };
            let Some(ui) = current() else { return };
            if ui.root().as_ref() != Some(&root) {
                return;
            }
            let (dialog, body) = widgets::dialog("Stashes", 620);
            crate::dialog_dismissal::add(&dialog, &body, "Close");
            let save = gtk::Button::with_label("Save working changes…");
            save.set_halign(gtk::Align::Start);
            let r = root.clone();
            let d = dialog.clone();
            save.connect_clicked(move |_| {
            d.close(); let root = r.clone();
            widgets::ask_text("Stash changes", "Save staged, unstaged and untracked files, then remove those changes from the working tree. Ignored files stay in place.", "", "Save stash", move |message| run(&root, Operation::StashSave(message)));
        });
            body.append(&save);
            let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
            rows.add_css_class("group-list");
            let fields: Vec<_> = bytes.split(|b| *b == 0).collect();
            for fields in fields.chunks(3) {
                if fields.len() < 3 {
                    continue;
                }
                let hash = String::from_utf8_lossy(fields[0]).trim().to_string();
                let id = String::from_utf8_lossy(fields[1]).to_string();
                let title = String::from_utf8_lossy(fields[2]).to_string();
                let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                for (verb, tip) in [
                    ("Inspect", "View the saved diff"),
                    ("Apply", "Restore changes, keeping the stash"),
                    ("Pop", "Restore changes and remove the stash only on success"),
                    ("Drop", "Delete the saved stash"),
                ] {
                    let b = gtk::Button::with_label(verb);
                    b.set_tooltip_text(Some(tip));
                    let root = root.clone();
                    let id = id.clone();
                    let hash = hash.clone();
                    let d = dialog.clone();
                    b.connect_clicked(move |_| {
                    d.close();
                    match verb {
                        "Inspect" => show_stash_diff(root.clone(), hash.clone()),
                        "Apply" => run(&root, Operation::StashApply(hash.clone())),
                        "Pop" => run(&root, Operation::StashPop {id: id.clone(), expected: hash.clone()}),
                        _ => {
                            let root = root.clone(); let id = id.clone(); let hash = hash.clone();
                            confirm("Drop stash?", &format!("Delete {id} permanently from the stash list. This does not apply its changes; recovery is not guaranteed."), "Drop stash", move || run(&root, Operation::StashDrop {id: id.clone(), expected: hash.clone()}));
                        }
                    }
                });
                    actions.append(&b);
                }
                rows.append(&widgets::row(&id, &gtk::glib::markup_escape_text(&title), Some(actions.upcast_ref())));
            }
            if bytes.is_empty() {
                rows.append(&widgets::row("No stashes", "Save working changes to return to them later.", None));
            }
            body.append(&list_scroll(&rows));
            close_button(&dialog, &body);
            dialog.present();
        },
    );
}
pub(super) fn show_stash_diff(root: PathBuf, hash: String) {
    nyx_core::cmd::background(
        move || operations::run(&root, Operation::StashDiff(hash)),
        |result| match result {
            Ok(bytes) => {
                let (dialog, body) = widgets::dialog("Stash diff", 760);
                crate::dialog_dismissal::add(&dialog, &body, "Close");
                let diff = Rc::new(super::diff_view::DiffView::new());
                if bytes.len() <= 2 * 1024 * 1024 {
                    diff.set_text(&String::from_utf8_lossy(&bytes));
                } else {
                    diff.set_text("Diff exceeds 2 MiB. Inspect this stash in an external tool.");
                }
                diff.root.set_min_content_height(380);
                body.append(&diff.root);
                close_button(&dialog, &body);
                dialog.present();
            }
            Err(e) => {
                if let Some(ui) = current() {
                    ui.error(&format!("{e:#}"));
                }
            }
        },
    );
}

pub fn push_branch(root: PathBuf, branch: String) {
    nyx_core::cmd::background(
        move || operations::run(&root, Operation::Remotes).map(|bytes| (root, bytes)),
        move |result| {
            let (root, bytes) = match result {
                Ok(v) => v,
                Err(e) => {
                    if let Some(ui) = current() {
                        ui.error(&format!("{e:#}"));
                    }
                    return;
                }
            };
            let remotes: Vec<String> = String::from_utf8_lossy(&bytes).lines().map(String::from).collect();
            if remotes.is_empty() {
                if let Some(ui) = current() {
                    ui.toast("No remotes configured. Add a remote with system Git before pushing.");
                }
                return;
            }
            let (dialog, body) = widgets::dialog("Push branch", 480);
            crate::dialog_dismissal::add(&dialog, &body, "Cancel");
            let label = widgets::label(
                &format!(
                    "Push {branch} to the selected remote and set it as the upstream. This publishes commits without force pushing."
                ),
                "dim-label",
            );
            label.set_wrap(true);
            body.append(&label);
            let dropdown = gtk::DropDown::from_strings(&remotes.iter().map(String::as_str).collect::<Vec<_>>());
            body.append(&dropdown);
            let push = gtk::Button::with_label("Push and set upstream");
            let d = dialog.clone();
            push.connect_clicked(move |_| {
                if let Some(remote) = remotes.get(dropdown.selected() as usize) {
                    d.close();
                    run(&root, Operation::PushBranch { remote: remote.clone(), branch: branch.clone() });
                }
            });
            body.append(&push);
            close_button(&dialog, &body);
            dialog.present();
        },
    );
}
