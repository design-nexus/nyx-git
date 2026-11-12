use super::{diff_view::DiffView, window::current};
use gtk::{glib, prelude::*};
use nyx_git::core::{
    history::{self, Details},
    operations::{self, Operation},
};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::PathBuf,
};
pub struct CommitDetails {
    pub root: gtk::Box,
    metadata: gtk::Label,
    hash: gtk::Label,
    person: gtk::Box,
    avatar: gtk::Label,
    author: gtk::Label,
    when: gtk::Label,
    card: gtk::Box,
    subject: gtk::Label,
    message: gtk::Label,
    actions: gtk::Box,
    files: gtk::ListBox,
    files_title: gtk::Label,
    files_scroll: gtk::ScrolledWindow,
    paths: RefCell<Vec<PathBuf>>,
    data: RefCell<Option<Details>>,
    generation: Cell<u64>,
    diff_generation: Cell<u64>,
    syncing: Cell<bool>,
    pub diff: DiffView,
}
impl CommitDetails {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.add_css_class("git-insp");
        root.add_css_class("git-commit-details");
        root.set_hexpand(true);
        root.set_vexpand(true);
        let top = widgets::hbox(4);
        top.add_css_class("git-insp-title");
        let hash = widgets::label("", "git-commit-hash");
        hash.set_selectable(true);
        top.append(&hash);
        let copy = widgets::bar_button("git-copy-symbolic", "Copy the commit hash (Ctrl+C)");
        copy.connect_clicked(|b| {
            if let Some(ui) = current()
                && let Some(data) = ui.graph.details.data.borrow().as_ref()
            {
                b.display().clipboard().set_text(&data.id);
                ui.toast("Commit hash copied.");
            }
        });
        top.append(&copy);
        root.append(&top);
        let metadata = widgets::label("Select a commit to inspect it.", "git-insp-note");
        metadata.set_selectable(true);
        metadata.set_wrap(true);
        metadata.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        root.append(&metadata);
        let person = widgets::hbox(10);
        person.add_css_class("git-person");
        let avatar = widgets::label("", "git-avatar");
        avatar.set_xalign(0.5);
        avatar.set_valign(gtk::Align::Center);
        person.append(&avatar);
        let who = widgets::vbox(1);
        let author = widgets::label("", "git-person-name");
        author.set_selectable(true);
        author.set_ellipsize(gtk::pango::EllipsizeMode::End);
        who.append(&author);
        let when = widgets::label("", "git-insp-note");
        when.set_selectable(true);
        when.set_wrap(true);
        who.append(&when);
        person.append(&who);
        root.append(&person);
        let card = widgets::vbox(4);
        card.add_css_class("git-message-card");
        let subject = widgets::label("", "git-message-subject");
        subject.set_wrap(true);
        subject.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        subject.set_selectable(true);
        card.append(&subject);
        let message = widgets::label("", "git-message-body");
        message.set_wrap(true);
        message.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        message.set_selectable(true);
        message.update_property(&[gtk::accessible::Property::Label("Commit message")]);
        card.append(&message);
        let card_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .propagate_natural_height(true)
            .max_content_height(150)
            .child(&card)
            .build();
        root.append(&card_scroll);
        let actions = widgets::hbox(6);
        actions.add_css_class("git-commit-actions");
        let target = |icon: &str, label: &str, tip: &str, kind: &'static str| {
            let b = widgets::labeled_button(icon, label);
            b.set_tooltip_text(Some(tip));
            b.connect_clicked(move |_| {
                if let Some(ui) = current()
                    && let Some(id) = ui.graph.details.data.borrow().as_ref().map(|d| d.id.clone())
                {
                    super::advanced::target_action(kind, id);
                }
            });
            b
        };
        actions.append(&target("git-branch-symbolic", "Branch here", "Create a branch at this commit", "branch"));
        actions.append(&target("git-cherry-symbolic", "Cherry-pick", "Apply this commit's changes on the current branch", "cherry"));
        let more = widgets::bar_button("git-more-symbolic", "Check out, revert, reset, tag and compare (Shift+F10)");
        more.connect_clicked(|_| {
            if let Some(ui) = current() {
                ui.graph.selected_menu();
            }
        });
        actions.append(&more);
        root.append(&actions);
        let heading = widgets::hbox(8);
        let files_title = widgets::label("CHANGED FILES", "git-insp-group");
        files_title.set_hexpand(true);
        files_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        heading.append(&files_title);
        let history = gtk::Button::with_label("File history");
        history.add_css_class("git-link");
        history.set_valign(gtk::Align::Center);
        history.set_tooltip_text(Some("The selected file's history up to this commit"));
        history.connect_clicked(|_| {
            if let Some(ui) = current()
                && let Some(root) = ui.root()
            {
                let details = &ui.graph.details;
                if let Some(file) = details.selected_path()
                    && let Some(data) = details.data.borrow().as_ref()
                {
                    super::file_history::open(root, file, Some(data.id.clone()));
                }
            }
        });
        heading.append(&history);
        root.append(&heading);
        let files = gtk::ListBox::new();
        files.add_css_class("git-commit-files");
        files.set_selection_mode(gtk::SelectionMode::Single);
        files.update_property(&[gtk::accessible::Property::Label("Files changed in selected commit")]);
        files.connect_row_selected(|_, _| {
            if let Some(ui) = current()
                && !ui.graph.details.syncing.get()
            {
                ui.graph.details.load_diff();
            }
        });
        let files_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .propagate_natural_height(true)
            .max_content_height(180)
            .child(&files)
            .build();
        root.append(&files_scroll);
        let diff = DiffView::new();
        diff.root.add_css_class("git-diff-card");
        diff.root.set_overflow(gtk::Overflow::Hidden);
        root.append(&diff.root);
        let me = Self {
            root,
            metadata,
            hash,
            person,
            avatar,
            author,
            when,
            card,
            subject,
            message,
            actions,
            files,
            files_title,
            files_scroll,
            paths: RefCell::new(Vec::new()),
            data: RefCell::new(None),
            generation: Cell::new(0),
            diff_generation: Cell::new(0),
            syncing: Cell::new(false),
            diff,
        };
        me.show_commit(false);
        me
    }
    /// The commit's own parts, shown once one is loaded.
    fn show_commit(&self, shown: bool) {
        for w in [self.hash.upcast_ref::<gtk::Widget>(), self.person.upcast_ref(), self.card.upcast_ref(), self.actions.upcast_ref()] {
            w.set_visible(shown);
        }
        if let Some(top) = self.hash.parent() {
            top.set_visible(shown);
        }
        if let Some(scroll) = self.card.parent().and_then(|v| v.parent()) {
            scroll.set_visible(shown);
        }
        self.metadata.set_visible(!shown);
    }
    fn selected_path(&self) -> Option<PathBuf> {
        let index = self.files.selected_row()?.index();
        self.paths.borrow().get(index as usize).cloned()
    }
    pub fn focus(&self) -> bool {
        self.metadata.grab_focus()
    }
    pub fn clear(&self) {
        self.generation.set(self.generation.get() + 1);
        self.diff_generation.set(self.diff_generation.get() + 1);
        self.metadata.set_text("Select a commit to inspect it.");
        self.message.set_text("");
        self.show_commit(false);
        *self.data.borrow_mut() = None;
        self.paths.borrow_mut().clear();
        self.diff.set_text("");
        self.files_title.set_text("CHANGED FILES");
        self.syncing.set(true);
        self.files.remove_all();
        self.syncing.set(false);
    }
    pub fn select(&self, id: String) {
        self.clear();
        let generation = self.generation.get();
        let Some(ui) = current() else { return };
        let Some(root) = ui.root() else { return };
        self.metadata.set_text("Loading commit…");
        nyx_core::cmd::background(
            move || -> anyhow::Result<_> {
                let data = history::details(&root, &id)?;
                let repo = gix::discover(&root)?;
                let boundary = repo.shallow_commits()?.is_some_and(|ids| ids.iter().any(|oid| oid.to_string() == id))
                    && !data.parents.is_empty();
                let files = if boundary {
                    Vec::new()
                } else {
                    operations::run(&root, Operation::CommitFiles { id, parent: data.parents.first().cloned() })?
                };
                let records: Vec<_> = files.split(|b| *b == 0).filter(|b| !b.is_empty()).collect();
                let mut paths = Vec::new();
                let mut names = Vec::new();
                for pair in records.as_chunks::<2>().0 {
                    let path = PathBuf::from(OsString::from_vec(pair[1].to_vec()));
                    names.push(String::from_utf8_lossy(pair[0]).chars().next().unwrap_or('M'));
                    paths.push(path);
                }
                Ok((root, data, paths, names, boundary))
            },
            move |result| {
                let Some(ui) = current() else { return };
                let me = &ui.graph.details;
                if me.generation.get() != generation {
                    return;
                }
                match result {
                    Ok((root, data, paths, names, boundary)) if ui.root().as_ref() == Some(&root) => {
                        let time = glib::DateTime::from_unix_local(data.timestamp)
                            .ok()
                            .and_then(|t| t.format("%e %b %Y at %H:%M").ok())
                            .map(|t| t.trim().to_string())
                            .unwrap_or_default();
                        let parents = match data.parents.len() {
                            0 => "root commit".to_string(),
                            1 => format!("parent {}", &data.parents[0][..7]),
                            _ => format!("merge of {}", data.parents.iter().map(|p| &p[..7]).collect::<Vec<_>>().join(" and ")),
                        };
                        me.hash.set_text(&data.id[..data.id.len().min(10)]);
                        me.hash.set_tooltip_text(Some(&data.id));
                        me.avatar.set_text(&initials(&data.author));
                        me.author.set_text(&data.author);
                        me.author.set_tooltip_text(Some(&format!("{} <{}>", data.author, data.email)));
                        me.when.set_text(&format!("{} · {time} · {parents}", super::graph_renderer::ago(data.timestamp)));
                        let (first, rest) = data.message.trim_end().split_once('\n').unwrap_or((data.message.trim_end(), ""));
                        me.subject.set_text(first);
                        me.message.set_text(rest.trim_start_matches('\n'));
                        me.message.set_visible(!rest.trim().is_empty());
                        me.show_commit(true);
                        me.files_title.set_text(&format!(
                            "CHANGED FILES  {}{}",
                            paths.len(),
                            if data.parents.len() > 1 { " · vs first parent" } else { "" }
                        ));
                        *me.data.borrow_mut() = Some(data);
                        me.syncing.set(true);
                        for (code, path) in names.iter().zip(paths.iter()) {
                            me.files.append(&file_row(*code, path));
                        }
                        *me.paths.borrow_mut() = paths;
                        me.files.select_row(me.files.row_at_index(0).as_ref());
                        me.files_scroll.vadjustment().set_value(0.0);
                        me.syncing.set(false);
                        if boundary {
                            me.diff.set_text("Shallow history boundary: the parent object is unavailable. Fetch more history to inspect these changes.");
                        } else {
                            me.load_diff();
                        }
                    }
                    Ok(_) => {}
                    Err(e) => me.metadata.set_text(&format!("{e:#}")),
                }
            },
        );
    }
    fn load_diff(&self) {
        let Some(ui) = current() else { return };
        let Some(root) = ui.root() else { return };
        let generation = self.diff_generation.get() + 1;
        self.diff_generation.set(generation);
        let Some(data) = self.data.borrow().clone() else { return };
        let Some(path) = self.selected_path() else {
            self.diff.set_text("No file changes in this commit.");
            return;
        };
        self.diff.set_text("Loading diff…");
        nyx_core::cmd::background(
            move || {
                operations::run(&root, Operation::CommitDiff { id: data.id, parent: data.parents.first().cloned(), file: path })
                    .map(|bytes| (root, bytes))
            },
            move |result| {
                let Some(ui) = current() else { return };
                let me = &ui.graph.details;
                if me.diff_generation.get() != generation {
                    return;
                }
                match result {
                    Ok((root, bytes)) if ui.root().as_ref() == Some(&root) => me.show_bytes(&bytes),
                    Ok(_) => {}
                    Err(e) => me.diff.set_text(&format!("{e:#}")),
                }
            },
        );
    }
    pub fn compare(&self, base: String, tip: String) {
        self.clear();
        let generation = self.generation.get();
        self.metadata.set_text(&format!("Compare {} → {}", &base[..10], &tip[..10]));
        self.diff.set_text("Loading comparison…");
        let Some(ui) = current() else { return };
        let Some(root) = ui.root() else { return };
        nyx_core::cmd::background(
            move || operations::run(&root, Operation::Compare { base, tip }).map(|bytes| (root, bytes)),
            move |result| {
                let Some(ui) = current() else { return };
                let me = &ui.graph.details;
                if me.generation.get() != generation {
                    return;
                }
                match result {
                    Ok((root, bytes)) if ui.root().as_ref() == Some(&root) => me.show_bytes(&bytes),
                    Ok(_) => {}
                    Err(e) => me.diff.set_text(&format!("{e:#}")),
                }
            },
        );
    }
    fn show_bytes(&self, bytes: &[u8]) {
        self.diff.set_text(if bytes.len() > 2 * 1024 * 1024 {
            "Diff exceeds 2 MiB. Inspect it with an external tool."
        } else if bytes.is_empty() {
            "No content differences."
        } else {
            std::str::from_utf8(bytes).unwrap_or("Non-UTF-8 diff. Inspect the original bytes with an external tool.")
        });
    }
}
/// "Ken Smith" as "KS".
fn initials(name: &str) -> String {
    let mut out: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
    if out.is_empty() {
        out.push('?');
    }
    out.to_uppercase()
}
/// A changed file: its status letter, name and folder.
fn file_row(code: char, path: &std::path::Path) -> gtk::Box {
    let row = widgets::hbox(10);
    row.add_css_class("git-file-row");
    let chip = widgets::label(&code.to_string(), "git-chip");
    chip.set_xalign(0.5);
    chip.add_css_class(match code {
        'A' | 'C' => "added",
        'D' => "deleted",
        'R' => "renamed",
        _ => "modified",
    });
    chip.set_valign(gtk::Align::Center);
    row.append(&chip);
    let name = widgets::label(&path.file_name().unwrap_or_default().to_string_lossy(), "git-file-name");
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    row.append(&name);
    let dir = widgets::label(&path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(), "git-file-dir");
    dir.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    dir.set_hexpand(true);
    row.append(&dir);
    row.set_tooltip_text(Some(&path.to_string_lossy()));
    row
}
