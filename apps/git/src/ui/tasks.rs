use super::window::{Ui, current};
use gtk::{gio, glib, prelude::*};
use nyx_git::core::{jobs::Control, operations::Operation};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
#[derive(Clone)]
pub struct Record {
    pub id: u64,
    pub root: PathBuf,
    pub common: PathBuf,
    pub label: &'static str,
    pub control: Arc<Control>,
    pub started: Instant,
    pub finished: Option<Instant>,
    pub result: Option<Result<(), String>>,
    pub retry: bool,
}
impl Record {
    fn description(&self) -> String {
        let state = match &self.result {
            None => {
                if self.control.cancelled() {
                    "Stopping"
                } else {
                    "Running"
                }
            }
            Some(Ok(())) => "Completed",
            Some(Err(_)) if self.control.cancelled() => "Stopped / incomplete",
            _ => "Failed",
        };
        let elapsed = self.finished.unwrap_or_else(Instant::now).duration_since(self.started).as_secs();
        let detail = match &self.result {
            None => self.control.progress(),
            Some(Err(e)) => e.clone(),
            _ => String::new(),
        };
        format!("{}\n{state} · {elapsed}s\n{detail}", self.root.display())
    }
}
pub struct Tasks {
    pub bar: gtk::Box,
    spinner: gtk::Spinner,
    label: gtk::Label,
    stop: gtk::Button,
    pub records: RefCell<Vec<Record>>,
    next: Cell<u64>,
}
impl Tasks {
    pub fn new() -> Self {
        let bar = widgets::hbox(8);
        bar.add_css_class("git-actions");
        let spinner = gtk::Spinner::new();
        bar.append(&spinner);
        let label = widgets::label("", "dim-label");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_hexpand(true);
        bar.append(&label);
        let stop = gtk::Button::with_label("Stop");
        stop.set_tooltip_text(Some("Stop the active transfer. Partial results are retained for inspection."));
        stop.connect_clicked(|_| {
            if let Some(ui) = current() {
                let root = ui.root();
                let records = ui.tasks.records.borrow();
                if let Some(r) = records
                    .iter()
                    .filter(|r| r.result.is_none())
                    .find(|r| root.as_ref() == Some(&r.root))
                    .or_else(|| records.iter().find(|r| r.result.is_none()))
                {
                    r.control.cancel();
                }
            }
        });
        bar.append(&stop);
        bar.set_visible(false);
        Self { bar, spinner, label, stop, records: RefCell::new(Vec::new()), next: Cell::new(0) }
    }
    pub fn active(&self) -> usize {
        self.records.borrow().iter().filter(|r| r.result.is_none()).count()
    }
    pub fn busy(&self, common: &Path) -> bool {
        self.records.borrow().iter().any(|r| r.result.is_none() && r.common == common)
    }
    pub fn start(&self, root: PathBuf, common: PathBuf, operation: &Operation) -> anyhow::Result<(u64, Arc<Control>)> {
        anyhow::ensure!(self.active() < 32, "There are already 32 active operations; wait for them to finish");
        anyhow::ensure!(
            !self.busy(&common) || matches!(operation, Operation::Fetch),
            "This repository or a related worktree already has an operation running. Fetch can queue; wait before other changes."
        );
        let id = self.next.get() + 1;
        self.next.set(id);
        let control = Control::new(operation.cancellable());
        let mut records = self.records.borrow_mut();
        while records.len() >= 100 {
            if let Some(i) = records.iter().position(|r| r.result.is_some()) {
                records.remove(i);
            } else {
                break;
            }
        }
        records.push(Record {
            id,
            root,
            common,
            label: operation.label(),
            control: control.clone(),
            started: Instant::now(),
            finished: None,
            result: None,
            retry: matches!(operation, Operation::Fetch),
        });
        Ok((id, control))
    }
    pub fn finish(&self, id: u64, result: Result<(), String>) {
        if let Some(r) = self.records.borrow_mut().iter_mut().find(|r| r.id == id) {
            r.finished = Some(Instant::now());
            r.result = Some(result.map_err(|s| s.chars().take(16_384).collect()));
        }
    }
    pub fn tick(&self, root: Option<&Path>) {
        let records = self.records.borrow();
        let active = records.iter().filter(|r| r.result.is_none()).collect::<Vec<_>>();
        self.bar.set_visible(!active.is_empty());
        if active.is_empty() {
            self.spinner.stop();
            return;
        }
        self.spinner.start();
        let task = active.iter().find(|r| Some(r.root.as_path()) == root).copied().unwrap_or(active[0]);
        let progress = task.control.progress();
        self.label.set_text(&format!(
            "{} active · {} · {} · {}",
            active.len(),
            task.label,
            task.root.file_name().unwrap_or_default().to_string_lossy(),
            progress
        ));
        self.label.set_tooltip_text(Some(&task.description()));
        self.stop.set_sensitive(task.control.cancellable() && !task.control.cancelled());
    }
}
impl Ui {
    pub fn update_tasks(&self) {
        let root = self.root();
        self.tasks.tick(root.as_deref());
        let blocked = self.snapshot.borrow().as_ref().is_some_and(|s| self.tasks.busy(&s.repository.common_dir));
        // A background status read leaves everything usable; opening a repository doesn't.
        let loading = self.busy.get() && !self.quiet.get();
        let enabled = !loading && !blocked && self.snapshot.borrow().as_ref().is_some_and(|s| !s.repository.bare);
        if enabled != self.workflow.is_enabled() {
            self.workflow.sensitive(enabled);
        }
        self.header.set_loading(loading);
        self.update_sync();
    }
    pub fn task_finished(self: &std::rc::Rc<Self>, id: u64, result: Result<(), String>) {
        self.tasks.finish(id, result.clone());
        self.update_tasks();
        if !gtk::Window::list_toplevels().iter().any(|w| w.downcast_ref::<gtk::Window>().is_some_and(|w| w.is_active()))
            && std::env::var_os("NYX_GIT_SNAPSHOT").is_none()
            && let Some(record) = self.tasks.records.borrow().iter().find(|r| r.id == id)
            && let Some(app) = self.window.application()
        {
            let notification = gio::Notification::new(&format!(
                "{} {}",
                record.label,
                if result.is_ok() { "completed" } else { "needs attention" }
            ));
            notification.set_body(Some(&record.root.file_name().unwrap_or_default().to_string_lossy()));
            app.send_notification(Some("git-operation"), &notification);
        }
    }
}
pub fn show() {
    let Some(ui) = current() else { return };
    let (dialog, body) = widgets::dialog("Git operations", 720);
    let rows = widgets::vbox(0);
    rows.add_css_class("group-list");
    let mut live = Vec::new();
    for record in ui.tasks.records.borrow().iter().rev() {
        let actions = widgets::hbox(6);
        let stop = gtk::Button::with_label("Stop");
        stop.set_visible(record.result.is_none() && record.control.cancellable());
        let c = record.control.clone();
        stop.connect_clicked(move |_| {
            c.cancel();
        });
        actions.append(&stop);
        if record.retry && record.result.as_ref().is_some_and(Result::is_err) {
            let b = gtk::Button::with_label("Retry fetch");
            let root = record.root.clone();
            let d = dialog.clone();
            b.connect_clicked(move |_| {
                d.close();
                if let Some(ui) = current() {
                    ui.execute_at(root.clone(), Operation::Fetch);
                }
            });
            actions.append(&b);
        }
        let r = widgets::row(record.label, &glib::markup_escape_text(&record.description()), Some(actions.upcast_ref()));
        if let Some(label) = r.first_child().and_then(|text| text.last_child()).and_downcast::<gtk::Label>() {
            label.set_max_width_chars(64);
            live.push((record.id, label, stop));
        }
        rows.append(&r);
    }
    if ui.tasks.records.borrow().is_empty() {
        rows.append(&widgets::row(
            "No Git operations this session",
            "Transfers and mutations appear here with their results.",
            None,
        ));
    }
    body.append(
        &gtk::ScrolledWindow::builder()
            .child(&rows)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .min_content_height(220)
            .max_content_height(500)
            .propagate_natural_height(true)
            .build(),
    );
    let close = gtk::Button::with_label("Close");
    close.set_halign(gtk::Align::End);
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    body.append(&close);
    let weak = dialog.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
        if weak.upgrade().is_none_or(|d| !d.is_visible()) {
            return glib::ControlFlow::Break;
        }
        if let Some(ui) = current() {
            let records = ui.tasks.records.borrow();
            for (id, label, stop) in &live {
                if let Some(record) = records.iter().find(|r| r.id == *id) {
                    label.set_text(&record.description());
                    stop.set_sensitive(record.result.is_none() && !record.control.cancelled());
                }
            }
        }
        glib::ControlFlow::Continue
    });
    dialog.present();
}

/// Developer fixture aid: start and stop a transfer without publishing or modifying work.
pub fn preview() {
    let Some(ui) = current() else { return };
    let empty = ui.tasks.records.borrow().is_empty();
    if !empty {
        return;
    }
    ui.execute(Operation::Fetch);
    glib::timeout_add_local_once(std::time::Duration::from_millis(300), || {
        if let Some(ui) = current() {
            for r in ui.tasks.records.borrow().iter().filter(|r| r.result.is_none()) {
                r.control.cancel();
            }
        }
    });
    glib::timeout_add_local_once(std::time::Duration::from_millis(1400), show);
}
