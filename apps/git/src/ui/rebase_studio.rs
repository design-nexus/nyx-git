//! Rebase Studio: interactive history editing with commits as cards. Drag (or Alt+Up and
//! Alt+Down) to reorder; choose pick, reword, edit, split, squash, fixup or drop. Beside the
//! plan: the history it would make, commits a remote already has, other branches left on the
//! old commits, and where reordering may conflict. Starting it goes through the usual preview;
//! nothing is ever force-pushed.
use super::window::current;
use gtk::{gdk, glib, prelude::*};
use nyx_git::core::advanced::{self, Facts, Pick, Step};
use nyx_ui::widgets;
use std::{cell::RefCell, path::PathBuf, rc::Rc};

const ACTIONS: &[(Pick, &str)] = &[
    (Pick::Pick, "Pick"),
    (Pick::Reword, "Reword"),
    (Pick::Edit, "Edit"),
    (Pick::Split, "Split"),
    (Pick::Squash, "Squash"),
    (Pick::Fixup, "Fixup"),
    (Pick::Drop, "Drop"),
];
pub struct RebaseStudio {
    pub root: gtk::Box,
    title: gtk::Label,
    cards: gtk::ListBox,
    preview: gtk::Box,
    state: RefCell<Option<Plan>>,
}
struct Plan {
    root: PathBuf,
    base: String,
    original: Vec<Step>,
    steps: Vec<Step>,
    facts: Facts,
}
fn studio(run: impl FnOnce(&RebaseStudio)) {
    if let Some(ui) = current() {
        run(&ui.destinations.rebase);
    }
}
/// Plan an interactive rebase of the commits after `base`, in the studio.
pub fn open(root: PathBuf, base: String, steps: Vec<Step>) {
    let (r, b) = (root.clone(), base.clone());
    nyx_core::cmd::background(
        move || advanced::facts(&r, &b),
        move |result| match result {
            Ok(facts) => studio(|s| {
                *s.state.borrow_mut() = Some(Plan { root, base, original: steps.clone(), steps, facts });
                s.render();
                if let Some(ui) = current() {
                    ui.show_view("rebase");
                }
            }),
            Err(e) => {
                if let Some(ui) = current() {
                    ui.error(&format!("{e:#}"));
                }
            }
        },
    );
}
impl RebaseStudio {
    pub fn new() -> Rc<Self> {
        let root = widgets::vbox(0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        let bar = super::pagebar::PageBar::new();
        let back = super::pagebar::action("git-chev-l-symbolic", "History", "Back to History, leaving the plan", || {
            if let Some(ui) = current() {
                ui.show_view("history");
            }
        });
        bar.start(&back);
        let title = widgets::label("Rebase Studio", "git-bar-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        bar.start(&title);
        let hint = widgets::label("Drag cards or Alt+↑ ↓ to reorder", "git-bar-note");
        hint.set_tooltip_text(Some(
            "Oldest first. Squash and fixup join the commit above; edit and split stop so you can change or divide a commit, then Continue.",
        ));
        bar.end(&hint);
        let reset = super::pagebar::action("git-undo-symbolic", "Reset plan", "Back to every commit picked in its original order", || studio(|s| {
            if let Some(p) = s.state.borrow_mut().as_mut() {
                p.steps = p.original.clone();
            }
            s.render();
        }));
        bar.end(&reset);
        let start = gtk::Button::with_label("Review and start…");
        start.add_css_class("suggested-action");
        start.connect_clicked(|_| studio(|s| s.start()));
        bar.end(&start);
        root.append(&bar.root);
        let cards = gtk::ListBox::new();
        cards.add_css_class("git-rebase-cards");
        cards.set_selection_mode(gtk::SelectionMode::None);
        cards.update_property(&[gtk::accessible::Property::Label("Rebase plan, oldest commit first")]);
        let cards_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .hexpand(true)
            .child(&cards)
            .build();
        let preview = widgets::vbox(4);
        preview.add_css_class("git-insp");
        let preview_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .min_content_width(280)
            .child(&preview)
            .build();
        let body = gtk::Paned::new(gtk::Orientation::Horizontal);
        body.set_start_child(Some(&cards_scroll));
        body.set_end_child(Some(&preview_scroll));
        body.set_resize_end_child(false);
        body.set_shrink_end_child(false);
        body.set_vexpand(true);
        super::layout::persist(&body, "rebase.preview", 360, true);
        root.append(&body);
        Rc::new(Self { root, title, cards, preview, state: RefCell::new(None) })
    }
    fn move_step(&self, from: usize, to: usize) {
        if let Some(p) = self.state.borrow_mut().as_mut()
            && from < p.steps.len()
            && to < p.steps.len()
            && from != to
        {
            let step = p.steps.remove(from);
            p.steps.insert(to, step);
        }
        self.render();
        if let Some(row) = self.cards.row_at_index(to as i32) {
            row.grab_focus();
        }
    }
    fn set_action(&self, index: usize, action: Pick) {
        if let Some(p) = self.state.borrow_mut().as_mut()
            && let Some(step) = p.steps.get_mut(index)
        {
            if action == Pick::Reword && step.message.is_none() {
                step.message = p.facts.messages.get(&step.id).cloned().or_else(|| Some(step.subject.clone()));
            }
            step.action = action;
        }
        self.render();
    }
    fn render(&self) {
        while let Some(c) = self.cards.first_child() {
            self.cards.remove(&c);
        }
        let state = self.state.borrow();
        let Some(p) = state.as_ref() else { return };
        self.title.set_text(&format!("Rebase {} onto {} · {} commits", p.facts.branch, &p.base[..p.base.len().min(7)], p.steps.len()));
        for (i, step) in p.steps.iter().enumerate() {
            let row = gtk::ListBoxRow::new();
            row.set_activatable(false);
            let card = widgets::vbox(6);
            card.add_css_class("git-rebase-card");
            if step.action == Pick::Drop {
                card.add_css_class("dropped");
            }
            let line = widgets::hbox(10);
            let grip = gtk::Image::from_icon_name("git-grip-symbolic");
            grip.add_css_class("git-grip");
            grip.set_tooltip_text(Some("Drag to reorder"));
            line.append(&grip);
            let hash = widgets::label(&step.id[..7], "git-hash");
            line.append(&hash);
            let subject = widgets::label(&step.subject, "");
            subject.set_hexpand(true);
            subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&subject);
            if p.facts.published.contains(&step.id) {
                line.append(&widgets::tag("on remote", ""));
            }
            let files = p.facts.files.get(&step.id).map_or(0, Vec::len);
            let count = widgets::label(&format!("{files} file{}", if files == 1 { "" } else { "s" }), "git-bar-note");
            line.append(&count);
            let picker = gtk::DropDown::from_strings(&ACTIONS.iter().map(|(_, l)| *l).collect::<Vec<_>>());
            super::compact_dropdown(&picker);
            picker.add_css_class("git-pick");
            picker.add_css_class(&format!("{:?}", step.action).to_lowercase());
            picker.set_selected(ACTIONS.iter().position(|(a, _)| *a == step.action).unwrap_or(0) as u32);
            picker.update_property(&[gtk::accessible::Property::Label(&format!("Action for {}", step.subject))]);
            picker.connect_selected_notify(move |d| {
                let action = ACTIONS[d.selected() as usize].0.clone();
                glib::idle_add_local_once(move || studio(|s| s.set_action(i, action)));
            });
            line.append(&picker);
            for (icon, tip, delta) in [("git-chev-u-symbolic", "Move up (Alt+Up)", -1i32), ("git-chev-d-symbolic", "Move down (Alt+Down)", 1)] {
                let b = widgets::icon_button(icon, tip);
                let to = i as i32 + delta;
                b.set_sensitive(to >= 0 && (to as usize) < p.steps.len());
                b.connect_clicked(move |_| studio(|s| s.move_step(i, to as usize)));
                line.append(&b);
            }
            card.append(&line);
            if step.action == Pick::Reword {
                let editor = gtk::TextView::new();
                editor.set_wrap_mode(gtk::WrapMode::WordChar);
                editor.set_monospace(true);
                editor.add_css_class("git-message");
                editor.set_top_margin(6);
                editor.set_bottom_margin(6);
                editor.set_left_margin(8);
                editor.buffer().set_text(step.message.as_deref().unwrap_or_default());
                editor.update_property(&[gtk::accessible::Property::Label("New commit message")]);
                editor.buffer().connect_changed(move |b| {
                    let text = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
                    studio(|s| {
                        if let Some(p) = s.state.borrow_mut().as_mut()
                            && let Some(step) = p.steps.get_mut(i)
                        {
                            step.message = Some(text);
                        }
                        s.render_preview();
                    })
                });
                card.append(&editor);
            }
            row.set_child(Some(&card));
            // Drag a card onto another to move it there.
            let source = gtk::DragSource::new();
            source.set_actions(gdk::DragAction::MOVE);
            source.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&(i as u32).to_value())));
            row.add_controller(source);
            let target = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
            target.connect_drop(move |_, value, _, _| {
                if let Ok(from) = value.get::<u32>() {
                    glib::idle_add_local_once(move || studio(|s| s.move_step(from as usize, i)));
                    return true;
                }
                false
            });
            row.add_controller(target);
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(move |_, key, _, mods| {
                if mods.contains(gdk::ModifierType::ALT_MASK) && matches!(key, gdk::Key::Up | gdk::Key::Down) {
                    let to = if key == gdk::Key::Up { i.checked_sub(1) } else { Some(i + 1) };
                    if let Some(to) = to {
                        glib::idle_add_local_once(move || studio(|s| s.move_step(i, to)));
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            row.add_controller(keys);
            row.set_focusable(true);
            self.cards.append(&row);
        }
        drop(state);
        self.render_preview();
    }
    fn render_preview(&self) {
        while let Some(c) = self.preview.first_child() {
            self.preview.remove(&c);
        }
        let state = self.state.borrow();
        let Some(p) = state.as_ref() else { return };
        let add = |text: &str, class: &str| {
            let l = widgets::label(text, class);
            l.set_wrap(true);
            l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            self.preview.append(&l);
        };
        let proposed = advanced::proposed(&p.steps);
        add("Proposed history", "git-insp-title");
        add(&format!("Oldest first · {} commit{} from {}", proposed.len(), if proposed.len() == 1 { "" } else { "s" }, p.original.len()), "git-insp-sub");
        for c in &proposed {
            let folded = if c.ids.len() > 1 { format!(" (+{} folded in)", c.ids.len() - 1) } else { String::new() };
            let mark = match c.action {
                Pick::Edit => " · stops to edit",
                Pick::Split => " · stops to split",
                Pick::Reword => " · new message",
                _ => "",
            };
            add(&format!("{}{folded}{mark}", c.subject), "");
        }
        let dropped = p.steps.iter().filter(|s| s.action == Pick::Drop).count();
        if dropped > 0 {
            add(&format!("{dropped} commit{} dropped", if dropped == 1 { "" } else { "s" }), "git-insp-note");
        }
        let reordered = p.steps.iter().map(|s| &s.id).ne(p.original.iter().map(|s| &s.id));
        let changed = reordered || p.steps.iter().any(|s| s.action != Pick::Pick);
        add("ORIGINAL", "git-insp-group");
        add(&format!("{} commits on {}, oldest {} … newest {}", p.original.len(), p.facts.branch, &p.original[0].id[..8], &p.original[p.original.len() - 1].id[..8]), "git-insp-note");
        let published = p.steps.iter().filter(|s| p.facts.published.contains(&s.id)).count();
        if published > 0 {
            self.preview.append(&widgets::banner(
                &format!(
                    "{published} of these commits {} already on a remote. Rewriting them means force-pushing later, and anyone who has them must reset or rebase. Nyx Git won't force-push for you.",
                    if published == 1 { "is" } else { "are" }
                ),
                true,
            ));
        }
        let warnings = advanced::reorder_warnings(&p.original, &p.steps, &p.facts.files);
        if !warnings.is_empty() {
            add("LIKELY CONFLICTS", "git-insp-group");
            for w in &warnings {
                add(w, "git-warning");
            }
            add("A preview can't promise either way: Git may stop for you to resolve, or may not.", "git-insp-note");
        }
        if !p.facts.other_refs.is_empty() {
            add("OTHER REFERENCES", "git-insp-group");
            let names: Vec<_> = p.facts.other_refs.iter().map(|(n, _)| n.as_str()).collect();
            add(&format!("{} point into this range and stay on the old commits.", names.join(", ")), "git-insp-note");
        }
        add("RECOVERY", "git-insp-group");
        add("Nyx Git saves a recovery point at the current tip first. Abort during the rebase returns the branch to where it was.", "git-insp-note");
        if !changed {
            add("The plan matches the original history; there's nothing to rebase yet.", "git-insp-note");
        }
    }
    fn start(&self) {
        let state = self.state.borrow();
        let Some(p) = state.as_ref() else { return };
        let Some(first) = p.steps.iter().find(|s| s.action != Pick::Drop) else {
            if let Some(ui) = current() {
                ui.toast("Keep at least one commit, or reset to the base from History instead.");
            }
            return;
        };
        if !first.action.keeps() {
            if let Some(ui) = current() {
                ui.toast("The first kept commit can't be squashed or fixed up: there's nothing above it to join.");
            }
            return;
        }
        if let Some(s) = p.steps.iter().find(|s| s.action == Pick::Reword && s.message.as_ref().is_none_or(|m| m.trim().is_empty())) {
            if let Some(ui) = current() {
                ui.toast(&format!("Enter a message for {} first.", &s.id[..8]));
            }
            return;
        }
        let (root, base, steps) = (p.root.clone(), p.base.clone(), p.steps.clone());
        drop(state);
        match std::env::current_exe() {
            Ok(editor) => super::advanced::prepare_interactive(root, base, steps, editor),
            Err(e) => {
                if let Some(ui) = current() {
                    ui.error(&format!("{e}"));
                }
            }
        }
    }
}
