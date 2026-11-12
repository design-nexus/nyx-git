//! The five places in the window, switched from the header: icons and names, names
//! hidden in narrow windows.
use gtk::prelude::*;
use nyx_ui::widgets;
use std::rc::Rc;

pub struct Destination {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub description: &'static str,
}
pub const ALL: &[Destination] = &[
    Destination {
        id: "flow",
        title: "Flow",
        icon: "git-flow-symbolic",
        description: "Branches, worktrees, reviews and agents in progress",
    },
    Destination {
        id: "changes",
        title: "Changes",
        icon: "git-changes-symbolic",
        description: "Stage working changes and commit them",
    },
    Destination { id: "history", title: "History", icon: "git-history-symbolic", description: "The commit graph" },
    Destination {
        id: "review",
        title: "Review",
        icon: "git-review-symbolic",
        description: "Pull and merge requests with their checks",
    },
    Destination {
        id: "activity",
        title: "Activity",
        icon: "git-activity-symbolic",
        description: "What happened in the repository, and what can be recovered",
    },
];
/// Ctrl+1 … Ctrl+5 in the order shown.
pub fn key(id: &str) -> Option<usize> {
    ALL.iter().position(|d| d.id == id).map(|i| i + 1)
}

pub struct Switch {
    pub root: gtk::Box,
    buttons: Vec<(&'static str, gtk::ToggleButton, gtk::Label)>,
}
impl Switch {
    /// A segmented switch over the destinations in `ids`, in the order of [`ALL`].
    pub fn new(ids: &[&str], current: &str, on_change: impl Fn(&'static str) + 'static) -> Self {
        let root = widgets::hbox(0);
        root.add_css_class("segmented");
        root.add_css_class("view-switch");
        // Equal segments: the switch is as wide on every page and in every state.
        root.set_homogeneous(true);
        root.set_valign(gtk::Align::Center);
        let on_change = Rc::new(on_change);
        let mut first: Option<gtk::ToggleButton> = None;
        let mut buttons = Vec::new();
        for d in ALL.iter().filter(|d| ids.contains(&d.id)) {
            let b = gtk::ToggleButton::new();
            b.add_css_class("segment");
            let row = widgets::hbox(6);
            row.append(&gtk::Image::from_icon_name(d.icon));
            let label = widgets::label(d.title, "");
            row.append(&label);
            b.set_child(Some(&row));
            let shortcut = key(d.id).map(|k| format!(" (Ctrl+{k})")).unwrap_or_default();
            b.set_tooltip_text(Some(&format!("{}: {}{shortcut}", d.title, d.description)));
            b.update_property(&[gtk::accessible::Property::Label(d.title)]);
            if let Some(f) = &first {
                b.set_group(Some(f));
            } else {
                first = Some(b.clone());
            }
            b.set_active(d.id == current);
            let (id, cb) = (d.id, on_change.clone());
            b.connect_toggled(move |b| {
                if b.is_active() {
                    cb(id);
                }
            });
            root.append(&b);
            buttons.push((d.id, b, label));
        }
        Self { root, buttons }
    }
    pub fn ids(&self) -> Vec<&'static str> {
        self.buttons.iter().map(|(id, _, _)| *id).collect()
    }
    pub fn select(&self, id: &str) {
        for (bid, button, _) in &self.buttons {
            if *bid == id && !button.is_active() {
                button.set_active(true);
            }
        }
    }
    /// Icons only, for narrow windows.
    pub fn set_compact(&self, compact: bool) {
        for (_, _, label) in &self.buttons {
            label.set_visible(!compact);
        }
    }
}
