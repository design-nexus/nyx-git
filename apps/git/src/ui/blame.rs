//! Blame: who last changed each line of a file, newest commits tinted strongest.
use super::window::current;
use gtk::{gio, glib, prelude::*};
use nyx_git::core::files::{self, BlameLine};
use nyx_ui::widgets;
use std::path::PathBuf;

pub fn open(root: PathBuf, file: PathBuf) {
    let (dialog, body) = widgets::dialog("Blame", 880);
    dialog.set_default_height(720);
    let heading = widgets::label(&file.display().to_string(), "dim-label");
    heading.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    heading.set_selectable(true);
    body.append(&heading);
    let readout = widgets::label("Reading blame…", "dim-label");
    body.append(&readout);
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let row = widgets::hbox(12);
        row.add_css_class("git-blame-row");
        let who = widgets::label("", "git-blame-who");
        who.set_width_chars(34);
        who.set_max_width_chars(34);
        who.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let number = widgets::label("", "git-blame-number");
        number.set_width_chars(5);
        number.set_xalign(1.0);
        let text = widgets::label("", "git-blame-text");
        text.set_hexpand(true);
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&who);
        row.append(&number);
        row.append(&text);
        item.set_child(Some(&row));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let object = item.item().and_downcast::<glib::BoxedAnyObject>().unwrap();
        let (index, line, first) = object.borrow::<(usize, BlameLine, bool)>().clone();
        let row = item.child().unwrap();
        let who = row.first_child().and_downcast::<gtk::Label>().unwrap();
        let number = who.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let text = number.next_sibling().and_downcast::<gtk::Label>().unwrap();
        who.set_text(&if !first {
            String::new()
        } else if line.uncommitted() {
            "Not committed yet".into()
        } else {
            format!("{} · {} · {}", &line.commit[..8], line.author, super::graph_renderer::relative_time(line.time))
        });
        who.set_tooltip_text((!line.uncommitted()).then(|| format!("{}\n{}", line.commit, line.summary)).as_deref());
        number.set_text(&(index + 1).to_string());
        text.set_text(&line.text.replace('\t', "    "));
    });
    let selection = gtk::SingleSelection::new(Some(store.clone()));
    let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
    list.add_css_class("git-blame");
    list.update_property(&[gtk::accessible::Property::Label("Lines and the commits that last changed them")]);
    body.append(
        &gtk::ScrolledWindow::builder()
            .vscrollbar_policy(super::scrollbars::policy())
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .min_content_height(420)
            .child(&list)
            .build(),
    );
    let footer = widgets::hbox(8);
    let detail = widgets::label("Select a line to see its commit.", "dim-label");
    detail.set_hexpand(true);
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    footer.append(&detail);
    let copy = gtk::Button::with_label("Copy commit hash");
    copy.set_sensitive(false);
    footer.append(&copy);
    let history = gtk::Button::with_label("File history");
    footer.append(&history);
    let close = gtk::Button::with_label("Close");
    footer.append(&close);
    body.append(&footer);
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    let (r, f, d) = (root.clone(), file.clone(), dialog.clone());
    history.connect_clicked(move |_| {
        d.close();
        super::file_history::open(r.clone(), f.clone(), None);
    });
    {
        let (detail, copy) = (detail.clone(), copy.clone());
        selection.connect_selected_item_notify(move |s| {
            let line = s.selected_item().and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<(usize, BlameLine, bool)>().1.clone());
            match line {
                Some(line) if !line.uncommitted() => {
                    detail.set_text(&format!("{} {} · {}", &line.commit[..10], line.summary, line.author));
                    copy.set_sensitive(true);
                }
                Some(_) => {
                    detail.set_text("This line has changes that aren't committed yet.");
                    copy.set_sensitive(false);
                }
                None => copy.set_sensitive(false),
            }
        });
    }
    {
        let selection = selection.clone();
        copy.connect_clicked(move |b| {
            if let Some(line) =
                selection.selected_item().and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<(usize, BlameLine, bool)>().1.clone())
            {
                b.display().clipboard().set_text(&line.commit);
            }
        });
    }
    let d = dialog.clone();
    nyx_core::cmd::background(
        move || files::blame(&root, &file, None),
        move |result| {
            if !d.is_visible() {
                return;
            }
            match result {
                Ok(lines) => {
                    let commits = lines.iter().map(|l| &l.commit).collect::<std::collections::HashSet<_>>().len();
                    readout.set_text(&format!("{} lines · {commits} commits", lines.len()));
                    let mut previous: Option<String> = None;
                    let objects: Vec<_> = lines
                        .into_iter()
                        .enumerate()
                        .map(|(i, line)| {
                            let first = previous.as_ref() != Some(&line.commit);
                            previous = Some(line.commit.clone());
                            glib::BoxedAnyObject::new((i, line, first))
                        })
                        .collect();
                    store.splice(0, 0, &objects);
                }
                Err(e) => {
                    readout.set_text(&format!("{e:#}"));
                    if let Some(ui) = current() {
                        ui.error(&format!("{e:#}"));
                    }
                }
            }
        },
    );
    dialog.present();
}
