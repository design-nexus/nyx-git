use super::window::{Ui, current};
use crate::prefs;
use gtk::{glib, prelude::*};
use nyx_git::core::workspace::RepositoryState;
use nyx_ui::widgets;
use std::{
    path::{Path, PathBuf},
    rc::Rc,
};
impl Ui {
    pub fn rebuild_tabs(&self) {
        let workspace = prefs::get().workspace;
        let active = self.root();
        if !self.tabs_changed(&workspace.open, active.as_ref()) {
            return;
        }
        let tabs: Vec<nyx_ui::tabs::Tab> = workspace
            .open
            .iter()
            .map(|root| nyx_ui::tabs::Tab {
                title: root.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                icon: Some("git-folder-symbolic".into()),
                tooltip: Some(format!("{}\nCtrl+W closes it; its commit draft is kept", nyx_core::paths::pretty(root))),
                dirty: false,
            })
            .collect();
        let index = active.as_ref().and_then(|a| workspace.open.iter().position(|r| r == a)).unwrap_or(0);
        self.tabs.root.set_visible(tabs.len() > 1);
        self.tabs.set_tabs(&tabs, index);
    }
    pub fn save_repository(&self) {
        if self.restoring.get() {
            return;
        }
        let Some(root) = self.root() else { return };
        let position = self.split.position();
        let narrow = self.split.orientation() == gtk::Orientation::Vertical;
        let view = self.view_stack.visible_child_name().unwrap_or_else(|| "history".into()).to_string();
        let view = if super::pages::is_studio(&view) { "changes".to_string() } else { view };
        let zoom = self.graph.zoom();
        let message = self.workflow.message();
        let amend = self.workflow.amend.is_active();
        // The divider means something only on a page with the details pane.
        let shown = self.detail_scroll.is_visible();
        prefs::update(|p| {
            let entry = p.workspace.entry(&root);
            match (shown, narrow) {
                (true, true) => entry.narrow_position = position,
                (true, false) => entry.details_position = position,
                _ => {}
            }
            entry.view = view;
            entry.zoom = zoom;
            entry.message = message;
            entry.amend = amend;
        });
    }
    pub fn restore_repository(&self, root: &Path) {
        if self.root().as_deref() == Some(root) {
            return;
        }
        self.save_repository();
        let p = prefs::get();
        let state = p.workspace.repositories.iter().find(|r| r.root == root).cloned().unwrap_or_else(|| RepositoryState {
            details_position: p.details_position,
            zoom: p.graph_zoom,
            ..Default::default()
        });
        self.restoring.set(true);
        self.search.set_text("");
        self.graph.search.set_text("");
        self.workflow.category.set_selected(0);
        self.workflow.set_message(&state.message);
        self.workflow.amend.set_active(state.amend);
        self.split.set_position(
            if self.split.orientation() == gtk::Orientation::Vertical { state.narrow_position.max(200) } else { state.details_position.max(100) },
        );
        self.show_view(&state.view);
        self.graph.restore_zoom(state.zoom);
        self.restoring.set(false);
    }
    pub fn close_repository(self: &Rc<Self>, root: &Path) {
        if self.busy.get() && !self.quiet.get() {
            self.toast("Wait for the repository to finish loading before closing a tab.");
            return;
        }
        // A background status read of the closing repository is dropped when it returns.
        self.loads.set(self.loads.get() + 1);
        self.save_repository();
        let active = self.root().as_deref() == Some(root);
        let mut next = None;
        prefs::update(|p| {
            next = p.workspace.close(root);
            if active {
                p.last_repository = next.clone();
            }
        });
        if active {
            if let Some(next) = next {
                self.open(next);
            } else {
                self.restoring.set(true);
                *self.snapshot.borrow_mut() = None;
                self.graph.close();
                self.sidebar.clear();
                self.monitor.clear();
                self.workflow.select(None, false);
                self.workflow.set_message("");
                self.workflow.amend.set_active(false);
                self.workflow.diff.set_text("");
                self.workflow.sensitive(false);
                self.workflow.operation_status(None, false);
                self.files.remove_all();
                self.pages.set_visible_child_name("empty");
                self.show_starts(true);
                self.empty.set_text("Open a repository to view working changes.");
                self.header.show_repository(None, "");
                self.header.show_branch(None, None, "");
                nyx_ui::chrome::set_title(&self.window, "Git", "");
                self.restoring.set(false);
                self.update_readout();
                self.update_sync();
            }
        }
        self.rebuild_tabs();
        self.rebuild_navigation();
    }
    pub fn next_repository(self: &Rc<Self>, backwards: bool) {
        if let Some(root) = prefs::get().workspace.next(backwards) {
            self.open(root);
        }
    }
}
pub fn external(root: PathBuf, kind: &'static str) {
    nyx_core::cmd::background(
        move || -> anyhow::Result<()> {
            anyhow::ensure!(root.is_dir(), "The repository directory is unavailable");
            let (program, args) = match kind {
                "folder" => ("xdg-open".into(), vec![root.as_os_str().to_os_string()]),
                "editor" if nyx_core::cmd::present("omarchy-launch-editor") => {
                    ("omarchy-launch-editor".into(), vec![root.as_os_str().to_os_string()])
                }
                "terminal" if nyx_core::cmd::present("xdg-terminal-exec") => {
                    ("xdg-terminal-exec".into(), vec!["--dir".into(), root.as_os_str().into()])
                }
                _ => {
                    let variable = if kind == "editor" { "VISUAL" } else { "TERMINAL" };
                    let command = std::env::var(variable)
                        .or_else(|_| if kind == "editor" { std::env::var("EDITOR") } else { Err(std::env::VarError::NotPresent) })
                        .map_err(|_| {
                            anyhow::anyhow!("No preferred {kind} configured. Set {variable} or use Omarchy defaults.")
                        })?;
                    let mut words = glib::shell_parse_argv(command)?.into_iter();
                    let program = words.next().ok_or_else(|| anyhow::anyhow!("Empty application command"))?;
                    let mut args: Vec<_> = words.collect();
                    if kind == "editor" {
                        args.push(root.as_os_str().into());
                    }
                    (program, args)
                }
            };
            let mut child =
                std::process::Command::new(program).args(args).current_dir(root).stdin(std::process::Stdio::null()).spawn()?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        },
        |result| {
            if let Err(e) = result
                && let Some(ui) = current()
            {
                ui.error(&format!("Could not open the application: {e:#}"));
            }
        },
    );
}
pub fn catalog() {
    let Some(ui) = current() else { return };
    ui.save_repository();
    let p = prefs::get();
    let mut paths = p.recent.clone();
    paths.extend(p.workspace.open.clone());
    paths.extend(p.workspace.repositories.iter().filter(|r| r.favorite || !r.group.is_empty()).map(|r| r.root.clone()));
    paths.sort();
    paths.dedup();
    let (dialog, body) = widgets::dialog("Repositories", 700);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search paths or groups"));
    body.append(&search);
    let rows = widgets::vbox(0);
    rows.add_css_class("group-list");
    let mut filters = Vec::new();
    for path in paths {
        let state = p.workspace.repositories.iter().find(|r| r.root == path).cloned().unwrap_or_default();
        let actions = widgets::hbox(6);
        let open = gtk::Button::with_label("Open");
        let (root, d) = (path.clone(), dialog.clone());
        open.connect_clicked(move |_| {
            d.close();
            if let Some(ui) = current() {
                ui.open(root.clone());
            }
        });
        actions.append(&open);
        let star = gtk::ToggleButton::new();
        star.set_icon_name("starred-symbolic");
        star.add_css_class("flat");
        star.set_active(state.favorite);
        star.set_tooltip_text(Some("Favorite: pinned at the top of the sidebar"));
        let root = path.clone();
        star.connect_toggled(move |b| {
            prefs::update(|p| p.workspace.entry(&root).favorite = b.is_active());
            if let Some(ui) = current() {
                ui.rebuild_navigation();
            }
        });
        actions.append(&star);
        let root = path.clone();
        let d = dialog.clone();
        let group = state.group.clone();
        actions.append(&super::menu_button("view-more-symbolic", None, "More", move || {
            let (g, folder, editor, terminal, remove) = (root.clone(), root.clone(), root.clone(), root.clone(), root.clone());
            let (d, current_group) = (d.clone(), group.clone());
            vec![
                super::item("Group…", move || {
                    d.close();
                    let root = g.clone();
                    widgets::ask_text(
                        "Repository group",
                        "Repositories in a group are listed together in the sidebar. Leave it empty to remove the group.",
                        &current_group,
                        "Save",
                        move |group| {
                            prefs::update(|p| p.workspace.entry(&root).group = group.trim().to_string());
                            if let Some(ui) = current() {
                                ui.rebuild_navigation();
                            }
                            catalog();
                        },
                    );
                }),
                super::Item::Separator,
                super::item("Open folder", move || external(folder.clone(), "folder")),
                super::item("Open in editor", move || external(editor.clone(), "editor")),
                super::item("Open in terminal", move || external(terminal.clone(), "terminal")),
                super::Item::Separator,
                super::item("Remove from recent", move || {
                    prefs::update(|p| p.recent.retain(|r| r != &remove));
                    if let Some(ui) = current() {
                        ui.rebuild_navigation();
                    }
                }),
            ]
        }));
        let description = format!(
            "{}{}",
            path.display(),
            if state.group.is_empty() { String::new() } else { format!("\nGroup: {}", state.group) }
        );
        let row = widgets::row(
            &path.file_name().unwrap_or_default().to_string_lossy(),
            &glib::markup_escape_text(&description),
            Some(actions.upcast_ref()),
        );
        filters.push((row.clone(), format!("{} {}", path.display(), state.group).to_lowercase()));
        rows.append(&row);
    }
    if filters.is_empty() {
        rows.append(&widgets::row("No repositories yet", "Open, clone or discover repositories to list them here.", None));
    }
    search.connect_search_changed(move |s| {
        let query = s.text().to_lowercase();
        for (row, text) in &filters {
            row.set_visible(text.contains(&query));
        }
    });
    let scroll = gtk::ScrolledWindow::builder()
        .child(&rows)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .min_content_height(280)
        .max_content_height(520)
        .propagate_natural_height(true)
        .build();
    body.append(&scroll);
    let controls = widgets::hbox(6);
    let open = gtk::Button::with_label("Open repository…");
    let d = dialog.clone();
    open.connect_clicked(move |_| {
        d.close();
        super::window::choose_folder(false);
    });
    controls.append(&open);
    let spacer = widgets::hbox(0);
    spacer.set_hexpand(true);
    controls.append(&spacer);
    let close = gtk::Button::with_label("Close");
    let d = dialog.clone();
    close.connect_clicked(move |_| d.close());
    controls.append(&close);
    body.append(&controls);
    dialog.present();
    search.grab_focus();
}
