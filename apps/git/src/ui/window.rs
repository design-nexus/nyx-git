use super::{Item, item};
use crate::{prefs, theme};
use gtk::{gdk, gio, glib, prelude::*};
use nyx_git::core::{repository::Snapshot, status::FileStatus};
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
/// Fetch, Pull and Push, in the header for both views: icons, with the upstream's
/// counts beside Pull and Push.
pub struct Sync {
    fetch: gtk::Button,
    pull: gtk::Button,
    pull_count: gtk::Label,
    push: gtk::Button,
    push_icon: gtk::Image,
    push_count: gtk::Label,
    menu: gtk::MenuButton,
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
    pub readout: gtk::Label,
    pub refresh: gtk::Button,
    pub workflow: super::workflow::Workflow,
    pub graph: super::commit_graph::CommitGraph,
    pub tasks: super::tasks::Tasks,
    pub monitor: super::monitor::Monitor,
    pub sidebar: super::sidebar::Sidebar,
    pub(super) tabs: gtk::Box,
    tabs_shown: RefCell<Option<(Vec<PathBuf>, Option<PathBuf>)>>,
    pub(super) restoring: Cell<bool>,
    pub(super) view_stack: gtk::Stack,
    detail_stack: gtk::Stack,
    detail_scroll: gtk::ScrolledWindow,
    view_switch: super::destinations::Switch,
    pub destinations: super::pages::Pages,
    sync: Sync,
    selection: gtk::SingleSelection,
    nav: gtk::Box,
    navigation: gtk::Box,
    pub(super) search: gtk::SearchEntry,
    pub(super) files: gio::ListStore,
    pub(super) pages: gtk::Stack,
    pub(super) empty: gtk::Label,
    filter: gtk::CustomFilter,
    pub(super) summary: gtk::Label,
    pub(super) details: gtk::Label,
    notice: gtk::Box,
    notice_label: gtk::Label,
    notice_generation: Cell<u64>,
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
    (&["Ctrl", "Shift", "P"], "Command palette"),
    (&["F1"], "Keyboard shortcuts"),
    (&["Esc"], "Close settings, dismiss a message or clear search"),
    (&["Ctrl", "Q"], "Quit"),
];

/// Repository-wide tools: one menu instead of buttons scattered over the window.
fn repository_menu() -> Vec<Item> {
    let external = |kind: &'static str| {
        move || {
            if let Some(root) = current().and_then(|ui| ui.root()) {
                super::workspace::external(root, kind);
            }
        }
    };
    vec![
        item("Branches…", super::repository_actions::branches),
        item("Stashes…", super::repository_actions::stashes),
        item("Remotes…", super::remotes::show),
        item("Worktrees…", super::worktree_manager::show),
        item("Submodules…", super::submodules::show),
        item("Git LFS…", super::lfs::show),
        Item::Separator,
        item("Git actions…", || super::advanced::show(None)),
        item("Recovery points…", super::advanced::recovery),
        item("Operation history…", super::tasks::show),
        Item::Separator,
        item("Open folder", external("folder")),
        item("Open in editor", external("editor")),
        item("Open in terminal", external("terminal")),
    ]
}
/// Ways to add a repository, behind the tab strip's + button.
pub fn add_menu() -> Vec<Item> {
    vec![
        item("Open repository…", || choose_folder(false)),
        item("Clone repository…", super::providers::clone_url),
        item("Initialize repository…", super::initialize::show),
        item("Discover repositories…", || choose_folder(true)),
        Item::Separator,
        item("Repositories, favorites and groups…", super::workspace::catalog),
        item("Hosting accounts…", super::providers::show),
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
    search.set_placeholder_text(Some("Filter changed files"));
    search.add_css_class("bar-search");
    search.set_width_chars(22);
    search.set_valign(gtk::Align::Center);
    let (tab_strip, tabs) = super::workspace::tab_strip();
    content.append(&tab_strip);
    let destinations = super::pages::Pages::new();
    let ids = destinations.ids();
    let initial = nyx_theme::start_page("history", &prefs::get().last_view);
    let initial = if ids.contains(&initial.as_str()) { initial } else { "history".into() };
    let view_switch = super::destinations::Switch::new(&ids, &initial, |id| {
        if let Some(ui) = current() {
            ui.show_view(id);
        }
    });
    let sync_button = |icon: &str, tip: &str, run: fn()| {
        let b = gtk::Button::new();
        b.add_css_class("bar-button");
        b.add_css_class("sync-button");
        let row = widgets::hbox(4);
        let image = gtk::Image::from_icon_name(icon);
        row.append(&image);
        let count = widgets::label("", "sync-count");
        count.set_visible(false);
        row.append(&count);
        b.set_child(Some(&row));
        b.set_tooltip_text(Some(tip));
        b.set_valign(gtk::Align::Center);
        b.connect_clicked(move |_| run());
        (b, image, count)
    };
    let (fetch, _, _) =
        sync_button("network-receive-symbolic", "Fetch all remotes (Ctrl+Shift+F)", || run(nyx_git::core::operations::Operation::Fetch));
    let (pull, _, pull_count) = sync_button("go-down-symbolic", "Fast-forward from the upstream (Ctrl+Shift+L)", || {
        run(nyx_git::core::operations::Operation::Pull)
    });
    let (push_button, push_icon, push_count) = sync_button("go-up-symbolic", "Push to the upstream (Ctrl+Alt+P)", push);
    let sync = Sync {
        fetch,
        pull,
        pull_count,
        push: push_button,
        push_icon,
        push_count,
        menu: super::menu_button(
            "open-menu-symbolic",
            None,
            "Repository tools: branches, stashes, remotes and more",
            repository_menu,
        ),
    };
    let refresh = widgets::bar_button("view-refresh-symbolic", "Refresh status (F5)");
    let tasks = super::tasks::Tasks::new();
    content.append(&tasks.bar);

    let workflow = super::workflow::Workflow::new();
    content.append(&workflow.operation_bar);
    content.append(&workflow.toolbar);
    // GTK recycles visible rows rather than constructing a widget for every file.
    let files = gio::ListStore::new::<glib::BoxedAnyObject>();
    let search2 = search.clone();
    let category = workflow.category.clone();
    let filter = gtk::CustomFilter::new(move |object| {
        let Some(object) = object.downcast_ref::<glib::BoxedAnyObject>() else { return false };
        let f = object.borrow::<FileStatus>();
        let category_match = match category.selected() {
            1 => f.worktree != '.' && f.index != '?',
            2 => f.index != '.' && f.index != '?',
            3 => f.index == '?',
            _ => true,
        };
        category_match && f.path.to_string_lossy().to_lowercase().contains(&search2.text().to_lowercase())
    });
    let filtered = gtk::FilterListModel::new(Some(files.clone()), Some(filter.clone()));
    let selection = gtk::SingleSelection::new(Some(filtered));
    selection.set_autoselect(false);
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let kind = widgets::label("", "dim-label");
        kind.set_width_chars(10);
        let path = widgets::label("", "");
        path.set_hexpand(true);
        path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        row.append(&kind);
        row.append(&path);
        item.set_child(Some(&row));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let object = item.item().unwrap().downcast::<glib::BoxedAnyObject>().unwrap();
        let file = object.borrow::<FileStatus>();
        let row = item.child().unwrap();
        let kind = row.first_child().unwrap().downcast::<gtk::Label>().unwrap();
        kind.set_text(file.short_kind());
        kind.set_tooltip_text(Some(file.kind()));
        if file.conflicted {
            kind.add_css_class("git-error");
        } else {
            kind.remove_css_class("git-error");
        }
        let path = kind.next_sibling().unwrap().downcast::<gtk::Label>().unwrap();
        match &file.original {
            Some(original) => path.set_text(&format!("{} → {}", original.to_string_lossy(), file.path.to_string_lossy())),
            None => path.set_text(&file.path.to_string_lossy()),
        }
        path.set_tooltip_text(Some(&file.path.to_string_lossy()));
    });
    let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
    list.add_css_class("git-files");
    let scroll = gtk::ScrolledWindow::builder()
        .vscrollbar_policy(super::scrollbars::policy())
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    let pages = gtk::Stack::new();
    pages.set_vexpand(true);
    pages.add_named(&scroll, Some("files"));
    let empty_box = gtk::Box::new(gtk::Orientation::Vertical, 16);
    empty_box.set_halign(gtk::Align::Center);
    empty_box.set_valign(gtk::Align::Center);
    let empty = widgets::label("Open a repository to view working changes.", "dim-label");
    empty.set_wrap(true);
    empty.set_justify(gtk::Justification::Center);
    empty.set_xalign(0.5);
    empty_box.append(&empty);
    let starts = widgets::hbox(8);
    starts.set_halign(gtk::Align::Center);
    starts.add_css_class("git-start");
    let open = gtk::Button::with_label("Open repository…");
    open.add_css_class("suggested-action");
    open.connect_clicked(|_| choose_folder(false));
    starts.append(&open);
    let clone = gtk::Button::with_label("Clone…");
    clone.connect_clicked(|_| super::providers::clone_url());
    starts.append(&clone);
    let init = gtk::Button::with_label("Initialize…");
    init.connect_clicked(|_| super::initialize::show());
    starts.append(&init);
    empty_box.append(&starts);
    pages.add_named(&empty_box, Some("empty"));
    pages.set_visible_child_name("empty");
    let details = widgets::label("Select a changed file to inspect its status.", "dim-label");
    details.set_wrap(true);
    details.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    details.set_selectable(true);
    details.set_valign(gtk::Align::Start);
    let detail_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    detail_box.add_css_class("git-details");
    detail_box.append(&widgets::label("FILE DETAILS", "group-title"));
    detail_box.append(&details);
    detail_box.append(&workflow.detail);
    let changes = gtk::Box::new(gtk::Orientation::Vertical, 0);
    changes.append(&pages);
    changes.append(&workflow.composer);
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
    detail_stack.add_named(&detail_box, Some("changes"));
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
    // The app and the repository, the sidebar toggle; History or Changes in the middle;
    // what's changed, Fetch, Pull, Push, the repository's tools, search, refresh and
    // Settings at the end.
    let header = chrome::app_header(&window, "folder-symbolic", "Git");
    // The repository and branch, after the app's name.
    let summary = header.subtitle.clone();
    summary.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    summary.set_text("Open a repository");
    summary.set_visible(true);
    let collapse = widgets::bar_button("sidebar-show-symbolic", "Collapse the sidebar (Ctrl+B)");
    collapse.connect_clicked(|_| toggle_sidebar());
    header.start.append(&collapse);
    header.center.append(&view_switch.root);
    header.end.set_spacing(4);
    let readout = widgets::label("No repository open", "header-status");
    readout.set_ellipsize(gtk::pango::EllipsizeMode::End);
    readout.set_margin_end(6);
    header.end.append(&readout);
    for b in [&sync.fetch, &sync.pull, &sync.push] {
        header.end.append(b);
    }
    header.end.append(&sync.menu);
    // Changes filters its files from the header; History has its own find bar.
    let find = widgets::bar_button("system-search-symbolic", "Find commits (Ctrl+F)");
    find.connect_clicked(|_| show_search());
    header.end.append(&search);
    header.end.append(&find);
    search.set_visible(initial == "changes");
    find.set_visible(initial == "history");
    {
        let (search, find) = (search.clone(), find.clone());
        view_stack.connect_visible_child_name_notify(move |stack| {
            let name = stack.visible_child_name();
            search.set_visible(name.as_deref() == Some("changes"));
            find.set_visible(name.as_deref() == Some("history"));
        });
    }
    header.end.append(&refresh);
    header.end.append(&chrome::settings_button(super::settings::open));
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("window-frame");
    frame.append(&header.root);
    frame.append(&body);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&frame));
    window.set_child(Some(&overlay));
    let notice = widgets::hbox(10);
    notice.add_css_class("git-notice");
    notice.set_halign(gtk::Align::Center);
    notice.set_valign(gtk::Align::End);
    notice.set_visible(false);
    let notice_label = widgets::label("", "");
    notice_label.set_wrap(true);
    notice_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    notice_label.set_max_width_chars(70);
    notice_label.set_selectable(true);
    notice_label.set_hexpand(true);
    notice.append(&notice_label);
    let dismiss = widgets::icon_button("window-close-symbolic", "Dismiss (Esc)");
    dismiss.set_valign(gtk::Align::Start);
    notice.append(&dismiss);
    overlay.add_overlay(&notice);
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
        refresh,
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
        sync,
        selection: selection.clone(),
        nav,
        navigation,
        search,
        files,
        pages,
        empty,
        filter,
        summary,
        details,
        notice,
        notice_label,
        notice_generation: Cell::new(0),
        split,
        compact: Cell::new(None),
        holding: Cell::new(false),
    });
    UI.with(|u| *u.borrow_mut() = Some(ui.clone()));
    dismiss.connect_clicked(|_| {
        if let Some(ui) = current() {
            ui.dismiss();
        }
    });
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

    ui.refresh.connect_clicked(|_| {
        if let Some(ui) = current() {
            ui.refresh();
        }
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
    ui.workflow.message.buffer().connect_changed(move |_| {
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
            gdk::Key::k | gdk::Key::K if ctrl => super::palette::show(super::palette::Mode::Repositories),
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
            gdk::Key::b | gdk::Key::B if ctrl && shift => super::palette::show(super::palette::Mode::Branches),
            gdk::Key::p | gdk::Key::P if ctrl && !shift => super::palette::show(super::palette::Mode::Files),
            gdk::Key::f | gdk::Key::F if ctrl => show_search(),
            gdk::Key::b | gdk::Key::B if ctrl => toggle_sidebar(),
            gdk::Key::F5 => ui.refresh(),
            gdk::Key::F1 => show_shortcuts(),
            gdk::Key::F6 => ui.focus_panel(shift),
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
                    if ui.notice.is_visible() {
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
    ui.window.present();
    super::dev::capture(app, &ui.window);
    if prefs::take_broken() {
        ui.error("Settings could not be read. The previous file is kept as settings.toml.bak.");
    }
    ui
}

fn run(operation: nyx_git::core::operations::Operation) {
    if let Some(ui) = current() {
        ui.execute(operation);
    }
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

fn nav_button(icon: &str, title: &str, tooltip: &str, run: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.append(&gtk::Image::from_icon_name(icon));
    let label = widgets::label(title, "nav-label");
    label.add_css_class("compact-hide");
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.append(&label);
    button.set_child(Some(&row));
    button.set_tooltip_text(Some(tooltip));
    button.connect_clicked(move |_| run());
    button
}

/// What a status letter from `git status --porcelain=v2` means.
fn change(code: char) -> &'static str {
    match code {
        'M' => "modified",
        'T' => "type changed",
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'U' => "unmerged",
        _ => "",
    }
}
fn describe(file: &FileStatus) -> String {
    let mut lines = vec![file.path.to_string_lossy().into_owned()];
    if let Some(original) = &file.original {
        lines.push(format!("Renamed from {}", original.display()));
    }
    lines.push(String::new());
    if file.conflicted {
        lines.push("Conflict: resolve it before committing.".into());
    } else if file.index == '?' {
        lines.push("Untracked: not yet in the repository.".into());
    } else {
        if file.index != '.' {
            lines.push(format!("Staged: {}", change(file.index)));
        }
        if file.worktree != '.' {
            lines.push(format!("Not staged: {}", change(file.worktree)));
        }
    }
    if file.submodule {
        lines.push("A submodule".into());
    }
    lines.join("\n")
}
/// "+2 -1" from `# branch.ab` as ahead and behind counts.
fn divergence(text: Option<&str>) -> (u64, u64) {
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
        self.workflow.toolbar.set_visible(name == "changes");
        self.view_switch.select(name);
        self.destinations.shown(name);
        // A studio is a task in progress, not a place to come back to on restart.
        if !super::pages::is_studio(name) {
            prefs::update(|p| p.last_view = name.into());
            self.save_repository();
        }
    }
    pub fn apply(&self, snapshot: Snapshot) {
        let previous_root = self.root();
        let same_root = previous_root.as_ref() == Some(&snapshot.repository.root);
        let previous_file = self.workflow.selected.borrow().as_ref().map(|f| f.path.clone()).filter(|_| same_root);
        self.restore_repository(&snapshot.repository.root);
        let repo = &snapshot.repository;
        self.workflow.operation_status(snapshot.progress, snapshot.status.files.iter().any(|f| f.conflicted));
        self.graph.open(repo.root.clone(), repo.head.clone());
        self.monitor.install(&snapshot);
        let name = repo.root.file_name().unwrap_or_default().to_string_lossy().into_owned();
        chrome::set_title(&self.window, "Git", &name);
        self.summary.set_text(&format!(
            "{name} · {}{}",
            repo.branch,
            if repo.bare {
                " · bare"
            } else if repo.unborn {
                " · no commits yet"
            } else {
                ""
            }
        ));
        self.summary.set_tooltip_text(Some(&format!(
            "{}\nGit directory: {}\nHEAD: {}",
            repo.root.display(),
            repo.git_dir.display(),
            repo.head.as_deref().unwrap_or("No commits yet")
        )));
        self.empty.set_text(if repo.bare { "Bare repository: no working tree." } else { "Working tree clean." });
        self.pages.set_visible_child_name(if snapshot.status.files.is_empty() { "empty" } else { "files" });
        let files = snapshot.status.files.clone();
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
        // Change only the rows that differ, so the selection, the diff and its scroll
        // position stay put while files elsewhere in the repository change.
        let old: Vec<FileStatus> = if same_root {
            (0..self.files.n_items())
                .filter_map(|i| self.files.item(i).and_downcast::<glib::BoxedAnyObject>())
                .map(|o| o.borrow::<FileStatus>().clone())
                .collect()
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
            if self.selection.selected_item().is_none()
                && let Some(path) = previous_file
                && let Some(model) = self.selection.model()
                && let Some(i) = (0..model.n_items()).find(|i| {
                    model.item(*i).and_downcast::<glib::BoxedAnyObject>().is_some_and(|o| o.borrow::<FileStatus>().path == path)
                })
            {
                self.selection.set_selected(i);
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
        let file =
            self.selection.selected_item().and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<FileStatus>().clone());
        if file.is_some() && *self.workflow.selected.borrow() == file {
            return false;
        }
        self.details.set_text(&file.as_ref().map_or_else(|| "Select a changed file to inspect its status.".into(), describe));
        self.workflow.select(file);
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
        self.readout.set_text(&(text + &tracking));
    }
    /// Fetch, Pull and Push follow the repository: counts from the upstream, and Push
    /// becomes Publish for a branch without one.
    pub fn update_sync(&self) {
        let snapshot = self.snapshot.borrow();
        let blocked = snapshot.as_ref().is_some_and(|s| self.tasks.busy(&s.repository.common_dir));
        let usable = snapshot.as_ref().is_some_and(|s| !s.repository.bare) && !blocked && !(self.busy.get() && !self.quiet.get());
        let (ahead, behind) = snapshot.as_ref().map_or((0, 0), |s| divergence(s.status.divergence.as_deref()));
        let upstream = snapshot.as_ref().is_some_and(|s| s.status.upstream.is_some());
        let unborn = snapshot.as_ref().is_none_or(|s| s.repository.unborn);
        let detached = snapshot.as_ref().is_some_and(|s| s.repository.branch == "Detached HEAD");
        self.sync.fetch.set_sensitive(usable);
        self.sync.pull.set_sensitive(usable && upstream);
        self.sync.pull_count.set_text(&format!("{behind}"));
        self.sync.pull_count.set_visible(behind > 0);
        self.sync.pull.set_tooltip_text(Some(&if behind > 0 {
            format!("Pull {behind} commit{} from the upstream (Ctrl+Shift+L)", if behind == 1 { "" } else { "s" })
        } else {
            "Fast-forward from the upstream (Ctrl+Shift+L)".into()
        }));
        self.sync.push.set_sensitive(usable && !unborn && !detached);
        if upstream || unborn || detached {
            self.sync.push_icon.set_icon_name(Some("go-up-symbolic"));
            self.sync.push_count.set_text(&format!("{ahead}"));
            self.sync.push_count.set_visible(ahead > 0);
            self.sync.push.set_tooltip_text(Some(&if ahead > 0 {
                format!("Push {ahead} commit{} to the upstream (Ctrl+Alt+P)", if ahead == 1 { "" } else { "s" })
            } else {
                "Push to the upstream (Ctrl+Alt+P)".into()
            }));
        } else {
            self.sync.push_icon.set_icon_name(Some("send-to-symbolic"));
            self.sync.push_count.set_visible(false);
            self.sync.push.set_tooltip_text(Some("Publish: push this branch to a remote and track it (Ctrl+Alt+P)"));
        }
        self.sync.menu.set_sensitive(snapshot.is_some());
    }
    pub fn rebuild_navigation(&self) {
        while let Some(child) = self.navigation.first_child() {
            self.navigation.remove(&child);
        }
        if self.root().is_some() {
            self.navigation.append(&self.sidebar.root);
        }
        let p = prefs::get();
        let active = self.root();
        let mut sections = std::collections::BTreeMap::<String, Vec<PathBuf>>::new();
        for entry in &p.workspace.repositories {
            if entry.favorite {
                sections.entry("FAVORITES".into()).or_default().push(entry.root.clone());
            }
            if !entry.group.is_empty() {
                sections.entry(format!("GROUP · {}", entry.group)).or_default().push(entry.root.clone());
            }
        }
        sections.insert(
            "RECENT REPOSITORIES".into(),
            p.recent
                .into_iter()
                .filter(|path| !p.workspace.repositories.iter().any(|r| r.root == *path && (r.favorite || !r.group.is_empty())))
                .collect(),
        );
        for (title, paths) in sections {
            if paths.is_empty() {
                continue;
            }
            let label = widgets::label(&title, "nav-group");
            label.add_css_class("compact-hide");
            label.set_max_width_chars(22);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            self.navigation.append(&label);
            let recent = title == "RECENT REPOSITORIES";
            for path in paths {
                let path2 = path.clone();
                let button = nav_button(
                    if title == "FAVORITES" { "starred-symbolic" } else { "folder-symbolic" },
                    &path.file_name().unwrap_or_default().to_string_lossy(),
                    &format!("{}\nRight-click for more", path.to_string_lossy()),
                    move || {
                        if let Some(ui) = current() {
                            ui.open(path2.clone());
                        }
                    },
                );
                if active.as_ref() == Some(&path) {
                    button.add_css_class("active");
                }
                let menu = move |button: &gtk::Button, x: f64, y: f64| {
                    let (open, folder, remove) = (path.clone(), path.clone(), path.clone());
                    let mut items = vec![
                        item("Open", move || {
                            if let Some(ui) = current() {
                                ui.open(open.clone());
                            }
                        }),
                        item("Open folder", move || super::workspace::external(folder.clone(), "folder")),
                        item("Favorites and groups…", super::workspace::catalog),
                    ];
                    if recent {
                        items.push(Item::Separator);
                        items.push(item("Remove from recent", move || {
                            prefs::update(|p| p.recent.retain(|r| r != &remove));
                            if let Some(ui) = current() {
                                ui.rebuild_navigation();
                            }
                        }));
                    }
                    super::popup_at(button, items, x, y);
                };
                super::sidebar::context_menu(&button, menu);
                self.navigation.append(&button);
            }
        }
        self.compact.set(None);
        self.resize();
    }
    pub fn resize(&self) {
        let width = if self.window.width() > 0 { self.window.width() } else { self.window.default_width() };
        let compact = width < 980 || prefs::get().sidebar_collapsed;
        nyx_ui::settings_card::fit(&self.window);
        self.readout.set_visible(width >= 1180);
        self.view_switch.set_compact(width < 980);
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
            self.split.set_position(if width < 980 { state.narrow_position } else { state.details_position });
            self.restoring.set(previous);
        }
    }
    fn notify(self: &Rc<Self>, message: &str, error: bool) {
        self.notice_label.set_text(message);
        if error {
            self.notice.add_css_class("error");
        } else {
            self.notice.remove_css_class("error");
        }
        self.notice.set_visible(true);
        let generation = self.notice_generation.get() + 1;
        self.notice_generation.set(generation);
        // Errors stay until dismissed; they often need reading twice.
        if error {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(6), move || {
            if let Some(ui) = weak.upgrade()
                && ui.notice_generation.get() == generation
            {
                ui.notice.set_visible(false);
            }
        });
    }
    /// A short message that goes by itself.
    pub fn toast(self: &Rc<Self>, message: &str) {
        self.notify(message, false);
    }
    /// A problem, shown until dismissed (Esc or ×).
    pub fn error(self: &Rc<Self>, message: &str) {
        self.notify(message, true);
    }
    pub fn dismiss(&self) {
        self.notice_generation.set(self.notice_generation.get() + 1);
        self.notice.set_visible(false);
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
fn toggle_sidebar() {
    prefs::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    if let Some(ui) = current() {
        ui.resize();
    }
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
    fn reads_divergence_and_describes_changes() {
        assert_eq!(divergence(Some("+2 -1")), (2, 1));
        assert_eq!(divergence(None), (0, 0));
        let file =
            FileStatus { path: "a".into(), original: None, index: 'M', worktree: 'D', conflicted: false, submodule: false };
        assert!(describe(&file).contains("Staged: modified\nNot staged: deleted"));
    }
}
