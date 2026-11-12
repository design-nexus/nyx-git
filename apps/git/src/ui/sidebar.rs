use super::{Item, item, window::current};
use crate::prefs;
use gtk::{gdk, glib, prelude::*};
use nyx_git::core::navigation::{self, Action, Inventory, Kind};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};
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
impl Sidebar {
    pub fn new() -> Self {
        let root = widgets::vbox(4);
        root.add_css_class("compact-hide");
        root.set_hexpand(false);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Filter loaded items…"));
        search.set_width_chars(12);
        search.set_max_width_chars(22);
        search.update_property(&[gtk::accessible::Property::Label(
            "Filter sidebar branches, tags, stashes, worktrees and remotes",
        )]);
        search.connect_search_changed(|_| {
            if let Some(ui) = current() {
                ui.sidebar.render();
            }
        });
        search.connect_stop_search(|entry| entry.set_text(""));
        root.append(&search);
        let status = widgets::label("", "dim-label");
        status.set_wrap(true);
        status.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        root.append(&status);
        let groups = widgets::vbox(2);
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
        self.status.set_text("Loading repository items…");
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
                        sidebar.status.set_text("");
                        if sidebar.data.borrow().as_ref() != Some(&data) {
                            *sidebar.data.borrow_mut() = Some(data);
                            sidebar.render();
                        }
                    }
                    Ok(_) => {}
                    Err(e) => sidebar.status.set_text(&format!("{e:#}\nF5 to retry")),
                }
                if sidebar.dirty.get()
                    && let Some(snapshot) = ui.snapshot.borrow().as_ref()
                {
                    sidebar.refresh(snapshot.repository.root.clone(), snapshot.repository.head.clone());
                }
            },
        );
    }
    fn render(&self) {
        while let Some(child) = self.groups.first_child() {
            self.groups.remove(&child);
        }
        let Some(root) = self.repository.borrow().clone() else { return };
        let query = self.search.text().to_lowercase();
        let data = self.data.borrow();
        let Some(data) = data.as_ref() else { return };
        let closed = prefs::get()
            .workspace
            .repositories
            .iter()
            .find(|r| r.root == root)
            .map(|r| r.sidebar_closed.clone())
            .unwrap_or_else(|| nyx_git::core::workspace::RepositoryState::default().sidebar_closed);
        for group in &data.groups {
            let matches = group
                .items
                .iter()
                .filter(|item| query.is_empty() || format!("{} {}", item.name, item.detail).to_lowercase().contains(&query))
                .collect::<Vec<_>>();
            let expander = gtk::Expander::new(Some(&format!("{} ({})", group.kind.title(), matches.len())));
            expander.add_css_class("git-sidebar-section");
            expander.set_expanded(!query.is_empty() || !closed.iter().any(|key| key == group.kind.key()));
            let (r, key) = (root.clone(), group.kind.key());
            expander.connect_expanded_notify(move |e| {
                prefs::update(|p| {
                    let list = &mut p.workspace.entry(&r).sidebar_closed;
                    list.retain(|k| k != key);
                    if !e.is_expanded() {
                        list.push(key.into());
                    }
                });
            });
            let rows = widgets::vbox(0);
            for item in matches.iter().take(100) {
                let button = gtk::Button::new();
                button.add_css_class("nav-item");
                let title = format!(
                    "{}{}",
                    if matches!(item.action, Action::Reference { current: true, .. }) { "● " } else { "" },
                    item.name
                );
                let label = widgets::label(&title, "nav-label");
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_max_width_chars(22);
                button.set_child(Some(&label));
                button.set_tooltip_text(Some(&format!("{}\n{}\nRight-click or Shift+F10 for actions", item.name, item.detail)));
                if matches!(item.action, Action::Reference { current: true, .. }) {
                    button.add_css_class("active");
                }
                let (r, action) = (root.clone(), item.action.clone());
                button.connect_clicked(move |_| activate(&r, &action));
                let (r, action) = (root.clone(), item.action.clone());
                context_menu(&button, move |b, x, y| menu(b, &r, &action, x, y));
                rows.append(&button);
            }
            if matches.is_empty() {
                rows.append(&widgets::label("No matching items", "dim-label"));
            }
            if matches.len() > 100 {
                let label = widgets::label("First 100 matches · filter to narrow", "dim-label");
                label.set_wrap(true);
                rows.append(&label);
            }
            if group.limited {
                let label = widgets::label("Some entries omitted · use Git for the full inventory", "dim-label");
                label.set_wrap(true);
                rows.append(&label);
            }
            let manage = gtk::Button::with_label("Manage…");
            let kind = group.kind;
            let r = root.clone();
            manage.connect_clicked(move |_| {
                if active(&r) {
                    manage_group(kind);
                }
            });
            rows.append(&manage);
            expander.set_child(Some(&rows));
            self.groups.append(&expander);
        }
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
    super::popup_at(button, items, x, y);
}
