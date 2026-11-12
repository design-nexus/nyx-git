mod dialog_dismissal;
mod app;
mod control;
mod prefs;
mod theme;
mod ui;
use gtk::{gio, glib, prelude::*};
fn main() -> glib::ExitCode {
    if let Some(code) = nyx_git::core::advanced::sequence_editor().or_else(nyx_git::core::advanced::message_editor) {
        return glib::ExitCode::from(code as u8);
    }
    let id = std::env::var("NYX_GIT_APP_ID").unwrap_or_else(|_| {
        if std::env::var_os("NYX_GIT_SNAPSHOT").is_some() {
            "io.github.design_nexus.nyx.Git.Preview".into()
        } else {
            "io.github.design_nexus.nyx.Git".into()
        }
    });
    nyx_core::control::describe_if_asked(&control::description(&id));
    nyx_core::app::init(nyx_core::app::App { slug: "nyx-git", name: "Git" });
    nyx_git::core::journal::enable();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "nyx_git=info".into()))
        .init();
    // D-Bus starts Git with --gapplication-service for a call from nyx, Glimpse or Nyx
    // Assistant: no window, and it quits once idle. GLib won't parse the flag for apps
    // that handle their own command line.
    let mut args: Vec<String> = std::env::args().collect();
    let service = args.iter().any(|a| a == "--gapplication-service");
    args.retain(|a| a != "--gapplication-service");
    if std::env::args_os().any(|a| a == "--help" || a == "-h") {
        println!("Usage: nyx-git [REPOSITORY…]\nOpen a repository, a linked worktree, or a folder inside one.");
        return glib::ExitCode::SUCCESS;
    }
    let mut flags = gio::ApplicationFlags::HANDLES_COMMAND_LINE;
    if service {
        flags |= gio::ApplicationFlags::IS_SERVICE;
    }
    // Developer aid: NYX_GIT_APP_ID gives a separate test instance (a copy on a virtual
    // monitor), so it doesn't hand its arguments to the copy that's open.
    let application = gtk::Application::builder().application_id(id).flags(flags).build();
    application.connect_startup(move |application| {
        control::serve(application);
        if service {
            application.set_inactivity_timeout(10_000);
        }
        if !nyx_theme::reopens_last_page() {
            prefs::update(|p| {
                p.workspace.open.clear();
                p.workspace.active = None;
            });
        }
    });
    application.connect_command_line(|application, command| {
        let ui = ui::window::present(application);
        let paths = command.arguments().iter().skip(1).filter_map(|p| command.create_file_for_arg(p).path()).collect::<Vec<_>>();
        if !paths.is_empty() {
            for path in paths {
                ui.open(path);
            }
        } else if ui.snapshot.borrow().is_none()
            && !ui.busy.get()
            && nyx_theme::reopens_last_page()
            && let Some(path) = {
                let p = prefs::get();
                p.workspace.active.or(p.last_repository)
            }
        {
            ui.open(path);
        }
        glib::ExitCode::SUCCESS
    });
    application.connect_shutdown(|_| {
        if let Some(ui) = ui::window::current() {
            ui.save_repository();
        }
        prefs::flush();
    });
    application.run_with_args(&args)
}
