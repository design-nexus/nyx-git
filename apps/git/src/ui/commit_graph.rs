use super::{
    Item,
    commit_details::CommitDetails,
    graph_renderer::{self, row_height},
    item,
    window::current,
};
use crate::prefs;
use gtk::{gdk, glib, prelude::*};
use nyx_git::{
    core::history::{Event, Request, Scope, Session},
    graph::{
        model::{Decoration, Query, Row},
        virtualization::{MAX_CACHE, PAGE_ROWS, visible_range},
    },
};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

#[derive(Default)]
struct State {
    rows: Vec<Row>,
    start: usize,
    scanned: usize,
    done: bool,
    lanes: usize,
    selected: Option<usize>,
    selected_id: Option<String>,
    hover: Option<usize>,
    query: Query,
    comparison: Option<String>,
    ready: bool,
}
impl State {
    fn row(&self, index: usize) -> Option<&Row> {
        index.checked_sub(self.start).and_then(|i| self.rows.get(i))
    }
}
/// Where the reader was before a reload: the selected commit and how far down the view it sat.
struct Keep {
    id: String,
    index: usize,
    y: f64,
}
/// Rows past a commit's old position to search for it after a reload.
const KEEP_WITHIN: usize = 2048;
pub struct CommitGraph {
    pub root: gtk::Box,
    pub details: CommitDetails,
    canvas: gtk::DrawingArea,
    adjustment: gtk::Adjustment,
    state: Rc<RefCell<State>>,
    session: RefCell<Option<Session>>,
    repository: RefCell<Option<PathBuf>>,
    head: RefCell<Option<String>>,
    scope: RefCell<Scope>,
    scopes: RefCell<Vec<Scope>>,
    scope_picker: gtk::DropDown,
    syncing: Cell<bool>,
    pending: Cell<bool>,
    stopped: Cell<bool>,
    dirty: Cell<bool>,
    polling: Cell<bool>,
    keep: RefCell<Option<Keep>>,
    locating: Cell<bool>,
    /// A search or a jump to the oldest commit is running; Stop is offered.
    long: Cell<bool>,
    search_query: RefCell<Option<String>>,
    pub search: gtk::SearchEntry,
    stop: gtk::Button,
    readout: gtk::Label,
    zoom: Cell<f64>,
    timeline: Cell<bool>,
    first_parent: Cell<bool>,
}
fn graph(run: impl FnOnce(&CommitGraph)) {
    if let Some(ui) = current() {
        run(&ui.graph);
    }
}
fn action(icon: &str, tip: &str, run: impl Fn() + 'static) -> gtk::Button {
    let button = widgets::bar_button(icon, tip);
    button.connect_clicked(move |_| run());
    button
}
impl CommitGraph {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toolbar.add_css_class("git-history-search");
        let scope_picker = gtk::DropDown::from_strings(&["All branches", "Current branch"]);
        super::compact_dropdown(&scope_picker);
        scope_picker.set_expression(Some(gtk::PropertyExpression::new(
            gtk::StringObject::static_type(),
            None::<gtk::Expression>,
            "string",
        )));
        scope_picker.set_enable_search(true);
        scope_picker.set_search_match_mode(gtk::StringFilterMatchMode::Substring);
        scope_picker.set_tooltip_text(Some("Show the history of all branches, the current branch, or one branch or tag"));
        scope_picker.update_property(&[gtk::accessible::Property::Label("Commit ancestry scope")]);
        toolbar.append(&scope_picker);
        let search = gtk::SearchEntry::new();
        search.set_hexpand(true);
        search.update_property(&[gtk::accessible::Property::Label("Search commits")]);
        search.set_width_chars(12);
        search.set_placeholder_text(Some("Search commits · author: · after:YYYY-MM-DD · before:"));
        search.set_tooltip_text(Some("Find messages, hashes, authors or refs. Use author:name after:2026-01-01 before:2026-12-31 to narrow matches. The complete graph stays visible."));
        toolbar.append(&search);
        toolbar.append(&action("go-up-symbolic", "Previous match (Shift+Enter)", || graph(|g| g.find(true))));
        toolbar.append(&action("go-down-symbolic", "Next match (Enter)", || graph(|g| g.find(false))));
        let stop = action("process-stop-symbolic", "Stop loading history or searching", || graph(|g| g.cancel()));
        stop.set_visible(false);
        toolbar.append(&stop);
        let collapse = gtk::ToggleButton::new();
        collapse.set_icon_name("view-conceal-symbolic");
        collapse.add_css_class("flat");
        collapse.set_tooltip_text(Some("Collapse merged branches: follow first parents only, so each merge stands for the branch it brought in"));
        collapse.set_active(prefs::get().first_parent);
        collapse.connect_toggled(|b| {
            prefs::update(|p| p.first_parent = b.is_active());
            graph(|g| {
                g.first_parent.set(b.is_active());
                g.reset(true);
            });
        });
        toolbar.append(&collapse);
        let timeline = gtk::ToggleButton::new();
        timeline.set_icon_name("view-continuous-symbolic");
        timeline.add_css_class("flat");
        timeline.set_tooltip_text(Some("Timeline: commits left to right, newest first, in their branch lanes"));
        timeline.set_active(prefs::get().timeline);
        timeline.connect_toggled(|b| {
            prefs::update(|p| p.timeline = b.is_active());
            graph(|g| g.set_timeline(b.is_active()));
        });
        toolbar.append(&timeline);
        root.append(&toolbar);
        let canvas = gtk::DrawingArea::new();
        canvas.set_hexpand(true);
        canvas.set_vexpand(true);
        canvas.set_focusable(true);
        canvas.set_accessible_role(gtk::AccessibleRole::Group);
        canvas.update_property(&[
            gtk::accessible::Property::Label("Commit history"),
            gtk::accessible::Property::Description("Use Up and Down to select commits, Home and End for newest and oldest, Shift+F10 for commit actions, and Enter for commit details."),
        ]);
        canvas.set_has_tooltip(true);
        canvas.add_css_class("git-graph");
        let adjustment = gtk::Adjustment::new(0.0, 0.0, 400.0, 36.0, 400.0, 400.0);
        let scrollbar = gtk::Scrollbar::new(gtk::Orientation::Vertical, Some(&adjustment));
        scrollbar.add_css_class("git-graph-scrollbar");
        scrollbar.set_visible(prefs::get().show_scrollbars);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.set_vexpand(true);
        body.append(&canvas);
        body.append(&scrollbar);
        root.append(&body);
        let readout = widgets::label("Open a repository to view commit history.", "git-history-readout");
        readout.set_ellipsize(gtk::pango::EllipsizeMode::End);
        root.append(&readout);
        let raw = prefs::get().graph_zoom;
        let zoom = if raw.is_finite() { raw.clamp(0.7, 1.6) } else { 1.0 };
        let me = Self {
            root,
            details: CommitDetails::new(),
            canvas,
            adjustment,
            state: Rc::new(RefCell::new(State::default())),
            session: RefCell::new(None),
            repository: RefCell::new(None),
            head: RefCell::new(None),
            scope: RefCell::new(Scope::All),
            scopes: RefCell::new(vec![Scope::All, Scope::Head]),
            scope_picker,
            syncing: Cell::new(false),
            pending: Cell::new(false),
            stopped: Cell::new(false),
            dirty: Cell::new(false),
            polling: Cell::new(false),
            keep: RefCell::new(None),
            locating: Cell::new(false),
            long: Cell::new(false),
            search_query: RefCell::new(None),
            search,
            stop,
            readout,
            zoom: Cell::new(zoom),
            timeline: Cell::new(prefs::get().timeline),
            first_parent: Cell::new(prefs::get().first_parent),
        };
        me.wire();
        me
    }
    /// The commit under a point: along y in the graph, along x in the timeline.
    fn index_at(&self, x: f64, y: f64) -> usize {
        let along = if self.timeline.get() { x } else { y };
        ((self.adjustment.value() + along) / self.step()).floor().max(0.0) as usize
    }
    /// Pixels from one commit to the next.
    fn step(&self) -> f64 {
        if self.timeline.get() { graph_renderer::timeline_column(self.zoom.get()) } else { row_height(self.zoom.get()) }
    }
    /// The canvas's length in the direction commits run.
    fn extent(&self) -> f64 {
        (if self.timeline.get() { self.canvas.width() } else { self.canvas.height() }).max(1) as f64
    }
    fn set_timeline(&self, on: bool) {
        let first = (self.adjustment.value() / self.step()).floor();
        self.timeline.set(on);
        self.update_adjustment();
        self.adjustment.set_value((first * self.step()).clamp(0.0, (self.adjustment.upper() - self.adjustment.page_size()).max(0.0)));
        self.canvas.queue_draw();
        self.poll();
    }
    fn wire(&self) {
        let state = self.state.clone();
        let adjustment = self.adjustment.clone();
        self.canvas.set_draw_func(move |area, cr, w, h| {
            let state = state.borrow();
            let zoom = current().map_or(1.0, |ui| ui.graph.zoom.get());
            let timeline = current().is_some_and(|ui| ui.graph.timeline.get());
            let draw = if timeline { graph_renderer::draw_timeline } else { graph_renderer::draw };
            draw(
                area,
                cr,
                w as f64,
                h as f64,
                graph_renderer::Frame {
                    rows: &state.rows,
                    start: state.start,
                    offset: adjustment.value(),
                    zoom,
                    selected: state.selected,
                    hover: state.hover,
                    lanes: state.lanes.max(1),
                    query: &state.query,
                    comparison: state.comparison.as_deref(),
                },
            );
        });
        let canvas = self.canvas.clone();
        self.adjustment.connect_value_changed(move |_| {
            canvas.queue_draw();
            graph(|g| g.poll());
        });
        self.canvas.connect_resize(|_, _, _| {
            graph(|g| {
                g.update_adjustment();
                g.poll();
            })
        });
        self.root.connect_map(|_| graph(|g| g.poll()));
        self.scope_picker.connect_selected_notify(|picker| {
            graph(|g| {
                if g.syncing.get() {
                    return;
                }
                let scope = g.scopes.borrow().get(picker.selected() as usize).cloned();
                if let Some(scope) = scope {
                    *g.scope.borrow_mut() = scope;
                    g.reset(true);
                }
            })
        });
        self.search.connect_search_changed(|entry| {
            graph(|g| match Query::parse(&entry.text()) {
                Ok(query) => {
                    entry.remove_css_class("error");
                    g.state.borrow_mut().query = query;
                    g.canvas.queue_draw();
                    g.update_readout();
                }
                Err(e) => {
                    entry.add_css_class("error");
                    g.readout.set_text(&format!("{e:#}"));
                }
            })
        });
        self.search.connect_activate(|_| graph(|g| g.find(false)));
        self.search.connect_stop_search(|entry| entry.set_text(""));
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(|_, key, _, mods| {
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            match key {
                gdk::Key::Up | gdk::Key::Left => graph(|g| g.move_selection(-1)),
                gdk::Key::Down | gdk::Key::Right => graph(|g| g.move_selection(1)),
                gdk::Key::Page_Up => graph(|g| g.move_selection(-((g.extent() / g.step()) as isize).max(1))),
                gdk::Key::Page_Down => graph(|g| g.move_selection(((g.extent() / g.step()) as isize).max(1))),
                gdk::Key::Home => graph(|g| g.select(0)),
                gdk::Key::Menu => graph(|g| g.selected_menu()),
                gdk::Key::F10 if mods.contains(gdk::ModifierType::SHIFT_MASK) => graph(|g| g.selected_menu()),
                gdk::Key::Return | gdk::Key::KP_Enter => graph(|g| {
                    g.details.focus();
                }),
                gdk::Key::End => graph(|g| {
                    if g.send(Request::End) {
                        g.long.set(true);
                        g.readout.set_text("Finding oldest commit…");
                    }
                }),
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add if ctrl => graph(|g| g.set_zoom(g.zoom.get() + 0.1)),
                gdk::Key::minus | gdk::Key::KP_Subtract if ctrl => graph(|g| g.set_zoom(g.zoom.get() - 0.1)),
                gdk::Key::_0 | gdk::Key::KP_0 if ctrl => graph(|g| g.set_zoom(1.0)),
                gdk::Key::c | gdk::Key::C if ctrl => graph(|g| {
                    if let Some(id) = &g.state.borrow().selected_id {
                        g.canvas.display().clipboard().set_text(id);
                    }
                }),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.canvas.add_controller(keys);
        let searchkeys = gtk::EventControllerKey::new();
        searchkeys.set_propagation_phase(gtk::PropagationPhase::Capture);
        searchkeys.connect_key_pressed(|_, key, _, mods| {
            if key == gdk::Key::Return && mods.contains(gdk::ModifierType::SHIFT_MASK) {
                graph(|g| g.find(true));
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.search.add_controller(searchkeys);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.connect_scroll(|controller, dx, dy| {
            graph(|g| {
                // A sideways swipe moves along the timeline too.
                let dy = if g.timeline.get() && dy == 0.0 { dx } else { dy };
                if controller.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                    g.set_zoom(g.zoom.get() - dy.signum() * 0.1);
                    return;
                }
                let amount = if controller.unit() == gdk::ScrollUnit::Surface { dy } else { dy * g.step() * 2.0 };
                g.adjustment.set_value(
                    (g.adjustment.value() + amount).clamp(0.0, (g.adjustment.upper() - g.adjustment.page_size()).max(0.0)),
                );
            });
            glib::Propagation::Stop
        });
        self.canvas.add_controller(scroll);
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.connect_pressed(|gesture, _, x, y| {
            graph(|g| {
                g.canvas.grab_focus();
                let index = g.index_at(x, y);
                // Ctrl+click compares the selected commit with this one, keeping the selection.
                if gesture.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) && gesture.current_button() == 1 {
                    g.compare_with(index);
                    return;
                }
                g.select(index);
                if gesture.current_button() == 3 {
                    g.context_menu(index, x, y);
                }
            })
        });
        self.canvas.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(|_, x, y| {
            graph(|g| {
                let index = g.index_at(x, y);
                let mut state = g.state.borrow_mut();
                if state.hover != Some(index) {
                    state.hover = Some(index);
                    g.canvas.queue_draw();
                }
            })
        });
        motion.connect_leave(|_| {
            graph(|g| {
                g.state.borrow_mut().hover = None;
                g.canvas.queue_draw();
            })
        });
        self.canvas.add_controller(motion);
        let state = self.state.clone();
        let adjustment = self.adjustment.clone();
        self.canvas.connect_query_tooltip(move |_, x, y, keyboard, tooltip| {
            let zoom = current().map_or(1.0, |ui| ui.graph.zoom.get());
            let timeline = current().is_some_and(|ui| ui.graph.timeline.get());
            let (along, step) = if timeline { (x, graph_renderer::timeline_column(zoom)) } else { (y, row_height(zoom)) };
            let state = state.borrow();
            let index = if keyboard {
                state.selected.unwrap_or(0)
            } else {
                ((adjustment.value() + along as f64) / step).max(0.0) as usize
            };
            let Some(row) = state.row(index) else { return false };
            let refs = row.commit.refs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(" · ");
            tooltip.set_text(Some(&format!(
                "{}\n{}\n{} <{}> · {}{}{}\nRight-click for actions · Ctrl+click to compare",
                &row.commit.id[..10],
                row.commit.subject,
                row.commit.author,
                row.commit.email,
                graph_renderer::relative_time(row.commit.timestamp),
                if refs.is_empty() { String::new() } else { format!("\n{refs}") },
                if row.commit.shallow {
                    "\nShallow history boundary".to_string()
                } else if row.commit.collapsed > 0 {
                    format!("\nMerge: {} merged parent{} collapsed", row.commit.collapsed, if row.commit.collapsed == 1 { "" } else { "s" })
                } else {
                    String::new()
                },
            )));
            true
        });
    }
    /// The graph polls its history worker only while it waits for something.
    pub fn install_poll(&self) {
        self.poll();
    }
    fn poll(&self) {
        // The timer reaches the graph through the window, which exists once it's built.
        if current().is_none() || self.polling.replace(true) {
            return;
        }
        glib::timeout_add_local(Duration::from_millis(33), move || {
            let mut more = false;
            graph(|g| {
                g.tick();
                more = g.waiting();
                if !more {
                    g.polling.set(false);
                }
            });
            if more { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
        });
    }
    /// Whether an answer from the worker is still due.
    fn waiting(&self) -> bool {
        let active = self.root.is_mapped() && self.repository.borrow().is_some() && !self.stopped.get();
        active && (self.session.borrow().is_none() || self.pending.get() || !self.state.borrow().ready)
    }
    pub fn open(&self, root: PathBuf, head: Option<String>) {
        let changed = self.repository.borrow().as_ref() != Some(&root);
        let moved = *self.head.borrow() != head;
        if changed || moved || self.dirty.replace(false) {
            // Keep the reader's place, unless they were looking at the tip that just moved.
            let at_tip =
                moved && self.state.borrow().selected_id.is_some() && self.state.borrow().selected_id == *self.head.borrow();
            *self.repository.borrow_mut() = Some(root);
            *self.head.borrow_mut() = head;
            if changed {
                *self.scope.borrow_mut() = Scope::All;
            }
            self.reset(!changed && !at_tip);
        }
    }
    pub fn zoom(&self) -> f64 {
        self.zoom.get()
    }
    pub fn restore_zoom(&self, value: f64) {
        self.zoom.set(if value.is_finite() { value.clamp(0.7, 1.6) } else { 1. });
        self.update_adjustment();
        self.redraw();
    }
    pub fn close(&self) {
        self.repository.borrow_mut().take();
        self.head.borrow_mut().take();
        self.reset(false);
    }
    /// Read the history again (refs changed), keeping the selected commit and the scroll position.
    pub fn invalidate(&self) {
        self.dirty.set(false);
        self.reset(true);
    }
    fn reset(&self, keep: bool) {
        let keep = keep
            .then(|| {
                let s = self.state.borrow();
                let index = s.selected?;
                let y = index as f64 * self.step() - self.adjustment.value();
                Some(Keep { id: s.selected_id.clone()?, index, y })
            })
            .flatten();
        let comparison = if keep.is_some() { self.state.borrow().comparison.clone() } else { None };
        self.session.borrow_mut().take();
        self.pending.set(false);
        self.stopped.set(false);
        self.locating.set(false);
        self.long.set(false);
        let query = Query::parse(&self.search.text()).unwrap_or_default();
        *self.state.borrow_mut() =
            State { query, comparison, selected_id: keep.as_ref().map(|k| k.id.clone()), ..Default::default() };
        if keep.is_none() {
            self.details.clear();
            self.canvas.update_property(&[gtk::accessible::Property::Label("Commit history")]);
            self.adjustment.set_value(0.0);
        }
        *self.keep.borrow_mut() = keep;
        self.update_adjustment();
        self.canvas.queue_draw();
        if self.repository.borrow().is_none() {
            self.readout.set_text("Open a repository to view commit history.");
        } else if self.keep.borrow().is_none() {
            self.readout.set_text("Loading commit history…");
        }
        self.poll();
    }
    pub fn cancel(&self) {
        self.session.borrow_mut().take();
        self.pending.set(false);
        self.stopped.set(true);
        self.locating.set(false);
        self.long.set(false);
        self.stop.set_visible(false);
        self.readout.set_text("History loading stopped · Reload with F5.");
    }
    fn send(&self, request: Request) -> bool {
        if self.pending.get() {
            self.readout.set_text("History is loading… Use Stop to cancel.");
            return false;
        }
        if let Some(session) = self.session.borrow().as_ref()
            && session.request(request)
        {
            self.pending.set(true);
            self.poll();
            return true;
        }
        false
    }
    pub fn tick(&self) {
        if !self.root.is_mapped() {
            return;
        }
        if self.session.borrow().is_none()
            && !self.stopped.get()
            && let Some(root) = self.repository.borrow().clone()
        {
            *self.session.borrow_mut() = Some(Session::start_with(root, self.scope.borrow().clone(), self.first_parent.get()));
        }
        loop {
            let event = self.session.borrow().as_ref().and_then(Session::poll);
            let Some(event) = event else { break };
            match event {
                Event::Ready { refs } => {
                    self.state.borrow_mut().ready = true;
                    self.populate_scopes(refs);
                    let target = self.keep.borrow().as_ref().map(|k| (k.id.clone(), k.index + KEEP_WITHIN));
                    if let Some((id, within)) = target
                        && self.send(Request::Locate { id, within })
                    {
                        self.locating.set(true);
                    }
                }
                Event::Page { start, rows, scanned, done, lanes } => {
                    self.pending.set(false);
                    {
                        let mut state = self.state.borrow_mut();
                        state.start = start;
                        state.rows = rows;
                        state.scanned = scanned;
                        state.done = done;
                        state.lanes = lanes;
                    }
                    self.update_adjustment();
                    self.update_readout();
                    self.canvas.queue_draw();
                    let select = {
                        let state = self.state.borrow();
                        if let Some(index) = state.selected {
                            state.row(index).filter(|row| state.selected_id.as_ref() != Some(&row.commit.id)).map(|_| index)
                        } else if start == 0 && scanned > 0 {
                            Some(0)
                        } else {
                            None
                        }
                    };
                    if let Some(index) = select {
                        self.select(index);
                    }
                }
                Event::Found { index, scanned, done, lanes } => {
                    self.pending.set(false);
                    self.long.set(false);
                    {
                        let mut s = self.state.borrow_mut();
                        s.scanned = scanned;
                        s.done = done;
                        s.lanes = lanes;
                    }
                    self.update_adjustment();
                    self.update_readout();
                    if self.locating.replace(false) {
                        self.restore(index);
                        continue;
                    }
                    let current_query = self.search_query.borrow_mut().take();
                    if current_query.as_deref().is_none_or(|q| q == self.search.text().as_str()) {
                        if let Some(index) = index {
                            self.select(index);
                        } else {
                            self.readout.set_text("No further matching commits in this direction.");
                        }
                    }
                }
                Event::Error(error) => {
                    self.cancel();
                    self.readout.set_text(&error);
                    if let Some(ui) = current() {
                        ui.error(&error);
                    }
                }
            }
        }
        self.stop.set_visible(self.pending.get() && self.long.get());
        if self.pending.get() || !self.state.borrow().ready || self.stopped.get() {
            return;
        }
        let s = self.state.borrow();
        let total = s.scanned + if s.done { 0 } else { PAGE_ROWS };
        let range = visible_range(self.adjustment.value(), self.extent(), self.step(), total);
        let count = (range.end.saturating_sub(range.start)).min(MAX_CACHE);
        let missing = count > 0 && (range.start < s.start || range.end > s.start + s.rows.len());
        drop(s);
        if missing {
            self.send(Request::Page { start: range.start, count });
        }
    }
    /// Put the reader back where they were after a reload, or at the newest commit if
    /// their commit is gone.
    fn restore(&self, index: Option<usize>) {
        let Some(keep) = self.keep.borrow_mut().take() else { return };
        match index {
            Some(index) => {
                self.state.borrow_mut().selected = Some(index);
                self.update_adjustment();
                let value = index as f64 * self.step() - keep.y;
                self.adjustment.set_value(value.clamp(0.0, (self.adjustment.upper() - self.adjustment.page_size()).max(0.0)));
            }
            None => {
                let mut s = self.state.borrow_mut();
                s.selected = None;
                s.selected_id = None;
                drop(s);
                self.details.clear();
                self.adjustment.set_value(0.0);
            }
        }
        self.canvas.queue_draw();
    }
    fn populate_scopes(&self, refs: Vec<Decoration>) {
        let mut names = vec!["All branches".to_string(), "Current branch".to_string()];
        let mut scopes = vec![Scope::All, Scope::Head];
        for r in refs {
            names.push(r.full_name.trim_start_matches("refs/").trim_start_matches("heads/").to_string());
            scopes.push(Scope::Reference(r.full_name));
        }
        let selected = scopes.iter().position(|s| *s == *self.scope.borrow()).unwrap_or(0);
        *self.scopes.borrow_mut() = scopes;
        self.syncing.set(true);
        self.scope_picker.set_model(Some(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())));
        self.scope_picker.set_selected(selected as u32);
        self.syncing.set(false);
    }
    fn update_readout(&self) {
        let s = self.state.borrow();
        let loaded = if s.done && s.scanned == 0 {
            "No commits in this scope".to_string()
        } else if s.done {
            format!("{} commits", s.scanned)
        } else {
            format!("{} commits loaded · scroll for older", s.scanned)
        };
        let comparing = s.comparison.as_ref().map(|c| format!(" · comparing from {}", &c[..10])).unwrap_or_default();
        self.readout.set_text(&(loaded + &comparing));
    }
    fn update_adjustment(&self) {
        let s = self.state.borrow();
        let count = s.scanned + if s.done { 0 } else { PAGE_ROWS };
        drop(s);
        let height = self.extent();
        let h = self.step();
        let upper = (count as f64 * h).max(height);
        self.adjustment.configure(self.adjustment.value().min((upper - height).max(0.0)), 0.0, upper, h, height * 0.9, height);
    }
    fn select(&self, index: usize) {
        let mut s = self.state.borrow_mut();
        if s.done && index >= s.scanned {
            return;
        }
        s.selected = Some(index);
        let row = s.row(index).cloned();
        let new_id = row.as_ref().map(|r| r.commit.id.clone());
        let changed = new_id.is_some() && new_id != s.selected_id;
        if changed {
            s.selected_id = new_id;
        }
        drop(s);
        let y = index as f64 * self.step();
        let value = self.adjustment.value();
        let height = self.extent();
        if y < value {
            self.adjustment.set_value(y);
        } else if y + self.step() > value + height {
            self.adjustment.set_value((y - height + self.step()).max(0.0));
        }
        if changed && let Some(row) = row {
            let summary = format!(
                "Commit {}, {}: {}. By {}. {}. {}. {} parents{}",
                index + 1,
                &row.commit.id[..10],
                row.commit.subject,
                row.commit.author,
                graph_renderer::relative_time(row.commit.timestamp),
                row.commit.refs.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "),
                row.commit.parents.len(),
                if row.commit.shallow { ". Shallow history boundary" } else { "" }
            );
            self.canvas.update_property(&[gtk::accessible::Property::Label(&format!("Commit history. {summary}"))]);
            if self.canvas.has_focus() {
                self.canvas.announce(&summary, gtk::AccessibleAnnouncementPriority::Low);
            }
            self.details.select(row.commit.id);
        }
        self.canvas.queue_draw();
    }
    fn move_selection(&self, delta: isize) {
        let selected = self.state.borrow().selected.unwrap_or(0);
        self.select(selected.saturating_add_signed(delta));
    }
    pub fn redraw(&self) {
        self.canvas.queue_draw();
    }
    pub fn focus_search(&self) {
        self.search.grab_focus();
    }
    pub fn show_reference(&self, reference: String) {
        *self.scope.borrow_mut() = Scope::Reference(reference);
        self.reset(false);
        self.focus();
    }
    pub fn focus(&self) -> bool {
        self.canvas.grab_focus()
    }
    fn selected_menu(&self) {
        let Some(index) = self.state.borrow().selected else { return };
        let along = ((index as f64 + 0.5) * self.step() - self.adjustment.value()).clamp(0., self.extent());
        if self.timeline.get() {
            self.context_menu(index, along, 24.);
        } else {
            self.context_menu(index, 24., along);
        }
    }
    pub fn find(&self, backwards: bool) {
        let query = match Query::parse(&self.search.text()) {
            Ok(query) if !query.is_empty() => query,
            Ok(_) => {
                self.readout.set_text("Enter text or author:/after:/before: filters to find commits.");
                return;
            }
            Err(e) => {
                self.readout.set_text(&format!("{e:#}"));
                return;
            }
        };
        self.state.borrow_mut().query = query.clone();
        let selected = self.state.borrow().selected;
        let from = if backwards { selected.unwrap_or(0) } else { selected.map_or(0, |i| i + 1) };
        if self.send(Request::Find { query, from, backwards }) {
            self.long.set(true);
            *self.search_query.borrow_mut() = Some(self.search.text().to_string());
            self.readout.set_text("Searching history… Use Stop to cancel.");
        }
    }
    fn set_zoom(&self, zoom: f64) {
        let offset = self.adjustment.value() / self.step();
        let zoom = zoom.clamp(0.7, 1.6);
        self.zoom.set(zoom);
        prefs::update(|p| p.graph_zoom = zoom);
        if let Some(ui) = current() {
            ui.save_repository();
        }
        self.update_adjustment();
        self.adjustment.set_value(offset * self.step());
        self.canvas.queue_draw();
    }
    /// Compare the selected commit with the one at `index`, older first.
    fn compare_with(&self, index: usize) {
        let s = self.state.borrow();
        let (Some(selected), Some(row)) = (s.selected, s.row(index)) else { return };
        let Some(selected_id) = s.selected_id.clone() else { return };
        let other = row.commit.id.clone();
        drop(s);
        if other == selected_id {
            return;
        }
        let (base, tip) = if index > selected { (other, selected_id) } else { (selected_id, other) };
        self.details.compare(base, tip);
    }
    fn context_menu(&self, index: usize, x: f64, y: f64) {
        let Some(row) = self.state.borrow().row(index).cloned() else { return };
        let id = row.commit.id.clone();
        let head = self.head.borrow().clone();
        let is_head = head.as_deref() == Some(id.as_str());
        let target = |kind: &'static str| {
            let id = id.clone();
            move || super::advanced::target_action(kind, id.clone())
        };
        let copy = id.clone();
        let mut items = vec![
            item("Copy commit hash", move || {
                graph(|g| g.canvas.display().clipboard().set_text(&copy));
            }),
            Item::Separator,
            item("Check out this commit…", target("checkout")),
            item("Create branch here…", target("branch")),
            item("Create tag here…", target("tag")),
            Item::Separator,
        ];
        if !is_head {
            items.push(item("Merge into current branch…", target("merge")));
            items.push(item("Rebase current branch onto this…", target("rebase")));
            items.push(item("Cherry-pick…", target("cherry")));
        }
        items.push(item("Revert…", target("revert")));
        if !is_head {
            items.push(item("Reset to here, keeping changes…", target("mixed")));
            items.push(item("Reset to here, discarding changes…", target("hard")));
        }
        items.push(Item::Separator);
        let selected = self.state.borrow().selected_id.clone();
        if selected.as_ref().is_some_and(|s| *s != id) {
            items.push(item("Compare with selected commit", move || graph(|g| g.compare_with(index))));
        }
        let base = id.clone();
        items.push(item("Mark for comparison", move || {
            graph(|g| {
                g.state.borrow_mut().comparison = Some(base.clone());
                g.update_readout();
                g.canvas.queue_draw();
            })
        }));
        if let Some(base) = self.state.borrow().comparison.clone().filter(|b| *b != id) {
            let tip = id.clone();
            items.push(item("Compare marked commit → this", move || {
                graph(|g| {
                    g.details.compare(base.clone(), tip.clone());
                    g.state.borrow_mut().comparison = None;
                    g.update_readout();
                    g.canvas.queue_draw();
                })
            }));
        }
        items.push(Item::Separator);
        items.push(item("Interactive rebase after this…", target("interactive")));
        items.push(item("More Git actions…", move || super::advanced::show(Some(id.clone()))));
        super::popup_at(&self.canvas, items, x, y);
    }
}
