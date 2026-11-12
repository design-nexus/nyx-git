//! The sidebar: the repository's worktrees, branches, remotes, tags and stashes, each a
//! group that folds, with `+` to make one. Counts are commits ahead of (and behind) the
//! upstream; a green dot is an agent working there, amber one waiting for you. Right-click
//! or Shift+F10 on a row for what else it can do.
use super::{Item, item, window::current};
use crate::prefs;
use gtk::{gdk, glib, prelude::*};
use nyx_git::core::{
    agents,
    navigation::{self, Action, Inventory, Kind},
};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
};
/// Rows shown per group; the rest are behind the group's manager.
const ROWS: usize = 100;
/// The groups in the order shown, with their titles and what `+` makes.
const GROUPS: &[(Kind, &str, &str)] = &[
    (Kind::Worktree, "Worktrees", "New worktree…"),
    (Kind::Local, "Branches", "New branch from here…"),
    (Kind::Remote, "Remotes", "Add a remote…"),
    (Kind::Tag, "Tags", ""),
    (Kind::Stash, "Stashes", "Stash the working changes…"),
];
pub struct Sidebar {
    pub root: gtk::Box,
    search: gtk::SearchEntry,
    status: gtk::Label,
    groups: gtk::Box,
    repository: RefCell<Option<PathBuf>>,
    head: RefCell<Option<String>>,
    data: RefCell<Option<Inventory>>,
    dirty: Cell<bool>,
    pending: Cell<bool>,
    generation: Cell<u64>,
}
/// Whether `key`'s group is folded in `root`. Remotes and tags start folded.
fn closed(root: &Path, key: &str) -> bool {
    let p = prefs::get();
    let list = p.workspace.repositories.iter().find(|r| r.root == root).map(|r| r.sidebar_closed.clone()).unwrap_or_default();
    if list.iter().any(|k| k == "2/seen") {
        list.iter().any(|k| *k == format!("2/{key}"))
    } else {
        matches!(key, "remotes" | "tags")
    }
}
fn set_closed(root: &Path, key: &str, fold: bool) {
    prefs::update(|p| {
        let list = &mut p.workspace.entry(root).sidebar_closed;
        if !list.iter().any(|k| k == "2/seen") {
            list.retain(|k| !k.starts_with("2/"));
            list.extend(["2/seen", "2/remotes", "2/tags"].map(String::from));
        }
        let name = format!("2/{key}");
        list.retain(|k| *k != name);
        if fold {
            list.push(name);
        }
    });
}
/// A row in the shared sidebar style: icon, name, then a count or a dot.
fn nav_row(icon: &str, title: &str, readout: &str, dot: Option<&str>, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    let content = widgets::hbox(10);
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class("nav-icon");
    content.append(&image);
    let label = widgets::label(title, "nav-label");
    label.add_css_class("compact-hide");
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&label);
    if !readout.is_empty() {
        let r = widgets::label(readout, "nav-readout");
        r.add_css_class("compact-hide");
        content.append(&r);
    }
    if let Some(tone) = dot {
        let d = widgets::hbox(0);
        d.add_css_class("git-dot");
        d.add_css_class(tone);
        d.set_valign(gtk::Align::Center);
        content.append(&d);
    }
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(tooltip));
    button.update_property(&[gtk::accessible::Property::Label(title)]);
    button
}
fn track(track: Option<(usize, usize)>) -> String {
    match track {
        Some((0, 0)) | None => String::new(),
        Some((a, 0)) => format!("↑{a}"),
        Some((0, b)) => format!("↓{b}"),
        Some((a, b)) => format!("↓{b} ↑{a}"),
    }
}
impl Sidebar {
    pub fn new() -> Self {
        let root = widgets::vbox(1);
        root.set_hexpand(false);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Filter"));
        search.add_css_class("sidebar-search");
        search.add_css_class("git-sidebar-filter");
        search.add_css_class("compact-hide");
        search.set_width_chars(12);
        search.set_max_width_chars(22);
        search.update_property(&[gtk::accessible::Property::Label("Filter worktrees, branches, remotes, tags and stashes")]);
        search.connect_search_changed(|_| {
            if let Some(ui) = current() {
                ui.sidebar.render();
            }
        });
        search.connect_stop_search(|entry| entry.set_text(""));
        root.append(&search);
        let status = widgets::label("", "git-insp-note");
        status.add_css_class("compact-hide");
        status.set_wrap(true);
        status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        status.set_margin_start(8);
        status.set_visible(false);
        root.append(&status);
        let groups = widgets::vbox(1);
        root.append(&groups);
        Self {
            root,
            search,
            status,
            groups,
            repository: RefCell::new(None),
            head: RefCell::new(None),
            data: RefCell::new(None),
            dirty: Cell::new(true),
            pending: Cell::new(false),
            generation: Cell::new(0),
        }
    }
    fn set_status(&self, text: &str) {
        self.status.set_text(text);
        self.status.set_visible(!text.is_empty());
    }
    pub fn invalidate(&self) {
        self.dirty.set(true);
    }
    /// The branches, tags, stashes, worktrees and remotes last read.
    pub fn inventory(&self) -> Option<Inventory> {
        self.data.borrow().clone()
    }
    pub fn focus(&self) -> bool {
        self.root.is_mapped() && self.search.grab_focus()
    }
    pub fn clear(&self) {
        self.repository.borrow_mut().take();
        self.head.borrow_mut().take();
        self.data.borrow_mut().take();
        self.generation.set(self.generation.get() + 1);
        self.pending.set(false);
        self.dirty.set(true);
        self.render();
    }
    pub fn refresh(&self, root: PathBuf, head: Option<String>) {
        if self.repository.borrow().as_ref() != Some(&root) {
            *self.repository.borrow_mut() = Some(root.clone());
            self.data.borrow_mut().take();
            self.generation.set(self.generation.get() + 1);
            self.pending.set(false);
            self.dirty.set(true);
            self.search.set_text("");
            self.render();
        }
        if *self.head.borrow() != head {
            *self.head.borrow_mut() = head;
            self.dirty.set(true);
        }
        if self.pending.get() || !self.dirty.replace(false) {
            return;
        }
        self.pending.set(true);
        if self.data.borrow().is_none() {
            self.set_status("Reading the repository…");
        }
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        nyx_core::cmd::background(
            move || navigation::read(&root).map(|data| (root, data)),
            move |result| {
                let Some(ui) = current() else { return };
                let sidebar = &ui.sidebar;
                if sidebar.generation.get() != generation {
                    return;
                }
                sidebar.pending.set(false);
                match result {
                    Ok((root, data)) if ui.root().as_ref() == Some(&root) => {
                        sidebar.set_status("");
                        if sidebar.data.borrow().as_ref() != Some(&data) {
                            *sidebar.data.borrow_mut() = Some(data);
                            sidebar.render();
                        }
                    }
                    Ok(_) => {}
                    Err(e) => sidebar.set_status(&format!("{e:#}\nF5 to retry")),
                }
                if sidebar.dirty.get()
                    && let Some(snapshot) = ui.snapshot.borrow().as_ref()
                {
                    sidebar.refresh(snapshot.repository.root.clone(), snapshot.repository.head.clone());
                }
            },
        );
    }
    /// The changed-file count beside the open worktree follows the status.
    pub fn changed(&self) {
        if self.data.borrow().is_some() {
            self.render();
        }
    }
    fn render(&self) {
        while let Some(child) = self.groups.first_child() {
            self.groups.remove(&child);
        }
        let Some(root) = self.repository.borrow().clone() else { return };
        let query = self.search.text().to_lowercase();
        let data = self.data.borrow();
        let Some(data) = data.as_ref() else { return };
        let ui = current();
        let changed = ui.as_ref().and_then(|ui| ui.snapshot.borrow().as_ref().map(|s| s.status.files.len())).unwrap_or(0);
        let sessions = agents::sessions();
        let worktree_paths: Vec<PathBuf> = data
            .groups
            .iter()
            .filter(|g| g.kind == Kind::Worktree)
            .flat_map(|g| g.items.iter())
            .filter_map(|i| if let Action::Worktree(p) = &i.action { Some(p.clone()) } else { None })
            .collect();
        // A dot for the agents working in a worktree or on a branch: amber when one waits.
        let dot = |worktree: Option<&Path>, branch: Option<&str>| -> Option<(&'static str, String)> {
            let here: Vec<&agents::Session> = sessions
                .iter()
                .filter(|s| {
                    worktree.is_some_and(|w| agents::worktree_of(s, &worktree_paths).is_some_and(|p| p == w))
                        || branch.is_some_and(|b| s.branch.as_deref() == Some(b))
                })
                .collect();
            let first = here.first()?;
            let waiting = here.iter().any(|s| s.activity == "blocked");
            Some((if waiting { "warn" } else { "ok" }, format!("{} · {}", first.agent(), first.state())))
        };
        let mut first = true;
        for (kind, title, add) in GROUPS {
            let Some(group) = data.groups.iter().find(|g| g.kind == *kind) else { continue };
            let matches = group
                .items
                .iter()
                .filter(|item| query.is_empty() || format!("{} {}", item.name, item.detail).to_lowercase().contains(&query))
                .collect::<Vec<_>>();
            if !query.is_empty() && matches.is_empty() {
                continue;
            }
            let fold = query.is_empty() && closed(&root, kind.key());
            self.groups.append(&heading(&root, *kind, title, add, group.items.len(), fold, first));
            first = false;
            if fold {
                continue;
            }
            for item in matches.iter().take(ROWS) {
                let (icon, name, readout, active, mark) = match &item.action {
                    Action::Worktree(path) => {
                        let here = path == &root;
                        (
                            "git-worktree-symbolic",
                            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| item.name.clone()),
                            if here && changed > 0 { changed.to_string() } else { String::new() },
                            here,
                            dot(Some(path), None),
                        )
                    }
                    Action::Reference { current: is_current, .. } if *kind == Kind::Local => (
                        "git-branch-symbolic",
                        item.name.clone(),
                        track(item.track),
                        *is_current,
                        dot(None, Some(&item.name)),
                    ),
                    Action::Reference { .. } => ("git-tag-symbolic", item.name.clone(), String::new(), false, None),
                    Action::Stash(_) => ("git-stash-symbolic", item.detail.lines().next().unwrap_or(&item.name).to_string(), String::new(), false, None),
                    Action::Remote(_) => ("git-remote-symbolic", item.name.clone(), String::new(), false, None),
                };
                let tip = format!(
                    "{}\n{}{}\nRight-click or Shift+F10 for actions",
                    if matches!(item.action, Action::Worktree(_)) { format!("{name} · {}", item.name) } else { item.name.clone() },
                    item.detail.lines().filter(|l| !l.is_empty() && *l != item.name).collect::<Vec<_>>().join("\n"),
                    mark.as_ref().map(|(_, m)| format!("\n{m}")).unwrap_or_default()
                );
                let button = nav_row(icon, &name, &readout, mark.as_ref().map(|(t, _)| *t), tip.trim());
                if active {
                    button.add_css_class("active");
                }
                // The icon rail keeps the worktrees; the other groups fold away.
                if *kind != Kind::Worktree {
                    button.add_css_class("compact-hide");
                }
                let (r, action) = (root.clone(), item.action.clone());
                button.connect_clicked(move |_| activate(&r, &action));
                let (r, action) = (root.clone(), item.action.clone());
                context_menu(&button, move |b, x, y| menu(b, &r, &action, x, y));
                self.groups.append(&button);
            }
            if matches.len() > ROWS || group.limited {
                let more = gtk::Button::with_label(&format!("All {}…", title.to_lowercase()));
                more.add_css_class("git-link");
                more.add_css_class("compact-hide");
                more.set_halign(gtk::Align::Start);
                more.set_margin_start(6);
                let kind = *kind;
                let r = root.clone();
                more.connect_clicked(move |_| {
                    if active(&r) {
                        manage_group(kind);
                    }
                });
                self.groups.append(&more);
            }
        }
        if let Some(ui) = current() {
            ui.sidebar_rendered();
        }
    }
}
/// A group's label: click folds it, `+` makes one, right-click manages them all.
fn heading(root: &Path, kind: Kind, title: &str, add: &str, count: usize, fold: bool, first: bool) -> gtk::Box {
    let row = widgets::hbox(0);
    row.add_css_class("nav-group-row");
    row.add_css_class("compact-hide");
    row.add_css_class("git-group-head");
    if first {
        row.add_css_class("first");
    }
    let toggle = gtk::Button::new();
    toggle.add_css_class("git-group-toggle");
    toggle.set_hexpand(true);
    let inside = widgets::hbox(6);
    inside.append(&widgets::label(&title.to_uppercase(), "nav-group"));
    if fold {
        inside.append(&widgets::label(&count.to_string(), "git-group-count"));
    }
    let chevron = gtk::Image::from_icon_name(if fold { "git-chev-r-symbolic" } else { "git-chev-d-symbolic" });
    chevron.add_css_class("git-group-chev");
    inside.append(&chevron);
    toggle.set_child(Some(&inside));
    toggle.set_tooltip_text(Some(&format!("{} {title}", if fold { "Show" } else { "Fold" })));
    let r = root.to_path_buf();
    let key = kind.key();
    toggle.connect_clicked(move |_| {
        set_closed(&r, key, !fold);
        if let Some(ui) = current() {
            ui.sidebar.render();
        }
    });
    let r = root.to_path_buf();
    nyx_ui::menu::on_right_click(&toggle, move |w, x, y| {
        let r = r.clone();
        nyx_ui::menu::popup_at(
            w,
            x,
            y,
            vec![nyx_ui::menu::item(&format!("Manage {}…", kind.title().to_lowercase()), move || {
                if active(&r) {
                    manage_group(kind);
                }
            })],
        );
    });
    row.append(&toggle);
    if !add.is_empty() {
        let plus = widgets::icon_button("git-plus-symbolic", add);
        plus.add_css_class("nav-group-add");
        plus.set_valign(gtk::Align::Center);
        let r = root.to_path_buf();
        plus.connect_clicked(move |_| {
            if active(&r) {
                create(&r, kind);
            }
        });
        row.append(&plus);
    }
    // The first group in the app's accent, the others from the theme's [sections.git].
    let section = if first { nyx_ui::sidebar::ACCENT_SECTION.to_string() } else { title.to_lowercase() };
    nyx_ui::sidebar::set_section(&row, &section);
    row
}
/// `+` on a group.
fn create(root: &Path, kind: Kind) {
    match kind {
        Kind::Worktree => super::worktree_manager::create_for(root.to_path_buf(), None, None),
        Kind::Local => super::advanced::target_action("branch", "HEAD".into()),
        Kind::Remote | Kind::RemoteBranch => super::remotes::show(),
        Kind::Stash => super::repository_actions::stashes(),
        Kind::Tag => super::advanced::show(None),
    }
}
fn active(root: &PathBuf) -> bool {
    current().is_some_and(|ui| {
        if ui.root().as_ref() == Some(root) {
            true
        } else {
            ui.toast("The active repository changed. Choose the sidebar item again.");
            false
        }
    })
}
fn activate(root: &PathBuf, action: &Action) {
    if !active(root) {
        return;
    }
    match action {
        Action::Reference { reference, commit, .. } => {
            if let Some(ui) = current() {
                if *commit {
                    ui.show_view("history");
                    ui.graph.show_reference(reference.clone());
                } else {
                    ui.toast("This reference points to a non-commit object. Use Git actions to manage its tag.");
                }
            }
        }
        Action::Stash(hash) => super::repository_actions::show_stash_diff(root.clone(), hash.clone()),
        Action::Worktree(path) => {
            if let Some(ui) = current() {
                ui.open(path.clone());
            }
        }
        Action::Remote(name) => super::remotes::show_selected(Some(name.clone())),
    }
}
fn manage_group(kind: Kind) {
    match kind {
        Kind::Local => super::repository_actions::branches(),
        Kind::RemoteBranch | Kind::Remote => super::remotes::show(),
        Kind::Tag => super::advanced::show(None),
        Kind::Stash => super::repository_actions::stashes(),
        Kind::Worktree => super::worktree_manager::show(),
    }
}
/// Right-click, the Menu key or Shift+F10 on `button` opens `open` at that point.
pub fn context_menu(button: &gtk::Button, open: impl Fn(&gtk::Button, f64, f64) + 'static) {
    let open = std::rc::Rc::new(open);
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    let (o, b) = (open.clone(), button.downgrade());
    gesture.connect_pressed(move |_, _, x, y| {
        if let Some(b) = b.upgrade() {
            o(&b, x, y);
        }
    });
    button.add_controller(gesture);
    let keys = gtk::EventControllerKey::new();
    let b = button.downgrade();
    keys.connect_key_pressed(move |_, key, _, mods| {
        if key == gdk::Key::Menu || (key == gdk::Key::F10 && mods.contains(gdk::ModifierType::SHIFT_MASK)) {
            if let Some(b) = b.upgrade() {
                open(&b, b.width() as f64 / 2.0, b.height() as f64);
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    button.add_controller(keys);
}
fn menu(button: &gtk::Button, root: &PathBuf, action: &Action, x: f64, y: f64) {
    if !active(root) {
        return;
    }
    let (r, a) = (root.clone(), action.clone());
    let mut items = vec![item(
        match action {
            Action::Reference { .. } => "Show its history",
            Action::Stash(_) => "Inspect stash",
            Action::Worktree(_) => "Open worktree",
            Action::Remote(_) => "Manage remote…",
        },
        move || activate(&r, &a),
    )];
    if let Action::Reference { reference, current: is_current, commit } = action {
        if let Some(branch) = reference.strip_prefix("refs/heads/")
            && !is_current
        {
            let (r, name) = (root.clone(), branch.to_string());
            items.push(item("Switch to this branch", move || {
                if let Some(ui) = current() {
                    ui.execute_at(r.clone(), nyx_git::core::operations::Operation::SwitchBranch(name.clone()));
                }
            }));
            if *commit {
                let name = reference.clone();
                items.push(item("Merge into current branch…", move || super::advanced::target_action("merge", name.clone())));
            }
        }
        if *commit {
            let name = reference.clone();
            items.push(item("Create branch here…", move || super::advanced::target_action("branch", name.clone())));
        }
        let (r, name) = (root.clone(), reference.clone());
        items.push(item("More Git actions…", move || {
            if active(&r) {
                super::advanced::show(Some(name.clone()));
            }
        }));
        items.push(Item::Separator);
        let name = reference.clone();
        let short = reference.trim_start_matches("refs/").trim_start_matches("heads/").trim_start_matches("tags/").to_string();
        let b = button.clone();
        items.push(item("Copy name", move || b.display().clipboard().set_text(&short)));
        let b = button.clone();
        items.push(item("Copy full reference", move || b.display().clipboard().set_text(&name)));
    }
    if let Action::Worktree(path) = action {
        let p = path.clone();
        items.push(item("Open folder", move || super::workspace::external(p.clone(), "folder")));
        let p = path.clone();
        items.push(item("Open in terminal", move || super::workspace::external(p.clone(), "terminal")));
        items.push(Item::Separator);
        items.push(item("Manage worktrees…", super::worktree_manager::show));
    }
    super::popup_at(button, items, x, y);
}
