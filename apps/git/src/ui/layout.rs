//! Pane sizes kept across sessions, by name (`flow.inspector`, `review.list`, …).
use crate::prefs;
use gtk::{glib, prelude::*};
use std::{
    cell::Cell,
    rc::Rc,
    time::Duration,
};

/// Keep `paned`'s divider where the reader leaves it. With `end`, the size of the end
/// child is kept (an inspector), so resizing the window doesn't move it.
pub fn persist(paned: &gtk::Paned, key: &'static str, default: i32, end: bool) {
    let saved = prefs::get().layout.get(key).copied().unwrap_or(default).max(80);
    let size = Rc::new(Cell::new(saved));
    let setting = Rc::new(Cell::new(false));
    let place = {
        let (size, setting) = (size.clone(), setting.clone());
        move |paned: &gtk::Paned| {
            let max = paned.max_position();
            if max <= 0 {
                return;
            }
            let position = if end { max - size.get() } else { size.get() };
            setting.set(true);
            paned.set_position(position.clamp(paned.min_position(), max));
            setting.set(false);
        }
    };
    let placed = Rc::new(Cell::new(false));
    {
        let (place, placed) = (place.clone(), placed.clone());
        paned.connect_max_position_notify(move |paned| {
            // An end pane follows the window; a start pane is placed once.
            if end || !placed.replace(true) {
                place(paned);
            }
        });
    }
    let generation = Rc::new(Cell::new(0u64));
    paned.connect_position_notify(move |paned| {
        if setting.get() || !paned.is_mapped() || paned.max_position() <= 0 {
            return;
        }
        let value = if end { paned.max_position() - paned.position() } else { paned.position() };
        size.set(value.max(80));
        let g = generation.get() + 1;
        generation.set(g);
        let generation = generation.clone();
        let size = size.clone();
        glib::timeout_add_local_once(Duration::from_millis(400), move || {
            if generation.get() == g {
                prefs::update(|p| {
                    p.layout.insert(key.into(), size.get());
                });
            }
        });
    });
}
