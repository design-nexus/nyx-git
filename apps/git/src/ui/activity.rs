//! Activity: what happened in the repository, newest first. Operations Nyx Git ran (from its
//! journal) and HEAD's moves from Git's reflog are told apart; recovery means making a branch
//! at an earlier commit, never resetting anything.
use super::window::current;
use gtk::{gio, glib, prelude::*};
use nyx_git::core::activity::{self, Event, Kind, Source};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

pub struct Activity {
    pub root: gtk::Box,
    store: gio::ListStore,
    filter: gtk::CustomFilter,
    selection: gtk::SingleSelection,
    search: gtk::SearchEntry,
    source: Rc<Cell<u8>>,
    kind: gtk::DropDown,
    kinds: RefCell<Vec<Option<Kind>>>,
    readout: gtk::Label,
    inspector: gtk::Box,
    loaded: RefCell<Option<PathBuf>>,
    generation: Cell<u64>,
}
fn icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Commit => "object-select-symbolic",
        Kind::Branch => "media-playlist-consecutive-symbolic",
        Kind::Checkout => "go-jump-symbolic",
        Kind::Fetch => "network-receive-symbolic",
        Kind::Pull => "go-down-symbolic",
        Kind::Push => "go-up-symbolic",
        Kind::Merge | Kind::Rebase | Kind::CherryPick | Kind::Revert => "system-run-symbolic",
        Kind::Reset | Kind::Discard => "edit-undo-symbolic",
        Kind::Stash => "folder-symbolic",
        Kind::Worktree => "folder-open-symbolic",
        Kind::Tag => "bookmark-new-symbolic",
        Kind::Recovery => "edit-undo-symbolic",
        Kind::Other => "emblem-system-symbolic",
    }
}
fn when(time: i64) -> String {
    glib::DateTime::from_unix_local(time).and_then(|d| d.format("%Y-%m-%d %H:%M:%S")).map(|s| s.to_string()).unwrap_or_default()
}
impl Activity {
    pub fn new() -> Rc<Self> {
        let root = widgets::vbox(0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        let toolbar = widgets::hbox(6);
        toolbar.add_css_class("git-history-search");
        let source = Rc::new(Cell::new(0u8));
        let s = source.clone();
        let sources = widgets::segmented(&widgets::opts(&[("all", "All"), ("journal", "Nyx Git"), ("reflog", "Git reflog")]), "all", move |id| {
            s.set(match id.as_str() {
                "journal" => 1,
                "reflog" => 2,
                _ => 0,
            });
            if let Some(ui) = current() {
                ui.destinations.activity.refilter();
            }
        });
        sources.set_tooltip_text(Some("Nyx Git: operations it ran and recorded. Git reflog: HEAD's moves, by any Git tool."));
        toolbar.append(&sources);
        let kind = gtk::DropDown::from_strings(&["All kinds"]);
        super::compact_dropdown(&kind);
        kind.set_tooltip_text(Some("Show one kind of activity"));
        toolbar.append(&kind);
        let search = gtk::SearchEntry::new();
        search.set_hexpand(true);
        search.set_placeholder_text(Some("Filter activity"));
        toolbar.append(&search);
        let refresh = widgets::bar_button("view-refresh-symbolic", "Read activity again");
        refresh.connect_clicked(|_| {
            if let Some(ui) = current() {
                ui.destinations.activity.load(true);
            }
        });
        toolbar.append(&refresh);
        root.append(&toolbar);
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let filter = gtk::CustomFilter::new(|_| true);
        let filtered = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        let selection = gtk::SingleSelection::new(Some(filtered));
        selection.set_autoselect(false);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = widgets::hbox(12);
            row.add_css_class("git-activity-row");
            let image = gtk::Image::new();
            row.append(&image);
            let text = widgets::vbox(0);
            text.set_hexpand(true);
            let title = widgets::label("", "");
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let detail = widgets::label("", "dim-label");
            detail.add_css_class("git-palette-detail");
            detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            text.append(&title);
            text.append(&detail);
            row.append(&text);
            let source = widgets::tag("", "");
            row.append(&source);
            let time = widgets::label("", "dim-label");
            time.set_width_chars(5);
            time.set_xalign(1.0);
            row.append(&time);
            item.set_child(Some(&row));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let e = item.item().and_downcast::<glib::BoxedAnyObject>().unwrap().borrow::<Event>().clone();
            let row = item.child().unwrap();
            let image = row.first_child().and_downcast::<gtk::Image>().unwrap();
            image.set_icon_name(Some(icon(e.kind)));
            let text = image.next_sibling().unwrap();
            let title = text.first_child().and_downcast::<gtk::Label>().unwrap();
            title.set_text(&e.title);
            if e.ok {
                title.remove_css_class("git-error");
            } else {
                title.add_css_class("git-error");
            }
            title.next_sibling().and_downcast::<gtk::Label>().unwrap().set_text(&e.detail);
            let tag = text.next_sibling().and_downcast::<gtk::Label>().unwrap();
            tag.set_text(if e.source == Source::Journal { "Nyx Git" } else { "reflog" });
            let time = tag.next_sibling().and_downcast::<gtk::Label>().unwrap();
            time.set_text(&super::graph_renderer::relative_time(e.time));
            time.set_tooltip_text(Some(&when(e.time)));
        });
        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("git-files");
        list.update_property(&[gtk::accessible::Property::Label("Repository activity")]);
        let scroll = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(super::scrollbars::policy())
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .child(&list)
            .build();
        let inspector = widgets::vbox(8);
        inspector.add_css_class("git-details");
        let inspector_scroll = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(super::scrollbars::policy())
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(260)
            .child(&inspector)
            .build();
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&scroll));
        paned.set_end_child(Some(&inspector_scroll));
        paned.set_resize_end_child(false);
        paned.set_shrink_end_child(false);
        paned.set_shrink_start_child(false);
        paned.set_vexpand(true);
        super::layout::persist(&paned, "activity.inspector", 340, true);
        root.append(&paned);
        let readout = widgets::label("", "git-history-readout");
        root.append(&readout);
        let me = Rc::new(Self {
            root,
            store,
            filter,
            selection: selection.clone(),
            search: search.clone(),
            source,
            kind: kind.clone(),
            kinds: RefCell::new(vec![None]),
            readout,
            inspector,
            loaded: RefCell::new(None),
            generation: Cell::new(0),
        });
        let weak = Rc::downgrade(&me);
        me.filter.set_filter_func(move |o| {
            let Some(me) = weak.upgrade() else { return true };
            let e = o.downcast_ref::<glib::BoxedAnyObject>().unwrap().borrow::<Event>().clone();
            me.matches(&e)
        });
        search.connect_search_changed(|_| {
            if let Some(ui) = current() {
                ui.destinations.activity.refilter();
            }
        });
        kind.connect_selected_notify(|_| {
            if let Some(ui) = current() {
                ui.destinations.activity.refilter();
            }
        });
        selection.connect_selected_item_notify(|s| {
            let e = s.selected_item().and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<Event>().clone());
            if let Some(ui) = current() {
                ui.destinations.activity.inspect(e.as_ref());
            }
        });
        me.inspect(None);
        me
    }
    fn matches(&self, e: &Event) -> bool {
        let source = match self.source.get() {
            1 => e.source == Source::Journal,
            2 => e.source == Source::Reflog,
            _ => true,
        };
        let kind = self.kinds.borrow().get(self.kind.selected() as usize).copied().flatten().is_none_or(|k| k == e.kind);
        let q = self.search.text().to_lowercase();
        let text = q.is_empty()
            || format!("{} {} {}", e.title, e.detail, e.after.as_deref().unwrap_or_default()).to_lowercase().contains(&q);
        source && kind && text
    }
    fn refilter(&self) {
        self.filter.changed(gtk::FilterChange::Different);
    }
    /// Read the activity of the open repository; `force` reads it again even if shown.
    pub fn load(&self, force: bool) {
        let Some(ui) = current() else { return };
        let Some((root, common)) = ui.snapshot.borrow().as_ref().map(|s| (s.repository.root.clone(), s.repository.common_dir.clone())) else {
            self.store.remove_all();
            self.readout.set_text("Open a repository to see its activity.");
            return;
        };
        if !force && self.loaded.borrow().as_ref() == Some(&root) && self.store.n_items() > 0 {
            return;
        }
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        if self.store.n_items() == 0 {
            self.readout.set_text("Reading activity…");
        }
        nyx_core::cmd::background(
            move || activity::read(&root, &common).map(|events| (root, events)),
            move |result| {
                let Some(ui) = current() else { return };
                let me = &ui.destinations.activity;
                if me.generation.get() != generation {
                    return;
                }
                match result {
                    Ok((root, events)) if ui.root().as_ref() == Some(&root) => me.fill(root, events),
                    Ok(_) => {}
                    Err(e) => me.readout.set_text(&format!("{e:#}")),
                }
            },
        );
    }
    fn fill(&self, root: PathBuf, events: Vec<Event>) {
        let selected = self.selection.selected_item().and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<Event>().clone());
        let journal = events.iter().filter(|e| e.source == Source::Journal).count();
        self.readout.set_text(&format!(
            "{} events · {journal} recorded by Nyx Git · {} from Git's reflog",
            events.len(),
            events.len() - journal
        ));
        let mut kinds: Vec<Kind> = events.iter().map(|e| e.kind).collect();
        kinds.sort();
        kinds.dedup();
        let current_kind = self.kinds.borrow().get(self.kind.selected() as usize).copied().flatten();
        let mut names = vec!["All kinds"];
        names.extend(kinds.iter().map(|k| k.title()));
        let mut list = vec![None];
        list.extend(kinds.iter().copied().map(Some));
        let at = list.iter().position(|k| *k == current_kind).unwrap_or(0);
        *self.kinds.borrow_mut() = list;
        self.kind.set_model(Some(&gtk::StringList::new(&names)));
        self.kind.set_selected(at as u32);
        let objects: Vec<_> = events.into_iter().map(glib::BoxedAnyObject::new).collect();
        self.store.splice(0, self.store.n_items(), &objects);
        *self.loaded.borrow_mut() = Some(root);
        if let Some(selected) = selected
            && let Some(model) = self.selection.model()
            && let Some(i) = (0..model.n_items())
                .find(|i| model.item(*i).and_downcast::<glib::BoxedAnyObject>().is_some_and(|o| *o.borrow::<Event>() == selected))
        {
            self.selection.set_selected(i);
        }
        if self.store.n_items() == 0 {
            self.readout.set_text("No activity yet. Commits, checkouts and Nyx Git's operations appear here.");
        }
    }
    fn inspect(&self, e: Option<&Event>) {
        while let Some(c) = self.inspector.first_child() {
            self.inspector.remove(&c);
        }
        let Some(e) = e else {
            let l = widgets::label("Select an event to see what changed and what can be recovered.", "dim-label");
            l.set_wrap(true);
            self.inspector.append(&l);
            return;
        };
        let add = |text: &str, class: &str| {
            let l = widgets::label(text, class);
            l.set_wrap(true);
            l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            l.set_selectable(true);
            self.inspector.append(&l);
        };
        add(&e.title, "section-title");
        add(&format!("{} · {}", when(e.time), e.kind.title()), "dim-label");
        add(&e.detail, if e.ok { "" } else { "git-error" });
        add(
            match e.source {
                Source::Journal => "Recorded by Nyx Git when it ran this operation, with every reference it moved.",
                Source::Reflog => "From Git's reflog: HEAD moved here. Any Git tool may have done it; Nyx Git knows only what the reflog says.",
            },
            "dim-label",
        );
        if !e.changes.is_empty() {
            add("REFERENCES", "group-title");
            for c in &e.changes {
                let s = |v: &Option<String>| v.as_deref().map_or("none".to_string(), |v| v[..v.len().min(10)].to_string());
                add(&format!("{}: {} → {}", activity::short(&c.name), s(&c.before), s(&c.after)), "");
            }
        }
        let Some(root) = current().and_then(|ui| ui.root()) else { return };
        add("RECOVERY", "group-title");
        add("A branch at a commit keeps it; nothing else changes. Uncommitted work that was lost isn't in Git and can't be brought back here.", "dim-label");
        let actions = widgets::vbox(6);
        let mut offered = false;
        let mut offer = |label: &str, id: &str| {
            let b = gtk::Button::with_label(label);
            b.set_tooltip_text(Some(id));
            let (r, id) = (root.clone(), id.to_string());
            b.connect_clicked(move |_| super::advanced::branch_at(r.clone(), id.clone()));
            actions.append(&b);
            offered = true;
        };
        if let Some(before) = e.before.as_deref() {
            offer(&format!("Branch at the earlier commit {}…", &before[..before.len().min(8)]), before);
        }
        for c in e.changes.iter().filter(|c| c.name != "HEAD" && c.after.is_none()) {
            if let Some(b) = c.before.as_deref().filter(|b| Some(*b) != e.before.as_deref()) {
                offer(&format!("Branch at deleted {}…", activity::short(&c.name)), b);
            }
        }
        if let Some(after) = e.after.as_deref() {
            let copy = gtk::Button::with_label("Copy commit hash");
            let id = after.to_string();
            copy.connect_clicked(move |b| b.display().clipboard().set_text(&id));
            actions.append(&copy);
        }
        if !offered {
            add("Nothing to recover from this event.", "dim-label");
        }
        self.inspector.append(&actions);
    }
}
