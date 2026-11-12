//! Flow: the repository's work in progress on a canvas. Cards are branches (and worktrees on
//! no branch); lines say how they're related, each kind drawn its own way:
//!
//! - solid: Git ancestry, a branch started from the one to its left;
//! - a card's lower strip: the worktree it's checked out in, with its changes;
//! - chips: Nyx Agents sessions working in that worktree;
//! - dashed: a pull or merge request into another branch;
//! - dotted, amber: derived, two lines of work touching the same files;
//! - dotted, accent, with an arrow: an action being considered, before it's taken.
//!
//! Placement is a deterministic tree; cards the reader drags stay where they're put
//! (kept apart from Git, per repository) until Auto layout.
use super::window::current;
use gtk::{cairo, gdk, glib, pango, prelude::*};
use nyx_git::core::{
    agents,
    flow::{self, Branch, CARD_H, CARD_W, Flow, Place},
};

/// A pull or merge request on a branch, as Review last read it.
#[derive(Clone, Debug)]
pub struct Link {
    pub number: u64,
    pub title: String,
    pub source: String,
    pub target: String,
    /// The provider's state with its checks, in words.
    pub state: String,
    pub url: String,
}
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
struct Camera {
    x: f64,
    y: f64,
    zoom: f64,
}
impl Camera {
    fn world(&self, sx: f64, sy: f64) -> (f64, f64) {
        (sx / self.zoom + self.x, sy / self.zoom + self.y)
    }
}
enum Drag {
    Pan { camera: Camera },
    Card { id: String, from: Place },
}
#[derive(Default)]
struct State {
    flow: Option<Flow>,
    nodes: Vec<(String, Option<String>)>,
    auto: BTreeMap<String, Place>,
    manual: BTreeMap<String, Place>,
    selected: Option<String>,
    hover: Option<String>,
    /// An action being considered: (from, to, label).
    proposal: Option<(String, String, String)>,
    query: String,
    /// Pull and merge requests by source branch, once Review has read them.
    links: Vec<Link>,
}
impl State {
    fn place(&self, id: &str) -> Option<Place> {
        self.manual.get(id).or_else(|| self.auto.get(id)).copied()
    }
    fn branch(&self, id: &str) -> Option<&Branch> {
        let name = id.strip_prefix("branch:")?;
        self.flow.as_ref()?.branches.iter().find(|b| b.name == name)
    }
    fn worktree(&self, id: &str) -> Option<&flow::Worktree> {
        let flow = self.flow.as_ref()?;
        if let Some(name) = id.strip_prefix("branch:") {
            return flow.worktrees.iter().find(|w| w.branch.as_deref() == Some(name));
        }
        let path = id.strip_prefix("worktree:")?;
        flow.worktrees.iter().find(|w| w.path.to_string_lossy() == path)
    }
    fn agents(&self, id: &str) -> Vec<&agents::Session> {
        let Some(w) = self.worktree(id) else { return Vec::new() };
        let Some(flow) = self.flow.as_ref() else { return Vec::new() };
        flow.agents.iter().filter(|(_, p)| p.as_ref() == Some(&w.path)).map(|(s, _)| s).collect()
    }
    fn title(&self, id: &str) -> String {
        if let Some(name) = id.strip_prefix("branch:") {
            return name.into();
        }
        self.worktree(id).map(|w| w.path.file_name().unwrap_or_default().to_string_lossy().into_owned()).unwrap_or_default()
    }
    fn matches(&self, id: &str) -> bool {
        self.query.is_empty() || self.title(id).to_lowercase().contains(&self.query)
    }
    fn hit(&self, wx: f64, wy: f64) -> Option<String> {
        self.nodes.iter().rev().map(|(id, _)| id).find(|id| {
            self.place(id).is_some_and(|p| wx >= p.x && wx <= p.x + CARD_W && wy >= p.y && wy <= p.y + CARD_H)
        }).cloned()
    }
}
pub struct FlowPage {
    pub root: gtk::Box,
    canvas: gtk::DrawingArea,
    list: gtk::ListBox,
    inspector: gtk::Box,
    readout: gtk::Label,
    inactive: gtk::ToggleButton,
    state: Rc<RefCell<State>>,
    camera: Rc<Cell<Camera>>,
    drag: RefCell<Option<Drag>>,
    loaded: RefCell<Option<PathBuf>>,
    common: RefCell<Option<PathBuf>>,
    generation: Cell<u64>,
    pending: Cell<bool>,
    last: Cell<Option<Instant>>,
    fitted: Cell<bool>,
    /// The reader panned or zoomed; the view stops fitting itself to the canvas.
    moved: Cell<bool>,
    monitor: RefCell<Option<gtk::gio::FileMonitor>>,
}
fn page(run: impl FnOnce(&FlowPage)) {
    if let Some(ui) = current() {
        run(&ui.destinations.flow);
    }
}
fn rgba(c: &str) -> gdk::RGBA {
    gdk::RGBA::parse(c).unwrap_or(gdk::RGBA::BLACK)
}
fn mix(a: &gdk::RGBA, b: &gdk::RGBA, t: f32) -> gdk::RGBA {
    gdk::RGBA::new(a.red() * (1.0 - t) + b.red() * t, a.green() * (1.0 - t) + b.green() * t, a.blue() * (1.0 - t) + b.blue() * t, 1.0)
}
fn set(cr: &cairo::Context, c: &gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, alpha);
}
fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}
/// `at` is (x, y, width); `font` is (size, bold).
fn text(area: &gtk::DrawingArea, cr: &cairo::Context, value: &str, at: (f64, f64, f64), font: (f64, bool), c: &gdk::RGBA, alpha: f64) {
    let ((x, y, width), (size, bold)) = (at, font);
    if width < 8.0 {
        return;
    }
    let layout = area.create_pango_layout(Some(value));
    let mut font = area.pango_context().font_description().unwrap_or_default();
    font.set_absolute_size(size * pango::SCALE as f64);
    if bold {
        font.set_weight(pango::Weight::Semibold);
    }
    layout.set_font_description(Some(&font));
    layout.set_width((width * pango::SCALE as f64) as i32);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_single_paragraph_mode(true);
    set(cr, c, alpha);
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, &layout);
}
fn divergence(b: &Branch, trunk: &str) -> String {
    if b.trunk {
        return "trunk".into();
    }
    match b.divergence {
        Some((0, _)) => format!("merged into {trunk}"),
        Some((a, 0)) => format!("↑{a} ahead of {}", b.parent.as_deref().unwrap_or(trunk)),
        Some((a, behind)) => format!("↑{a} ↓{behind} vs {trunk}"),
        None => "not compared (inactive)".into(),
    }
}
impl FlowPage {
    pub fn new() -> Rc<Self> {
        let root = widgets::vbox(0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        let toolbar = widgets::hbox(6);
        toolbar.add_css_class("git-history-search");
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Find a branch or worktree"));
        search.set_width_chars(18);
        toolbar.append(&search);
        let inactive = gtk::ToggleButton::with_label("Inactive");
        inactive.add_css_class("flat");
        inactive.set_tooltip_text(Some("Also show merged branches and branches untouched for 30 days"));
        toolbar.append(&inactive);
        let legend = widgets::label("— ancestry  ┄ request  ··· shared files (derived)  ⇢ considered", "dim-label");
        legend.add_css_class("git-palette-detail");
        legend.set_hexpand(true);
        legend.set_ellipsize(pango::EllipsizeMode::End);
        legend.set_tooltip_text(Some(
            "Solid lines are Git ancestry: a branch started from the one to its left. A card's lower strip is the worktree it's checked out in. Chips are Nyx Agents sessions working there. Dashed lines are pull or merge requests. Dotted amber lines are derived: two lines of work changed the same files. A dotted accent arrow shows an action before you take it. Nearness on the canvas means nothing.",
        ));
        toolbar.append(&legend);
        let show_list = gtk::ToggleButton::new();
        show_list.set_icon_name("view-list-symbolic");
        show_list.add_css_class("flat");
        show_list.set_tooltip_text(Some("List the cards beside the canvas"));
        show_list.set_active(crate::prefs::get().flow_list);
        toolbar.append(&show_list);
        let new_branch = widgets::bar_button("list-add-symbolic", "New branch from HEAD…");
        new_branch.connect_clicked(|_| super::advanced::target_action("branch", "HEAD".into()));
        toolbar.append(&new_branch);
        let new_worktree = widgets::bar_button("folder-new-symbolic", "New worktree…");
        new_worktree.connect_clicked(|_| {
            if let Some(root) = current().and_then(|ui| ui.root()) {
                super::worktree_manager::create_for(root, None, None);
            }
        });
        toolbar.append(&new_worktree);
        let auto = widgets::bar_button("view-grid-symbolic", "Auto layout: put moved cards back");
        auto.connect_clicked(|_| page(|p| p.auto_layout()));
        toolbar.append(&auto);
        let fit = widgets::bar_button("zoom-fit-best-symbolic", "Fit everything (0)");
        fit.connect_clicked(|_| page(|p| {
            p.moved.set(false);
            p.fit();
        }));
        toolbar.append(&fit);
        let refresh = widgets::bar_button("view-refresh-symbolic", "Read the work in progress again");
        refresh.connect_clicked(|_| page(|p| p.load(true)));
        toolbar.append(&refresh);
        root.append(&toolbar);

        let list = gtk::ListBox::new();
        list.add_css_class("git-palette");
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.update_property(&[gtk::accessible::Property::Label("Branches and worktrees in Flow")]);
        let list_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .min_content_width(180)
            .child(&list)
            .build();
        let canvas = gtk::DrawingArea::new();
        canvas.set_hexpand(true);
        canvas.set_vexpand(true);
        canvas.set_focusable(true);
        canvas.add_css_class("git-graph");
        canvas.set_accessible_role(gtk::AccessibleRole::Group);
        canvas.update_property(&[
            gtk::accessible::Property::Label("Flow canvas"),
            gtk::accessible::Property::Description("Arrow keys move between cards, Enter opens, Shift+F10 shows actions, plus and minus zoom, 0 fits."),
        ]);
        let inspector = widgets::vbox(8);
        inspector.add_css_class("git-details");
        let inspector_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .min_content_width(260)
            .child(&inspector)
            .build();
        let inner = gtk::Paned::new(gtk::Orientation::Horizontal);
        inner.set_start_child(Some(&canvas));
        inner.set_end_child(Some(&inspector_scroll));
        inner.set_resize_end_child(false);
        inner.set_shrink_end_child(false);
        inner.set_shrink_start_child(false);
        super::layout::persist(&inner, "flow.inspector", 320, true);
        let outer = gtk::Paned::new(gtk::Orientation::Horizontal);
        outer.set_start_child(Some(&list_scroll));
        outer.set_end_child(Some(&inner));
        outer.set_resize_start_child(false);
        outer.set_shrink_start_child(false);
        outer.set_shrink_end_child(false);
        outer.set_vexpand(true);
        super::layout::persist(&outer, "flow.list", 200, false);
        list_scroll.set_visible(show_list.is_active());
        show_list.connect_toggled(move |b| {
            crate::prefs::update(|p| p.flow_list = b.is_active());
            list_scroll.set_visible(b.is_active());
        });
        root.append(&outer);
        let readout = widgets::label("", "git-history-readout");
        readout.set_ellipsize(pango::EllipsizeMode::End);
        root.append(&readout);
        let me = Rc::new(Self {
            root,
            canvas,
            list,
            inspector,
            readout,
            inactive,
            state: Rc::default(),
            camera: Rc::new(Cell::new(Camera { x: -40.0, y: -40.0, zoom: 1.0 })),
            drag: RefCell::new(None),
            loaded: RefCell::new(None),
            common: RefCell::new(None),
            generation: Cell::new(0),
            pending: Cell::new(false),
            last: Cell::new(None),
            fitted: Cell::new(false),
            moved: Cell::new(false),
            monitor: RefCell::new(None),
        });
        me.wire(&search);
        me.watch_agents();
        me.inspect();
        me
    }
    fn wire(self: &Rc<Self>, search: &gtk::SearchEntry) {
        let (state, camera) = (self.state.clone(), self.camera.clone());
        self.canvas.set_draw_func(move |area, cr, w, h| draw(area, cr, w as f64, h as f64, &state.borrow(), camera.get()));
        search.connect_search_changed(|e| {
            page(|p| {
                p.state.borrow_mut().query = e.text().to_lowercase();
                p.rebuild_list();
                p.canvas.queue_draw();
            })
        });
        self.inactive.connect_toggled(|_| page(|p| p.relayout(false)));
        self.list.connect_row_selected(|_, row| {
            let Some(row) = row else { return };
            let id = row.widget_name().to_string();
            page(|p| {
                if p.state.borrow().selected.as_deref() != Some(id.as_str()) {
                    p.select(Some(id), true);
                }
            })
        });
        self.list.connect_row_activated(|_, row| {
            let id = row.widget_name().to_string();
            primary(&id);
        });
        let drag = gtk::GestureDrag::new();
        drag.set_button(0);
        drag.connect_drag_begin(|g, x, y| {
            page(|p| {
                p.canvas.grab_focus();
                let cam = p.camera.get();
                let (wx, wy) = cam.world(x, y);
                let hit = p.state.borrow().hit(wx, wy);
                let middle = g.current_button() == 2;
                *p.drag.borrow_mut() = match hit {
                    Some(id) if !middle && g.current_button() == 1 => {
                        let from = p.state.borrow().place(&id);
                        from.map(|from| Drag::Card { id, from })
                    }
                    _ => Some(Drag::Pan { camera: cam }),
                };
            })
        });
        drag.connect_drag_update(|_, dx, dy| {
            page(|p| {
                let cam = p.camera.get();
                match &*p.drag.borrow() {
                    Some(Drag::Pan { camera }) => {
                        p.moved.set(true);
                        p.camera.set(Camera { x: camera.x - dx / cam.zoom, y: camera.y - dy / cam.zoom, ..cam })
                    }
                    Some(Drag::Card { id, from }) if dx.abs() + dy.abs() > 4.0 => {
                        p.state.borrow_mut().manual.insert(id.clone(), Place { x: from.x + dx / cam.zoom, y: from.y + dy / cam.zoom });
                    }
                    _ => {}
                }
                p.canvas.queue_draw();
            })
        });
        drag.connect_drag_end(|_, dx, dy| {
            page(|p| {
                let moved = matches!(&*p.drag.borrow(), Some(Drag::Card { .. })) && dx.abs() + dy.abs() > 4.0;
                p.drag.borrow_mut().take();
                if moved {
                    p.save_positions();
                }
            })
        });
        self.canvas.add_controller(drag);
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.connect_pressed(|g, n, x, y| {
            page(|p| {
                let (wx, wy) = p.camera.get().world(x, y);
                let hit = p.state.borrow().hit(wx, wy);
                if g.current_button() == 1 || g.current_button() == 3 {
                    p.select(hit.clone(), false);
                }
                if g.current_button() == 3
                    && let Some(id) = hit.clone()
                {
                    super::popup_at(&p.canvas, actions(&id).into_iter().map(|(l, f)| super::item(&l, move || f())).collect(), x, y);
                } else if n == 2
                    && let Some(id) = hit
                {
                    primary(&id);
                }
            })
        });
        self.canvas.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        motion.connect_motion(|_, x, y| {
            page(|p| {
                let (wx, wy) = p.camera.get().world(x, y);
                let hit = p.state.borrow().hit(wx, wy);
                if p.state.borrow().hover != hit {
                    p.state.borrow_mut().hover = hit;
                    p.canvas.queue_draw();
                }
            })
        });
        self.canvas.add_controller(motion);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        scroll.connect_scroll(|c, dx, dy| {
            page(|p| {
                let cam = p.camera.get();
                if c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                    p.zoom_by(if dy < 0.0 { 1.1 } else { 1.0 / 1.1 });
                } else {
                    p.moved.set(true);
                    let step = if c.unit() == gdk::ScrollUnit::Surface { 1.0 } else { 40.0 };
                    p.camera.set(Camera { x: cam.x + dx * step / cam.zoom, y: cam.y + dy * step / cam.zoom, ..cam });
                    p.canvas.queue_draw();
                }
            });
            glib::Propagation::Stop
        });
        self.canvas.add_controller(scroll);
        let pinch = gtk::GestureZoom::new();
        let start = Rc::new(Cell::new(1.0));
        let s = start.clone();
        pinch.connect_begin(move |_, _| page(|p| s.set(p.camera.get().zoom)));
        pinch.connect_scale_changed(move |_, scale| page(|p| p.set_zoom(start.get() * scale)));
        self.canvas.add_controller(pinch);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(|_, key, _, mods| {
            let mut handled = true;
            page(|p| match key {
                gdk::Key::Left => p.step(-1.0, 0.0),
                gdk::Key::Right => p.step(1.0, 0.0),
                gdk::Key::Up => p.step(0.0, -1.0),
                gdk::Key::Down => p.step(0.0, 1.0),
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => p.zoom_by(1.15),
                gdk::Key::minus | gdk::Key::KP_Subtract => p.zoom_by(1.0 / 1.15),
                gdk::Key::_0 | gdk::Key::KP_0 => p.fit(),
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    if let Some(id) = p.state.borrow().selected.clone() {
                        primary(&id);
                    }
                }
                gdk::Key::Menu => p.menu_for_selected(),
                gdk::Key::F10 if mods.contains(gdk::ModifierType::SHIFT_MASK) => p.menu_for_selected(),
                _ => handled = false,
            });
            if handled { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        self.canvas.add_controller(keys);
        self.canvas.connect_resize(|_, _, _| {
            page(|p| {
                if !p.moved.get() && p.state.borrow().flow.is_some() {
                    p.fit();
                }
            })
        });
        nyx_theme::subscribe(&self.canvas, || page(|p| p.canvas.queue_draw()));
    }
    /// Read the flow again when Nyx Agents' sessions change.
    fn watch_agents(&self) {
        let Some(path) = agents::hub_path() else { return };
        let file = gtk::gio::File::for_path(&path);
        if let Ok(monitor) = file.monitor_file(gtk::gio::FileMonitorFlags::WATCH_MOVES, gtk::gio::Cancellable::NONE) {
            monitor.set_rate_limit(1000);
            monitor.connect_changed(|_, _, _, _| page(|p| p.load(true)));
            *self.monitor.borrow_mut() = Some(monitor);
        }
    }
    fn menu_for_selected(&self) {
        let Some(id) = self.state.borrow().selected.clone() else { return };
        let cam = self.camera.get();
        let Some(place) = self.state.borrow().place(&id) else { return };
        let (x, y) = ((place.x + 24.0 - cam.x) * cam.zoom, (place.y + CARD_H - cam.y) * cam.zoom);
        super::popup_at(&self.canvas, actions(&id).into_iter().map(|(l, f)| super::item(&l, move || f())).collect(), x, y);
    }
    fn set_zoom(&self, zoom: f64) {
        self.moved.set(true);
        let cam = self.camera.get();
        let zoom = zoom.clamp(0.25, 2.5);
        let (w, h) = (self.canvas.width() as f64 / 2.0, self.canvas.height() as f64 / 2.0);
        let (cx, cy) = cam.world(w, h);
        self.camera.set(Camera { x: cx - w / zoom, y: cy - h / zoom, zoom });
        self.canvas.queue_draw();
    }
    fn zoom_by(&self, factor: f64) {
        self.set_zoom(self.camera.get().zoom * factor);
    }
    /// Show every card.
    fn fit(&self) {
        let s = self.state.borrow();
        let places: Vec<Place> = s.nodes.iter().filter_map(|(id, _)| s.place(id)).collect();
        drop(s);
        let (w, h) = (self.canvas.width() as f64, self.canvas.height() as f64);
        if places.is_empty() || w < 10.0 || h < 10.0 {
            return;
        }
        self.fitted.set(true);
        let (x0, y0) = places.iter().fold((f64::MAX, f64::MAX), |(x, y), p| (x.min(p.x), y.min(p.y)));
        let (x1, y1) = places.iter().fold((f64::MIN, f64::MIN), |(x, y), p| (x.max(p.x + CARD_W), y.max(p.y + CARD_H)));
        let margin = 48.0;
        let zoom = ((w - margin * 2.0) / (x1 - x0)).min((h - margin * 2.0) / (y1 - y0)).clamp(0.35, 1.0);
        self.camera.set(Camera { x: x0 - (w / zoom - (x1 - x0)) / 2.0, y: y0 - (h / zoom - (y1 - y0)).max(0.0) / 2.0, zoom });
        self.canvas.queue_draw();
    }
    /// Keep the selected card in view.
    fn reveal(&self, id: &str) {
        let Some(p) = self.state.borrow().place(id) else { return };
        let cam = self.camera.get();
        let (w, h) = (self.canvas.width() as f64 / cam.zoom, self.canvas.height() as f64 / cam.zoom);
        let mut c = cam;
        if p.x < cam.x + 16.0 || p.x + CARD_W > cam.x + w - 16.0 {
            c.x = p.x + CARD_W / 2.0 - w / 2.0;
        }
        if p.y < cam.y + 16.0 || p.y + CARD_H > cam.y + h - 16.0 {
            c.y = p.y + CARD_H / 2.0 - h / 2.0;
        }
        self.camera.set(c);
    }
    /// Move the selection to the nearest card in a direction.
    fn step(&self, dx: f64, dy: f64) {
        let s = self.state.borrow();
        let Some(from) = s.selected.as_ref().and_then(|id| s.place(id)).or_else(|| s.nodes.first().and_then(|(id, _)| s.place(id))) else { return };
        let best = s
            .nodes
            .iter()
            .filter(|(id, _)| Some(id) != s.selected.as_ref())
            .filter_map(|(id, _)| s.place(id).map(|p| (id.clone(), p)))
            .filter(|(_, p)| (p.x - from.x) * dx + (p.y - from.y) * dy > 1.0)
            .min_by(|(_, a), (_, b)| {
                let d = |p: &Place| {
                    let (ex, ey) = (p.x - from.x, p.y - from.y);
                    let along = ex * dx + ey * dy;
                    let across = (ex * dy - ey * dx).abs();
                    along + across * 2.0
                };
                d(a).total_cmp(&d(b))
            })
            .map(|(id, _)| id);
        let fallback = s.selected.is_none().then(|| s.nodes.first().map(|(id, _)| id.clone())).flatten();
        drop(s);
        if let Some(id) = best.or(fallback) {
            self.select(Some(id), true);
        }
    }
    fn select(&self, id: Option<String>, reveal: bool) {
        self.state.borrow_mut().selected = id.clone();
        if let Some(id) = &id {
            if reveal {
                self.reveal(id);
            }
            let mut row = self.list.first_child();
            while let Some(r) = row {
                if let Some(r) = r.downcast_ref::<gtk::ListBoxRow>()
                    && r.widget_name() == *id
                    && !r.is_selected()
                {
                    self.list.select_row(Some(r));
                }
                row = r.next_sibling();
            }
            let title = self.state.borrow().title(id);
            if self.canvas.has_focus() {
                self.canvas.announce(&title, gtk::AccessibleAnnouncementPriority::Low);
            }
        }
        self.inspect();
        self.canvas.queue_draw();
    }
    /// The open repository changed or was shown: read its flow, at most every few seconds.
    pub fn load(&self, force: bool) {
        let Some(ui) = current() else { return };
        let Some((root, common)) = ui.snapshot.borrow().as_ref().map(|s| (s.repository.root.clone(), s.repository.common_dir.clone())) else {
            *self.state.borrow_mut() = State::default();
            self.rebuild_list();
            self.readout.set_text("Open a repository to see its work in progress.");
            self.inspect();
            self.canvas.queue_draw();
            return;
        };
        let same = self.loaded.borrow().as_ref() == Some(&root);
        if same && !force && self.state.borrow().flow.is_some() {
            return;
        }
        if same && self.last.get().is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            // A burst of file changes: read once, a little later.
            if !self.pending.replace(true) {
                glib::timeout_add_local_once(Duration::from_secs(2), || page(|p| {
                    p.pending.set(false);
                    p.load(true);
                }));
            }
            return;
        }
        self.last.set(Some(Instant::now()));
        if !same {
            let manual = flow::load_positions(&common);
            *self.state.borrow_mut() = State { manual, ..Default::default() };
            self.fitted.set(false);
            self.moved.set(false);
            self.readout.set_text("Reading work in progress…");
        }
        *self.loaded.borrow_mut() = Some(root.clone());
        *self.common.borrow_mut() = Some(common);
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let open = root.clone();
        nyx_core::cmd::background(
            move || flow::read(&root, &open, agents::sessions()).map(|f| (root, f)),
            move |result| {
                page(|p| {
                    if p.generation.get() != generation {
                        return;
                    }
                    match result {
                        Ok((root, f)) if p.loaded.borrow().as_ref() == Some(&root) => p.apply(f),
                        Ok(_) => {}
                        Err(e) => p.readout.set_text(&format!("{e:#}")),
                    }
                })
            },
        );
    }
    fn apply(&self, f: Flow) {
        let active = f.branches.iter().filter(|b| b.active).count();
        let agents = f.agents.len();
        self.readout.set_text(&format!(
            "{active} active of {} branches{} · {} worktree{} · {agents} agent session{}{}",
            f.branches.len(),
            if f.limited { "+" } else { "" },
            f.worktrees.len(),
            if f.worktrees.len() == 1 { "" } else { "s" },
            if agents == 1 { "" } else { "s" },
            if f.overlaps.is_empty() { String::new() } else { format!(" · {} pairs touch the same files", f.overlaps.len()) }
        ));
        self.state.borrow_mut().flow = Some(f);
        self.relayout(true);
    }
    fn relayout(&self, keep_selection: bool) {
        {
            let mut s = self.state.borrow_mut();
            let Some(f) = s.flow.take() else { return };
            let nodes = flow::nodes(&f, self.inactive.is_active());
            s.auto = flow::layout(&f, &nodes);
            s.nodes = nodes;
            s.flow = Some(f);
            let s = &mut *s;
            let f = s.flow.as_ref().unwrap();
            let ids: Vec<String> = s.nodes.iter().map(|(id, _)| id.clone()).collect();
            if !keep_selection || s.selected.as_ref().is_some_and(|id| !ids.contains(id)) {
                s.selected = None;
            }
            if s.selected.is_none() {
                // Start on the open worktree's branch.
                let open = f.worktrees.iter().find(|w| w.open).and_then(|w| w.branch.clone()).map(|b| flow::branch_id(&b));
                s.selected = open.filter(|id| ids.contains(id));
            }
        }
        self.rebuild_list();
        if !self.fitted.get() {
            self.fit();
        }
        self.inspect();
        self.canvas.queue_draw();
    }
    fn auto_layout(&self) {
        self.moved.set(false);
        self.state.borrow_mut().manual.clear();
        self.save_positions();
        self.fit();
    }
    fn save_positions(&self) {
        let Some(common) = self.common.borrow().clone() else { return };
        let manual = self.state.borrow().manual.clone();
        nyx_core::cmd::background(move || flow::save_positions(&common, &manual), |r: anyhow::Result<()>| {
            if let Err(e) = r {
                tracing::warn!(%e, "Could not save Flow positions");
            }
        });
    }
    fn rebuild_list(&self) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let s = self.state.borrow();
        let Some(f) = s.flow.as_ref() else { return };
        let mut ids: Vec<&String> = s.nodes.iter().map(|(id, _)| id).filter(|id| s.matches(id)).collect();
        ids.sort_by_key(|id| s.place(id).map(|p| ((p.y * 10.0) as i64, (p.x * 10.0) as i64)));
        for id in ids {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(id);
            let b = widgets::vbox(0);
            let title = widgets::label(&s.title(id), "");
            title.set_ellipsize(pango::EllipsizeMode::Middle);
            b.append(&title);
            let detail = match s.branch(id) {
                Some(br) => {
                    let mut d = divergence(br, f.trunk.as_deref().unwrap_or("trunk"));
                    if let Some(w) = s.worktree(id) {
                        d.push_str(&format!(" · worktree{}", w.dirty.filter(|n| *n > 0).map(|n| format!(", {n} changed")).unwrap_or_default()));
                    }
                    let a = s.agents(id).len();
                    if a > 0 {
                        d.push_str(&format!(" · {a} agent{}", if a == 1 { "" } else { "s" }));
                    }
                    d
                }
                None => "worktree on a detached HEAD".into(),
            };
            let d = widgets::label(&detail, "dim-label");
            d.add_css_class("git-palette-detail");
            d.set_ellipsize(pango::EllipsizeMode::End);
            b.append(&d);
            row.set_child(Some(&b));
            self.list.append(&row);
            if s.selected.as_ref() == Some(id) {
                self.list.select_row(Some(&row));
            }
        }
    }
    fn inspect(&self) {
        while let Some(c) = self.inspector.first_child() {
            self.inspector.remove(&c);
        }
        let add = |text: &str, class: &str| {
            let l = widgets::label(text, class);
            l.set_wrap(true);
            l.set_wrap_mode(pango::WrapMode::WordChar);
            l.set_selectable(true);
            self.inspector.append(&l);
            l
        };
        let s = self.state.borrow();
        let Some(id) = s.selected.clone() else {
            add("Select a card to see what Git knows about it and what can be done.", "dim-label");
            return;
        };
        let Some(f) = s.flow.as_ref() else { return };
        let trunk = f.trunk.clone().unwrap_or_else(|| "trunk".into());
        add(&s.title(&id), "section-title");
        if let Some(b) = s.branch(&id) {
            add(
                &match (b.trunk, b.parent.as_deref()) {
                    (true, _) => "Trunk: the branch other work starts from".to_string(),
                    (false, Some(p)) if p != trunk => format!("Builds on {p}"),
                    _ => "Branch".to_string(),
                },
                "dim-label",
            );
            add("GIT", "group-title");
            add(&format!("{} {} · {}", &b.tip[..b.tip.len().min(10)], b.subject, super::graph_renderer::relative_time(b.time)), "");
            add(&divergence(b, &trunk), "");
            if let Some(fork) = &b.fork {
                add(&format!("Shares history with {} up to {}", b.parent.as_deref().unwrap_or(&trunk), &fork[..fork.len().min(10)]), "dim-label");
            }
            match (&b.upstream, b.tracking) {
                (Some(up), Some((0, 0))) => add(&format!("Up to date with {up}"), "dim-label"),
                (Some(up), Some((a, behind))) => add(&format!("{up}: ↑{a} to push, ↓{behind} to pull"), "dim-label"),
                _ => add("Not published: no upstream branch", "dim-label"),
            };
            for link in s.links.iter().filter(|l| l.source == b.name) {
                add("REVIEW", "group-title");
                add(&format!("#{} {} → {}", link.number, link.title, link.target), "");
                add(&link.state, "dim-label");
            }
        }
        if let Some(w) = s.worktree(&id) {
            add("WORKTREE", "group-title");
            add(&w.path.display().to_string(), "");
            add(
                &format!(
                    "{}{}{}{}",
                    match w.dirty {
                        Some(0) => "Clean".to_string(),
                        Some(n) => format!("{n} changed file{}", if n == 1 { "" } else { "s" }),
                        None => "Changes unknown".into(),
                    },
                    if w.open { " · open in this window" } else { "" },
                    if w.main { " · main worktree" } else { "" },
                    if w.locked { " · locked" } else { "" }
                ),
                "dim-label",
            );
        }
        let sessions = s.agents(&id);
        if !sessions.is_empty() {
            add("AGENTS", "group-title");
            for a in sessions {
                add(&format!("{} · {}", a.agent(), a.state()), "");
                add(&a.title, "dim-label");
                let open = super::wrapping_button("Open in Nyx Agents");
                let session = a.clone();
                open.connect_clicked(move |_| {
                    if let Err(e) = agents::open(&session)
                        && let Some(ui) = current()
                    {
                        ui.error(&format!("Could not open the session: {e:#}"));
                    }
                });
                self.inspector.append(&open);
            }
        }
        if let Some(name) = id.strip_prefix("branch:") {
            let shared: Vec<_> = f.overlaps.iter().filter(|o| o.a == name || o.b == name).collect();
            if !shared.is_empty() {
                add("SHARED FILES", "group-title");
                add("Derived from what each changed since it forked, plus uncommitted changes. Not a Git relationship.", "dim-label");
                for o in shared {
                    let other = if o.a == name { &o.b } else { &o.a };
                    let files: Vec<_> = o.files.iter().take(5).map(|f| f.display().to_string()).collect();
                    add(&format!("{other}: {}{}", files.join(", "), if o.files.len() > 5 { format!(" +{}", o.files.len() - 5) } else { String::new() }), "git-warning");
                }
            }
        }
        add("ACTIONS", "group-title");
        let box_ = widgets::vbox(6);
        drop(s);
        for (label, run) in actions(&id) {
            let b = super::wrapping_button(&label);
            b.set_halign(gtk::Align::Fill);
            let run2 = run.clone();
            b.connect_clicked(move |_| run2());
            // Show what Merge and Rebase would connect before they're chosen.
            if let Some((from, to, what)) = proposal_for(&id, &label) {
                let motion = gtk::EventControllerMotion::new();
                let p = (from, to, what);
                motion.connect_enter(move |_, _, _| page(|pg| {
                    pg.state.borrow_mut().proposal = Some(p.clone());
                    pg.canvas.queue_draw();
                }));
                motion.connect_leave(|_| page(|pg| {
                    pg.state.borrow_mut().proposal = None;
                    pg.canvas.queue_draw();
                }));
                b.add_controller(motion);
            }
            box_.append(&b);
        }
        self.inspector.append(&box_);
    }
    /// Pull and merge requests, from Review, to show on their branches.
    pub fn set_links(&self, links: Vec<Link>) {
        self.state.borrow_mut().links = links;
        self.inspect();
        self.canvas.queue_draw();
    }
}
/// The current branch's card id.
fn current_branch() -> Option<String> {
    let ui = current()?;
    let s = ui.destinations.flow.state.borrow();
    let f = s.flow.as_ref()?;
    f.worktrees.iter().find(|w| w.open).and_then(|w| w.branch.clone()).map(|b| flow::branch_id(&b))
}
fn proposal_for(id: &str, label: &str) -> Option<(String, String, String)> {
    let here = current_branch()?;
    if here == id {
        return None;
    }
    if label.starts_with("Merge into") {
        Some((id.into(), here, "merge".into()))
    } else if label.starts_with("Rebase") {
        Some((here, id.into(), "rebase onto".into()))
    } else {
        None
    }
}
/// Double-click or Enter: a branch's history, or open a worktree on no branch.
fn primary(id: &str) {
    let Some(ui) = current() else { return };
    if let Some(name) = id.strip_prefix("branch:") {
        ui.show_view("history");
        ui.graph.show_reference(format!("refs/heads/{name}"));
    } else if let Some(path) = id.strip_prefix("worktree:") {
        ui.open(PathBuf::from(path));
    }
}
type Run = Rc<dyn Fn()>;
/// What can be done with a card, worked out from what Git said about it.
fn actions(id: &str) -> Vec<(String, Run)> {
    let Some(ui) = current() else { return Vec::new() };
    let Some(root) = ui.root() else { return Vec::new() };
    let s = ui.destinations.flow.state.borrow();
    let mut out: Vec<(String, Run)> = Vec::new();
    let worktree = s.worktree(id).cloned();
    let open_here = worktree.as_ref().is_some_and(|w| w.open);
    if let Some(w) = worktree.as_ref().filter(|w| !w.open) {
        let path = w.path.clone();
        out.push(("Open worktree".into(), Rc::new(move || if let Some(ui) = current() { ui.open(path.clone()) })));
    }
    if let Some(w) = worktree.as_ref().filter(|w| w.dirty.unwrap_or(0) > 0) {
        let path = w.path.clone();
        out.push(("Open its changes".into(), Rc::new(move || {
            if let Some(ui) = current() {
                if ui.root().as_ref() != Some(&path) {
                    ui.open(path.clone());
                }
                ui.show_view("changes");
            }
        })));
    }
    if let Some(b) = s.branch(id).cloned() {
        let reference = b.reference.clone();
        out.push(("Show history".into(), Rc::new({
            let id = id.to_string();
            move || primary(&id)
        })));
        if worktree.is_none() && !b.current {
            let (r, name) = (root.clone(), b.name.clone());
            out.push(("Switch to this branch".into(), Rc::new(move || {
                if let Some(ui) = current() {
                    ui.execute_at(r.clone(), nyx_git::core::operations::Operation::SwitchBranch(name.clone()));
                }
            })));
            let (r, name) = (root.clone(), b.name.clone());
            out.push(("Create a worktree for it…".into(), Rc::new(move || super::worktree_manager::create_for(r.clone(), Some(name.clone()), None))));
        }
        let (r, tip) = (root.clone(), b.tip.clone());
        out.push(("New worktree with a branch from here…".into(), Rc::new(move || super::worktree_manager::create_for(r.clone(), None, Some(tip.clone())))));
        let tip = b.tip.clone();
        out.push(("New branch from here…".into(), Rc::new(move || super::advanced::target_action("branch", tip.clone()))));
        if !open_here && !b.current {
            let target = reference.clone();
            out.push(("Merge into the current branch…".into(), Rc::new(move || super::advanced::target_action("merge", target.clone()))));
            let target = reference.clone();
            out.push(("Rebase the current branch onto this…".into(), Rc::new(move || super::advanced::target_action("rebase", target.clone()))));
        }
        for link in s.links.iter().filter(|l| l.source == b.name) {
            let url = link.url.clone();
            out.push((format!("Open request #{}", link.number), Rc::new(move || super::open_url(&url))));
        }
        if worktree.is_none() && !b.current && !b.trunk {
            let (r, name, merged) = (root.clone(), b.name.clone(), b.merged);
            out.push(("Delete branch…".into(), Rc::new(move || {
                if merged {
                    let (root, n) = (r.clone(), name.clone());
                    super::preview::confirm(
                        r.clone(),
                        nyx_git::core::preview::Subject::DeleteMergedBranch(name.clone()),
                        "Delete merged branch?".into(),
                        format!("Delete {name}. Its commits are already in the trunk. Working files and remote branches are unchanged."),
                        "Delete branch".into(),
                        true,
                        move || if let Some(ui) = current() { ui.execute_at(root.clone(), nyx_git::core::operations::Operation::DeleteBranch(n.clone())) },
                    );
                } else {
                    super::advanced::target_action("delete-branch", name.clone());
                }
            })));
        }
        let name = b.name.clone();
        out.push(("Copy branch name".into(), Rc::new(move || if let Some(ui) = current() { gtk::prelude::WidgetExt::display(&ui.window).clipboard().set_text(&name) })));
    }
    if let Some(w) = worktree.as_ref().filter(|w| !w.main && !w.open) {
        let (r, path) = (root.clone(), w.path.clone());
        out.push(("Remove worktree…".into(), Rc::new(move || super::worktree_manager::remove_at(r.clone(), path.clone()))));
    }
    if let Some(w) = worktree.as_ref() {
        let path = w.path.clone();
        out.push(("Open in terminal".into(), Rc::new(move || super::workspace::external(path.clone(), "terminal"))));
    }
    for a in s.agents(id) {
        let session = a.clone();
        out.push((format!("Open {} session", a.agent()), Rc::new(move || {
            if let Err(e) = agents::open(&session)
                && let Some(ui) = current()
            {
                ui.error(&format!("Could not open the session: {e:#}"));
            }
        })));
    }
    out
}

fn draw(area: &gtk::DrawingArea, cr: &cairo::Context, w: f64, h: f64, s: &State, cam: Camera) {
    let palette = nyx_theme::palette();
    let (bg, fg, dim, accent, danger) = (rgba(&palette.bg), rgba(&palette.text), rgba(&palette.dim_text), rgba(&palette.accent), rgba(&palette.danger));
    let tokens = nyx_theme::tokens();
    let warning = rgba(tokens.color("status.warning"));
    let success = rgba(tokens.color("status.success"));
    let card = mix(&bg, &fg, 0.05);
    set(cr, &bg, 1.0);
    let _ = cr.paint();
    let Some(f) = s.flow.as_ref() else {
        text(area, cr, "Open a repository to see its branches, worktrees and agents.", (24.0, 24.0, w - 48.0), (13.0, false), &dim, 1.0);
        return;
    };
    // A quiet dot grid, so panning reads as movement.
    let step = 40.0 * cam.zoom;
    if step > 12.0 {
        set(cr, &dim, 0.12);
        let (ox, oy) = ((-cam.x * cam.zoom).rem_euclid(step), (-cam.y * cam.zoom).rem_euclid(step));
        let mut x = ox;
        while x < w {
            let mut y = oy;
            while y < h {
                cr.rectangle(x, y, 1.2, 1.2);
                y += step;
            }
            x += step;
        }
        let _ = cr.fill();
    }
    cr.save().ok();
    cr.scale(cam.zoom, cam.zoom);
    cr.translate(-cam.x, -cam.y);
    let visible = |p: &Place| {
        let (x0, y0, x1, y1) = (cam.x, cam.y, cam.x + w / cam.zoom, cam.y + h / cam.zoom);
        p.x + CARD_W >= x0 - 200.0 && p.x <= x1 + 200.0 && p.y + CARD_H >= y0 - 200.0 && p.y <= y1 + 200.0
    };
    let focus = |id: &str| s.selected.as_deref() == Some(id) || s.hover.as_deref() == Some(id);
    // Ancestry.
    cr.set_line_width(2.0);
    for (id, parent) in &s.nodes {
        let (Some(parent), Some(to)) = (parent.as_ref(), s.place(id)) else { continue };
        let Some(from) = s.place(parent) else { continue };
        if !visible(&from) && !visible(&to) {
            continue;
        }
        let strong = focus(id) || focus(parent);
        set(cr, if strong { &accent } else { &dim }, if strong { 0.9 } else { 0.45 });
        let (x0, y0, x1, y1) = (from.x + CARD_W, from.y + 30.0, to.x, to.y + 30.0);
        cr.move_to(x0, y0);
        let mid = (x0 + x1) / 2.0;
        cr.curve_to(mid, y0, mid, y1, x1, y1);
        let _ = cr.stroke();
    }
    let centre = |p: Place| (p.x + CARD_W / 2.0, p.y + CARD_H / 2.0);
    // Requests: dashed, into their target branch.
    cr.set_dash(&[7.0, 5.0], 0.0);
    for link in &s.links {
        let (Some(a), Some(b)) = (s.place(&flow::branch_id(&link.source)), s.place(&flow::branch_id(&link.target))) else { continue };
        let ((x0, y0), (x1, y1)) = (centre(a), centre(b));
        set(cr, &accent, 0.55);
        cr.move_to(x0, y0);
        cr.line_to(x1, y1);
        let _ = cr.stroke();
    }
    // Derived overlaps: dotted amber, with the count of shared files.
    cr.set_dash(&[2.0, 5.0], 0.0);
    for o in &f.overlaps {
        let (Some(a), Some(b)) = (s.place(&flow::branch_id(&o.a)), s.place(&flow::branch_id(&o.b))) else { continue };
        let ((x0, y0), (x1, y1)) = (centre(a), centre(b));
        set(cr, &warning, 0.8);
        cr.move_to(x0, y0);
        cr.line_to(x1, y1);
        let _ = cr.stroke();
        let label = format!("{} shared file{}", o.files.len(), if o.files.len() == 1 { "" } else { "s" });
        text(area, cr, &label, ((x0 + x1) / 2.0 - 50.0, (y0 + y1) / 2.0 - 8.0, 120.0), (11.0, false), &warning, 1.0);
    }
    // A considered action: dotted accent arrow.
    if let Some((from, to, label)) = &s.proposal
        && let (Some(a), Some(b)) = (s.place(from), s.place(to))
    {
        let ((x0, y0), (x1, y1)) = (centre(a), centre(b));
        cr.set_dash(&[3.0, 4.0], 0.0);
        cr.set_line_width(2.5);
        set(cr, &accent, 1.0);
        cr.move_to(x0, y0);
        cr.line_to(x1, y1);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        let angle = (y1 - y0).atan2(x1 - x0);
        let tip = (x1 - angle.cos() * 60.0, y1 - angle.sin() * 60.0);
        cr.move_to(tip.0, tip.1);
        cr.line_to(tip.0 - 12.0 * (angle - 0.4).cos(), tip.1 - 12.0 * (angle - 0.4).sin());
        cr.line_to(tip.0 - 12.0 * (angle + 0.4).cos(), tip.1 - 12.0 * (angle + 0.4).sin());
        cr.close_path();
        let _ = cr.fill();
        text(area, cr, label, ((x0 + x1) / 2.0 + 6.0, (y0 + y1) / 2.0 + 4.0, 140.0), (12.0, true), &accent, 1.0);
    }
    cr.set_dash(&[], 0.0);
    // Cards.
    let trunk = f.trunk.as_deref().unwrap_or("trunk");
    for (id, _) in &s.nodes {
        let Some(p) = s.place(id) else { continue };
        if !visible(&p) {
            continue;
        }
        let alpha = if s.matches(id) { 1.0 } else { 0.35 };
        let selected = s.selected.as_deref() == Some(id.as_str());
        let branch = s.branch(id);
        let worktree = s.worktree(id);
        let faded = branch.is_some_and(|b| b.merged || !b.active);
        rounded(cr, p.x, p.y, CARD_W, CARD_H, 11.0);
        set(cr, &if s.hover.as_deref() == Some(id.as_str()) { mix(&card, &accent, 0.06) } else { card }, alpha);
        let _ = cr.fill_preserve();
        set(cr, if selected { &accent } else { &dim }, if selected { 1.0 } else { 0.22 * alpha });
        cr.set_line_width(if selected { 2.0 } else { 1.0 });
        let _ = cr.stroke();
        if branch.is_some_and(|b| b.trunk) {
            set(cr, &accent, alpha);
            rounded(cr, p.x, p.y + 10.0, 3.0, CARD_H - 20.0, 1.5);
            let _ = cr.fill();
        }
        let title_c = if faded { dim } else { fg };
        let title = match branch {
            Some(b) if b.current => format!("● {}", b.name),
            Some(b) => b.name.clone(),
            None => s.title(id),
        };
        text(area, cr, &title, (p.x + 14.0, p.y + 10.0, CARD_W - 28.0), (13.5, true), &title_c, alpha);
        let line2 = match branch {
            Some(b) => divergence(b, trunk),
            None => format!("detached at {}", worktree.and_then(|w| w.head.as_deref()).map(|h| &h[..h.len().min(8)]).unwrap_or("?")),
        };
        text(area, cr, &line2, (p.x + 14.0, p.y + 31.0, CARD_W - 28.0), (11.5, false), &if branch.is_some_and(|b| b.trunk) { accent } else { dim }, alpha);
        if let Some(b) = branch {
            text(area, cr, &format!("{} · {}", b.subject, super::graph_renderer::relative_time(b.time)), (p.x + 14.0, p.y + 49.0, CARD_W - 28.0), (11.5, false), &fg, 0.8 * alpha);
            if let Some(link) = s.links.iter().find(|l| l.source == b.name) {
                let chip = format!("#{} {}", link.number, link.state);
                text(area, cr, &chip, (p.x + 14.0, p.y + 66.0, CARD_W - 28.0), (11.0, false), &accent, alpha);
            } else if let (Some(up), Some((a, behind))) = (&b.upstream, b.tracking) {
                let sync = if a == 0 && behind == 0 { format!("{up} · in sync") } else { format!("{up} · ↑{a} ↓{behind}") };
                text(area, cr, &sync, (p.x + 14.0, p.y + 66.0, CARD_W - 28.0), (11.0, false), &dim, alpha);
            }
        }
        // The worktree strip along the bottom.
        if let Some(w) = worktree {
            let y = p.y + CARD_H - 26.0;
            set(cr, &mix(&card, &accent, if w.open { 0.16 } else { 0.07 }), alpha);
            cr.rectangle(p.x + 1.0, y, CARD_W - 2.0, 25.0);
            let _ = cr.fill();
            let name = w.path.file_name().unwrap_or_default().to_string_lossy();
            let state = match w.dirty {
                Some(0) => "clean".to_string(),
                Some(n) => format!("{n} changed"),
                None => "unavailable".into(),
            };
            let label = format!("⌂ {name} · {state}{}", if w.open { " · open" } else { "" });
            let c = if w.dirty.unwrap_or(0) > 0 { warning } else { dim };
            text(area, cr, &label, (p.x + 14.0, y + 5.0, CARD_W - 28.0), (11.0, false), &c, alpha);
        }
        // Agent chips, top right.
        let mut x = p.x + CARD_W - 12.0;
        for a in s.agents(id).iter().take(3) {
            let color = match a.activity.as_str() {
                "working" => success,
                "blocked" | "error" => danger,
                "done" => accent,
                _ => dim,
            };
            let label = a.agent();
            let width = 14.0 + label.chars().count() as f64 * 6.2;
            x -= width;
            rounded(cr, x, p.y + 8.0, width, 18.0, 9.0);
            set(cr, &mix(&card, &color, 0.22), alpha);
            let _ = cr.fill();
            set(cr, &color, alpha);
            cr.arc(x + 8.0, p.y + 17.0, 3.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            text(area, cr, &label, (x + 14.0, p.y + 10.0, width - 14.0), (10.5, false), &fg, alpha);
            x -= 4.0;
        }
    }
    cr.restore().ok();
}
