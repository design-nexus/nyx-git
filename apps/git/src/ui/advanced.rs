use super::window::current;
use gtk::prelude::*;
use nyx_git::core::{
    advanced::{self, Action, Guard, ResetMode, Step},
    operations::Operation,
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
/// Actions that can lose work or rewrite what others may have.
fn destructive(action: &Action) -> bool {
    matches!(
        action,
        Action::Reset { mode: ResetMode::Hard, .. }
            | Action::Discard { .. }
            | Action::Abort
            | Action::Skip
            | Action::DeleteTag { .. }
            | Action::DeleteBranch { .. }
            | Action::ForcePush { .. }
            | Action::Interactive { .. }
            | Action::Squash { .. }
    )
}
fn execute(root: PathBuf, action: Action, guard: Guard) {
    if let Some(ui) = current() {
        ui.execute_at(root, Operation::Advanced { action: Box::new(action), guard: Box::new(guard) });
    }
}
/// Run `action` straight away (it was just asked for in a dialog), with a fresh guard.
fn execute_now(root: PathBuf, action: Action) {
    nyx_core::cmd::background(
        move || Guard::capture(&root).map(|g| (root, g)),
        move |result| match result {
            Ok((root, guard)) => execute(root, action, guard),
            Err(e) => fail(format!("{e:#}")),
        },
    );
}
pub fn prepare(action: Action, effects: String) {
    let Some(root) = current().and_then(|ui| ui.root()) else { return };
    prepare_at(root, action, effects);
}
fn prepare_at(root: PathBuf, action: Action, effects: String) {
    nyx_core::cmd::background(
        move || Guard::capture(&root).map(|g| (root, g)),
        move |result| match result {
            Ok((root, guard)) => {
                let a = action.clone();
                super::preview::confirm(
                    root.clone(),
                    nyx_git::core::preview::Subject::Advanced(action.clone()),
                    format!("{}?", action.label()),
                    effects,
                    action.label().into(),
                    destructive(&action),
                    move || execute(root.clone(), a.clone(), guard.clone()),
                );
            }
            Err(e) => fail(format!("{e:#}")),
        },
    );
}
pub fn show(target: Option<String>) {
    let Some(root) = current().and_then(|ui| ui.root()) else {
        error("Open a repository first.");
        return;
    };
    let (dialog, body) = widgets::dialog("Advanced Git", 620);
    crate::dialog_dismissal::add(&dialog, &body, "Cancel");
    let note = widgets::label(
        "Choose a commit, branch or tag. History rewrites retain a recovery reference; uncommitted contents are not backed up.",
        "dim-label",
    );
    note.set_wrap(true);
    body.append(&note);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Branch, tag, full commit hash or HEAD"));
    entry.set_text(target.as_deref().unwrap_or("HEAD"));
    body.append(&entry);
    let actions = gtk::FlowBox::new();
    actions.set_selection_mode(gtk::SelectionMode::None);
    actions.set_max_children_per_line(3);
    actions.set_column_spacing(6);
    actions.set_row_spacing(6);
    for (title, kind) in [
        ("Check out…", "checkout"),
        ("Create branch…", "branch"),
        ("Merge…", "merge"),
        ("Rebase onto…", "rebase"),
        ("Cherry-pick…", "cherry"),
        ("Revert…", "revert"),
        ("Reset soft…", "soft"),
        ("Reset mixed…", "mixed"),
        ("Reset hard…", "hard"),
        ("Interactive rebase…", "interactive"),
        ("Squash after base…", "squash"),
        ("Create tag…", "tag"),
        ("Delete tag…", "delete-tag"),
        ("Delete unmerged branch…", "delete-branch"),
    ] {
        let b = gtk::Button::with_label(title);
        let e = entry.clone();
        let d = dialog.clone();
        let expected_root = root.clone();
        b.connect_clicked(move |_| {
            if current().and_then(|ui| ui.root()).as_ref() != Some(&expected_root) {
                d.close();
                error("The active repository changed. Choose the action again.");
                return;
            }
            let text = e.text().to_string();
            d.close();
            target_action(kind, text);
        });
        actions.insert(&b, -1);
    }
    body.append(&actions);
    let tools = gtk::FlowBox::new();
    tools.set_selection_mode(gtk::SelectionMode::None);
    tools.set_max_children_per_line(3);
    for (title, kind) in [
        ("Resolve conflicts…", "conflicts"),
        ("Continue operation…", "continue"),
        ("Abort operation…", "abort"),
        ("Recovery…", "recovery"),
        ("Force push with lease…", "force"),
    ] {
        let b = gtk::Button::with_label(title);
        let d = dialog.clone();
        b.connect_clicked(move |_|{d.close();match kind{
            "conflicts"=>super::conflict_editor::choose(),"continue"=>prepare(Action::Continue,"Continue the active operation using Git's prepared commit message. All conflicts must already be resolved and staged.".into()),
            "abort"=>prepare(Action::Abort,"Abort the active operation and return toward its original state. Resolution edits and changes made during the operation may be lost. Git may refuse if it cannot restore safely.".into()),
            "recovery"=>recovery(),_=>force_push(),
        }});
        tools.insert(&b, -1);
    }
    body.append(&tools);
    dialog.present();
}
/// Prepare `kind` (merge, rebase, cherry, revert, soft, mixed, hard, interactive, squash,
/// tag, checkout, branch, delete-tag, delete-branch) at `text`: a ref name or full hash.
pub(super) fn target_action(kind: &'static str, text: String) {
    let Some(root) = current().and_then(|ui| ui.root()) else { return };
    nyx_core::cmd::background(
        move || -> anyhow::Result<_> {
            let id = if kind == "delete-tag" || kind == "delete-branch" {
                let prefix = if kind == "delete-tag" { "refs/tags/" } else { "refs/heads/" };
                advanced::valid_ref(prefix, &text)?;
                gix::discover(&root)?.find_reference(format!("{prefix}{text}").as_str())?.id().to_string()
            } else {
                advanced::resolve_target(&root, &text)?
            };
            let parents = if kind == "cherry" || kind == "revert" {
                gix::discover(&root)?
                    .find_object(gix::ObjectId::from_hex(id.as_bytes())?)?
                    .try_into_commit()?
                    .parent_ids()
                    .count()
            } else {
                0
            };
            Ok((root, id, parents, text))
        },
        move |result| {
            let (root, id, parents, text) = match result {
                Ok(v) => v,
                Err(e) => {
                    fail(format!("{e:#}"));
                    return;
                }
            };
            let tip = format!("Target: {text}\n{id}\n");
            match kind {
                "merge" => prepare_at(
                    root.clone(),
                    Action::Merge(id),
                    format!(
                        "{tip}Merge into the current branch. Conflicts stop for explicit resolution. Requires a clean working tree."
                    ),
                ),
                "rebase" => prepare_at(
                    root.clone(),
                    Action::Rebase(id),
                    format!(
                        "{tip}Replay the current branch onto this commit, preserving merge topology. Commit hashes change. Avoid rewriting shared commits. Requires a clean working tree."
                    ),
                ),
                "cherry" | "revert" => {
                    let apply = move |mainline| {
                        let action = if kind == "cherry" {
                            Action::CherryPick { id: id.clone(), mainline }
                        } else {
                            Action::Revert { id: id.clone(), mainline }
                        };
                        prepare_at(
                            root.clone(),
                            action,
                            format!(
                                "{tip}{} on the current branch. Conflicts stop for explicit resolution. Requires a clean working tree.",
                                if kind == "cherry" {
                                    "Apply this commit's changes"
                                } else {
                                    "Create a commit undoing this commit"
                                }
                            ),
                        );
                    };
                    if parents > 1 {
                        widgets::ask_text(
                            "Mainline parent",
                            &format!(
                                "This merge commit has {parents} parents. Choose the parent number whose perspective defines the change."
                            ),
                            "1",
                            "Choose",
                            move |value| match value.parse::<u32>() {
                                Ok(n) if n > 0 && n as usize <= parents => apply(Some(n)),
                                _ => error("Invalid mainline parent"),
                            },
                        );
                    } else {
                        apply(None);
                    }
                }
                "soft" | "mixed" | "hard" => {
                    let (mode, effect) = match kind {
                        "soft" => (ResetMode::Soft, "Move HEAD and keep the index and working files."),
                        "mixed" => {
                            (ResetMode::Mixed, "Move HEAD and reset the index. Working files remain; staged selections are lost.")
                        }
                        _ => (
                            ResetMode::Hard,
                            "Move HEAD and overwrite the index and tracked files. Uncommitted contents and untracked or ignored paths obstructing checkout can be permanently deleted. Recovery references retain committed history only.",
                        ),
                    };
                    prepare_at(root.clone(), Action::Reset { target: id, mode }, format!("{tip}{effect}"));
                }
                "checkout" => prepare_at(
                    root.clone(),
                    Action::Checkout(id),
                    format!(
                        "{tip}Check out this commit without a branch (a detached HEAD). Git keeps local changes or refuses to switch. Create a branch here to keep new commits."
                    ),
                ),
                "branch" => {
                    widgets::ask_text(
                        "Create branch",
                        "Create a branch at this commit and switch to it. Git keeps local changes or refuses to switch.",
                        "",
                        "Create and switch",
                        move |name| execute_now(root.clone(), Action::SwitchNew { name, target: id.clone() }),
                    );
                }
                "tag" => {
                    widgets::ask_text("Create annotated tag", "Enter a new tag name.", "", "Next", move |name| {
                        let id = id.clone();
                        let root = root.clone();
                        widgets::ask_text(
                            "Tag message",
                            "Annotated tags preserve a description and use Git's signing configuration.",
                            "",
                            "Create tag",
                            move |message| {
                                execute_now(root.clone(), Action::CreateTag { name: name.clone(), target: id.clone(), message })
                            },
                        );
                    });
                }
                "delete-tag" => prepare_at(
                    root.clone(),
                    Action::DeleteTag { name: text, expected: id },
                    "Delete this local tag. Remote tags remain. A recovery reference retains its object.".into(),
                ),
                "delete-branch" => prepare_at(
                    root.clone(),
                    Action::DeleteBranch { branch: text.clone(), expected: id },
                    format!(
                        "Delete local branch {text} even if it has unmerged commits. A recovery reference retains its tip. Git refuses the current branch or a branch checked out in another worktree."
                    ),
                ),
                "interactive" | "squash" => nyx_core::cmd::background(
                    move || advanced::plan(&root, &id).map(|steps| (root, id, steps)),
                    move |result| match result {
                        Ok((root, base, steps)) => {
                            if kind == "interactive" {
                                super::rebase_studio::open(root, base, steps);
                            } else {
                                widgets::ask_text(
                                    "Squash commits",
                                    &format!(
                                        "Replace {} commits after the selected base with one commit. Commit hashes change. Requires a clean working tree.",
                                        steps.len()
                                    ),
                                    "",
                                    "Squash…",
                                    move |message| {
                                        prepare_at(root.clone(),Action::Squash{base:base.clone(),message},"Replace the selected linear range with one commit. Recovery retains the original HEAD. Avoid rewriting shared history.".into())
                                    },
                                );
                            }
                        }
                        Err(e) => fail(format!("{e:#}")),
                    },
                ),
                _ => {}
            }
        },
    );
}
/// Start the studio's plan through the usual preview. When the rebase stops (to edit, split
/// or resolve a conflict), Changes is where it carries on.
pub fn prepare_interactive(root: PathBuf, base: String, steps: Vec<Step>, editor: PathBuf) {
    let action = Action::Interactive { base, steps, editor };
    nyx_core::cmd::background(
        move || Guard::capture(&root).map(|g| (root, g)),
        move |result| match result {
            Ok((root, guard)) => {
                let a = action.clone();
                super::preview::confirm(
                    root.clone(),
                    nyx_git::core::preview::Subject::Advanced(action.clone()),
                    "Start the rebase?".into(),
                    "Replay the commits as planned. Commit hashes change from the first changed commit on. Edit and split steps stop so you can work in Changes; Continue carries on.".into(),
                    "Start rebase".into(),
                    true,
                    move || {
                        let Some(ui) = current() else { return };
                        ui.execute_at_then(root.clone(), Operation::Advanced { action: Box::new(a.clone()), guard: Box::new(guard.clone()) }, |result| {
                            let Some(ui) = current() else { return };
                            let stopped = ui.snapshot.borrow().as_ref().and_then(|s| s.progress).is_some();
                            if result.is_ok() && stopped {
                                ui.show_view("changes");
                                ui.toast("The rebase stopped to edit or split a commit. Make the changes, then Continue.");
                            } else if result.is_err() && stopped {
                                super::conflict_studio_show(None);
                            } else if result.is_ok() {
                                ui.show_view("history");
                            }
                        });
                    },
                );
            }
            Err(e) => fail(format!("{e:#}")),
        },
    );
}
/// Ask for a name, then create a branch at `id` without switching to it.
pub fn branch_at(root: PathBuf, id: String) {
    widgets::ask_text(
        "Recovery branch",
        &format!("Create a branch at {} without changing working files. Switch to it later when ready.", &id[..id.len().min(10)]),
        "",
        "Create branch",
        move |name| execute_now(root.clone(), Action::BranchAt { name, target: id.clone() }),
    );
}
pub(super) fn recovery() {
    let Some(root) = current().and_then(|ui| ui.root()) else { return };
    nyx_core::cmd::background(
        move || advanced::recovery(&root).map(|records| (root, records)),
        move |result| match result {
            Ok((root, records)) => {
                let (dialog, body) = widgets::dialog("Recovery points and reflog", 700);
                crate::dialog_dismissal::add(&dialog, &body, "Close");
                let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
                rows.add_css_class("group-list");
                for record in records {
                    let b = gtk::Button::with_label("Create branch…");
                    let d = dialog.clone();
                    let root = root.clone();
                    b.connect_clicked(move |_| {
                        d.close();
                        let id = record.id.clone();
                        let root = root.clone();
                        widgets::ask_text(
                            "Recovery branch",
                            "Create a branch at this saved commit without changing working files.",
                            "",
                            "Create…",
                            move |name| {
                                prepare_at(
                                    root.clone(),
                                    Action::BranchAt { name, target: id.clone() },
                                    "Create a branch pointing to the saved commit. Switch to it later when ready.".into(),
                                )
                            },
                        );
                    });
                    rows.append(&widgets::row(
                        &record.label,
                        &gtk::glib::markup_escape_text(&record.summary),
                        Some(b.upcast_ref()),
                    ));
                }
                let scroll = gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(super::scrollbars::policy())
                    .vscrollbar_policy(super::scrollbars::policy())
                    .child(&rows)
                    .min_content_height(350)
                    .max_content_height(500)
                    .propagate_natural_height(true)
                    .build();
                body.append(&scroll);
                dialog.present();
            }
            Err(e) => fail(format!("{e:#}")),
        },
    );
}
fn force_push() {
    let Some(root) = current().and_then(|ui| ui.root()) else { return };
    widgets::ask_text(
        "Force push with lease",
        "Enter the remote name (or URL). Only the current HEAD will be published.",
        "origin",
        "Next",
        move |remote| {
            let root = root.clone();
            widgets::ask_text(
                "Remote branch",
                "Enter the remote branch to replace. Its exact current tip will be read before confirmation.",
                "",
                "Read remote tip",
                move |branch| {
                    let root = root.clone();
                    let remote = remote.clone();
                    nyx_core::cmd::background(
                        move || advanced::remote_tip(&root, &remote, &branch).map(|lease| (root, remote, branch, lease)),
                        move |result| match result {
                            Ok((root, remote, branch, lease)) => prepare_at(
                                root,
                                Action::ForcePush { remote: remote.clone(), branch: branch.clone(), lease: lease.clone() },
                                format!(
                                    "Replace {remote}/{branch} with HEAD even if this rewrites published history. The push is refused if the remote moved since the observed tip {lease}. Coordinate with other contributors."
                                ),
                            ),
                            Err(e) => fail(format!("{e:#}")),
                        },
                    );
                },
            );
        },
    );
}
