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
    paned: gtk::Paned,
    store: gio::ListStore,
    filter: gtk::CustomFilter,
    selection: gtk::SingleSelection,
    search: gtk::SearchEntry,
    source: Rc<Cell<u8>>,
    kind: gtk::DropDown,
    kinds: RefCell<Vec<Option<Kind>>>,
    readout: gtk::Label,
    inspector: super::inspector::Inspector,
    loaded: RefCell<Option<PathBuf>>,
    generation: Cell<u64>,
}
fn icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Commit => "git-commit-symbolic",
        Kind::Branch | Kind::Checkout => "git-branch-symbolic",
        Kind::Fetch | Kind::Pull => "git-down-symbolic",
        Kind::Push => "git-up-symbolic",
        Kind::Merge => "git-merge-symbolic",
        Kind::Rebase => "git-rebase-symbolic",
        Kind::CherryPick => "git-cherry-symbolic",
        Kind::Revert | Kind::Reset | Kind::Discard | Kind::Recovery => "git-undo-symbolic",
        Kind::Stash => "git-stash-symbolic",
        Kind::Worktree => "git-worktree-symbolic",
        Kind::Tag => "git-tag-symbolic",
        Kind::Other => "git-activity-symbolic",
    }
}
/// The local day `time` falls on, for the list's headings.
fn day(time: i64) -> i64 {
    glib::DateTime::from_unix_local(time).map(|d| (d.year() as i64) * 400 + d.day_of_year() as i64).unwrap_or(0)
}
/// "Today", "Yesterday", or the date.
fn day_title(time: i64) -> String {
    let now = glib::DateTime::now_local().map(|d| d.to_unix()).unwrap_or(time);
    if day(time) == day(now) {
        "Today".into()
    } else if day(time) == day(now - 86400) {
        "Yesterday".into()
    } else {
        glib::DateTime::from_unix_local(time).and_then(|d| d.format("%A, %e %B")).map(|s| s.to_string()).unwrap_or_default()
    }
}
fn event_of(o: &glib::Object) -> Option<Event> {
    o.downcast_ref::<glib::BoxedAnyObject>().map(|o| o.borrow::<Event>().clone())
}
fn when(time: i64) -> String {
    glib::DateTime::from_unix_local(time).and_then(|d| d.format("%Y-%m-%d %H:%M:%S")).map(|s| s.to_string()).unwrap_or_default()
}
impl Activity {
    pub fn new() -> Rc<Self> {
        let root = widgets::vbox(0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        let toolbar = super::pagebar::PageBar::new();
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
        sources.add_css_class("git-segments");
        sources.set_tooltip_text(Some("Nyx Git: operations it ran and recorded. Git reflog: HEAD's moves, by any Git tool."));
        toolbar.start(&sources);
        let kind = gtk::DropDown::from_strings(&["All kinds"]);
        super::compact_dropdown(&kind);
        kind.add_css_class("git-dropdown");
        kind.set_tooltip_text(Some("Show one kind of activity"));
        toolbar.start(&kind);
        let search = gtk::SearchEntry::new();
        toolbar.start(&super::pagebar::search(&search, "Filter activity", "", 16));
        let refresh = widgets::bar_button("git-refresh-symbolic", "Read activity again");
        refresh.connect_clicked(|_| {
            if let Some(ui) = current() {
                ui.destinations.activity.load(true);
            }
        });
        toolbar.end(&refresh);
        root.append(&toolbar.root);
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let filter = gtk::CustomFilter::new(|_| true);
        let filtered = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        // A heading for each day, newest first (the events already are).
        let sorted = gtk::SortListModel::new(Some(filtered), None::<gtk::Sorter>);
        sorted.set_section_sorter(Some(&gtk::CustomSorter::new(|a, b| {
            let (a, b) = (event_of(a).map(|e| day(e.time)), event_of(b).map(|e| day(e.time)));
            b.cmp(&a).into()
        })));
        let selection = gtk::SingleSelection::new(Some(sorted));
        selection.set_autoselect(false);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = widgets::hbox(12);
            row.add_css_class("git-activity-row");
            let image = gtk::Image::new();
            row.append(&image);
            let text = widgets::vbox(1);
            text.set_hexpand(true);
            let title = widgets::label("", "git-activity-title");
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let detail = widgets::label("", "git-activity-detail");
            detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            text.append(&title);
            text.append(&detail);
            row.append(&text);
            let source = widgets::label("", "git-pill");
            source.set_valign(gtk::Align::Center);
            row.append(&source);
            let time = widgets::label("", "git-activity-time");
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
            tag.set_text(if e.source == Source::Journal { "Nyx Git" } else { "Git reflog" });
            if e.source == Source::Journal {
                tag.add_css_class("acc");
            } else {
                tag.remove_css_class("acc");
            }
            let time = tag.next_sibling().and_downcast::<gtk::Label>().unwrap();
            time.set_text(&glib::DateTime::from_unix_local(e.time).and_then(|d| d.format("%H:%M")).map(|s| s.to_string()).unwrap_or_default());
            time.set_tooltip_text(Some(&format!("{} · {}", when(e.time), super::graph_renderer::ago(e.time))));
        });
        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        let headers = gtk::SignalListItemFactory::new();
        headers.connect_setup(|_, h| {
            let h = h.downcast_ref::<gtk::ListHeader>().unwrap();
            h.set_child(Some(&widgets::label("", "git-day-head")));
        });
        headers.connect_bind(|_, h| {
            let h = h.downcast_ref::<gtk::ListHeader>().unwrap();
            if let (Some(e), Some(label)) = (h.item().and_then(|o| event_of(&o)), h.child().and_downcast::<gtk::Label>()) {
                label.set_text(&day_title(e.time).to_uppercase());
            }
        });
        list.set_header_factory(Some(&headers));
        list.add_css_class("git-files");
        list.add_css_class("git-activity");
        list.update_property(&[gtk::accessible::Property::Label("Repository activity")]);
        let scroll = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(super::scrollbars::policy())
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .child(&list)
            .build();
        let inspector = super::inspector::Inspector::new();
        let inspector_scroll = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(super::scrollbars::policy())
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(260)
            .child(&inspector.root)
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
            paned: paned.clone(),
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
    /// Narrow windows put the inspector under the list.
    pub fn set_narrow(&self, narrow: bool) {
        let o = if narrow { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal };
        if self.paned.orientation() != o {
            self.paned.set_orientation(o);
        }
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
        let insp = &self.inspector;
        insp.clear();
        let Some(e) = e else {
            insp.note("Select an event to see what changed and what can be recovered.");
            return;
        };
        insp.title(Some(icon(e.kind)), &e.title);
        insp.sub(&format!("{} · {}", when(e.time), e.kind.title()));
        let detail = insp.text(&e.detail);
        if !e.ok {
            detail.add_css_class("bad");
        }
        insp.note(match e.source {
            Source::Journal => "Recorded by Nyx Git when it ran this, with every reference it moved.",
            Source::Reflog => "From Git's reflog: HEAD moved here. Any Git tool may have done it; Nyx Git knows only what the reflog says.",
        });
        if !e.changes.is_empty() {
            insp.group("References");
            for c in &e.changes {
                let s = |v: &Option<String>| v.as_deref().map_or("none".to_string(), |v| v[..v.len().min(7)].to_string());
                let value = insp.kv(&activity::short(&c.name), &format!("{} → {}", s(&c.before), s(&c.after)), "");
                value.add_css_class("git-hash");
            }
        }
        let Some(root) = current().and_then(|ui| ui.root()) else { return };
        insp.group("Recovery");
        insp.note("A branch at a commit keeps it; nothing else changes. Uncommitted work that was lost isn't in Git and can't be brought back here.");
        let actions = insp.actions();
        let mut offered = false;
        let mut offer = |label: &str, id: &str| {
            let b = super::inspector::action("git-branch-symbolic", label, super::inspector::Style::Plain);
            b.set_tooltip_text(Some(id));
            let (r, id) = (root.clone(), id.to_string());
            b.connect_clicked(move |_| super::advanced::branch_at(r.clone(), id.clone()));
            actions.append(&b);
            offered = true;
        };
        if let Some(before) = e.before.as_deref() {
            offer(&format!("Branch at the earlier commit {}…", &before[..before.len().min(7)]), before);
        }
        for c in e.changes.iter().filter(|c| c.name != "HEAD" && c.after.is_none()) {
            if let Some(b) = c.before.as_deref().filter(|b| Some(*b) != e.before.as_deref()) {
                offer(&format!("Branch at deleted {}…", activity::short(&c.name)), b);
            }
        }
        if let Some(after) = e.after.as_deref() {
            let copy = super::inspector::action("git-copy-symbolic", "Copy commit hash", super::inspector::Style::Plain);
            let id = after.to_string();
            copy.connect_clicked(move |b| b.display().clipboard().set_text(&id));
            actions.append(&copy);
            let show = super::inspector::action("git-history-symbolic", "Show in History", super::inspector::Style::Plain);
            let id = after.to_string();
            show.connect_clicked(move |_| {
                if let Some(ui) = current() {
                    ui.show_view("history");
                    ui.graph.show_reference(id.clone());
                }
            });
            actions.append(&show);
        }
        if !offered {
            insp.note("Nothing to recover from this event.");
        }
    }
}
