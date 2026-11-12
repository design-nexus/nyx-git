//! The inspector beside a page's content, built the same way on every page: a title, groups
//! of key and value rows, then the actions as rows with icons. At most one is the primary
//! action (solid accent); actions that delete or rewrite are red.
use gtk::{pango, prelude::*};
use nyx_ui::widgets;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Primary,
    Plain,
    Danger,
}

pub struct Inspector {
    pub root: gtk::Box,
}
impl Inspector {
    pub fn new() -> Self {
        let root = widgets::vbox(0);
        root.add_css_class("git-insp");
        Self { root }
    }
    pub fn clear(&self) {
        while let Some(c) = self.root.first_child() {
            self.root.remove(&c);
        }
    }
    pub fn append(&self, w: &impl IsA<gtk::Widget>) {
        self.root.append(w);
    }
    /// The thing inspected, with its kind's icon.
    pub fn title(&self, icon: Option<&str>, text: &str) -> gtk::Label {
        let row = widgets::hbox(8);
        row.add_css_class("git-insp-title");
        if let Some(icon) = icon {
            let image = gtk::Image::from_icon_name(icon);
            image.set_valign(gtk::Align::Start);
            row.append(&image);
        }
        let label = widgets::label(text, "");
        label.set_wrap(true);
        label.set_wrap_mode(pango::WrapMode::WordChar);
        label.set_selectable(true);
        label.set_hexpand(true);
        row.append(&label);
        self.root.append(&row);
        label
    }
    /// One dim line under the title.
    pub fn sub(&self, text: &str) -> gtk::Label {
        self.line(text, "git-insp-sub")
    }
    /// A group's label (GIT, WORKTREE, ACTIONS).
    pub fn group(&self, text: &str) {
        self.root.append(&widgets::label(&text.to_uppercase(), "git-insp-group"));
    }
    /// A key and its value; `tone` tints the value ("ok", "warn", "bad", "acc" or "").
    pub fn kv(&self, key: &str, value: &str, tone: &str) -> gtk::Label {
        let row = widgets::hbox(10);
        row.add_css_class("git-insp-kv");
        let k = widgets::label(key, "git-insp-key");
        k.set_valign(gtk::Align::Start);
        row.append(&k);
        let v = widgets::label(value, "git-insp-value");
        if !tone.is_empty() {
            v.add_css_class(tone);
        }
        v.set_wrap(true);
        v.set_wrap_mode(pango::WrapMode::WordChar);
        v.set_selectable(true);
        v.set_hexpand(true);
        row.append(&v);
        self.root.append(&row);
        v
    }
    /// Wrapped dim text (an explanation under a group).
    pub fn note(&self, text: &str) -> gtk::Label {
        self.line(text, "git-insp-note")
    }
    /// Wrapped body text.
    pub fn text(&self, text: &str) -> gtk::Label {
        self.line(text, "git-insp-text")
    }
    fn line(&self, text: &str, class: &str) -> gtk::Label {
        let l = widgets::label(text, class);
        l.set_wrap(true);
        l.set_wrap_mode(pango::WrapMode::WordChar);
        l.set_selectable(true);
        self.root.append(&l);
        l
    }
    /// A column for [`action`] rows.
    pub fn actions(&self) -> gtk::Box {
        let b = widgets::vbox(2);
        b.add_css_class("git-insp-actions");
        self.root.append(&b);
        b
    }
}

/// An action row: icon and label, full width. Its label wraps, so it never sets the
/// inspector's width.
pub fn action(icon: &str, text: &str, style: Style) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("git-insp-action");
    match style {
        Style::Primary => b.add_css_class("primary"),
        Style::Danger => b.add_css_class("danger"),
        Style::Plain => {}
    }
    let row = widgets::hbox(10);
    row.append(&gtk::Image::from_icon_name(icon));
    let l = widgets::label(text, "");
    l.set_wrap(true);
    l.set_wrap_mode(pango::WrapMode::WordChar);
    l.set_hexpand(true);
    row.append(&l);
    b.set_child(Some(&row));
    b.set_halign(gtk::Align::Fill);
    b
}

/// The icon for an action, from what its label says it does.
pub fn icon_for(label: &str) -> &'static str {
    let l = label.to_lowercase();
    let has = |w: &str| l.contains(w);
    if has("delete") || has("remove") || has("drop") || has("discard") {
        "git-trash-symbolic"
    } else if has("merge") {
        "git-merge-symbolic"
    } else if has("rebase") {
        "git-rebase-symbolic"
    } else if has("cherry") {
        "git-cherry-symbolic"
    } else if has("worktree") && (has("new") || has("create")) {
        "git-plus-symbolic"
    } else if has("worktree") {
        "git-worktree-symbolic"
    } else if has("changes") {
        "git-changes-symbolic"
    } else if has("history") {
        "git-history-symbolic"
    } else if has("agents") || has("session") {
        "git-agent-symbolic"
    } else if has("copy") {
        "git-copy-symbolic"
    } else if has("open") || has("browser") || has("page") {
        "git-open-symbolic"
    } else if has("branch") || has("switch") {
        "git-branch-symbolic"
    } else if has("publish") || has("push") {
        "git-up-symbolic"
    } else if has("pull") || has("fetch") {
        "git-down-symbolic"
    } else if has("tag") {
        "git-tag-symbolic"
    } else if has("reset") || has("restore") || has("recover") || has("undo") {
        "git-undo-symbolic"
    } else if has("flow") {
        "git-flow-symbolic"
    } else if has("review") || has("request") {
        "git-review-symbolic"
    } else {
        "git-chev-r-symbolic"
    }
}
/// Deleting, discarding, resetting and force-pushing read as dangerous.
pub fn is_danger(label: &str) -> bool {
    let l = label.to_lowercase();
    ["delete", "remove", "discard", "drop", "reset", "force"].iter().any(|w| l.contains(w))
}
