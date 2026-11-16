use super::window::{SHORTCUTS, current};
use crate::prefs;
use gtk::prelude::*;
use nyx_ui::settings_card::{self, About, Standard};
use nyx_ui::widgets;

pub fn open() {
    let Some(ui) = current() else { return };
    settings_card::open(
        &ui.overlay,
        &ui.window,
        settings_card::Spec {
            groups: &[
                ("Repositories", "folder-symbolic", "Clone folder, background fetch"),
                ("Views", "view-reveal-symbolic", "Scrollbars"),
            ],
            search_text: None,
            standard: Some(Standard {
                app: "git",
                about: About {
                    name: "Git",
                    icon: "io.github.design_nexus.nyx.Git",
                    version: env!("CARGO_PKG_VERSION"),
                    description: "Your repositories' history, changes and branches.",
                    licence: "MIT",
                    website: "https://github.com/design-nexus/nyx-git",
                    issues: "https://github.com/design-nexus/nyx-git/issues",
                    credits: &[("Git", "every change"), ("gitoxide", "reading history")],
                },
                shortcuts: SHORTCUTS,
                reduce_motion: || prefs::get().reduce_motion,
                set_reduce_motion: |on| prefs::update(|p| p.reduce_motion = on),
            }),
        },
        build,
    );
}
fn group(body: &gtk::Box, name: &str) -> gtk::Box {
    let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 12);
    wrapper.add_css_class("settings-group");
    wrapper.set_widget_name(name);
    wrapper.append(&widgets::label(&name.to_uppercase(), "group-title"));
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    rows.add_css_class("group-list");
    wrapper.append(&rows);
    body.append(&wrapper);
    rows
}
fn build() -> (gtk::Widget, gtk::Box) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let rows = group(&body, "Repositories");
    let folder = gtk::Entry::new();
    folder.set_text(&prefs::get().clone_dir);
    folder.set_placeholder_text(Some("Choose a parent folder"));
    folder.set_width_chars(18);
    folder.connect_changed(|e| prefs::update(|p| p.clone_dir = e.text().to_string()));
    rows.append(&widgets::row(
        "Clone into",
        "Default parent folder for new clones. Use an absolute path.",
        Some(folder.upcast_ref()),
    ));
    let (row, _) = widgets::choice_row(
        "Fetch in the background",
        "Fetch open repositories without merging or changing working files.",
        vec![
            ("0".into(), "Off".into()),
            ("5".into(), "Every 5 minutes".into()),
            ("15".into(), "Every 15 minutes".into()),
            ("30".into(), "Every 30 minutes".into()),
        ],
        &prefs::get().fetch_minutes.to_string(),
        |id| prefs::update(|p| p.fetch_minutes = id.parse().unwrap_or(0)),
    );
    rows.append(&row);
    let rows = group(&body, "Views");
    let p = prefs::get();
    let (row, _) = widgets::switch_row(
        "Show scrollbars",
        "Show scrollbars in Git views and dialogs. Scrolling works when hidden.",
        p.show_scrollbars,
        |on| {
            prefs::update(|p| p.show_scrollbars = on);
            super::scrollbars::apply();
        },
    );
    rows.append(&row);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .child(&body)
        .build();
    (scroll.upcast(), body)
}
