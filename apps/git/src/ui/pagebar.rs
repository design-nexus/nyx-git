//! The 44 px bar at the top of every page: its mode or filter and its search at the start,
//! the page's actions at the end.
use gtk::prelude::*;
use nyx_ui::widgets;

pub struct PageBar {
    pub root: gtk::Box,
    start: gtk::Box,
    end: gtk::Box,
}
impl PageBar {
    pub fn new() -> Self {
        let root = widgets::hbox(8);
        root.add_css_class("git-pagebar");
        let start = widgets::hbox(8);
        // The start asks for no width of its own: a narrow window clips it rather than
        // growing past the screen.
        let holder = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(true)
            .hexpand(true)
            .child(&start)
            .build();
        let end = widgets::hbox(6);
        root.append(&holder);
        root.append(&end);
        Self { root, start, end }
    }
    pub fn start(&self, w: &impl IsA<gtk::Widget>) {
        let w = w.upcast_ref::<gtk::Widget>();
        detach(w);
        w.set_valign(gtk::Align::Center);
        self.start.append(w);
    }
    pub fn end(&self, w: &impl IsA<gtk::Widget>) {
        let w = w.upcast_ref::<gtk::Widget>();
        detach(w);
        w.set_valign(gtk::Align::Center);
        self.end.append(w);
    }
}

fn detach(w: &gtk::Widget) {
    if let Some(p) = w.parent().and_downcast::<gtk::Box>() {
        p.remove(w);
    }
}

/// A page's search field: `entry` with its key hint at the end, `chars` wide.
pub fn search(entry: &gtk::SearchEntry, placeholder: &str, key: &str, chars: i32) -> gtk::Box {
    let field = widgets::hbox(0);
    field.add_css_class("git-search");
    detach(entry.upcast_ref());
    entry.set_placeholder_text(Some(placeholder));
    entry.set_width_chars(chars);
    entry.set_hexpand(true);
    entry.remove_css_class("bar-search");
    field.append(entry);
    if !key.is_empty() {
        let hint = widgets::label(key, "git-key");
        hint.set_valign(gtk::Align::Center);
        field.append(&hint);
        let h = hint.clone();
        entry.connect_changed(move |e| h.set_visible(e.text().is_empty()));
    }
    field.set_valign(gtk::Align::Center);
    // As wide as `chars`, not the whole bar.
    field.set_hexpand(false);
    field
}

/// Labelled segments (All | Unstaged | Staged) that follow `model`'s selection and set it.
/// Pages keep their `DropDown` as the state other code reads; this is how it's shown.
pub fn segments(model: &gtk::DropDown, labels: &[&str]) -> gtk::Box {
    let root = widgets::hbox(0);
    root.add_css_class("segmented");
    root.add_css_class("git-segments");
    root.set_valign(gtk::Align::Center);
    let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let b = gtk::ToggleButton::with_label(label);
        b.add_css_class("segment");
        if let Some(first) = buttons.first() {
            b.set_group(Some(first));
        }
        b.set_active(model.selected() == i as u32);
        let m = model.clone();
        b.connect_toggled(move |b| {
            if b.is_active() && m.selected() != i as u32 {
                m.set_selected(i as u32);
            }
        });
        root.append(&b);
        buttons.push(b);
    }
    model.connect_selected_notify(move |m| {
        if let Some(b) = buttons.get(m.selected() as usize)
            && !b.is_active()
        {
            b.set_active(true);
        }
    });
    root
}

/// Two labelled segments over a toggle (Unified | Split, Graph | Timeline): the second is
/// on while `toggle` is.
pub fn toggle_segments(toggle: &gtk::ToggleButton, off: &str, on: &str) -> gtk::Box {
    let root = widgets::hbox(0);
    root.add_css_class("segmented");
    root.add_css_class("git-segments");
    root.set_valign(gtk::Align::Center);
    let a = gtk::ToggleButton::with_label(off);
    let b = gtk::ToggleButton::with_label(on);
    for s in [&a, &b] {
        s.add_css_class("segment");
        root.append(s);
    }
    b.set_group(Some(&a));
    b.set_active(toggle.is_active());
    a.set_active(!toggle.is_active());
    let t = toggle.clone();
    b.connect_toggled(move |b| {
        if t.is_active() != b.is_active() {
            t.set_active(b.is_active());
        }
    });
    let (a2, b2) = (a.clone(), b.clone());
    toggle.connect_toggled(move |t| {
        if t.is_active() {
            b2.set_active(true);
        } else {
            a2.set_active(true);
        }
    });
    toggle.bind_property("sensitive", &root, "sensitive").sync_create().build();
    root
}

/// A text button with an icon, flat until hovered, for a page bar's end.
pub fn action(icon: &str, text: &str, tip: &str, run: impl Fn() + 'static) -> gtk::Button {
    let b = widgets::labeled_button(icon, text);
    b.add_css_class("git-bar-action");
    if !tip.is_empty() {
        b.set_tooltip_text(Some(tip));
    }
    b.connect_clicked(move |_| run());
    b
}
