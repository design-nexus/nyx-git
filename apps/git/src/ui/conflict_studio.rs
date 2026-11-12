//! Conflict Studio: every conflicted file of the operation in progress, with Base, Ours and
//! Theirs beside an editable Result. Each conflict can take a side (or both); a file is staged
//! only when "Mark resolved" is chosen and no markers remain. Files that aren't text, or that
//! one side deleted, get explicit whole-file choices instead.
use super::{diff_view::DiffView, window::current, workflow::confirm};
use gtk::prelude::*;
use nyx_git::core::{
    conflicts::{self, Choice, Conflict, Resolution, Shape},
    operations::Operation,
};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

pub struct ConflictStudio {
    pub root: gtk::Box,
    files: gtk::ListBox,
    title: gtk::Label,
    note: gtk::Label,
    remaining: gtk::Label,
    stack: gtk::Stack,
    base: Rc<DiffView>,
    ours: Rc<DiffView>,
    theirs: Rc<DiffView>,
    result: Rc<DiffView>,
    sides: gtk::Paned,
    whole: gtk::Box,
    conflict: RefCell<Option<Conflict>>,
    focused: Cell<bool>,
    finish: gtk::Box,
}
fn studio(run: impl FnOnce(&ConflictStudio)) {
    if let Some(ui) = current() {
        run(&ui.destinations.conflicts);
    }
}
fn text_of(side: &Option<conflicts::Side>) -> String {
    side.as_ref().map(|s| String::from_utf8_lossy(&s.bytes).into_owned()).unwrap_or_else(|| "[No file on this side]".into())
}
fn pane(title: &str, view: &DiffView) -> gtk::Box {
    let b = widgets::vbox(4);
    b.append(&widgets::label(title, "git-insp-group"));
    b.append(&view.root);
    view.root.set_min_content_height(120);
    b
}
impl ConflictStudio {
    pub fn new() -> Rc<Self> {
        let root = widgets::vbox(0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        let bar = super::pagebar::PageBar::new();
        let back = super::pagebar::action("git-chev-l-symbolic", "Changes", "Back to Changes", || {
            if let Some(ui) = current() {
                ui.show_view("changes");
            }
        });
        bar.start(&back);
        let title = widgets::label("Conflict Studio", "git-bar-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        bar.start(&title);
        let remaining = widgets::label("", "git-bar-note");
        bar.end(&remaining);
        let layout = gtk::ToggleButton::with_label("Result only");
        layout.add_css_class("git-bar-action");
        layout.add_css_class("git-toggle");
        layout.set_tooltip_text(Some("Show only the result, or Base, Ours and Theirs above it"));
        layout.connect_toggled(|b| studio(|s| s.set_focused(b.is_active())));
        bar.end(&layout);
        root.append(&bar.root);
        let files = gtk::ListBox::new();
        files.add_css_class("git-palette");
        files.update_property(&[gtk::accessible::Property::Label("Conflicted files")]);
        files.connect_row_activated(|_, row| {
            let file = PathBuf::from(row.widget_name().as_str());
            studio(|s| s.open(file));
        });
        let files_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .min_content_width(200)
            .child(&files)
            .build();
        let (base, ours, theirs, result) = (Rc::new(DiffView::new()), Rc::new(DiffView::new()), Rc::new(DiffView::new()), Rc::new(DiffView::new()));
        if let Some(view) = result.root.child().and_downcast::<sourceview5::View>() {
            view.set_editable(true);
            view.upcast_ref::<gtk::TextView>().update_property(&[gtk::accessible::Property::Label("Editable result")]);
        }
        let three = widgets::hbox(6);
        three.set_homogeneous(true);
        three.append(&pane("BASE", &base));
        three.append(&pane("OURS · stage 2", &ours));
        three.append(&pane("THEIRS · stage 3", &theirs));
        let result_box = widgets::vbox(6);
        let tools = widgets::hbox(6);
        tools.add_css_class("git-diffbar");
        tools.append(&widgets::label("RESULT", "git-insp-group"));
        let spacer = widgets::hbox(0);
        spacer.set_hexpand(true);
        tools.append(&spacer);
        for (icon, tip, step) in [("git-chev-u-symbolic", "Previous conflict (Alt+Up)", -1), ("git-chev-d-symbolic", "Next conflict (Alt+Down)", 1)] {
            let b = widgets::bar_button(icon, tip);
            b.connect_clicked(move |_| studio(|s| s.jump(step)));
            tools.append(&b);
        }
        // One choice per conflict: a segmented row, as a single control.
        let choices = widgets::hbox(0);
        choices.add_css_class("segmented");
        choices.add_css_class("git-segments");
        for (label, choice) in [("Ours", Choice::Ours), ("Theirs", Choice::Theirs), ("Both", Choice::Both), ("Base", Choice::Base)] {
            let b = gtk::Button::with_label(label);
            b.add_css_class("segment");
            b.set_tooltip_text(Some(match choice {
                Choice::Ours => "Take our side of the conflict at the cursor",
                Choice::Theirs => "Take their side of the conflict at the cursor",
                Choice::Both => "Keep ours, then theirs, for the conflict at the cursor",
                Choice::Base => "Go back to the common base for the conflict at the cursor (diff3 markers only)",
            }));
            b.connect_clicked(move |_| studio(|s| s.choose(choice)));
            choices.append(&b);
        }
        tools.append(&choices);
        result_box.append(&tools);
        result.root.set_min_content_height(200);
        result_box.append(&result.root);
        let actions = widgets::hbox(6);
        actions.set_halign(gtk::Align::End);
        actions.set_margin_top(8);
        actions.set_margin_end(16);
        actions.set_margin_bottom(12);
        let reset = gtk::Button::with_label("Start over");
        reset.set_tooltip_text(Some("Put the working file's contents back in the result"));
        reset.connect_clicked(|_| studio(|s| s.reload_result()));
        actions.append(&reset);
        let delete = gtk::Button::with_label("Resolve as deletion…");
        delete.connect_clicked(|_| studio(|s| s.whole_file(Resolution::Delete)));
        actions.append(&delete);
        let save = gtk::Button::with_label("Mark resolved…");
        save.add_css_class("suggested-action");
        save.set_tooltip_text(Some("Save the result and stage it. Refused while conflict markers remain."));
        save.connect_clicked(|_| studio(|s| s.save()));
        actions.append(&save);
        result_box.append(&actions);
        let sides = gtk::Paned::new(gtk::Orientation::Vertical);
        sides.set_start_child(Some(&three));
        sides.set_end_child(Some(&result_box));
        sides.set_shrink_start_child(true);
        sides.set_shrink_end_child(false);
        sides.set_vexpand(true);
        super::layout::persist(&sides, "conflicts.sides", 220, false);
        let whole = widgets::vbox(10);
        whole.add_css_class("git-insp");
        let stack = gtk::Stack::new();
        stack.add_named(&sides, Some("text"));
        stack.add_named(&whole, Some("whole"));
        let empty = widgets::vbox(10);
        empty.set_valign(gtk::Align::Center);
        empty.set_halign(gtk::Align::Center);
        let finish = widgets::vbox(10);
        empty.append(&finish);
        stack.add_named(&empty, Some("done"));
        let note = widgets::label("", "dim-label");
        note.set_wrap(true);
        note.add_css_class("git-details");
        let center = widgets::vbox(0);
        center.append(&note);
        center.append(&stack);
        center.set_hexpand(true);
        let body = gtk::Paned::new(gtk::Orientation::Horizontal);
        body.set_start_child(Some(&files_scroll));
        body.set_end_child(Some(&center));
        body.set_resize_start_child(false);
        body.set_shrink_start_child(false);
        body.set_vexpand(true);
        super::layout::persist(&body, "conflicts.files", 220, false);
        root.append(&body);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(|_, key, _, mods| {
            if mods.contains(gtk::gdk::ModifierType::ALT_MASK) && matches!(key, gtk::gdk::Key::Up | gtk::gdk::Key::Down) {
                studio(|s| s.jump(if key == gtk::gdk::Key::Up { -1 } else { 1 }));
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        root.add_controller(keys);
        let me = Rc::new(Self {
            root,
            files,
            title,
            note,
            remaining,
            stack,
            base,
            ours,
            theirs,
            result,
            sides,
            whole,
            conflict: RefCell::new(None),
            focused: Cell::new(false),
            finish,
        });
        // Unresolved markers in the result are tinted like deletions.
        let b = me.result.buffer.clone();
        me.result.buffer.connect_changed(move |_| {
            b.remove_tag_by_name("deletion", &b.start_iter(), &b.end_iter());
            let text = b.text(&b.start_iter(), &b.end_iter(), false);
            for (line, l) in text.lines().enumerate() {
                if conflicts::has_markers(l)
                    && let Some(start) = b.iter_at_line(line as i32)
                {
                    let mut end = start;
                    end.forward_to_line_end();
                    b.apply_tag_by_name("deletion", &start, &end);
                }
            }
            studio(|s| s.count());
        });
        let weak = Rc::downgrade(&me);
        nyx_theme::subscribe(&me.root, move || {
            if let Some(s) = weak.upgrade() {
                for v in [&s.base, &s.ours, &s.theirs, &s.result] {
                    v.recolor();
                }
            }
        });
        me
    }
    fn set_focused(&self, on: bool) {
        self.focused.set(on);
        if let Some(child) = self.sides.start_child() {
            child.set_visible(!on);
        }
    }
    /// The conflicted files of the open repository, from its status.
    fn conflicted() -> Vec<PathBuf> {
        current()
            .and_then(|ui| ui.snapshot.borrow().as_ref().map(|s| s.status.files.iter().filter(|f| f.conflicted).map(|f| f.path.clone()).collect()))
            .unwrap_or_default()
    }
    /// Show the studio, at `file` or the first conflicted file.
    pub fn show(&self, file: Option<PathBuf>) {
        let Some(ui) = current() else { return };
        ui.show_view("conflicts");
        self.refresh_files();
        let files = Self::conflicted();
        match file.or_else(|| files.first().cloned()) {
            Some(f) => self.open(f),
            None => self.done(),
        }
    }
    fn refresh_files(&self) {
        while let Some(c) = self.files.first_child() {
            self.files.remove(&c);
        }
        let open = self.conflict.borrow().as_ref().map(|c| c.file.clone());
        for f in Self::conflicted() {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&f.to_string_lossy());
            let l = widgets::label(&f.to_string_lossy(), "");
            l.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            row.set_child(Some(&l));
            self.files.append(&row);
            if open.as_ref() == Some(&f) {
                self.files.select_row(Some(&row));
            }
        }
        let n = Self::conflicted().len();
        self.remaining.set_text(&format!("{n} file{} unresolved", if n == 1 { "" } else { "s" }));
    }
    fn open(&self, file: PathBuf) {
        let Some(root) = current().and_then(|ui| ui.root()) else { return };
        nyx_core::cmd::background(
            move || conflicts::load(&root, &file),
            |result| {
                studio(|s| match result {
                    Ok(c) => s.present(c),
                    Err(e) => {
                        s.note.set_text(&format!("{e:#}"));
                        s.stack.set_visible_child_name("whole");
                        while let Some(c) = s.whole.first_child() {
                            s.whole.remove(&c);
                        }
                    }
                })
            },
        );
    }
    fn present(&self, c: Conflict) {
        self.title.set_text(&format!("Conflict Studio · {}", c.file.display()));
        let progress = current().and_then(|ui| ui.snapshot.borrow().as_ref().and_then(|s| s.progress)).map(|p| p.name()).unwrap_or("Merge");
        let sides = match progress {
            "Rebase" => "Ours is the branch being rebased onto; theirs is the commit being replayed.",
            _ => "Ours is your current branch; theirs is what's coming in.",
        };
        let shape = c.shape();
        self.note.set_text(&format!(
            "{progress} in progress. {sides} {}",
            match shape {
                Shape::Content => "Choose a side for each conflict or edit the result, then Mark resolved.",
                Shape::AddedByBoth => "Both sides added this file. Combine them in the result, then Mark resolved.",
                Shape::DeletedByUs => "Our side deleted this file; theirs changed it. Keep their version or delete it.",
                Shape::DeletedByThem => "Their side deleted this file; ours changed it. Keep our version or delete it.",
                Shape::Binary => "This file isn't text, so it can't be merged line by line. Choose a whole side or delete it.",
            }
        ));
        while let Some(w) = self.whole.first_child() {
            self.whole.remove(&w);
        }
        if matches!(shape, Shape::Content | Shape::AddedByBoth) {
            self.base.set_text(&text_of(&c.base));
            self.ours.set_text(&text_of(&c.ours));
            self.theirs.set_text(&text_of(&c.theirs));
            self.result.set_text(c.working.as_ref().and_then(|b| std::str::from_utf8(b).ok()).unwrap_or_default());
            self.stack.set_visible_child_name("text");
        } else {
            let choices: Vec<(&str, Resolution, bool)> = vec![
                ("Keep our version", Resolution::Ours, c.ours.is_some()),
                ("Keep their version", Resolution::Theirs, c.theirs.is_some()),
                ("Delete the file", Resolution::Delete, true),
            ];
            for (label, resolution, available) in choices {
                if !available {
                    continue;
                }
                let b = gtk::Button::with_label(label);
                b.set_halign(gtk::Align::Start);
                b.connect_clicked(move |_| studio(|s| s.whole_file(resolution.clone())));
                self.whole.append(&b);
            }
            let sizes = |s: &Option<conflicts::Side>| s.as_ref().map_or("absent".to_string(), |s| format!("{} bytes", s.bytes.len()));
            self.whole.append(&widgets::label(
                &format!("Ours: {} · Theirs: {} · Base: {}", sizes(&c.ours), sizes(&c.theirs), sizes(&c.base)),
                "dim-label",
            ));
            self.stack.set_visible_child_name("whole");
        }
        *self.conflict.borrow_mut() = Some(c);
        self.refresh_files();
        self.count();
        self.jump(1);
    }
    fn result_text(&self) -> String {
        let b = &self.result.buffer;
        b.text(&b.start_iter(), &b.end_iter(), false).into()
    }
    fn count(&self) {
        if self.conflict.borrow().is_none() {
            return;
        }
        let n = conflicts::blocks(&self.result_text()).len();
        let files = Self::conflicted().len();
        self.remaining.set_text(&format!(
            "{n} conflict{} left here · {files} file{} unresolved",
            if n == 1 { "" } else { "s" },
            if files == 1 { "" } else { "s" }
        ));
    }
    /// The conflict block at (or after) the cursor.
    fn block_at_cursor(&self) -> Option<usize> {
        let line = self.result.buffer.iter_at_offset(self.result.buffer.cursor_position()).line() as usize;
        let blocks = conflicts::blocks(&self.result_text());
        blocks.iter().position(|b| line <= b.end)
    }
    fn jump(&self, step: i32) {
        let blocks = conflicts::blocks(&self.result_text());
        if blocks.is_empty() {
            return;
        }
        let line = self.result.buffer.iter_at_offset(self.result.buffer.cursor_position()).line() as usize;
        let target = if step > 0 {
            blocks.iter().find(|b| b.start > line).or(blocks.first())
        } else {
            blocks.iter().rev().find(|b| b.start < line).or(blocks.last())
        };
        if let Some(b) = target
            && let Some(mut iter) = self.result.buffer.iter_at_line(b.start as i32)
        {
            self.result.buffer.place_cursor(&iter);
            if let Some(view) = self.result.root.child().and_downcast::<sourceview5::View>() {
                view.scroll_to_iter(&mut iter, 0.15, false, 0.0, 0.0);
            }
        }
    }
    fn choose(&self, choice: Choice) {
        let Some(index) = self.block_at_cursor() else {
            if let Some(ui) = current() {
                ui.toast("No conflict at or after the cursor.");
            }
            return;
        };
        match conflicts::apply(&self.result_text(), index, choice) {
            Some(text) => {
                self.result.buffer.set_text(&text);
                self.jump(1);
            }
            None => {
                if let Some(ui) = current() {
                    ui.toast("This conflict has no base section. Set merge.conflictStyle to diff3 to see bases in markers.");
                }
            }
        }
    }
    fn reload_result(&self) {
        let working = self.conflict.borrow().as_ref().and_then(|c| c.working.clone());
        self.result.buffer.set_text(working.as_ref().and_then(|b| std::str::from_utf8(b).ok()).unwrap_or_default());
    }
    fn save(&self) {
        let text = self.result_text();
        if conflicts::has_markers(&text) {
            if let Some(ui) = current() {
                ui.toast("Conflict markers remain. Choose a side or edit each one before marking the file resolved.");
            }
            self.jump(1);
            return;
        }
        let Some(c) = self.conflict.borrow().clone() else { return };
        confirm(
            "Mark resolved?",
            "Save the result over the working file and stage it. The operation continues only when you choose Continue.",
            "Save and stage",
            move || submit(c.clone(), Resolution::Text(text.clone())),
        );
    }
    fn whole_file(&self, resolution: Resolution) {
        let Some(c) = self.conflict.borrow().clone() else { return };
        let (title, what) = match resolution {
            Resolution::Delete => ("Resolve as deletion?", "Remove the working file and stage its deletion. Its current contents are lost."),
            Resolution::Ours => ("Keep our version?", "Replace the working file with our side and stage it. Their changes to it are dropped."),
            _ => ("Keep their version?", "Replace the working file with their side and stage it. Our changes to it are dropped."),
        };
        confirm(title, what, "Resolve", move || submit(c.clone(), resolution.clone()));
    }
    /// Every file is resolved: offer to continue (or abort) the operation.
    fn done(&self) {
        self.conflict.borrow_mut().take();
        self.title.set_text("Conflict Studio");
        self.note.set_text("");
        while let Some(c) = self.finish.first_child() {
            self.finish.remove(&c);
        }
        let progress = current().and_then(|ui| ui.snapshot.borrow().as_ref().and_then(|s| s.progress));
        self.finish.append(&widgets::label("No conflicted files remain.", "section-title"));
        if let Some(p) = progress {
            self.finish.append(&widgets::label(&format!("Continue the {} when you're ready, or skip or abort it.", p.name().to_lowercase()), "dim-label"));
            let row = widgets::hbox(6);
            row.set_halign(gtk::Align::Center);
            for (label, action) in [
                ("Continue…", nyx_git::core::advanced::Action::Continue),
                ("Skip this commit…", nyx_git::core::advanced::Action::Skip),
                ("Abort…", nyx_git::core::advanced::Action::Abort),
            ] {
                if matches!(action, nyx_git::core::advanced::Action::Skip) && p == nyx_git::core::advanced::Active::Merge {
                    continue;
                }
                let b = gtk::Button::with_label(label);
                if matches!(action, nyx_git::core::advanced::Action::Continue) {
                    b.add_css_class("suggested-action");
                }
                let effects = match action {
                    nyx_git::core::advanced::Action::Continue => "Commit the resolved files and carry on with the operation.",
                    nyx_git::core::advanced::Action::Skip => "Leave out the commit that stopped and carry on with the next one.",
                    _ => "Stop the operation and return toward where it started. Resolutions made so far may be lost.",
                };
                b.connect_clicked(move |_| super::advanced::prepare(action.clone(), effects.into()));
                row.append(&b);
            }
            self.finish.append(&row);
        }
        self.stack.set_visible_child_name("done");
        self.refresh_files();
    }
}
fn submit(conflict: Conflict, resolution: Resolution) {
    let Some(ui) = current() else { return };
    let file = conflict.file.clone();
    ui.execute_at_then(conflict.root.clone(), Operation::ResolveConflict { conflict: Box::new(conflict), resolution }, move |result| {
        studio(|s| match result {
            Ok(()) => {
                let next = ConflictStudio::conflicted().into_iter().find(|f| *f != file);
                match next {
                    Some(f) => s.open(f),
                    None => s.done(),
                }
            }
            // The draft stays in the result for another try.
            Err(message) => s.note.set_text(&format!("{message}\nYour result is kept. If the repository changed, open the file again.")),
        })
    });
}
