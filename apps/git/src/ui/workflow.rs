use super::{diff_view::DiffView, item, window::current};
use gtk::prelude::*;
use nyx_git::core::{
    operations::{self, Operation},
    staging,
    status::FileStatus,
};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};

pub struct Workflow {
    pub toolbar: gtk::Box,
    pub category: gtk::DropDown,
    pub composer: gtk::Box,
    pub operation_bar: gtk::Box,
    operation_label: gtk::Label,
    pub detail: gtk::Box,
    pub diff: DiffView,
    pub selected: RefCell<Option<FileStatus>>,
    pub patch: RefCell<Vec<u8>>,
    pub generation: Cell<u64>,
    /// The file and side the diff view shows, so reloading it keeps the scroll position.
    shown: RefCell<Option<(PathBuf, bool)>>,
    pub message: gtk::TextView,
    pub amend: gtk::CheckButton,
    enabled: Cell<bool>,
    staged: gtk::ToggleButton,
    stage: gtk::Button,
    unstage: gtk::Button,
    hunk: gtk::Button,
    lines: gtk::Button,
    resolve: gtk::Button,
    more: gtk::MenuButton,
    commit: gtk::Button,
    length: gtk::Label,
    pub template: gtk::Button,
    /// The repository's `commit.template`, without its comment lines.
    pub template_text: RefCell<Option<String>>,
}
fn button(title: &str, tip: &str, run: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::with_label(title);
    button.set_tooltip_text(Some(tip));
    button.connect_clicked(move |_| run());
    button
}
fn execute(operation: Operation) {
    if let Some(ui) = current() {
        ui.execute(operation);
    }
}
fn blame_selected() {
    let Some(ui) = current() else { return };
    let Some(root) = ui.root() else { return };
    let Some(file) = ui.workflow.selected.borrow().clone() else { return };
    if file.index == '?' || file.worktree == 'D' {
        ui.toast("Blame needs a tracked file that's still in the working tree.");
        return;
    }
    super::blame::open(root, file.path);
}
fn discard() {
    let Some(ui) = current() else { return };
    let Some(file) = ui.workflow.selected.borrow().clone() else { return };
    if file.conflicted || file.submodule || file.original.is_some() {
        ui.toast("Resolve conflicts, or use an external tool for submodules and renames.");
        return;
    }
    super::advanced::prepare(
        nyx_git::core::advanced::Action::Discard { file: file.path.clone(), untracked: file.index == '?' },
        format!(
            "Discard the unstaged changes to {}. An untracked file is deleted; a tracked one goes back to its staged contents. Discarded changes can't be recovered.",
            file.path.display()
        ),
    );
}
impl Workflow {
    pub fn new() -> Self {
        let operation_bar = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let operation_label = widgets::label("", "dim-label");
        operation_label.set_wrap(true);
        operation_bar.add_css_class("git-actions");
        operation_bar.append(&operation_label);
        let controls = widgets::hbox(6);
        controls.append(&button("Conflict Studio…", "Resolve the conflicted files", super::conflict_editor::choose));
        controls.append(&button("Continue…", "Continue the active operation", || {
            super::advanced::prepare(
                nyx_git::core::advanced::Action::Continue,
                "Continue using Git's prepared message after all conflicts are resolved and staged.".into(),
            )
        }));
        controls.append(&button("Skip…", "Leave out the commit the operation stopped at", || {
            super::advanced::prepare(
                nyx_git::core::advanced::Action::Skip,
                "Leave out the commit that stopped the operation and carry on with the next one. Its changes won't be in the result.".into(),
            )
        }));
        controls.append(&button("Abort…", "Abort the active operation", || {
            super::advanced::prepare(nyx_git::core::advanced::Action::Abort,"Abort and restore the original operation state where Git can do so safely. Resolution edits and changes made during the operation may be lost.".into())
        }));
        operation_bar.append(&controls);
        operation_bar.set_visible(false);
        let toolbar = widgets::hbox(6);
        toolbar.add_css_class("git-actions");
        let category = gtk::DropDown::from_strings(&["All changes", "Unstaged", "Staged", "Untracked"]);
        category.set_tooltip_text(Some("Filter the file list by staging state"));
        toolbar.append(&category);
        let side = gtk::ToggleButton::new();
        side.set_icon_name("view-dual-symbolic");
        side.add_css_class("flat");
        side.set_tooltip_text(Some("Show diffs side by side, old and new"));
        side.set_active(crate::prefs::get().side_by_side);
        side.connect_toggled(|b| {
            crate::prefs::update(|p| p.side_by_side = b.is_active());
            if let Some(ui) = current() {
                ui.workflow.diff.set_side_by_side(b.is_active());
                ui.workflow.sensitive(ui.workflow.is_enabled());
            }
        });
        toolbar.append(&side);
        let spacer = widgets::hbox(0);
        spacer.set_hexpand(true);
        toolbar.append(&spacer);
        toolbar.append(&button("Stage all", "Stage all changed and untracked files", || execute(Operation::StageAll)));
        toolbar.append(&button("Unstage all", "Keep files and remove their changes from the next commit", || {
            if let Some(ui) = current()
                && let Some(s) = ui.snapshot.borrow().as_ref()
            {
                ui.execute(Operation::UnstageAll { unborn: s.repository.unborn });
            }
        }));
        toolbar.set_sensitive(false);
        let message = gtk::TextView::new();
        message.set_wrap_mode(gtk::WrapMode::WordChar);
        message.set_monospace(true);
        message.set_left_margin(10);
        message.set_right_margin(10);
        message.set_top_margin(8);
        message.set_bottom_margin(8);
        message.add_css_class("git-message");
        message.update_property(&[gtk::accessible::Property::Label("Commit message")]);
        let composer = gtk::Box::new(gtk::Orientation::Vertical, 8);
        composer.add_css_class("git-composer");
        let heading = widgets::hbox(8);
        let title = widgets::label("COMMIT MESSAGE", "dim-label");
        title.set_hexpand(true);
        heading.append(&title);
        let length = widgets::label("", "dim-label");
        length.set_tooltip_text(Some("Characters in the summary line. Aim for 50 or fewer; many tools cut it at 72."));
        heading.append(&length);
        composer.append(&heading);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(super::scrollbars::policy())
            .vscrollbar_policy(super::scrollbars::policy())
            .min_content_height(80)
            .max_content_height(140)
            .child(&message)
            .build();
        composer.append(&scroll);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let amend = gtk::CheckButton::with_label("Amend HEAD");
        amend.set_tooltip_text(Some("Replace the latest commit; its hash changes"));
        row.append(&amend);
        let last = button("Last message", "Copy the latest commit message into the editor", || {
            let Some(ui) = current() else { return };
            let Some(root) = ui.root() else { return };
            let before = ui.workflow.message();
            nyx_core::cmd::background(
                move || operations::run(&root, Operation::LastMessage).map(|bytes| (root, bytes)),
                move |result| {
                    let Some(ui) = current() else { return };
                    match result {
                        Ok((root, bytes)) if ui.root().as_ref() == Some(&root) && ui.workflow.message() == before => {
                            ui.workflow.message.buffer().set_text(String::from_utf8_lossy(&bytes).trim_end())
                        }
                        Ok(_) => ui.toast("The editor or repository changed; the message was left intact."),
                        Err(e) => ui.error(&format!("{e:#}")),
                    }
                },
            );
        });
        row.append(&last);
        let template = button("Template", "Insert the repository's commit template (commit.template), without its comment lines", || {
            let Some(ui) = current() else { return };
            let Some(text) = ui.workflow.template_text.borrow().clone() else { return };
            let buffer = ui.workflow.message.buffer();
            let mut end = buffer.end_iter();
            if buffer.char_count() > 0 {
                buffer.insert(&mut end, "\n");
            }
            buffer.insert(&mut end, &text);
            ui.workflow.message.grab_focus();
        });
        template.set_visible(false);
        row.append(&template);
        let commit = button("Commit", "Commit staged changes (Ctrl+Enter)", commit);
        commit.add_css_class("suggested-action");
        commit.set_hexpand(true);
        commit.set_halign(gtk::Align::End);
        row.append(&commit);
        composer.append(&row);
        composer.set_sensitive(false);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let actions = gtk::FlowBox::new();
        actions.set_selection_mode(gtk::SelectionMode::None);
        actions.set_max_children_per_line(6);
        actions.set_column_spacing(6);
        actions.set_row_spacing(6);
        let resolve = button("Resolve conflict…", "Open the selected file in the conflict editor", || {
            if let Some(ui) = current()
                && let Some(file) = ui.workflow.selected.borrow().as_ref().filter(|f| f.conflicted)
            {
                super::conflict_editor::open(file.path.clone());
            }
        });
        resolve.add_css_class("suggested-action");
        let stage = button("Stage file", "Stage all changes to the selected file", || selected_action(false));
        let unstage = button("Unstage file", "Unstage the selected file, preserving working changes", || selected_action(true));
        let staged = gtk::ToggleButton::with_label("Staged diff");
        staged.set_tooltip_text(Some("Compare the index with HEAD instead of working changes"));
        staged.connect_toggled(|_| {
            if let Some(ui) = current() {
                ui.load_diff();
            }
        });
        let hunk = button("Stage hunk", "Stage the diff hunk at the cursor", || partial(false));
        let lines = button("Stage lines", "Stage selected added or removed lines", || partial(true));
        let more = super::menu_button("view-more-symbolic", None, "More for this file", || {
            vec![
                item("File history…", super::file_history::selected_change),
                item("Blame…", blame_selected),
                item("Discard changes…", discard),
            ]
        });
        for b in [
            resolve.upcast_ref::<gtk::Widget>(),
            stage.upcast_ref(),
            unstage.upcast_ref(),
            staged.upcast_ref(),
            hunk.upcast_ref(),
            lines.upcast_ref(),
            more.upcast_ref(),
        ] {
            actions.insert(b, -1);
        }
        detail.append(&actions);
        let diff = DiffView::new();
        detail.append(&diff.switchable());
        diff.set_side_by_side(side.is_active());
        detail.set_vexpand(true);
        let me = Self {
            operation_bar,
            operation_label,
            toolbar,
            category,
            composer,
            detail,
            diff,
            selected: RefCell::new(None),
            patch: RefCell::new(Vec::new()),
            generation: Cell::new(0),
            shown: RefCell::new(None),
            message,
            amend,
            enabled: Cell::new(false),
            staged,
            stage,
            unstage,
            hunk,
            lines,
            resolve,
            more,
            commit,
            length,
            template,
            template_text: RefCell::new(None),
        };
        me.sensitive(false);
        me
    }
    pub fn operation_status(&self, progress: Option<nyx_git::core::advanced::Active>, conflicts: bool) {
        self.operation_bar.set_visible(progress.is_some() || conflicts);
        self.operation_label.set_text(&format!(
            "{}{}",
            progress.map_or("Git conflicts".into(), |p| format!("{} in progress", p.name())),
            if conflicts { " · resolve every conflicted file before continuing" } else { " · ready to continue or abort" }
        ));
    }
    pub fn message(&self) -> String {
        let b = self.message.buffer();
        b.text(&b.start_iter(), &b.end_iter(), false).into()
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled.get()
    }
    pub fn cached(&self) -> bool {
        self.staged.is_active()
    }
    pub fn select(&self, file: Option<FileStatus>) {
        self.generation.set(self.generation.get() + 1);
        self.patch.borrow_mut().clear();
        let cached = file.as_ref().is_some_and(|f| f.worktree == '.' && f.index != '.' && f.index != '?');
        *self.selected.borrow_mut() = file;
        let toggled = self.staged.is_active() != cached;
        self.staged.set_active(cached);
        if !toggled && let Some(ui) = current() {
            ui.load_diff();
        }
    }
    /// Enable what applies to the selected file; hide what never could.
    pub fn sensitive(&self, enabled: bool) {
        self.enabled.set(enabled);
        self.operation_bar.set_sensitive(enabled);
        self.toolbar.set_sensitive(enabled);
        self.composer.set_sensitive(enabled);
        let file = self.selected.borrow();
        let conflicted = file.as_ref().is_some_and(|f| f.conflicted);
        let stageable = file.as_ref().is_some_and(|f| f.worktree != '.' && !f.conflicted);
        let unstageable = file.as_ref().is_some_and(|f| f.index != '.' && f.index != '?' && !f.conflicted);
        self.resolve.set_visible(conflicted);
        self.resolve.set_sensitive(enabled);
        self.stage.set_visible(!conflicted);
        self.stage.set_sensitive(enabled && stageable);
        self.unstage.set_visible(!conflicted);
        self.unstage.set_sensitive(enabled && unstageable);
        self.staged.set_visible(!conflicted);
        self.staged.set_sensitive(enabled && file.is_some());
        let partial = enabled
            && !self.cached()
            && !self.diff.side_by_side()
            && file
                .as_ref()
                .is_some_and(|f| f.worktree == 'M' && f.index != '?' && !f.conflicted && !f.submodule && f.original.is_none());
        self.hunk.set_visible(!conflicted);
        self.hunk.set_sensitive(partial);
        self.lines.set_visible(!conflicted);
        self.lines.set_sensitive(partial);
        let tip = if self.diff.side_by_side() { "Switch to the unified diff to stage parts of a file" } else { "" };
        for (b, normal) in [(&self.hunk, "Stage the diff hunk at the cursor"), (&self.lines, "Stage selected added or removed lines")] {
            b.set_tooltip_text(Some(if tip.is_empty() { normal } else { tip }));
        }
        self.more.set_sensitive(enabled && file.is_some());
        drop(file);
        self.update_commit();
    }
    /// Commit needs a message and something to commit (or Amend), and no conflicts.
    pub fn update_commit(&self) {
        let message = self.message();
        let summary = message.lines().next().unwrap_or_default().chars().count();
        self.length.set_text(&if summary > 0 { format!("{summary}/50") } else { String::new() });
        if summary > 72 {
            self.length.add_css_class("git-error");
        } else {
            self.length.remove_css_class("git-error");
        }
        let (staged, conflicts) = current()
            .and_then(|ui| {
                let s = ui.snapshot.borrow();
                s.as_ref().map(|s| {
                    let files = &s.status.files;
                    (
                        files.iter().filter(|f| f.index != '.' && f.index != '?' && !f.conflicted).count(),
                        files.iter().any(|f| f.conflicted),
                    )
                })
            })
            .unwrap_or_default();
        let amend = self.amend.is_active();
        self.commit.set_sensitive(self.enabled.get() && !message.trim().is_empty() && (staged > 0 || amend) && !conflicts);
        self.commit.set_label(&match (amend, staged) {
            (true, _) => "Amend commit".to_string(),
            (false, 0) | (false, 1) => "Commit".to_string(),
            (false, n) => format!("Commit {n} files"),
        });
        self.commit.set_tooltip_text(Some(if conflicts {
            "Resolve the conflicted files first"
        } else if staged == 0 && !amend {
            "Stage changes to commit them"
        } else if message.trim().is_empty() {
            "Write a commit message first"
        } else {
            "Commit staged changes (Ctrl+Enter)"
        }));
    }
}
fn selected_action(unstage: bool) {
    let Some(ui) = current() else { return };
    let Some(file) = ui.workflow.selected.borrow().clone() else { return };
    let mut paths = vec![file.path];
    if let Some(original) = file.original {
        paths.push(original);
    }
    if unstage {
        let unborn = ui.snapshot.borrow().as_ref().is_some_and(|s| s.repository.unborn);
        ui.execute(Operation::Unstage { paths, unborn });
    } else {
        ui.execute(Operation::Stage(paths));
    }
}
pub fn commit() {
    let Some(ui) = current() else { return };
    let Some(root) = ui.root() else { return };
    let message = ui.workflow.message();
    if message.trim().is_empty() {
        ui.toast("Write a commit message first.");
        ui.workflow.message.grab_focus();
        return;
    }
    if !ui.workflow.commit.is_sensitive() {
        if let Some(tip) = ui.workflow.commit.tooltip_text() {
            ui.toast(&format!("{tip}."));
        }
        return;
    }
    let amend = ui.workflow.amend.is_active();
    if amend {
        ask(
            "Amend the latest commit?",
            "This replaces HEAD and changes its commit hash. Avoid amending commits already shared with others. Only staged changes are included.",
            "Amend commit",
            false,
            move || {
                if let Some(ui) = current() {
                    ui.execute_at(root.clone(), Operation::Commit { message: message.clone(), amend: true });
                }
            },
        );
    } else {
        ui.execute(Operation::Commit { message, amend: false });
    }
}
/// Confirm a destructive action.
pub fn confirm(title: &str, message: &str, action: &str, run: impl Fn() + 'static) {
    ask(title, message, action, true, run);
}
/// Confirm an action; `destructive` styles its button as one.
pub fn ask(title: &str, message: &str, action: &str, destructive: bool, run: impl Fn() + 'static) {
    let (dialog, body) = widgets::dialog(title, 480);
    let label = widgets::label(message, "dim-label");
    label.set_wrap(true);
    body.append(&label);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    row.append(&cancel);
    let go = gtk::Button::with_label(action);
    go.add_css_class(if destructive { "destructive-action" } else { "suggested-action" });
    let d = dialog.clone();
    go.connect_clicked(move |_| {
        d.close();
        run();
    });
    row.append(&go);
    body.append(&row);
    dialog.present();
    // Enter confirms a safe action; a destructive one needs a deliberate choice.
    if destructive {
        cancel.grab_focus();
    } else {
        go.grab_focus();
    }
}
fn partial(lines: bool) {
    let Some(ui) = current() else { return };
    let Some(file) = ui.workflow.selected.borrow().clone() else { return };
    if ui.workflow.cached() {
        ui.toast("Switch to the unstaged diff to stage changes.");
        return;
    }
    let expected = ui.workflow.patch.borrow().clone();
    let range = if lines {
        ui.workflow.diff.range().ok_or_else(|| anyhow::anyhow!("Select added or removed lines in the diff"))
    } else {
        staging::hunk_range(&expected, ui.workflow.diff.cursor_line())
    };
    match range {
        Ok(range) => ui.execute(Operation::StageSelection { file: file.path, expected, range }),
        Err(e) => ui.toast(&format!("{e:#}")),
    }
}

impl super::window::Ui {
    /// Offer the repository's commit template, read in the background.
    pub fn load_template(&self, root: std::path::PathBuf) {
        *self.workflow.template_text.borrow_mut() = None;
        self.workflow.template.set_visible(false);
        nyx_core::cmd::background(
            move || {
                let text = nyx_git::core::files::commit_template(&root).map(|t| {
                    t.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n").trim_end().to_string()
                });
                (root, text.filter(|t| !t.trim().is_empty()))
            },
            |(root, text)| {
                let Some(ui) = current() else { return };
                if ui.root().as_ref() == Some(&root) {
                    ui.workflow.template.set_visible(text.is_some());
                    *ui.workflow.template_text.borrow_mut() = text;
                }
            },
        );
    }
    pub fn load_diff(&self) {
        let generation = self.workflow.generation.get() + 1;
        self.workflow.generation.set(generation);
        self.workflow.patch.borrow_mut().clear();
        // Re-evaluate the file's buttons; whether actions are allowed at all is `update_tasks`'s call.
        self.workflow.sensitive(self.workflow.enabled.get());
        let show = |text: &str| {
            self.workflow.shown.borrow_mut().take();
            self.workflow.diff.set_text(text);
        };
        let Some(file) = self.workflow.selected.borrow().clone() else {
            show("Select a file to view its diff.");
            return;
        };
        let Some(root) = self.root() else { return };
        let cached = self.workflow.cached();
        if file.conflicted {
            show(
                "This file has unresolved conflicts. Use Resolve conflict to compare base, ours and theirs, edit the result, and mark it resolved.",
            );
            return;
        }
        if cached && (file.index == '.' || file.index == '?') {
            show("No staged changes for this file.");
            return;
        }
        // Reloading the diff already on screen keeps it there until the new one arrives.
        let key = (file.path.clone(), cached);
        if self.workflow.shown.borrow().as_ref() != Some(&key) {
            show("Loading diff…");
        }
        let operation = Operation::Diff { file: file.path, cached, untracked: file.index == '?' };
        nyx_core::cmd::background(
            move || operations::run(&root, operation).map(|bytes| (root, bytes)),
            move |result| {
                let Some(ui) = current() else { return };
                if generation != ui.workflow.generation.get() {
                    return;
                }
                match result {
                    Ok((root, bytes)) if ui.root().as_ref() == Some(&root) => {
                        if bytes.len() > 2 * 1024 * 1024 {
                            ui.workflow
                                .diff
                                .set_text("Diff exceeds 2 MiB. Use whole-file staging or inspect it in an external tool.");
                            return;
                        }
                        let binary = bytes.split(|b| *b == b'\n').any(|l| l.starts_with(b"Binary files ") && l.ends_with(b" differ"));
                        ui.workflow.diff.set_text(if bytes.is_empty() {
                            "No changes on this side of the index."
                        } else if binary {
                            "A binary file changed. Git shows no text diff for it: stage, unstage or discard the whole file."
                        } else {
                            std::str::from_utf8(&bytes)
                                .unwrap_or("Non-UTF-8 diff. Use whole-file staging to preserve the original bytes.")
                        });
                        *ui.workflow.shown.borrow_mut() = Some(key);
                        *ui.workflow.patch.borrow_mut() = bytes;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        ui.workflow.shown.borrow_mut().take();
                        ui.workflow.diff.set_text(&format!("{e:#}"))
                    }
                }
            },
        );
    }
}
