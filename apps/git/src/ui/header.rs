//! The header's two ends. After the app's name: the repository and branch switchers. Before
//! the window controls: Sync, Go to, the repository's tools and Settings. The destinations sit
//! centred between them, and neither end grows past what the centre leaves it.
use super::window::current;
use gtk::{gdk, glib, pango, prelude::*};
use nyx_git::core::{
    navigation::{Action, Kind},
    operations::Operation,
};
use nyx_ui::{menu, widgets};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

/// What Sync's main part does now: it follows the branch's state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyncAction {
    Fetch,
    Pull,
    Push,
    Publish,
}
impl SyncAction {
    /// Pull when only behind, Push when only ahead, Publish without an upstream, Fetch otherwise.
    pub fn choose(upstream: bool, publishable: bool, ahead: u64, behind: u64) -> Self {
        match (upstream, ahead, behind) {
            (false, _, _) if publishable => Self::Publish,
            (true, 0, b) if b > 0 => Self::Pull,
            (true, a, 0) if a > 0 => Self::Push,
            _ => Self::Fetch,
        }
    }
}

pub struct Header {
    pub repo: gtk::Button,
    repo_label: gtk::Label,
    pub branch: gtk::Button,
    branch_label: gtk::Label,
    branch_state: gtk::Label,
    pub sync_box: gtk::Box,
    sync: gtk::Button,
    sync_icon: gtk::Image,
    publish: gtk::Label,
    counts: gtk::Box,
    behind: gtk::Label,
    ahead: gtk::Label,
    sync_menu: gtk::Button,
    pub search: gtk::Button,
    pub more: gtk::Button,
    action: Cell<SyncAction>,
    /// The repository's path and its state in words, for the switcher's tooltip.
    about: RefCell<(String, String)>,
}

fn switch_button(icon: &str, tip: &str) -> (gtk::Button, gtk::Label, gtk::Box) {
    let b = gtk::Button::new();
    b.add_css_class("git-switch");
    let row = widgets::hbox(6);
    row.append(&gtk::Image::from_icon_name(icon));
    let label = widgets::label("", "git-switch-label");
    label.set_ellipsize(pango::EllipsizeMode::Middle);
    row.append(&label);
    let chev = gtk::Image::from_icon_name("git-chev-d-symbolic");
    chev.add_css_class("git-switch-chev");
    row.append(&chev);
    b.set_child(Some(&row));
    b.set_tooltip_text(Some(tip));
    b.set_valign(gtk::Align::Center);
    (b, label, row)
}

impl Header {
    pub fn new() -> Self {
        let (repo, repo_label, _) = switch_button("git-folder-symbolic", "Repositories (Ctrl+K)");
        repo_label.set_max_width_chars(16);
        repo_label.set_text("Open a repository");
        repo.connect_clicked(repositories);
        let (branch, branch_label, row) = switch_button("git-branch-symbolic", "Branches (Ctrl+Shift+B)");
        branch.add_css_class("branch");
        branch_label.set_max_width_chars(14);
        // "· rebasing", after the name in the warning tint.
        let branch_state = widgets::label("", "git-switch-state");
        branch_state.set_visible(false);
        row.insert_child_after(&branch_state, Some(&branch_label));
        branch.connect_clicked(branches);
        branch.set_visible(false);

        let sync_box = widgets::hbox(0);
        sync_box.add_css_class("git-sync");
        sync_box.set_valign(gtk::Align::Center);
        let sync = gtk::Button::new();
        sync.add_css_class("git-sync-main");
        let content = widgets::hbox(6);
        let sync_icon = gtk::Image::from_icon_name("git-sync-symbolic");
        content.append(&sync_icon);
        let publish = widgets::label("Publish", "git-sync-publish");
        publish.set_visible(false);
        content.append(&publish);
        let counts = widgets::hbox(4);
        let count = |icon: &str| {
            let image = gtk::Image::from_icon_name(icon);
            image.add_css_class("git-sync-arrow");
            counts.append(&image);
            // A fixed width, so the header doesn't reflow as the counts change.
            let label = widgets::label("0", "git-sync-count");
            label.set_width_chars(2);
            counts.append(&label);
            label
        };
        let behind = count("git-down-symbolic");
        let ahead = count("git-up-symbolic");
        content.append(&counts);
        sync.set_child(Some(&content));
        sync.connect_clicked(|_| run_sync());
        sync_box.append(&sync);
        let sync_menu = gtk::Button::from_icon_name("git-chev-d-symbolic");
        sync_menu.add_css_class("git-sync-more");
        sync_menu.set_tooltip_text(Some("Fetch, pull, push and publish"));
        sync_menu.connect_clicked(|b| menu::popup_below(b, sync_entries()));
        sync_box.append(&sync_menu);

        let search = widgets::bar_button("git-search-symbolic", "Go to a page, command, branch or file (Ctrl+Shift+P)");
        search.connect_clicked(|_| super::palette::open());
        let more = widgets::bar_button("git-more-symbolic", "Repository tools (F10)");
        more.connect_clicked(|b| menu::popup_below(b, tools()));
        Self {
            repo,
            repo_label,
            branch,
            branch_label,
            branch_state,
            sync_box,
            sync,
            sync_icon,
            publish,
            counts,
            behind,
            ahead,
            sync_menu,
            search,
            more,
            action: Cell::new(SyncAction::Fetch),
            about: RefCell::default(),
        }
    }
    /// The repository's name, or "Open a repository" with none open.
    pub fn show_repository(&self, name: Option<&str>, tip: &str) {
        self.repo_label.set_text(name.unwrap_or("Open a repository"));
        self.about.borrow_mut().0 = tip.into();
        self.describe();
    }
    fn describe(&self) {
        let (path, status) = self.about.borrow().clone();
        let lines: Vec<&str> = [path.as_str(), status.as_str(), "Repositories (Ctrl+K)"].into_iter().filter(|l| !l.is_empty()).collect();
        self.repo.set_tooltip_text(Some(&lines.join("\n")));
    }
    /// The branch, with an operation in progress after it in the warning tint.
    pub fn show_branch(&self, branch: Option<&str>, state: Option<&str>, tip: &str) {
        self.branch.set_visible(branch.is_some());
        self.branch_label.set_text(branch.unwrap_or_default());
        self.branch_state.set_text(&state.map(|s| format!("· {s}")).unwrap_or_default());
        self.branch_state.set_visible(state.is_some());
        if state.is_some() {
            self.branch.add_css_class("warning");
        } else {
            self.branch.remove_css_class("warning");
        }
        self.branch.set_tooltip_text(Some(&format!("{tip}{}Branches (Ctrl+Shift+B)", if tip.is_empty() { "" } else { "\n" })));
    }
    /// The repository's state in words, under its path in the switcher's tooltip.
    pub fn set_status(&self, text: &str) {
        self.about.borrow_mut().1 = text.into();
        self.describe();
    }
    /// A repository is being read: the switcher dims until it's done.
    pub fn set_loading(&self, loading: bool) {
        if loading {
            self.repo.add_css_class("loading");
        } else {
            self.repo.remove_css_class("loading");
        }
    }
    /// Shorter names in narrower windows, so both ends stay clear of the destinations.
    pub fn fit(&self, width: i32) {
        let (repo, branch) = match width {
            w if w >= 1280 => (14, 12),
            w if w >= 980 => (10, 10),
            _ => (8, 8),
        };
        self.repo_label.set_max_width_chars(repo);
        self.branch_label.set_max_width_chars(branch);
    }
    pub fn sync_action(&self) -> SyncAction {
        self.action.get()
    }
    /// Sync's counts, what its main part does, and whether it can.
    pub fn show_sync(&self, action: SyncAction, ahead: u64, behind: u64, usable: bool, upstream: Option<&str>) {
        self.action.set(action);
        self.sync_box.set_visible(true);
        self.sync.set_sensitive(usable);
        self.sync_menu.set_sensitive(usable);
        let publish = action == SyncAction::Publish;
        self.publish.set_visible(publish);
        self.counts.set_visible(!publish);
        self.sync_icon.set_icon_name(Some(if publish { "git-up-symbolic" } else { "git-sync-symbolic" }));
        self.behind.set_text(&behind.to_string());
        self.ahead.set_text(&ahead.to_string());
        for (label, n) in [(&self.behind, behind), (&self.ahead, ahead)] {
            if n > 0 {
                label.add_css_class("some");
            } else {
                label.remove_css_class("some");
            }
        }
        let commits = |n: u64| format!("{n} commit{}", if n == 1 { "" } else { "s" });
        let up = upstream.unwrap_or("the upstream");
        let tip = match action {
            SyncAction::Publish => "Publish: push this branch to a remote and track it (Ctrl+Alt+P)".to_string(),
            SyncAction::Pull => format!("Pull {} from {up} (Ctrl+Shift+L)", commits(behind)),
            SyncAction::Push => format!("Push {} to {up} (Ctrl+Alt+P)", commits(ahead)),
            SyncAction::Fetch if ahead > 0 && behind > 0 => format!(
                "Fetch from the remotes. {up} has {} this branch doesn't, and this branch has {} it doesn't, so Pull won't fast-forward: merge or rebase from the ▾ menu or Flow.",
                commits(behind),
                commits(ahead)
            ),
            SyncAction::Fetch if upstream.is_some() => format!("Fetch from the remotes · up to date with {up} (Ctrl+Shift+F)"),
            SyncAction::Fetch => "Fetch from the remotes (Ctrl+Shift+F)".to_string(),
        };
        self.sync.set_tooltip_text(Some(&tip));
    }
    pub fn hide_sync(&self) {
        self.sync_box.set_visible(false);
    }
    pub fn open_repositories(&self) {
        repositories(&self.repo);
    }
    pub fn open_branches(&self) {
        if self.branch.is_visible() {
            branches(&self.branch);
        }
    }
    pub fn open_tools(&self) {
        menu::popup_below(&self.more, tools());
    }
}

fn run_sync() {
    let Some(ui) = current() else { return };
    match ui.header.sync_action() {
        SyncAction::Fetch => ui.execute(Operation::Fetch),
        SyncAction::Pull => ui.execute(Operation::Pull),
        SyncAction::Push | SyncAction::Publish => super::window::push(),
    }
}

/// Sync's ▾: every way to sync, with what the branch tracks above them.
fn sync_entries() -> Vec<menu::Entry> {
    let Some(ui) = current() else { return Vec::new() };
    let snapshot = ui.snapshot.borrow();
    let Some(s) = snapshot.as_ref() else { return Vec::new() };
    let (ahead, behind) = super::window::divergence(s.status.divergence.as_deref());
    let upstream = s.status.upstream.clone();
    let commits = |n: u64| format!("{n} commit{}", if n == 1 { "" } else { "s" });
    let mut out = vec![menu::heading(&match &upstream {
        Some(up) => format!("{} tracks {up}", s.repository.branch),
        None => format!("{} isn't published yet", s.repository.branch),
    })];
    match &upstream {
        Some(_) => {
            out.push(
                menu::item(&if ahead > 0 { format!("Push {}", commits(ahead)) } else { "Push".into() }, super::window::push)
                    .icon("git-up-symbolic")
                    .key("Ctrl+Alt+P"),
            );
            out.push(
                menu::item(&if behind > 0 { format!("Pull {}", commits(behind)) } else { "Pull".into() }, || {
                    if let Some(ui) = current() {
                        ui.execute(Operation::Pull);
                    }
                })
                .icon("git-down-symbolic")
                .key("Ctrl+Shift+L"),
            );
        }
        None => out.push(menu::item("Publish branch…", super::window::push).icon("git-up-symbolic").key("Ctrl+Alt+P")),
    }
    out.push(
        menu::item("Fetch", || {
            if let Some(ui) = current() {
                ui.execute(Operation::Fetch);
            }
        })
        .icon("git-sync-symbolic")
        .key("Ctrl+Shift+F"),
    );
    out.push(menu::separator());
    out.push(
        menu::item("Fetch all open repositories", || {
            if let Some(ui) = current() {
                for root in crate::prefs::get().workspace.open.into_iter().take(32) {
                    if ui.root().as_ref() == Some(&root) {
                        ui.execute(Operation::Fetch);
                    } else {
                        ui.background_fetch(root);
                    }
                }
            }
        })
        .icon("git-sync-symbolic"),
    );
    out.push(menu::item("Remotes…", super::remotes::show).icon("git-remote-symbolic"));
    out
}

/// ⋯ (F10): the repository's tools, the ways to open it elsewhere, and the window.
fn tools() -> Vec<menu::Entry> {
    let open = current().and_then(|ui| ui.root()).is_some();
    let external = |kind: &'static str| {
        move || {
            if let Some(root) = current().and_then(|ui| ui.root()) {
                super::workspace::external(root, kind);
            }
        }
    };
    let collapsed = crate::prefs::get().sidebar_collapsed;
    vec![
        menu::item("Go to…", super::palette::open).icon("git-search-symbolic").key("Ctrl+Shift+P"),
        menu::item("Find a file…", || super::palette::show(super::palette::Mode::Files))
            .icon("git-file-symbolic")
            .key("Ctrl+P")
            .enabled(open),
        menu::separator(),
        menu::item("Branches…", super::repository_actions::branches).icon("git-branch-symbolic").enabled(open),
        menu::item("Worktrees…", super::worktree_manager::show).icon("git-worktree-symbolic").enabled(open),
        menu::item("Stashes…", super::repository_actions::stashes).icon("git-stash-symbolic").enabled(open),
        menu::item("Remotes…", super::remotes::show).icon("git-remote-symbolic").enabled(open),
        menu::item("Submodules…", super::submodules::show).icon("git-layers-symbolic").enabled(open),
        menu::item("Git LFS…", super::lfs::show).icon("git-server-symbolic").enabled(open),
        menu::separator(),
        menu::item("Git actions…", || super::advanced::show(None)).icon("git-merge-symbolic").enabled(open),
        menu::item("Recovery points…", super::advanced::recovery).icon("git-undo-symbolic").enabled(open),
        menu::item("Operation history…", super::tasks::show).icon("git-list-symbolic"),
        menu::separator(),
        menu::item("Open folder", external("folder")).icon("git-folder-symbolic").enabled(open),
        menu::item("Open in editor", external("editor")).icon("git-pen-symbolic").enabled(open),
        menu::item("Open in terminal", external("terminal")).icon("git-terminal-symbolic").enabled(open),
        menu::separator(),
        menu::item(if collapsed { "Show the sidebar" } else { "Collapse the sidebar" }, super::window::toggle_sidebar)
            .icon("git-sidebar-symbolic")
            .key("Ctrl+B"),
        menu::item("Refresh", || {
            if let Some(ui) = current() {
                ui.refresh();
            }
        })
        .icon("git-refresh-symbolic")
        .key("F5")
        .enabled(open),
        menu::item("Keyboard shortcuts", super::window::show_shortcuts).icon("git-keyboard-symbolic").key("F1"),
    ]
}

/// One row of a switcher.
struct Choice {
    icon: &'static str,
    title: String,
    detail: String,
    end: String,
    current: bool,
    run: Rc<dyn Fn()>,
}
fn choice(icon: &'static str, title: impl Into<String>, detail: impl Into<String>, run: impl Fn() + 'static) -> Choice {
    Choice { icon, title: title.into(), detail: detail.into(), end: String::new(), current: false, run: Rc::new(run) }
}

/// A switcher's popover under `anchor`: a filter, then groups of choices, then `foot`.
/// Typing filters; ↑ ↓ move, Enter chooses, Esc closes.
fn chooser(anchor: &gtk::Button, placeholder: &str, sections: Vec<(String, Vec<Choice>)>, foot: Option<gtk::Widget>) {
    let popover = gtk::Popover::new();
    popover.add_css_class("git-chooser");
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_halign(gtk::Align::Start);
    popover.set_parent(anchor);
    let body = widgets::vbox(6);
    body.set_size_request(380, -1);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(placeholder));
    body.append(&search);
    let list = gtk::ListBox::new();
    list.add_css_class("git-chooser-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    let mut runs: Vec<Option<Rc<dyn Fn()>>> = Vec::new();
    let mut texts: Vec<Option<String>> = Vec::new();
    for (title, choices) in sections {
        if choices.is_empty() {
            continue;
        }
        let heading = gtk::ListBoxRow::new();
        heading.set_selectable(false);
        heading.set_activatable(false);
        heading.set_focusable(false);
        heading.set_child(Some(&widgets::label(&title.to_uppercase(), "git-chooser-heading")));
        list.append(&heading);
        runs.push(None);
        texts.push(None);
        for c in choices {
            let row = widgets::hbox(10);
            row.add_css_class("git-chooser-row");
            row.append(&gtk::Image::from_icon_name(c.icon));
            let title = widgets::label(&c.title, "git-chooser-title");
            title.set_ellipsize(pango::EllipsizeMode::Middle);
            title.set_max_width_chars(24);
            row.append(&title);
            let detail = widgets::label(&c.detail, "git-chooser-detail");
            detail.set_ellipsize(pango::EllipsizeMode::Middle);
            detail.set_hexpand(true);
            row.append(&detail);
            if !c.end.is_empty() {
                row.append(&widgets::label(&c.end, "git-chooser-end"));
            }
            if c.current {
                let check = gtk::Image::from_icon_name("git-check-symbolic");
                check.add_css_class("git-chooser-check");
                row.append(&check);
            }
            let r = gtk::ListBoxRow::new();
            r.set_child(Some(&row));
            r.set_tooltip_text(Some(&format!("{}\n{}", c.title, c.detail)));
            list.append(&r);
            texts.push(Some(format!("{} {}", c.title, c.detail).to_lowercase()));
            runs.push(Some(c.run));
        }
    }
    let texts = Rc::new(texts);
    let runs = Rc::new(runs);
    {
        let (texts, search) = (texts.clone(), search.clone());
        list.set_filter_func(move |row| {
            let query = search.text().to_lowercase();
            match texts.get(row.index() as usize) {
                Some(Some(text)) => query.split_whitespace().all(|w| text.contains(w)),
                // Group headings only while nothing is typed.
                _ => query.trim().is_empty(),
            }
        });
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .propagate_natural_height(true)
        .max_content_height(420)
        .child(&list)
        .build();
    body.append(&scroll);
    let none = widgets::label("Nothing matches.", "git-chooser-detail");
    none.set_margin_start(10);
    none.set_visible(false);
    body.append(&none);
    if let Some(foot) = foot {
        body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        body.append(&foot);
    }
    popover.set_child(Some(&body));
    // The first visible row that can be chosen, from `from` on, stepping by `step`.
    let visible = {
        let (list, texts, search) = (list.clone(), texts.clone(), search.clone());
        move |from: i32, step: i32| -> Option<gtk::ListBoxRow> {
            let query = search.text().to_lowercase();
            let mut i = from;
            while i >= 0 && (i as usize) < texts.len() {
                if let Some(Some(text)) = texts.get(i as usize)
                    && query.split_whitespace().all(|w| text.contains(w))
                {
                    return list.row_at_index(i);
                }
                i += step;
            }
            None
        }
    };
    let select_first = {
        let (list, visible, none) = (list.clone(), visible.clone(), none.clone());
        move || {
            let first = visible(0, 1);
            none.set_visible(first.is_none());
            list.select_row(first.as_ref());
        }
    };
    select_first();
    {
        let (list, select_first) = (list.clone(), select_first.clone());
        search.connect_search_changed(move |_| {
            list.invalidate_filter();
            select_first();
        });
    }
    let choose = {
        let (runs, popover) = (runs.clone(), popover.clone());
        move |row: &gtk::ListBoxRow| {
            if let Some(Some(run)) = runs.get(row.index() as usize) {
                let run = run.clone();
                popover.popdown();
                run();
            }
        }
    };
    {
        let choose = choose.clone();
        list.connect_row_activated(move |_, row| choose(row));
    }
    {
        let (list, choose) = (list.clone(), choose.clone());
        search.connect_activate(move |_| {
            if let Some(row) = list.selected_row() {
                choose(&row);
            }
        });
    }
    let keys = gtk::EventControllerKey::new();
    {
        let list = list.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            let step = match key {
                gdk::Key::Down => 1,
                gdk::Key::Up => -1,
                _ => return glib::Propagation::Proceed,
            };
            let at = list.selected_row().map_or(if step > 0 { -1 } else { 0 }, |r| r.index());
            if let Some(row) = visible(at + step, step) {
                list.select_row(Some(&row));
                // Focusing the row scrolls it into view; typing carries on in the filter.
                row.grab_focus();
                search.grab_focus();
                search.set_position(-1);
            }
            glib::Propagation::Stop
        });
    }
    popover.add_controller(keys);
    let anchor = anchor.downgrade();
    popover.connect_closed(move |p| {
        let (p, a) = (p.clone(), anchor.clone());
        glib::idle_add_local_once(move || {
            if a.upgrade().is_some() && p.parent().is_some() {
                p.unparent();
            }
        });
    });
    popover.popup();
    if let Some(entry) = body.first_child() {
        entry.grab_focus();
    }
}

fn foot_button(icon: &str, text: &str, run: impl Fn() + 'static) -> gtk::Button {
    let b = widgets::labeled_button(icon, text);
    b.add_css_class("git-chooser-foot");
    b.connect_clicked(move |b| {
        if let Some(p) = b.ancestor(gtk::Popover::static_type()).and_downcast::<gtk::Popover>() {
            p.popdown();
        }
        run();
    });
    b
}

/// Ctrl+K: open tabs, favorites, groups and recent repositories, and ways to add one.
fn repositories(anchor: &gtk::Button) {
    let p = crate::prefs::get();
    let active = current().and_then(|ui| ui.root());
    let mut shown = std::collections::HashSet::<PathBuf>::new();
    let mut make = |path: &PathBuf, icon: &'static str| -> Option<Choice> {
        if !shown.insert(path.clone()) {
            return None;
        }
        let open = path.clone();
        let mut c = choice(
            icon,
            path.file_name().unwrap_or_default().to_string_lossy(),
            nyx_core::paths::pretty(path),
            move || {
                if let Some(ui) = current() {
                    ui.open(open.clone());
                }
            },
        );
        c.current = active.as_ref() == Some(path);
        Some(c)
    };
    let mut sections = vec![("Open".to_string(), p.workspace.open.iter().filter_map(|r| make(r, "git-folder-symbolic")).collect())];
    sections.push((
        "Favorites".into(),
        p.workspace.repositories.iter().filter(|r| r.favorite).filter_map(|r| make(&r.root, "git-star-symbolic")).collect(),
    ));
    let mut groups = std::collections::BTreeMap::<String, Vec<PathBuf>>::new();
    for r in p.workspace.repositories.iter().filter(|r| !r.group.is_empty()) {
        groups.entry(r.group.clone()).or_default().push(r.root.clone());
    }
    for (group, roots) in groups {
        sections.push((group, roots.iter().filter_map(|r| make(r, "git-folder-symbolic")).collect()));
    }
    sections.push(("Recent".into(), p.recent.iter().filter_map(|r| make(r, "git-clock-symbolic")).collect()));
    let foot = widgets::hbox(6);
    foot.append(&foot_button("git-folder-symbolic", "Open…", || super::window::choose_folder(false)));
    foot.append(&foot_button("git-download-symbolic", "Clone…", super::providers::clone_url));
    foot.append(&foot_button("git-plus-symbolic", "New…", super::initialize::show));
    let spacer = widgets::hbox(0);
    spacer.set_hexpand(true);
    foot.append(&spacer);
    let manage = foot_button("git-star-symbolic", "Manage…", super::workspace::catalog);
    manage.set_tooltip_text(Some("Favorites, groups and the recent list"));
    foot.append(&manage);
    chooser(anchor, "Find a repository", sections, Some(foot.upcast()));
}

/// Ctrl+Shift+B: local branches to switch to (one checked out in another worktree opens
/// there), remote branches to look at, and a new branch from here.
fn branches(anchor: &gtk::Button) {
    let Some(ui) = current() else { return };
    let Some(root) = ui.root() else { return };
    let here = ui.snapshot.borrow().as_ref().map(|s| s.repository.branch.clone()).unwrap_or_default();
    let mut local = Vec::new();
    let mut remote = Vec::new();
    let mut worktrees = std::collections::HashMap::<String, PathBuf>::new();
    let inventory = ui.sidebar.inventory();
    if let Some(data) = &inventory {
        for group in data.groups.iter().filter(|g| g.kind == Kind::Worktree) {
            for item in &group.items {
                if let Action::Worktree(path) = &item.action
                    && path != &root
                {
                    worktrees.insert(item.name.clone(), path.clone());
                }
            }
        }
        for group in &data.groups {
            for item in &group.items {
                let Action::Reference { reference, current: is_current, commit } = &item.action else { continue };
                if !commit {
                    continue;
                }
                match group.kind {
                    Kind::Local => {
                        let mut c = if *is_current {
                            choice("git-branch-symbolic", &item.name, "", || {})
                        } else if let Some(path) = worktrees.get(&item.name) {
                            let open = path.clone();
                            let mut c = choice("git-worktree-symbolic", &item.name, "", move || {
                                if let Some(ui) = current() {
                                    ui.open(open.clone());
                                }
                            });
                            c.end = format!("in {}", path.file_name().unwrap_or_default().to_string_lossy());
                            c
                        } else {
                            let (r, name) = (root.clone(), item.name.clone());
                            choice("git-branch-symbolic", &item.name, "", move || {
                                if let Some(ui) = current() {
                                    ui.execute_at(r.clone(), Operation::SwitchBranch(name.clone()));
                                }
                            })
                        };
                        c.current = *is_current;
                        if c.end.is_empty() {
                            c.end = match item.track {
                                Some((0, 0)) | None => String::new(),
                                Some((a, 0)) => format!("↑{a}"),
                                Some((0, b)) => format!("↓{b}"),
                                Some((a, b)) => format!("↓{b} ↑{a}"),
                            };
                        }
                        local.push(c);
                    }
                    Kind::RemoteBranch => {
                        let reference = reference.clone();
                        let mut c = choice("git-remote-symbolic", &item.name, "", move || {
                            if let Some(ui) = current() {
                                ui.show_view("history");
                                ui.graph.show_reference(reference.clone());
                            }
                        });
                        c.end = "history".into();
                        remote.push(c);
                    }
                    _ => {}
                }
            }
        }
    }
    let foot = widgets::vbox(4);
    let row = widgets::hbox(6);
    let from = if here.is_empty() || here == "Detached HEAD" { "here".to_string() } else { here };
    row.append(&foot_button("git-plus-symbolic", &format!("New branch from {from}…"), || {
        super::advanced::target_action("branch", "HEAD".into())
    }));
    let spacer = widgets::hbox(0);
    spacer.set_hexpand(true);
    row.append(&spacer);
    row.append(&foot_button("git-list-symbolic", "All branches…", super::repository_actions::branches));
    foot.append(&row);
    let note = widgets::label(
        if inventory.is_none() {
            "Reading branches…"
        } else {
            "A branch checked out in another worktree opens there. A remote branch shows its history."
        },
        "git-chooser-note",
    );
    note.set_wrap(true);
    note.set_max_width_chars(52);
    foot.append(&note);
    chooser(anchor, "Switch to a branch", vec![("Local".into(), local), ("Remote".into(), remote)], Some(foot.upcast()));
}

#[cfg(test)]
mod tests {
    use super::SyncAction;
    #[test]
    fn sync_follows_the_branch() {
        assert_eq!(SyncAction::choose(true, true, 0, 3), SyncAction::Pull);
        assert_eq!(SyncAction::choose(true, true, 2, 0), SyncAction::Push);
        assert_eq!(SyncAction::choose(true, true, 2, 3), SyncAction::Fetch);
        assert_eq!(SyncAction::choose(true, true, 0, 0), SyncAction::Fetch);
        assert_eq!(SyncAction::choose(false, true, 0, 0), SyncAction::Publish);
        assert_eq!(SyncAction::choose(false, false, 0, 0), SyncAction::Fetch);
    }
}
