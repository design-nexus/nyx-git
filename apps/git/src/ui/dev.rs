//! Developer previews (`NYX_GIT_SNAPSHOT`, `NYX_GIT_PREVIEW_*`): open one dialog once a
//! repository has loaded, so a snapshot can capture it. Nothing here runs otherwise.
use gtk::prelude::*;

pub fn active() -> bool {
    std::env::var_os("NYX_GIT_SNAPSHOT").is_some()
}

fn open_titled(title: &str) -> bool {
    gtk::Window::list_toplevels()
        .iter()
        .any(|w| w.downcast_ref::<gtk::Window>().is_some_and(|w| w.title().as_deref() == Some(title)))
}

/// Called after each snapshot is applied.
pub fn after_apply(ui: &super::window::Ui) {
    let var = |name: &str| std::env::var_os(name);
    // NYX_GIT_PREVIEW_VIEW=flow (etc.) and NYX_GIT_PREVIEW_SELECT=0 show a page and select a
    // changed file once, for screenshots of a test copy's main window.
    thread_local! { static ONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    if !ONCE.with(|o| o.replace(true)) {
        if let Some(view) = var("NYX_GIT_PREVIEW_VIEW") {
            ui.show_view(&view.to_string_lossy());
        }
        if let Some(index) = var("NYX_GIT_PREVIEW_SELECT").and_then(|v| v.to_string_lossy().parse().ok()) {
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                if let Some(ui) = super::window::current() {
                    ui.select_file(index);
                }
            });
        }
    }
    if !active() {
        return;
    }
    if var("NYX_GIT_PREVIEW_JOBS").is_some() {
        super::tasks::preview();
    } else if let Some(file) = var("NYX_GIT_PREVIEW_CONFLICT") {
        super::conflict_editor::open(file.into());
    } else if let Some(file) = var("NYX_GIT_PREVIEW_FILE_HISTORY") {
        if !open_titled("File history")
            && let Some(root) = ui.root()
        {
            super::file_history::open(root, file.into(), None);
        }
    } else if var("NYX_GIT_PREVIEW_WORKTREES").is_some() {
        super::worktree_manager::show();
    } else if var("NYX_GIT_PREVIEW_SUBMODULES").is_some() {
        if !open_titled("Submodules") {
            super::submodules::show();
        }
    } else if var("NYX_GIT_PREVIEW_HOSTING").is_some() {
        super::providers::preview();
    } else if var("NYX_GIT_PREVIEW_ADVANCED").is_some() {
        super::advanced::show(None);
    }
}

/// Capture the frontmost window to the PNG named by `NYX_GIT_SNAPSHOT`, then quit. The same
/// developer aid Music uses: it captures the actual GTK frame.
pub fn capture(app: &gtk::Application, window: &gtk::ApplicationWindow) {
    let Some(path) = std::env::var_os("NYX_GIT_SNAPSHOT") else { return };
    let w = window.clone();
    let application = app.clone();
    w.set_opacity(0.01);
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(2800), move || {
        if let Some(out) = std::env::var_os("NYX_GIT_PREVIEW_STATE")
            && let Some(ui) = super::window::current()
        {
            let s = ui.snapshot.borrow();
            let focus = gtk::prelude::GtkWindowExt::focus(&ui.window);
            let state = serde_json::json!({
                "root": ui.root(),
                "changed": s.as_ref().map(|s| s.status.files.len()),
                "focus": focus.as_ref().map(|w| w.type_().name()),
                "focus_label": focus.as_ref().and_then(|w| w.downcast_ref::<gtk::Button>()).and_then(|b| b.label()).map(|s| s.to_string()),
                "tasks": ui.tasks.records.borrow().iter().map(|r| serde_json::json!({
                    "root": r.root,
                    "done": r.result.is_some(),
                    "ok": r.result.as_ref().is_some_and(Result::is_ok),
                    "stopped": r.control.cancelled(),
                })).collect::<Vec<_>>(),
            });
            let _ = std::fs::write(out, serde_json::to_vec(&state).unwrap_or_default());
        }
        let w = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .find(|w| w.is_modal() && w.is_visible())
            .unwrap_or(w.upcast());
        if let Some(child) = w.child() {
            let (width, height) = (child.width(), child.height());
            let snapshot = gtk::Snapshot::new();
            let bg = gtk::gdk::RGBA::parse(nyx_theme::palette().bg).unwrap_or(gtk::gdk::RGBA::BLACK);
            snapshot.append_color(&bg, &gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32));
            w.snapshot_child(&child, &snapshot);
            if let (Some(node), Some(renderer)) = (snapshot.to_node(), w.renderer())
                && let Err(error) = renderer.render_texture(node, None).save_to_png(std::path::PathBuf::from(&path))
            {
                eprintln!("Snapshot failed: {error}");
            }
        }
        application.quit();
    });
}
