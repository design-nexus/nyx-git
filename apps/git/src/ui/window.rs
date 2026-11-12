use crate::{prefs, theme};
use gtk::{gdk, gio, glib, prelude::*};
use nyx_git::core::repository::Snapshot;
use nyx_ui::{chrome, widgets};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

thread_local! { static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) }; }
pub fn current() -> Option<Rc<Ui>> {
    UI.with(|u| u.borrow().clone())
}
pub struct Ui {
    pub window: gtk::ApplicationWindow,
    pub overlay: gtk::Overlay,
    pub snapshot: RefCell<Option<Snapshot>>,
    /// A repository read is under way; `quiet` when it's a background refresh.
    pub busy: Cell<bool>,
    pub quiet: Cell<bool>,
    /// Counts reads and Git actions, so a read overtaken by an action is dropped.
    pub loads: Cell<u64>,
    /// The repository that last failed to open, for F5 to retry.
    pub failed: RefCell<Option<PathBuf>>,
    pub pending: RefCell<std::collections::VecDeque<PathBuf>>,
    /// What the repository's state is, in words: the repository switcher's tooltip.
    pub readout: gtk::Label,
    pub header: super::header::Header,
    main: gtk::Stack,
    welcome_recent: gtk::Box,
    pub workflow: super::workflow::Workflow,
    pub graph: super::commit_graph::CommitGraph,
    pub tasks: super::tasks::Tasks,
    pub monitor: super::monitor::Monitor,
    pub sidebar: super::sidebar::Sidebar,
    pub(super) tabs: Rc<nyx_ui::tabs::TabRow>,
    tabs_shown: RefCell<Option<(Vec<PathBuf>, Option<PathBuf>)>>,
    pub(super) restoring: Cell<bool>,
    pub(super) view_stack: gtk::Stack,
    detail_stack: gtk::Stack,
    pub(super) detail_scroll: gtk::ScrolledWindow,
    view_switch: super::destinations::Switch,
    pub destinations: super::pages::Pages,
    pub(super) selection: gtk::SingleSelection,
    nav: gtk::Box,
    navigation: gtk::Box,
    pub(super) search: gtk::SearchEntry,
    pub(super) files: gio::ListStore,
    pub(super) pages: gtk::Stack,
    pub(super) empty: gtk::Label,
    filter: gtk::CustomFilter,
    pub(super) split: gtk::Paned,
    compact: Cell<Option<bool>>,
    /// Set while the file list is updated, so a row being replaced doesn't clear the diff.
    holding: Cell<bool>,
}

pub const SHORTCUTS: &[(&[&str], &str)] = &[
    (&["Ctrl", "O"], "Open repository"),
    (&["Ctrl", "Shift", "O"], "Discover repositories"),
    (&["Ctrl", "K"], "Switch repository"),
    (&["Ctrl", "Shift", "B"], "Switch branch"),
    (&["Ctrl", "Alt", "W"], "Open a worktree"),
    (&["Ctrl", "P"], "Find a tracked file (history, blame)"),
    (&["Ctrl", "1 … 5"], "Flow, Changes, History, Review, Activity"),
    (&["F5"], "Refresh status"),
    (&["Alt", "1"], "Commit history"),
    (&["Alt", "2"], "Working changes"),
    (&["Ctrl", "Enter"], "Commit staged changes"),
    (&["Ctrl", "Shift", "F"], "Fetch all remotes"),
    (&["Ctrl", "Shift", "L"], "Fast-forward pull"),
    (&["Ctrl", "Alt", "P"], "Push (or publish the branch)"),
    (&["Ctrl", "F"], "Find commits or filter changed files"),
    (&["↑", "↓", "Page Up", "Page Down"], "Navigate commits"),
    (&["Home", "End"], "Newest / oldest commit"),
    (&["Ctrl", "Click"], "Compare a commit with the selected one"),
    (&["Ctrl", "+", "−"], "Zoom the commit graph"),
    (&["Ctrl", "C"], "Copy selected commit hash"),
    (&["Shift", "F10"], "Actions for the selected commit or item (also Menu key)"),
    (&["Enter"], "Focus selected commit details"),
    (&["F6"], "Next panel (Shift+F6 for previous)"),
    (&["Ctrl", "Tab"], "Next repository tab"),
    (&["Ctrl", "Shift", "Tab"], "Previous repository tab"),
    (&["Ctrl", "W"], "Close repository tab"),
    (&["Ctrl", "B"], "Collapse or expand sidebar"),
    (&["Ctrl", ","], "Settings"),
    (&["Ctrl", "Shift", "P"], "Go to a page, command, branch or file"),
    (&["F10"], "Repository tools"),
    (&["F1"], "Keyboard shortcuts"),
    (&["Esc"], "Close settings, dismiss a message or clear search"),
    (&["Ctrl", "Q"], "Quit"),
];

/// Ways to add a repository, behind the tab row's + button.
pub fn add_menu() -> Vec<nyx_ui::menu::Entry> {
    use nyx_ui::menu;
    vec![
        menu::item("Open repository…", || choose_folder(false)).icon("git-folder-symbolic").key("Ctrl+O"),
        menu::item("Clone repository…", super::providers::clone_url).icon("git-download-symbolic"),
        menu::item("New repository…", super::initialize::show).icon("git-plus-symbolic"),
        menu::item("Discover repositories…", || choose_folder(true)).icon("git-search-symbolic").key("Ctrl+Shift+O"),
        menu::separator(),
        menu::item("Repositories, favorites and groups…", super::workspace::catalog).icon("git-star-symbolic"),
        menu::item("Hosting accounts…", super::providers::show).icon("git-users-symbolic"),
    ]
}

pub fn present(app: &gtk::Application) -> Rc<Ui> {
    if let Some(ui) = current() {
        ui.window.present();
        return ui;
    }
    theme::install();
    // Repositories that were moved or deleted drop out of the recent list.
    prefs::update(|p| p.recent.retain(|r| r.exists()));
    let window = gtk::ApplicationWindow::builder().application(app).title("Git").default_width(1280).default_height(800).build();
    window.add_css_class("git-window");
    window.set_icon_name(Some("io.github.design_nexus.nyx.Git"));
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    nyx_ui::keys::install(
        &window,
        nyx_ui::keys::Common {
            settings: Some(super::settings::open),
            shortcuts: Some(show_shortcuts),
            palette: Some(super::palette::open),
            close: false,
        },
    );
    let nav = gtk::Box::new(gtk::Orientation::Vertical, 0);
    nav.add_css_class("settings-navigation");
    nav.set_hexpand(false);
    let navigation = gtk::Box::new(gtk::Orientation::Vertical, 0);
    navigation.set_hexpand(false);
    let nav_scroll = gtk::ScrolledWindow::builder()
        .hexpand(false)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .child(&navigation)
        .build();
    nav.append(&nav_scroll);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.add_css_class("settings-content");
    let search = gtk::SearchEntry::new();
    // Repository tabs: only while two or more are open.
    let tabs = nyx_ui::tabs::TabRow::new("Open, clone or find a repository");
    tabs.root.add_css_class("git-tabs");
    tabs.root.set_visible(false);
    tabs.connect_select(|i| {
        if let (Some(ui), Some(root)) = (current(), prefs::get().workspace.open.get(i).cloned()) {
            ui.open(root);
        }
    });
    tabs.connect_close(|i| {
        if let (Some(ui), Some(root)) = (current(), prefs::get().workspace.open.get(i).cloned()) {
            ui.close_repository(&root);
        }
    });
    tabs.connect_new(|| {
        if let Some(ui) = current() {
            nyx_ui::menu::popup_below(&ui.tabs.add, add_menu());
        }
    });
    content.append(&tabs.root);
    let destinations = super::pages::Pages::new();
    let ids = destinations.ids();
    let initial = nyx_theme::start_page("history", &prefs::get().last_view);
    let initial = if ids.contains(&initial.as_str()) { initial } else { "history".into() };
    let view_switch = super::destinations::Switch::new(&ids, &initial, |id| {
        if let Some(ui) = current() {
            ui.show_view(id);
        }
    });
    let tasks = super::tasks::Tasks::new();
    content.append(&tasks.bar);

    let workflow = super::workflow::Workflow::new();
    // GTK recycles visible rows rather than constructing a widget for every file.
    let files = gio::ListStore::new::<glib::BoxedAnyObject>();
    let search2 = search.clone();
    let category = workflow.category.clone();
    let filter = gtk::CustomFilter::new(move |object| {
        super::changes::entry_of(object).is_some_and(|e| super::changes::matches(&e, category.selected(), &search2.text()))
    });
    let (list, selection) = super::changes::list(&files, &filter);
    let scroll = gtk::ScrolledWindow::builder()
        .vscrollbar_policy(super::scrollbars::policy())
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    let pages = gtk::Stack::new();
    pages.set_vexpand(true);
    pages.set_hhomogeneous(false);
    pages.add_named(&scroll, Some("files"));
    let empty_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    empty_box.add_css_class("empty-state");
    empty_box.set_halign(gtk::Align::Center);
    empty_box.set_valign(gtk::Align::Center);
    let empty_icon = gtk::Image::from_icon_name("git-check-c-symbolic");
    empty_icon.set_pixel_size(40);
    empty_icon.add_css_class("dim");
    empty_box.append(&empty_icon);
    let empty = widgets::label("Open a repository to view working changes.", "empty-title");
    empty.set_wrap(true);
    empty.set_justify(gtk::Justification::Center);
    empty.set_xalign(0.5);
    empty_box.append(&empty);
    let starts = widgets::hbox(8);
    starts.set_halign(gtk::Align::Center);
    starts.set_margin_top(8);
    starts.add_css_class("git-start");
    let open = gtk::Button::with_label("Open…");
    open.add_css_class("suggested-action");
    open.connect_clicked(|_| choose_folder(false));
    starts.append(&open);
    let clone = gtk::Button::with_label("Clone…");
    clone.connect_clicked(|_| super::providers::clone_url());
    starts.append(&clone);
    let init = gtk::Button::with_label("New…");
    init.connect_clicked(|_| super::initialize::show());
    starts.append(&init);
    empty_box.append(&starts);
    pages.add_named(&empty_box, Some("empty"));
    pages.set_visible_child_name("empty");
    // Changes: the page bar and any operation in progress over the files and composer,
    // beside the diff.
    let bar = super::pagebar::PageBar::new();
    bar.start(&super::pagebar::segments(&workflow.category, &["All", "Unstaged", "Staged"]));
    bar.start(&super::pagebar::search(&search, "Filter changed files", "Ctrl F", 18));
    bar.end(&super::pagebar::action("git-stash-symbolic", "Stash…", "Save, apply and drop stashes", super::repository_actions::stashes));
    let left = widgets::vbox(0);
    left.add_css_class("git-files-col");
    left.append(&pages);
    left.append(&workflow.composer);
    let columns = gtk::Paned::new(gtk::Orientation::Horizontal);
    columns.set_start_child(Some(&left));
    columns.set_end_child(Some(&workflow.detail));
    columns.set_resize_start_child(false);
    columns.set_shrink_start_child(false);
    columns.set_shrink_end_child(false);
    columns.set_vexpand(true);
    super::layout::persist(&columns, "changes.files", 320, false);
    let changes = gtk::Box::new(gtk::Orientation::Vertical, 0);
    changes.append(&bar.root);
    changes.append(&workflow.banner);
    changes.append(&columns);
    let graph = super::commit_graph::CommitGraph::new();
    let view_stack = gtk::Stack::new();
    view_stack.set_hhomogeneous(false);
    view_stack.set_vhomogeneous(false);
    view_stack.set_hexpand(true);
    view_stack.set_vexpand(true);
    view_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    view_stack.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });
    view_stack.add_named(&graph.root, Some("history"));
    view_stack.add_named(&changes, Some("changes"));
    destinations.add_to(&view_stack);
    let detail_stack = gtk::Stack::new();
    detail_stack.set_hhomogeneous(false);
    detail_stack.set_vhomogeneous(false);
    detail_stack.set_size_request(288, -1);
    detail_stack.set_hexpand(true);
    detail_stack.set_vexpand(true);
    detail_stack.add_named(&graph.details.root, Some("history"));
    let detail_scroll = gtk::ScrolledWindow::builder()
        .vscrollbar_policy(super::scrollbars::policy())
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(100)
        .min_content_width(288)
        .hexpand(true)
        .vexpand(true)
        .child(&detail_stack)
        .build();
    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_start_child(Some(&view_stack));
    split.set_end_child(Some(&detail_scroll));
    detail_scroll.set_visible(super::pages::has_details(&initial));
    split.set_resize_start_child(true);
    split.set_resize_end_child(false);
    split.set_shrink_start_child(false);
    split.set_shrink_end_child(false);
    split.set_position(prefs::get().details_position.max(160));
    split.set_vexpand(true);
    content.append(&split);
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.set_vexpand(true);
    body.append(&nav);
    body.append(&content);

    // ----- Header -----
    // The app, the repository and its branch; the destinations in the middle; Sync, Go to,
    // the repository's tools and Settings at the end.
    let header = chrome::app_header(&window, "git-git-symbolic", "Git");
    let parts = super::header::Header::new();
    header.start.set_spacing(4);
    header.start.append(&parts.repo);
    header.start.append(&parts.branch);
    header.center.append(&view_switch.root);
    header.end.set_spacing(4);
    header.end.append(&parts.sync_box);
    header.end.append(&parts.search);
    header.end.append(&parts.more);
    header.end.append(&chrome::settings_button(super::settings::open));
    parts.hide_sync();
    let readout = widgets::label("No repository open", "");
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("window-frame");
    frame.append(&header.root);
    // With no repository open, a welcome page takes the whole window.
    let (welcome, welcome_recent) = welcome();
    let main = gtk::Stack::new();
    main.set_vexpand(true);
    main.add_named(&body, Some("repo"));
    main.add_named(&welcome, Some("welcome"));
    frame.append(&main);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&frame));
    window.set_child(Some(&overlay));
    let ui = Rc::new(Ui {
        window,
        overlay,
        snapshot: RefCell::new(None),
        busy: Cell::new(false),
        quiet: Cell::new(false),
        loads: Cell::new(0),
        failed: RefCell::new(None),
        pending: RefCell::new(Default::default()),
        readout,
        header: parts,
        main,
        welcome_recent,
        workflow,
        graph,
        tasks,
        monitor: Default::default(),
        sidebar: super::sidebar::Sidebar::new(),
        tabs,
        tabs_shown: RefCell::new(None),
        restoring: Cell::new(false),
        view_stack,
        detail_stack,
        detail_scroll,
        view_switch,
        destinations,
        selection: selection.clone(),
        nav,
        navigation,
        search,
        files,
        pages,
        empty,
        filter,
        split,
        compact: Cell::new(None),
        holding: Cell::new(false),
    });
    UI.with(|u| *u.borrow_mut() = Some(ui.clone()));
    ui.show_welcome(true);
    ui.rebuild_navigation();
    ui.rebuild_tabs();
    ui.show_view(&initial);
    ui.update_sync();
    ui.graph.install_poll();
    let weak = Rc::downgrade(&ui);
    glib::timeout_add_local(Duration::from_millis(200), move || {
        let Some(ui) = weak.upgrade() else { return glib::ControlFlow::Break };
        if ui.tasks.active() > 0 || ui.tasks.bar.is_visible() {
            ui.update_tasks();
        }
        ui.monitor.tick(&ui);
        glib::ControlFlow::Continue
    });
    let weak = Rc::downgrade(&ui);
    ui.window.connect_close_request(move |_| {
        if let Some(ui) = weak.upgrade()
            && ui.tasks.active() > 0
        {
            ui.error("Finish active Git operations, or stop cancellable transfers in Operation history, before quitting.");
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });

    let weak = Rc::downgrade(&ui);
    ui.search.connect_search_changed(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.filter.changed(gtk::FilterChange::Different);
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.workflow.category.connect_selected_notify(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.filter.changed(gtk::FilterChange::Different);
        }
    });
    ui.search.connect_stop_search(|search| search.set_text(""));
    let weak = Rc::downgrade(&ui);
    selection.connect_selected_item_notify(move |_| {
        if let Some(ui) = weak.upgrade()
            && !ui.holding.get()
        {
            ui.selection_changed();
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.split.connect_position_notify(move |_split| {
        if let Some(ui) = weak.upgrade() {
            ui.save_repository();
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.workflow.connect_message_changed(move || {
        if let Some(ui) = weak.upgrade() {
            ui.save_repository();
            ui.workflow.update_commit();
        }
    });
    let weak = Rc::downgrade(&ui);
    ui.workflow.amend.connect_toggled(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.save_repository();
            ui.workflow.update_commit();
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(|_, key, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = mods.contains(gdk::ModifierType::ALT_MASK);
        let Some(ui) = current() else { return glib::Propagation::Proceed };
        match key {
            gdk::Key::q | gdk::Key::Q if ctrl => ui.window.close(),
            gdk::Key::w | gdk::Key::W if ctrl && alt => super::palette::show(super::palette::Mode::Worktrees),
            gdk::Key::w | gdk::Key::W if ctrl => match ui.root() {
                Some(root) => ui.close_repository(&root),
                None => ui.window.close(),
            },
            gdk::Key::Tab | gdk::Key::ISO_Left_Tab if ctrl => ui.next_repository(shift || key == gdk::Key::ISO_Left_Tab),
            gdk::Key::k | gdk::Key::K if ctrl => ui.header.open_repositories(),
            gdk::Key::_1 if alt => ui.show_view("history"),
            gdk::Key::_2 if alt => ui.show_view("changes"),
            gdk::Key::_1 | gdk::Key::_2 | gdk::Key::_3 | gdk::Key::_4 | gdk::Key::_5 if ctrl && !shift => {
                let n = key.to_unicode().and_then(|c| c.to_digit(10)).unwrap_or(0) as usize;
                if let Some(d) = n.checked_sub(1).and_then(|i| super::destinations::ALL.get(i)) {
                    ui.show_view(d.id);
                }
            }
            gdk::Key::Return | gdk::Key::KP_Enter if ctrl => super::workflow::commit(),
            gdk::Key::f | gdk::Key::F if ctrl && shift => ui.execute(nyx_git::core::operations::Operation::Fetch),
            gdk::Key::l | gdk::Key::L if ctrl && shift => ui.execute(nyx_git::core::operations::Operation::Pull),
            gdk::Key::p | gdk::Key::P if ctrl && alt => push(),
            gdk::Key::o | gdk::Key::O if ctrl => choose_folder(shift),
            gdk::Key::b | gdk::Key::B if ctrl && shift => ui.header.open_branches(),
            gdk::Key::p | gdk::Key::P if ctrl && !shift => super::palette::show(super::palette::Mode::Files),
            gdk::Key::f | gdk::Key::F if ctrl => show_search(),
            gdk::Key::b | gdk::Key::B if ctrl => toggle_sidebar(),
            gdk::Key::F5 => ui.refresh(),
            gdk::Key::F1 => show_shortcuts(),
            gdk::Key::F6 => ui.focus_panel(shift),
            gdk::Key::F10 if !shift && !ctrl && !alt => ui.header.open_tools(),
            gdk::Key::Escape => {
                if nyx_ui::settings_card::is_open() {
                    nyx_ui::settings_card::escape();
                } else {
                    // Let GTK close a focused popover before clearing the page's search.
                    if gtk::prelude::GtkWindowExt::focus(&ui.window)
                        .is_some_and(|focus| focus.ancestor(gtk::Popover::static_type()).is_some())
                    {
                        return glib::Propagation::Proceed;
                    }
                    if ui.has_toast() {
                        ui.dismiss();
                    } else {
                        ui.search.set_text("");
                        ui.graph.search.set_text("");
                    }
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    ui.window.add_controller(keys);
    let probe = gtk::DrawingArea::new();
    probe.set_can_target(false);
    probe.set_can_focus(false);
    ui.overlay.add_overlay(&probe);
    ui.overlay.set_measure_overlay(&probe, false);
    let weak = Rc::downgrade(&ui);
    probe.connect_resize(move |_, _, _| {
        let weak = weak.clone();
        glib::idle_add_local_once(move || {
            if let Some(ui) = weak.upgrade() {
                ui.resize();
            }
        });
    });
    // A slow fallback, as in Music, in case a resize slips by.
    let weak = Rc::downgrade(&ui);
    glib::timeout_add_local(Duration::from_millis(1500), move || {
        let Some(ui) = weak.upgrade() else { return glib::ControlFlow::Break };
        ui.resize();
        glib::ControlFlow::Continue
    });
    let weak = Rc::downgrade(&ui);
    let mut last_fetch = std::time::Instant::now();
    let mut last_interval = prefs::get().fetch_minutes;
    glib::timeout_add_seconds_local(30, move || {
        let Some(ui) = weak.upgrade() else { return glib::ControlFlow::Break };
        let p = prefs::get();
        if p.fetch_minutes != last_interval {
            last_interval = p.fetch_minutes;
            last_fetch = std::time::Instant::now();
        }
        if p.fetch_minutes > 0 && last_fetch.elapsed().as_secs() >= p.fetch_minutes.clamp(5, 30) * 60 {
            last_fetch = std::time::Instant::now();
            // Wait until current work finishes so periodic fetches cannot pile up.
            if ui.tasks.active() == 0 {
                for root in p.workspace.open.into_iter().take(32) {
                    ui.background_fetch(root);
                }
            }
        }
        glib::ControlFlow::Continue
    });
    // Restyle the parts GTK CSS doesn't reach when the theme changes.
    let weak = Rc::downgrade(&ui);
    nyx_theme::subscribe(&ui.window, move || {
        let Some(ui) = weak.upgrade() else { return };
        ui.workflow.diff.recolor();
        ui.graph.details.diff.recolor();
        ui.graph.redraw();
        ui.view_stack.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });
    });
    ui.resize();
    super::name_icon_buttons(ui.window.upcast_ref());
    ui.window.present();
    super::dev::capture(app, &ui.window);
    if prefs::take_broken() {
        ui.error("Settings could not be read. The previous file is kept as settings.toml.bak.");
    }
    ui
}

/// Push to the upstream, or publish a branch that has none yet.
pub fn push() {
    let Some(ui) = current() else { return };
    let publish = ui
        .snapshot
        .borrow()
        .as_ref()
        .filter(|s| s.status.upstream.is_none() && !s.repository.unborn && s.repository.branch != "Detached HEAD")
        .map(|s| (s.repository.root.clone(), s.repository.branch.clone()));
    match publish {
        Some((root, branch)) => super::repository_actions::push_branch(root, branch),
        None => ui.execute(nyx_git::core::operations::Operation::Push),
    }
}

/// "+2 -1" from `# branch.ab` as ahead and behind counts.
pub fn divergence(text: Option<&str>) -> (u64, u64) {
    let mut counts = text.unwrap_or_default().split_whitespace().map(|n| n.trim_start_matches(['+', '-']).parse().unwrap_or(0));
    (counts.next().unwrap_or(0), counts.next().unwrap_or(0))
}

impl Ui {
    fn focus_panel(&self, backwards: bool) {
        let regions: [&gtk::Widget; 3] = [self.nav.upcast_ref(), self.view_stack.upcast_ref(), self.detail_stack.upcast_ref()];
        let focus = gtk::prelude::GtkWindowExt::focus(&self.window);
        let current =
            focus.as_ref().and_then(|focus| regions.iter().position(|region| focus == *region || focus.is_ancestor(*region)));
        let initial = current.unwrap_or(if backwards { 0 } else { 2 });
        for step in 1..=3 {
            let index = (initial + if backwards { 3 - step } else { step }) % 3;
            let focused = if index == 0 && self.sidebar.focus() {
                true
            } else if index == 1 && self.view_stack.visible_child_name().as_deref() == Some("history") {
                self.graph.focus()
            } else {
                regions[index].child_focus(if backwards {
                    gtk::DirectionType::TabBackward
                } else {
                    gtk::DirectionType::TabForward
                })
            };
            if focused {
                break;
            }
        }
    }
    pub fn show_view(&self, name: &str) {
        let ids = self.view_switch.ids();
        let name = if ids.contains(&name) || super::pages::is_studio(name) { name } else { "history" };
        self.view_stack.set_visible_child_name(name);
        let details = super::pages::has_details(name);
        if details {
            self.detail_stack.set_visible_child_name(name);
        }
        self.detail_scroll.set_visible(details);
        self.view_switch.select(name);
        self.destinations.shown(name);
        // A studio is a task in progress, not a place to come back to on restart.
        if !super::pages::is_studio(name) {
            prefs::update(|p| p.last_view = name.into());
            self.save_repository();
        }
    }
    pub fn apply(&self, snapshot: Snapshot) {
        self.show_welcome(false);
        let previous_root = self.root();
        let same_root = previous_root.as_ref() == Some(&snapshot.repository.root);
        let previous = self.selection.selected_item().and_then(|o| super::changes::entry_of(&o)).filter(|_| same_root);
        self.restore_repository(&snapshot.repository.root);
        let repo = &snapshot.repository;
        self.workflow.operation_status(snapshot.progress, snapshot.status.files.iter().any(|f| f.conflicted));
        self.graph.open(repo.root.clone(), repo.head.clone());
        self.monitor.install(&snapshot);
        let name = repo.root.file_name().unwrap_or_default().to_string_lossy().into_owned();
        chrome::set_title(&self.window, "Git", &name);
        self.header.show_repository(
            Some(&name),
            &format!("{}\nGit directory: {}", nyx_core::paths::pretty(&repo.root), nyx_core::paths::pretty(&repo.git_dir)),
        );
        let detached = repo.branch == "Detached HEAD";
        let short = repo.head.as_deref().map(|h| &h[..h.len().min(7)]).unwrap_or("HEAD");
        let state = snapshot.progress.map(|p| match p {
            nyx_git::core::advanced::Active::Merge => "merging",
            nyx_git::core::advanced::Active::Rebase => "rebasing",
            nyx_git::core::advanced::Active::CherryPick => "cherry-picking",
            nyx_git::core::advanced::Active::Revert => "reverting",
        });
        self.header.show_branch(
            (!repo.bare).then_some(if detached { short } else { &repo.branch }),
            state,
            &if repo.unborn {
                format!("{}: no commits yet", repo.branch)
            } else if detached {
                format!("Detached HEAD at {short}: not on a branch")
            } else {
                format!("{} at {short}", repo.branch)
            },
        );
        self.empty.set_text(if repo.bare { "Bare repository: no working tree." } else { "Working tree clean." });
        self.pages.set_visible_child_name(if snapshot.status.files.is_empty() { "empty" } else { "files" });
        self.show_starts(false);
        let files = super::changes::entries(&snapshot.status.files);
        let count_changed = self.snapshot.borrow().as_ref().map(|s| s.status.files.len()) != Some(snapshot.status.files.len());
        prefs::update(|p| p.workspace.activate(snapshot.repository.root.clone()));
        *self.snapshot.borrow_mut() = Some(snapshot);
        self.rebuild_tabs();
        super::dev::after_apply(self);
        self.update_readout();
        if previous_root != self.root() {
            self.rebuild_navigation();
            if let Some(root) = self.root() {
                self.load_template(root);
            }
        }
        self.update_tasks();
        if let Some(snapshot) = self.snapshot.borrow().as_ref() {
            self.sidebar.refresh(snapshot.repository.root.clone(), snapshot.repository.head.clone());
        }
        if count_changed {
            self.sidebar.changed();
        }
        // Change only the rows that differ, so the selection, the diff and its scroll
        // position stay put while files elsewhere in the repository change.
        let old: Vec<super::changes::Entry> = if same_root {
            (0..self.files.n_items()).filter_map(|i| self.files.item(i)).filter_map(|o| super::changes::entry_of(&o)).collect()
        } else {
            Vec::new()
        };
        let new = &files;
        let mut shown = false;
        if !same_root || old != *new {
            self.holding.set(true);
            let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
            let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
            let objects: Vec<_> = new[prefix..new.len() - suffix].iter().cloned().map(glib::BoxedAnyObject::new).collect();
            let removed = if same_root { old.len() - prefix - suffix } else { self.files.n_items() as usize };
            self.files.splice(prefix as u32, removed as u32, &objects);
            // A changed row was replaced: select the same file again, then show what's selected.
            // The same file in the same group, else in another (it was staged or unstaged).
            if self.selection.selected_item().is_none()
                && let Some(previous) = previous
                && let Some(model) = self.selection.model()
            {
                let rows: Vec<Option<super::changes::Entry>> =
                    (0..model.n_items()).map(|i| model.item(i).and_then(|o| super::changes::entry_of(&o))).collect();
                let at = rows
                    .iter()
                    .position(|e| e.as_ref().is_some_and(|e| e.file.path == previous.file.path && e.group == previous.group))
                    .or_else(|| rows.iter().position(|e| e.as_ref().is_some_and(|e| e.file.path == previous.file.path)));
                if let Some(i) = at {
                    self.selection.set_selected(i as u32);
                }
            }
            self.holding.set(false);
            shown = self.selection_changed();
        }
        // The selected file may have changed on disk without changing its status.
        if same_root && !shown && self.workflow.selected.borrow().is_some() {
            self.load_diff();
        }
        self.workflow.update_commit();
        self.destinations.refresh();
        crate::control::changed();
    }
    /// Show the selected file's details and diff, unless they already are.
    /// Returns whether the diff was loaded again.
    fn selection_changed(&self) -> bool {
        let entry = self.selection.selected_item().and_then(|o| super::changes::entry_of(&o));
        let file = entry.as_ref().map(|e| e.file.clone());
        let cached = entry.as_ref().is_some_and(|e| e.group == super::changes::Group::Staged);
        if file.is_some() && *self.workflow.selected.borrow() == file && self.workflow.cached() == cached {
            return false;
        }
        self.workflow.select(file, cached);
        true
    }
    /// Select the `index`th file in the Changes list.
    pub fn select_file(&self, index: u32) {
        self.selection.set_selected(index);
    }
    pub fn update_readout(&self) {
        let snapshot = self.snapshot.borrow();
        let text = match snapshot.as_ref() {
            None => "No repository open".into(),
            Some(s) if s.repository.bare => "Bare repository · no working tree".into(),
            Some(s) if s.status.files.is_empty() => "Working tree clean".into(),
            Some(s) => {
                let files = &s.status.files;
                let staged = files.iter().filter(|f| f.index != '.' && f.index != '?' && !f.conflicted).count();
                let untracked = files.iter().filter(|f| f.index == '?').count();
                let conflicts = files.iter().filter(|f| f.conflicted).count();
                let mut parts = vec![format!("{} changed", files.len())];
                for (count, what) in [(staged, "staged"), (untracked, "untracked")] {
                    if count > 0 {
                        parts.push(format!("{count} {what}"));
                    }
                }
                if conflicts > 0 {
                    parts.push(format!("{conflicts} conflict{}", if conflicts == 1 { "" } else { "s" }));
                }
                parts.join(" · ")
            }
        };
        let tracking = snapshot
            .as_ref()
            .and_then(|s| {
                let up = s.status.upstream.as_ref()?;
                let (ahead, behind) = divergence(s.status.divergence.as_deref());
                Some(match (ahead, behind) {
                    (0, 0) => format!(" · {up} up to date"),
                    (a, 0) => format!(" · {up} ↑{a}"),
                    (0, b) => format!(" · {up} ↓{b}"),
                    (a, b) => format!(" · {up} ↑{a} ↓{b}"),
                })
            })
            .unwrap_or_default();
        let text = text + &tracking;
        self.readout.set_text(&text);
        self.header.set_status(&text);
    }
    /// Sync follows the repository: the upstream's counts, and what its main part does.
    pub fn update_sync(&self) {
        let snapshot = self.snapshot.borrow();
        let Some(s) = snapshot.as_ref() else {
            self.header.hide_sync();
            return;
        };
        let blocked = self.tasks.busy(&s.repository.common_dir);
        let usable = !s.repository.bare && !blocked && !(self.busy.get() && !self.quiet.get());
        let (ahead, behind) = divergence(s.status.divergence.as_deref());
        let upstream = s.status.upstream.as_deref();
        let publishable = !s.repository.unborn && s.repository.branch != "Detached HEAD";
        let action = super::header::SyncAction::choose(upstream.is_some(), publishable, ahead, behind);
        self.header.show_sync(action, ahead, behind, usable, upstream);
    }
    /// The sidebar shows the open repository; with none open there's no sidebar.
    pub fn rebuild_navigation(&self) {
        while let Some(child) = self.navigation.first_child() {
            self.navigation.remove(&child);
        }
        let open = self.root().is_some();
        if open {
            self.navigation.append(&self.sidebar.root);
        }
        self.nav.set_visible(open);
        self.sidebar_rendered();
    }
    /// The welcome page (no repository open) or the repository's pages.
    pub fn show_welcome(&self, on: bool) {
        self.main.set_visible_child_name(if on { "welcome" } else { "repo" });
        self.view_switch.root.set_visible(!on);
        if !on {
            return;
        }
        while let Some(c) = self.welcome_recent.first_child() {
            self.welcome_recent.remove(&c);
        }
        let recent: Vec<PathBuf> = prefs::get().recent.into_iter().filter(|r| r.exists()).take(8).collect();
        if recent.is_empty() {
            return;
        }
        self.welcome_recent.append(&widgets::label("RECENT", "git-insp-group"));
        for path in recent {
            let row = widgets::hbox(10);
            row.append(&gtk::Image::from_icon_name("git-folder-symbolic"));
            let name = widgets::label(&path.file_name().unwrap_or_default().to_string_lossy(), "git-welcome-name");
            row.append(&name);
            let place = widgets::label(&nyx_core::paths::pretty(&path), "git-file-dir");
            place.set_hexpand(true);
            place.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            row.append(&place);
            let b = gtk::Button::new();
            b.add_css_class("git-welcome-row");
            b.set_child(Some(&row));
            b.set_tooltip_text(Some(&format!("Open {}", path.display())));
            b.connect_clicked(move |_| {
                if let Some(ui) = current() {
                    ui.open(path.clone());
                }
            });
            self.welcome_recent.append(&b);
        }
    }
    /// New sidebar rows take the rail's compact layout.
    pub fn sidebar_rendered(&self) {
        self.compact.set(None);
        self.resize();
    }
    pub fn resize(&self) {
        let width = if self.window.width() > 0 { self.window.width() } else { self.window.default_width() };
        let compact = width < 980 || prefs::get().sidebar_collapsed;
        nyx_ui::settings_card::fit(&self.window);
        // Names in the middle only while both ends still fit beside them.
        self.view_switch.set_compact(width < 1180);
        self.header.fit(width);
        self.destinations.flow.set_narrow(width < 980);
        self.workflow.set_narrow(width < 980);
        self.destinations.activity.set_narrow(width < 980);
        if self.compact.replace(Some(compact)) != Some(compact) {
            if compact {
                self.nav.add_css_class("compact");
            } else {
                self.nav.remove_css_class("compact");
            }
            fn walk(widget: &gtk::Widget, compact: bool) {
                if widget.has_css_class("compact-hide") {
                    widget.set_visible(!compact);
                }
                if widget.has_css_class("nav-item")
                    && let Some(child) = widget.downcast_ref::<gtk::Button>().and_then(|b| b.child())
                {
                    child.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
                }
                let mut child = widget.first_child();
                while let Some(c) = child {
                    walk(&c, compact);
                    child = c.next_sibling();
                }
            }
            walk(self.nav.upcast_ref(), compact);
        }
        let orientation = if width < 980 { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal };
        if self.split.orientation() != orientation {
            let previous = self.restoring.replace(true);
            self.split.set_orientation(orientation);
            let p = prefs::get();
            let state = self
                .root()
                .and_then(|root| p.workspace.repositories.iter().find(|r| r.root == root).cloned())
                .unwrap_or_default();
            let position = if width < 980 { state.narrow_position.max(200) } else { state.details_position };
            self.split.set_position(position);
            self.restoring.set(previous);
            // Again once the window has its new size: the start pane takes up a resize.
            let split = self.split.clone();
            glib::timeout_add_local_once(Duration::from_millis(150), move || {
                if let Some(ui) = current() {
                    let previous = ui.restoring.replace(true);
                    split.set_position(position);
                    ui.restoring.set(previous);
                }
            });
        }
    }
    /// A short message that goes by itself.
    pub fn toast(self: &Rc<Self>, message: &str) {
        nyx_ui::toast::show(&self.overlay, message);
    }
    /// A problem: its first line, with Details for all of what Git said.
    pub fn error(self: &Rc<Self>, message: &str) {
        let first = message.lines().next().unwrap_or(message).trim();
        let short: String = if first.chars().count() > 96 { format!("{}…", first.chars().take(94).collect::<String>()) } else { first.into() };
        let full = message.to_string();
        nyx_ui::toast::show_action(&self.overlay, &short, "Details", move || show_details(&full));
        let mut child = self.overlay.first_child();
        while let Some(c) = child {
            if c.has_css_class("nyx-toast") {
                c.add_css_class("git-error-toast");
            }
            child = c.next_sibling();
        }
    }
    /// Open, Clone and New under the empty file list, only while no repository is open.
    pub fn show_starts(&self, show: bool) {
        if let Some(starts) = self.pages.child_by_name("empty").and_then(|b| b.last_child()) {
            starts.set_visible(show);
        }
    }
    pub fn has_toast(&self) -> bool {
        let mut child = self.overlay.first_child();
        while let Some(c) = child {
            if c.has_css_class("nyx-toast") {
                return true;
            }
            child = c.next_sibling();
        }
        false
    }
    pub fn dismiss(&self) {
        nyx_ui::toast::dismiss(&self.overlay);
    }
    pub(super) fn tabs_changed(&self, open: &[PathBuf], active: Option<&PathBuf>) -> bool {
        let key = (open.to_vec(), active.cloned());
        if self.tabs_shown.borrow().as_ref() == Some(&key) {
            return false;
        }
        *self.tabs_shown.borrow_mut() = Some(key);
        true
    }
}
fn show_search() {
    if let Some(ui) = current() {
        if ui.view_stack.visible_child_name().as_deref() == Some("history") {
            ui.graph.focus_search();
        } else {
            ui.search.grab_focus();
        }
    }
}
pub fn toggle_sidebar() {
    prefs::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    if let Some(ui) = current() {
        ui.resize();
    }
}
/// The page shown with no repository open: open, clone or start one, or pick a recent one.
fn welcome() -> (gtk::Box, gtk::Box) {
    let page = widgets::vbox(8);
    page.add_css_class("empty-state");
    page.add_css_class("git-welcome");
    page.set_halign(gtk::Align::Center);
    page.set_valign(gtk::Align::Center);
    let icon = gtk::Image::from_icon_name("git-git-symbolic");
    icon.set_pixel_size(48);
    icon.add_css_class("dim");
    page.append(&icon);
    let title = widgets::label("Open a repository", "git-welcome-title");
    title.set_xalign(0.5);
    page.append(&title);
    let note = widgets::label("Open a folder with a Git repository, clone one, or start a new one.", "dim-label");
    note.set_xalign(0.5);
    page.append(&note);
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::Center);
    buttons.set_margin_top(12);
    let open = widgets::labeled_button("git-folder-symbolic", "Open…");
    open.add_css_class("suggested-action");
    open.set_tooltip_text(Some("Open a repository (Ctrl+O)"));
    open.connect_clicked(|_| choose_folder(false));
    buttons.append(&open);
    let clone = widgets::labeled_button("git-download-symbolic", "Clone…");
    clone.connect_clicked(|_| super::providers::clone_url());
    buttons.append(&clone);
    let new = widgets::labeled_button("git-plus-symbolic", "New repository…");
    new.connect_clicked(|_| super::initialize::show());
    buttons.append(&new);
    page.append(&buttons);
    let discover = gtk::Button::with_label("Find repositories in a folder…");
    discover.add_css_class("git-link");
    discover.set_halign(gtk::Align::Center);
    discover.set_tooltip_text(Some("Ctrl+Shift+O"));
    discover.connect_clicked(|_| choose_folder(true));
    page.append(&discover);
    let recent = widgets::vbox(2);
    recent.add_css_class("git-welcome-recent");
    recent.set_size_request(460, -1);
    recent.set_margin_top(18);
    page.append(&recent);
    (page, recent)
}
pub fn choose_folder(discover: bool) {
    let Some(ui) = current() else { return };
    let dialog = gtk::FileDialog::builder()
        .title(if discover { "Find repositories in a folder" } else { "Open Git repository" })
        .modal(true)
        .build();
    let weak = Rc::downgrade(&ui);
    dialog.select_folder(Some(&ui.window), gio::Cancellable::NONE, move |result| {
        let Some(ui) = weak.upgrade() else { return };
        match result {
            Ok(folder) => {
                if let Some(path) = folder.path() {
                    if discover {
                        ui.discover(path);
                    } else {
                        ui.open(path);
                    }
                }
            }
            Err(error) if error.matches(gtk::DialogError::Dismissed) || error.matches(gtk::DialogError::Cancelled) => {}
            Err(error) => ui.error(&format!("Could not choose a folder: {error}")),
        }
    });
}
/// The whole of an error, to read and copy.
fn show_details(message: &str) {
    let (dialog, body) = widgets::dialog("What Git said", 620);
    let text = widgets::label(message, "git-error-details");
    text.add_css_class("mono");
    text.set_wrap(true);
    text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text.set_selectable(true);
    text.set_valign(gtk::Align::Start);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .propagate_natural_height(true)
        .max_content_height(420)
        .child(&text)
        .build();
    body.append(&scroll);
    let row = widgets::hbox(8);
    row.set_halign(gtk::Align::End);
    let copy = gtk::Button::with_label("Copy");
    let m = message.to_string();
    copy.connect_clicked(move |b| b.display().clipboard().set_text(&m));
    row.append(&copy);
    let close = gtk::Button::with_label("Close");
    close.add_css_class("suggested-action");
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    row.append(&close);
    body.append(&row);
    dialog.present();
    close.grab_focus();
}
pub fn show_shortcuts() {
    let (dialog, body) = widgets::dialog("Keyboard shortcuts", 520);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    rows.add_css_class("group-list");
    for (keys, title) in SHORTCUTS {
        rows.append(&widgets::row(title, "", Some(widgets::key_caps(keys).upcast_ref())));
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .propagate_natural_height(true)
        .max_content_height(560)
        .child(&rows)
        .build();
    body.append(&scroll);
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    body.append(&close);
    dialog.present();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_divergence() {
        assert_eq!(divergence(Some("+2 -1")), (2, 1));
        assert_eq!(divergence(None), (0, 0));
    }
}
