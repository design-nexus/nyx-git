pub mod advanced;
pub mod blame;
pub mod changes;
pub mod commit_details;
pub mod destinations;
pub mod dev;
pub mod header;
pub mod icons;
pub mod inspector;
pub mod pagebar;
pub mod layout;
pub mod pages;
pub mod palette;
pub mod preview;
pub mod activity;
pub mod flow;
pub mod commit_graph;
pub mod conflict_editor;
pub mod conflict_studio;
pub mod rebase_studio;
pub mod review;
pub mod diff_view;
pub mod file_history;
pub mod graph_renderer;
pub mod initialize;
pub mod remotes;
pub mod repository_actions;
pub mod scrollbars;
pub mod settings;
pub mod sidebar;
pub mod submodules;
pub mod window;
pub mod workflow;

/// Keep closed path/ref choices compact while preserving full text in tooltips.
pub fn compact_dropdown(dropdown: &gtk::DropDown) {
    use gtk::prelude::*;
    // The open list (`list`) also checks the current choice.
    fn factory(chars: i32, list: bool) -> gtk::SignalListItemFactory {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_max_width_chars(chars);
            if !list {
                item.set_child(Some(&label));
                return;
            }
            label.set_hexpand(true);
            let check = gtk::Image::from_icon_name("object-select-symbolic");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&label);
            row.append(&check);
            item.set_child(Some(&row));
            item.connect_selected_notify(move |it| check.set_opacity(if it.is_selected() { 1.0 } else { 0.0 }));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let text = item.item().unwrap().downcast::<gtk::StringObject>().unwrap().string();
            let child = item.child().unwrap();
            let label = child.downcast_ref::<gtk::Label>().cloned().or_else(|| child.first_child().and_downcast::<gtk::Label>()).unwrap();
            label.set_text(&text);
            label.set_tooltip_text(Some(&text));
            if let Some(check) = child.last_child().and_downcast::<gtk::Image>() {
                check.set_opacity(if item.is_selected() { 1.0 } else { 0.0 });
            }
        });
        factory
    }
    dropdown.set_factory(Some(&factory(24, false)));
    dropdown.set_list_factory(Some(&factory(60, true)));
}

/// A dropdown whose closed button shows at most `chars` characters of its choice.
pub fn narrow_dropdown(dropdown: &gtk::DropDown, chars: i32) {
    use gtk::prelude::*;
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(chars);
        item.downcast_ref::<gtk::ListItem>().unwrap().set_child(Some(&label));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let text = item.item().and_downcast::<gtk::StringObject>().map(|s| s.string()).unwrap_or_default();
        if let Some(label) = item.child().and_downcast::<gtk::Label>() {
            label.set_text(&text);
            label.set_tooltip_text(Some(&text));
        }
    });
    dropdown.set_factory(Some(&factory));
}

pub mod workspace;
pub mod worktree_manager;

pub mod providers;

pub mod monitor;
pub mod tasks;

pub mod lfs;

/// One entry of a `menu`: a button, or a separator between groups.
pub enum Item {
    Action(String, Box<dyn Fn()>),
    Separator,
}
pub fn item(label: &str, run: impl Fn() + 'static) -> Item {
    Item::Action(label.into(), Box::new(run))
}
/// A popover menu in the suite's menu style. Each action closes the menu first.
pub fn menu(items: Vec<Item>) -> gtk::Popover {
    use gtk::prelude::*;
    let popover = gtk::Popover::new();
    popover.add_css_class("menu-popover");
    popover.set_has_arrow(false);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for entry in items {
        match entry {
            Item::Separator => body.append(&gtk::Separator::new(gtk::Orientation::Horizontal)),
            Item::Action(label, run) => {
                let button = gtk::Button::with_label(&label);
                button.add_css_class("flat");
                button.add_css_class("menu-item");
                if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                    label.set_xalign(0.0);
                }
                let weak = popover.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(p) = weak.upgrade() {
                        p.popdown();
                    }
                    run();
                });
                body.append(&button);
            }
        }
    }
    popover.set_child(Some(&body));
    popover
}
/// Show a `menu` at a point inside `parent` (a right-click, or Shift+F10), focusing its first entry.
pub fn popup_at(parent: &impl gtk::prelude::IsA<gtk::Widget>, items: Vec<Item>, x: f64, y: f64) {
    use gtk::prelude::*;
    let popover = menu(items);
    popover.set_parent(parent);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    popover.connect_closed(|p| {
        // Unparent after the close animation's signal returns.
        let p = p.clone();
        gtk::glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
    if let Some(first) = popover.child().and_then(|c| c.first_child()) {
        first.grab_focus();
    }
}
/// A toolbar menu button whose entries are built each time it opens, so they follow the
/// current repository's state.
pub fn menu_button(icon: &str, label: Option<&str>, tooltip: &str, build: impl Fn() -> Vec<Item> + 'static) -> gtk::MenuButton {
    use gtk::prelude::*;
    let button = gtk::MenuButton::new();
    match label {
        Some(text) => {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&gtk::Image::from_icon_name(icon));
            row.append(&gtk::Label::new(Some(text)));
            button.set_child(Some(&row));
        }
        None => {
            button.set_icon_name(icon);
            button.add_css_class("bar-button");
        }
    }
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(gtk::Align::Center);
    button.set_create_popup_func(move |b| b.set_popover(Some(&menu(build()))));
    button
}

/// Open a web page in the default browser.
pub fn open_url(url: &str) {
    let launcher = gtk::UriLauncher::new(url);
    let parent = window::current().map(|ui| ui.window.clone());
    launcher.launch(parent.as_ref(), gtk::gio::Cancellable::NONE, |result| {
        if let Err(e) = result
            && let Some(ui) = window::current()
        {
            ui.error(&format!("Could not open the page: {e}"));
        }
    });
}

/// Show the Conflict Studio, at `file` or the first conflicted file.
pub fn conflict_studio_show(file: Option<std::path::PathBuf>) {
    if let Some(ui) = window::current() {
        ui.destinations.conflicts.show(file);
    }
}

/// Give every icon-only button an accessible name from its tooltip ("Refresh status (F5)"
/// becomes "Refresh status"), so screen readers don't announce a bare "button".
pub fn name_icon_buttons(root: &gtk::Widget) {
    use gtk::prelude::*;
    let mut stack = vec![root.clone()];
    while let Some(w) = stack.pop() {
        let mut child = w.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            stack.push(c);
        }
        if !(w.is::<gtk::Button>() || w.is::<gtk::ToggleButton>() || w.is::<gtk::MenuButton>()) {
            continue;
        }
        let has_text = w.downcast_ref::<gtk::Button>().and_then(|b| b.label()).is_some()
            || w.first_child().is_some_and(|c| c.is::<gtk::Label>() || c.first_child().is_some_and(|g| g.is::<gtk::Label>() && g.next_sibling().is_none()));
        if has_text {
            continue;
        }
        if let Some(tip) = w.tooltip_text() {
            let name = tip.split(" (").next().unwrap_or(&tip).split('\n').next().unwrap_or_default().trim_end_matches('…').to_string();
            if !name.is_empty() {
                w.update_property(&[gtk::accessible::Property::Label(&name)]);
            }
        }
    }
}
