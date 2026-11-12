use super::{diff_view::DiffView, window::current};
use gtk::{pango, prelude::*};
use nyx_git::core::{
    operations::{self, Operation},
    staging,
    status::FileStatus,
};
use nyx_ui::{menu, widgets};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};

/// Changes' parts that act on the selected file and the commit: the diff and its bar, the
/// composer card, and the banner for an operation in progress.
pub struct Workflow {
    /// All, Unstaged, Staged or Untracked: the file list's filter (shown as segments).
    pub category: gtk::DropDown,
    pub composer: gtk::Box,
    pub banner: gtk::Box,
    banner_title: gtk::Label,
    banner_note: gtk::Label,
    banner_studio: gtk::Button,
    banner_skip: gtk::Button,
    /// The diff bar over the diff, and the diff.
    pub detail: gtk::Box,
    pub diff: DiffView,
    pub selected: RefCell<Option<FileStatus>>,
    pub patch: RefCell<Vec<u8>>,
    pub generation: Cell<u64>,
    /// The file and side the diff view shows, so reloading it keeps the scroll position.
    shown: RefCell<Option<(PathBuf, bool)>>,
    /// The summary line and the rest of the message.
    pub summary: gtk::Entry,
    pub message: gtk::TextView,
    pub amend: gtk::CheckButton,
    enabled: Cell<bool>,
    /// A narrow window: Stage hunk and Stage lines move into ⋯.
    narrow: Cell<bool>,
    /// The staged side of the selected file (it's in the Staged group).
    staged: Cell<bool>,
    crumb: gtk::Label,
    side: gtk::ToggleButton,
    hunk: gtk::Button,
    lines: gtk::Button,
    discard: gtk::Button,
    resolve: gtk::Button,
    more: gtk::Button,
    commit: gtk::Button,
    commit_label: gtk::Label,
    length: gtk::Label,
    pub template: gtk::Button,
    /// The repository's `commit.template`, without its comment lines.
    pub template_text: RefCell<Option<String>>,
}
fn bar_button(title: &str, tip: &str, run: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::with_label(title);
    button.add_css_class("git-bar-action");
    button.set_tooltip_text(Some(tip));
    button.set_valign(gtk::Align::Center);
    button.connect_clicked(move |_| run());
    button
}
fn link(title: &str, tip: &str, run: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::with_label(title);
    button.add_css_class("git-link");
    button.set_tooltip_text(Some(tip));
    button.connect_clicked(move |_| run());
    button
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
fn resolve_selected() {
    if let Some(ui) = current()
        && let Some(file) = ui.workflow.selected.borrow().as_ref().filter(|f| f.conflicted)
    {
        super::conflict_studio_show(Some(file.path.clone()));
    }
}
impl Workflow {
    pub fn new() -> Self {
        // ----- An operation in progress -----
        let banner = widgets::hbox(12);
        banner.add_css_class("git-opbanner");
        let alert = gtk::Image::from_icon_name("git-alert-symbolic");
        alert.set_valign(gtk::Align::Center);
        banner.append(&alert);
        let words = widgets::vbox(2);
        words.set_hexpand(true);
        let banner_title = widgets::label("", "git-opbanner-title");
        banner_title.set_wrap(true);
        let banner_note = widgets::label("", "git-opbanner-note");
        banner_note.set_wrap(true);
        words.append(&banner_title);
        words.append(&banner_note);
        banner.append(&words);
        let banner_studio = gtk::Button::with_label("Conflict Studio");
        banner_studio.set_tooltip_text(Some("Resolve the conflicted files"));
        banner_studio.connect_clicked(|_| super::conflict_studio_show(None));
        let abort = gtk::Button::with_label("Abort…");
        abort.set_tooltip_text(Some("Abort the operation and go back to where it started"));
        abort.connect_clicked(|_| {
            super::advanced::prepare(nyx_git::core::advanced::Action::Abort, "Abort and restore the original operation state where Git can do so safely. Resolution edits and changes made during the operation may be lost.".into())
        });
        let banner_skip = gtk::Button::with_label("Skip…");
        banner_skip.set_tooltip_text(Some("Leave out the commit the operation stopped at"));
        banner_skip.connect_clicked(|_| {
            super::advanced::prepare(
                nyx_git::core::advanced::Action::Skip,
                "Leave out the commit that stopped the operation and carry on with the next one. Its changes won't be in the result.".into(),
            )
        });
        let next = gtk::Button::with_label("Continue…");
        next.add_css_class("suggested-action");
        next.set_tooltip_text(Some("Continue once every conflict is resolved and staged"));
        next.connect_clicked(|_| {
            super::advanced::prepare(
                nyx_git::core::advanced::Action::Continue,
                "Continue using Git's prepared message after all conflicts are resolved and staged.".into(),
            )
        });
        for b in [&banner_studio, &abort, &banner_skip, &next] {
            b.set_valign(gtk::Align::Center);
            banner.append(b);
        }
        banner.set_visible(false);

        let category = gtk::DropDown::from_strings(&["All changes", "Unstaged", "Staged", "Untracked"]);

        // ----- The composer card -----
        let composer = widgets::vbox(8);
        composer.add_css_class("git-composer-card");
        let field = widgets::hbox(6);
        field.add_css_class("git-field");
        let summary = gtk::Entry::new();
        summary.set_placeholder_text(Some("Summary"));
        summary.set_hexpand(true);
        summary.update_property(&[gtk::accessible::Property::Label("Commit summary")]);
        field.append(&summary);
        let length = widgets::label("", "git-count");
        length.set_tooltip_text(Some("Characters in the summary. Aim for 50 or fewer; many tools cut it at 72."));
        length.set_valign(gtk::Align::Center);
        field.append(&length);
        composer.append(&field);
        let message = gtk::TextView::new();
        message.set_wrap_mode(gtk::WrapMode::WordChar);
        message.set_left_margin(10);
        message.set_right_margin(10);
        message.set_top_margin(7);
        message.set_bottom_margin(7);
        message.add_css_class("git-message");
        message.update_property(&[gtk::accessible::Property::Label("Commit description")]);
        let placeholder = widgets::label("Description", "git-placeholder");
        placeholder.set_can_target(false);
        placeholder.set_valign(gtk::Align::Start);
        placeholder.set_margin_start(11);
        placeholder.set_margin_top(7);
        let p = placeholder.clone();
        message.buffer().connect_changed(move |b| p.set_visible(b.char_count() == 0));
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .min_content_height(58)
            .max_content_height(140)
            .propagate_natural_height(true)
            .child(&message)
            .build();
        scroll.add_css_class("git-description");
        let area = gtk::Overlay::new();
        area.set_child(Some(&scroll));
        area.add_overlay(&placeholder);
        composer.append(&area);
        // Enter in the summary moves on to the description.
        let m = message.clone();
        summary.connect_activate(move |_| {
            m.grab_focus();
        });
        let row = widgets::hbox(8);
        let amend = gtk::CheckButton::with_label("Amend");
        amend.set_tooltip_text(Some("Replace the latest commit; its hash changes"));
        row.append(&amend);
        let spacer = widgets::hbox(0);
        spacer.set_hexpand(true);
        row.append(&spacer);
        let template = link("Template", "Insert the repository's commit template (commit.template), without its comment lines", || {
            let Some(ui) = current() else { return };
            let Some(text) = ui.workflow.template_text.borrow().clone() else { return };
            let current = ui.workflow.message();
            ui.workflow.set_message(&if current.trim().is_empty() { text } else { format!("{current}\n{text}") });
            ui.workflow.summary.grab_focus();
        });
        template.set_visible(false);
        row.append(&template);
        let last = link("Last message", "Copy the latest commit message into the editor", || {
            let Some(ui) = current() else { return };
            let Some(root) = ui.root() else { return };
            let before = ui.workflow.message();
            nyx_core::cmd::background(
                move || operations::run(&root, Operation::LastMessage).map(|bytes| (root, bytes)),
                move |result| {
                    let Some(ui) = current() else { return };
                    match result {
                        Ok((root, bytes)) if ui.root().as_ref() == Some(&root) && ui.workflow.message() == before => {
                            ui.workflow.set_message(String::from_utf8_lossy(&bytes).trim_end())
                        }
                        Ok(_) => ui.toast("The editor or repository changed; the message was left intact."),
                        Err(e) => ui.error(&format!("{e:#}")),
                    }
                },
            );
        });
        row.append(&last);
        composer.append(&row);
        let commit = gtk::Button::new();
        commit.add_css_class("suggested-action");
        commit.add_css_class("git-commit");
        let inside = widgets::hbox(8);
        inside.set_halign(gtk::Align::Center);
        inside.append(&gtk::Image::from_icon_name("git-check-symbolic"));
        let commit_label = widgets::label("Commit", "");
        inside.append(&commit_label);
        inside.append(&widgets::label("Ctrl Enter", "git-commit-key"));
        commit.set_child(Some(&inside));
        commit.connect_clicked(|_| self::commit());
        composer.append(&commit);
        composer.set_sensitive(false);

        // ----- The diff bar and the diff -----
        let detail = widgets::vbox(0);
        detail.add_css_class("git-diffcol");
        let bar = widgets::hbox(6);
        bar.add_css_class("git-diffbar");
        let crumb = widgets::label("", "git-crumb");
        crumb.set_hexpand(true);
        crumb.set_ellipsize(pango::EllipsizeMode::Start);
        bar.append(&crumb);
        let side = gtk::ToggleButton::new();
        side.set_active(crate::prefs::get().side_by_side);
        side.connect_toggled(|b| {
            crate::prefs::update(|p| p.side_by_side = b.is_active());
            if let Some(ui) = current() {
                ui.workflow.diff.set_side_by_side(b.is_active());
                ui.workflow.sensitive(ui.workflow.is_enabled());
            }
        });
        let segments = super::pagebar::toggle_segments(&side, "Unified", "Split");
        segments.set_tooltip_text(Some("Show the diff as one column, or old and new side by side"));
        bar.append(&segments);
        let resolve = bar_button("Resolve…", "Resolve this file in Conflict Studio", resolve_selected);
        resolve.add_css_class("suggested-action");
        bar.append(&resolve);
        let hunk = bar_button("Stage hunk", "Stage the diff hunk at the cursor", || partial(false));
        bar.append(&hunk);
        let lines = bar_button("Stage lines", "Stage selected added or removed lines", || partial(true));
        bar.append(&lines);
        let discard_button = bar_button("Discard", "Discard this file's unstaged changes", discard);
        bar.append(&discard_button);
        let more = widgets::bar_button("git-more-symbolic", "File history, blame and more for this file (Shift+F10)");
        more.connect_clicked(|b| menu::popup_below(b, file_menu()));
        bar.append(&more);
        detail.append(&bar);
        let diff = DiffView::new();
        let view = diff.switchable();
        view.add_css_class("git-diff-card");
        view.set_overflow(gtk::Overflow::Hidden);
        detail.append(&view);
        diff.set_side_by_side(side.is_active());
        detail.set_vexpand(true);
        detail.set_hexpand(true);
        let me = Self {
            category,
            composer,
            banner,
            banner_title,
            banner_note,
            banner_studio,
            banner_skip,
            detail,
            diff,
            selected: RefCell::new(None),
            patch: RefCell::new(Vec::new()),
            generation: Cell::new(0),
            shown: RefCell::new(None),
            summary,
            message,
            amend,
            enabled: Cell::new(false),
            narrow: Cell::new(false),
            staged: Cell::new(false),
            crumb,
            side,
            hunk,
            lines,
            discard: discard_button,
            resolve,
            more,
            commit,
            commit_label,
            length,
            template,
            template_text: RefCell::new(None),
        };
        me.sensitive(false);
        me
    }
    pub fn operation_status(&self, progress: Option<nyx_git::core::advanced::Active>, conflicts: bool) {
        self.banner.set_visible(progress.is_some() || conflicts);
        self.banner_title.set_text(&match progress {
            Some(p) => format!("{} in progress", p.name()),
            None => "Conflicts in the working tree".into(),
        });
        self.banner_note.set_text(if conflicts {
            "Resolve every conflicted file, stage the results, then continue."
        } else {
            "Nothing is conflicted. Continue to carry on, or abort to go back."
        });
        self.banner_studio.set_visible(conflicts);
        self.banner_skip.set_visible(matches!(
            progress,
            Some(nyx_git::core::advanced::Active::Rebase | nyx_git::core::advanced::Active::CherryPick | nyx_git::core::advanced::Active::Revert)
        ));
    }
    /// The whole message: the summary, a blank line, then the description.
    pub fn message(&self) -> String {
        let b = self.message.buffer();
        let body = b.text(&b.start_iter(), &b.end_iter(), false);
        let summary = self.summary.text();
        if body.trim().is_empty() { summary.to_string() } else { format!("{summary}\n\n{body}") }
    }
    /// Split `text` into the summary (its first line) and the description (the rest).
    pub fn set_message(&self, text: &str) {
        let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
        self.summary.set_text(first);
        self.message.buffer().set_text(rest.trim_start_matches('\n'));
    }
    /// Whenever the summary or the description changes.
    pub fn connect_message_changed(&self, f: impl Fn() + 'static) {
        let f = std::rc::Rc::new(f);
        let g = f.clone();
        self.summary.connect_changed(move |_| g());
        self.message.buffer().connect_changed(move |_| f());
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled.get()
    }
    /// The diff compares the index with HEAD: the file was chosen in the Staged group.
    pub fn cached(&self) -> bool {
        self.staged.get()
    }
    /// Show `file`'s staged side (`cached`) or its working changes.
    pub fn select(&self, file: Option<FileStatus>, cached: bool) {
        self.generation.set(self.generation.get() + 1);
        self.patch.borrow_mut().clear();
        self.crumb.set_markup(&file.as_ref().map(|f| crumb(&f.path, f.original.as_deref())).unwrap_or_default());
        self.crumb.set_tooltip_text(file.as_ref().map(|f| f.path.to_string_lossy().into_owned()).as_deref());
        *self.selected.borrow_mut() = file;
        self.staged.set(cached);
        if let Some(ui) = current() {
            ui.load_diff();
        }
    }
    /// Enable what applies to the selected file; hide what never could.
    pub fn sensitive(&self, enabled: bool) {
        self.enabled.set(enabled);
        self.banner.set_sensitive(enabled);
        self.composer.set_sensitive(enabled);
        let file = self.selected.borrow();
        let conflicted = file.as_ref().is_some_and(|f| f.conflicted);
        let unstaged = !self.cached() && file.as_ref().is_some_and(|f| f.worktree != '.' && !f.conflicted);
        self.resolve.set_visible(conflicted);
        self.resolve.set_sensitive(enabled);
        let partial = enabled
            && !self.cached()
            && !self.diff.side_by_side()
            && file
                .as_ref()
                .is_some_and(|f| f.worktree == 'M' && f.index != '?' && !f.conflicted && !f.submodule && f.original.is_none());
        for b in [&self.hunk, &self.lines] {
            b.set_visible(!conflicted && !self.cached() && !self.narrow.get());
            b.set_sensitive(partial);
        }
        let tip = if self.diff.side_by_side() { "Switch to Unified to stage parts of a file" } else { "" };
        for (b, normal) in [(&self.hunk, "Stage the diff hunk at the cursor"), (&self.lines, "Stage selected added or removed lines")] {
            b.set_tooltip_text(Some(if tip.is_empty() { normal } else { tip }));
        }
        self.discard.set_visible(unstaged);
        self.discard.set_sensitive(enabled && file.as_ref().is_some_and(|f| !f.submodule && f.original.is_none()));
        self.side.set_sensitive(!conflicted);
        self.more.set_sensitive(enabled && file.is_some());
        drop(file);
        self.update_commit();
    }
    pub fn set_narrow(&self, narrow: bool) {
        if self.narrow.replace(narrow) != narrow {
            self.sensitive(self.enabled.get());
        }
    }
    /// Commit needs a message and something to commit (or Amend), and no conflicts.
    pub fn update_commit(&self) {
        let message = self.message();
        let summary = self.summary.text().chars().count();
        self.length.set_text(&if summary > 0 { format!("{summary}/50") } else { String::new() });
        for (class, on) in [("warn", summary > 50 && summary <= 72), ("bad", summary > 72)] {
            if on {
                self.length.add_css_class(class);
            } else {
                self.length.remove_css_class(class);
            }
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
        self.commit_label.set_text(&match (amend, staged) {
            (true, _) => "Amend commit".to_string(),
            (false, 0) => "Commit".to_string(),
            (false, 1) => "Commit 1 file".to_string(),
            (false, n) => format!("Commit {n} files"),
        });
        self.commit.set_tooltip_text(Some(if conflicts {
            "Resolve the conflicted files first"
        } else if staged == 0 && !amend {
            "Stage changes to commit them"
        } else if message.trim().is_empty() {
            "Write a summary first"
        } else {
            "Commit staged changes (Ctrl+Enter)"
        }));
    }
}
/// "apps › git › src › **window.rs**", with where a rename came from.
fn crumb(path: &std::path::Path, original: Option<&std::path::Path>) -> String {
    let parts: Vec<String> = path.iter().map(|p| gtk::glib::markup_escape_text(&p.to_string_lossy()).to_string()).collect();
    let (last, dirs) = parts.split_last().map(|(l, d)| (l.clone(), d.to_vec())).unwrap_or_default();
    let mut out = dirs.iter().map(|d| format!("{d}  <span alpha=\"50%\">›</span>  ")).collect::<String>();
    out.push_str(&format!("<b>{last}</b>"));
    if let Some(o) = original {
        out.push_str(&format!("  <span alpha=\"60%\">from {}</span>", gtk::glib::markup_escape_text(&o.to_string_lossy())));
    }
    out
}
/// The selected file's ⋯: what else can be done with it.
fn file_menu() -> Vec<menu::Entry> {
    let Some(ui) = current() else { return Vec::new() };
    let Some(file) = ui.workflow.selected.borrow().clone() else { return Vec::new() };
    let staged = ui.workflow.cached();
    let mut out = Vec::new();
    if ui.workflow.narrow.get() && !staged && !file.conflicted {
        let can = ui.workflow.hunk.is_sensitive();
        out.push(menu::item("Stage hunk", || partial(false)).icon("git-plus-symbolic").enabled(can));
        out.push(menu::item("Stage lines", || partial(true)).icon("git-plus-symbolic").enabled(can));
    }
    if !file.conflicted {
        out.push(if staged {
            menu::item("Unstage file", || stage_selected(true)).icon("git-minus-symbolic").key("Space")
        } else {
            menu::item("Stage file", || stage_selected(false)).icon("git-plus-symbolic").key("Space")
        });
        out.push(menu::separator());
    }
    out.push(menu::item("File history…", super::file_history::selected_change).icon("git-history-symbolic"));
    out.push(menu::item("Blame…", blame_selected).icon("git-user-symbolic").enabled(file.index != '?' && file.worktree != 'D'));
    let path = file.path.clone();
    out.push(menu::item("Copy path", move || {
        if let Some(ui) = current() {
            gtk::prelude::WidgetExt::display(&ui.window).clipboard().set_text(&path.to_string_lossy());
        }
    }).icon("git-copy-symbolic"));
    if !staged && !file.conflicted {
        out.push(menu::separator());
        out.push(menu::item("Discard changes…", discard).icon("git-trash-symbolic").danger());
    }
    out
}
/// Stage the selected file, or unstage it (`unstage`), with where a rename came from.
pub fn stage_selected(unstage: bool) {
    let Some(ui) = current() else { return };
    let Some(file) = ui.workflow.selected.borrow().clone() else { return };
    stage_file(&file, unstage);
}
pub fn stage_file(file: &FileStatus, unstage: bool) {
    let Some(ui) = current() else { return };
    let mut paths = vec![file.path.clone()];
    if let Some(original) = &file.original {
        paths.push(original.clone());
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
        ui.toast("Write a summary first.");
        ui.workflow.summary.grab_focus();
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
        ui.toast("Choose the file under Changes to stage parts of it.");
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
            show("This file has unresolved conflicts. Resolve opens it in Conflict Studio: compare base, ours and theirs, choose a result, and mark it resolved.");
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
