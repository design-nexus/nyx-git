//! Ctrl+Shift+P: one box for every destination, command, repository, branch, tag,
//! worktree and tracked file. The same box narrowed to one kind: Ctrl+Alt+W worktrees,
//! Ctrl+P files. Repositories (Ctrl+K) and branches (Ctrl+Shift+B) have the header's
//! switchers.
use super::window::current;
use gtk::{gdk, glib, pango, prelude::*};
use nyx_git::core::navigation::{Action, Kind as Group};
use nyx_ui::widgets;
use std::{cell::RefCell, path::PathBuf, rc::Rc};

/// Most results shown at once.
const LIMIT: usize = 60;
/// Tracked files listed for Ctrl+P.
const FILES: usize = 50_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    All,
    Worktrees,
    Files,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
enum Kind {
    Page,
    Command,
    Repository,
    Branch,
    Worktree,
    Tag,
    File,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Page => "Page",
            Self::Command => "Command",
            Self::Repository => "Repository",
            Self::Branch => "Branch",
            Self::Worktree => "Worktree",
            Self::Tag => "Tag",
            Self::File => "File",
        }
    }
}
/// A second choice for an entry (Alt+Enter): what it does, in words, and doing it.
type Alternate = (&'static str, Rc<dyn Fn()>);
#[derive(Clone)]
struct Entry {
    title: String,
    detail: String,
    kind: Kind,
    icon: &'static str,
    run: Rc<dyn Fn()>,
    /// Alt+Enter, when the entry has a second choice.
    alternate: Option<Alternate>,
}
fn entry(kind: Kind, icon: &'static str, title: impl Into<String>, detail: impl Into<String>, run: impl Fn() + 'static) -> Entry {
    Entry { title: title.into(), detail: detail.into(), kind, icon, run: Rc::new(run), alternate: None }
}
fn with_ui(run: impl Fn(&Rc<super::window::Ui>) + 'static) -> impl Fn() + 'static {
    move || {
        if let Some(ui) = current() {
            run(&ui);
        }
    }
}
fn commands() -> Vec<Entry> {
    use nyx_git::core::operations::Operation;
    let c = |title: &str, detail: &str, icon: &'static str, run: fn()| entry(Kind::Command, icon, title, detail, run);
    let external = |kind: &'static str| {
        with_ui(move |ui| {
            if let Some(root) = ui.root() {
                super::workspace::external(root, kind);
            }
        })
    };
    let mut out = vec![
        c("Fetch all remotes", "Ctrl+Shift+F", "network-receive-symbolic", || {
            if let Some(ui) = current() {
                ui.execute(Operation::Fetch)
            }
        }),
        c("Pull (fast-forward)", "Ctrl+Shift+L", "go-down-symbolic", || {
            if let Some(ui) = current() {
                ui.execute(Operation::Pull)
            }
        }),
        c("Push or publish the branch", "Ctrl+Alt+P", "go-up-symbolic", super::window::push),
        c("Stage all changes", "Changes", "list-add-symbolic", || {
            if let Some(ui) = current() {
                ui.execute(Operation::StageAll)
            }
        }),
        c("Commit staged changes", "Ctrl+Enter", "object-select-symbolic", super::workflow::commit),
        c("Branches…", "Create, rename, switch and delete branches", "media-playlist-consecutive-symbolic", super::repository_actions::branches),
        c("Stashes…", "Save, apply and drop stashes", "folder-symbolic", super::repository_actions::stashes),
        c("Remotes…", "Add, rename and edit remotes", "network-server-symbolic", super::remotes::show),
        c("Worktrees…", "Create, lock and remove worktrees", "folder-open-symbolic", super::worktree_manager::show),
        c("Submodules…", "Initialize and update submodules", "folder-symbolic", super::submodules::show),
        c("Git LFS…", "Track patterns and fetch objects", "folder-download-symbolic", super::lfs::show),
        c("Git actions…", "Merge, rebase, cherry-pick, revert, reset, tags", "system-run-symbolic", || super::advanced::show(None)),
        c("Recovery points…", "Commits Nyx Git kept, and HEAD's reflog", "edit-undo-symbolic", super::advanced::recovery),
        c("Operation history…", "Jobs this session, with retry and stop", "view-list-symbolic", super::tasks::show),
        c("Resolve conflicts…", "Open a conflicted file", "dialog-warning-symbolic", super::conflict_editor::choose),
        c("Open repository…", "Ctrl+O", "folder-open-symbolic", || super::window::choose_folder(false)),
        c("Discover repositories…", "Ctrl+Shift+O", "system-search-symbolic", || super::window::choose_folder(true)),
        c("Clone repository…", "From a URL", "folder-download-symbolic", super::providers::clone_url),
        c("Initialize repository…", "A new or existing folder", "folder-new-symbolic", super::initialize::show),
        c("Repositories, favorites and groups…", "Ctrl+K switches repository", "starred-symbolic", super::workspace::catalog),
        c("Hosting accounts…", "GitHub, GitLab, Forgejo and Gitea", "system-users-symbolic", super::providers::show),
        c("Refresh status", "F5", "view-refresh-symbolic", || {
            if let Some(ui) = current() {
                ui.refresh()
            }
        }),
        c("Settings", "Ctrl+,", "emblem-system-symbolic", super::settings::open),
        c("Keyboard shortcuts", "F1", "input-keyboard-symbolic", super::window::show_shortcuts),
    ];
    out.push(entry(Kind::Command, "folder-symbolic", "Open folder", "The repository in the file manager", external("folder")));
    out.push(entry(Kind::Command, "document-edit-symbolic", "Open in editor", "The repository in your editor", external("editor")));
    out.push(entry(Kind::Command, "utilities-terminal-symbolic", "Open in terminal", "A terminal in the repository", external("terminal")));
    out
}
fn pages() -> Vec<Entry> {
    let ids = current().map(|ui| ui.destinations.ids()).unwrap_or_default();
    super::destinations::ALL
        .iter()
        .filter(|d| ids.contains(&d.id))
        .map(|d| {
            let id = d.id;
            let key = super::destinations::key(id).map(|k| format!("Ctrl+{k} · ")).unwrap_or_default();
            entry(Kind::Page, d.icon, d.title, format!("{key}{}", d.description), with_ui(move |ui| ui.show_view(id)))
        })
        .collect()
}
fn repositories() -> Vec<Entry> {
    let p = crate::prefs::get();
    let mut paths: Vec<PathBuf> = p.workspace.open.clone();
    paths.extend(p.workspace.repositories.iter().filter(|r| r.favorite).map(|r| r.root.clone()));
    paths.extend(p.recent.clone());
    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| seen.insert(p.clone()));
    paths
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let open = path.clone();
            entry(Kind::Repository, "folder-symbolic", name, path.display().to_string(), with_ui(move |ui| ui.open(open.clone())))
        })
        .collect()
}
/// Branches, tags and worktrees from the sidebar's inventory, already read in the background.
fn inventory() -> Vec<Entry> {
    let Some(ui) = current() else { return Vec::new() };
    let Some(root) = ui.root() else { return Vec::new() };
    let Some(data) = ui.sidebar.inventory() else { return Vec::new() };
    let mut out = Vec::new();
    for group in data.groups {
        for item in group.items {
            match (&group.kind, item.action) {
                (Group::Local | Group::RemoteBranch | Group::Tag, Action::Reference { reference, current: is_current, commit }) => {
                    let show = {
                        let reference = reference.clone();
                        with_ui(move |ui| {
                            ui.show_view("history");
                            ui.graph.show_reference(reference.clone());
                        })
                    };
                    let tag = group.kind == Group::Tag;
                    let mut e = entry(
                        if tag { Kind::Tag } else { Kind::Branch },
                        if tag { "bookmark-new-symbolic" } else { "media-playlist-consecutive-symbolic" },
                        format!("{}{}", if is_current { "● " } else { "" }, item.name),
                        item.detail.lines().skip(1).collect::<Vec<_>>().join(" · "),
                        show,
                    );
                    if !commit {
                        continue;
                    }
                    if let Some(branch) = reference.strip_prefix("refs/heads/").filter(|_| !is_current) {
                        let (r, name) = (root.clone(), branch.to_string());
                        let switch: Rc<dyn Fn()> = Rc::new(with_ui(move |ui| {
                            ui.execute_at(r.clone(), nyx_git::core::operations::Operation::SwitchBranch(name.clone()))
                        }));
                        e.alternate = Some(("show its history", e.run.clone()));
                        e.run = switch;
                    }
                    out.push(e);
                }
                (Group::Worktree, Action::Worktree(path)) => {
                    let open = path.clone();
                    out.push(entry(
                        Kind::Worktree,
                        "folder-open-symbolic",
                        item.name,
                        path.display().to_string(),
                        with_ui(move |ui| ui.open(open.clone())),
                    ));
                }
                _ => {}
            }
        }
    }
    out
}
/// Lower is better: the title starts with the query, contains it, or every word of the
/// query appears in the title or detail.
fn score(e: &Entry, q: &str) -> Option<u8> {
    let title = e.title.trim_start_matches("● ").to_lowercase();
    if title.starts_with(q) {
        Some(0)
    } else if title.contains(q) {
        Some(1)
    } else {
        let hay = format!("{title} {}", e.detail.to_lowercase());
        q.split_whitespace().all(|w| hay.contains(w)).then_some(2)
    }
}
fn rank(entries: &[Entry], query: &str, mode: Mode) -> Vec<Entry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        let first = match mode {
            Mode::All => &[Kind::Page, Kind::Command][..],
            Mode::Worktrees => &[Kind::Worktree],
            Mode::Files => &[Kind::File],
        };
        return entries.iter().filter(|e| first.contains(&e.kind)).take(LIMIT * 4).cloned().collect();
    }
    let mut hits: Vec<(u8, &Entry)> = entries.iter().filter_map(|e| score(e, &q).map(|s| (s, e))).collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.kind.cmp(&b.1.kind)).then(a.1.title.len().cmp(&b.1.title.len())));
    hits.into_iter().take(LIMIT).map(|(_, e)| e.clone()).collect()
}
fn row(e: &Entry) -> gtk::ListBoxRow {
    let b = widgets::hbox(10);
    b.add_css_class("git-palette-row");
    b.append(&gtk::Image::from_icon_name(e.icon));
    let text = widgets::vbox(0);
    text.set_hexpand(true);
    let t = widgets::label(&e.title, "");
    t.set_ellipsize(pango::EllipsizeMode::Middle);
    text.append(&t);
    if !e.detail.is_empty() {
        let d = widgets::label(&e.detail, "dim-label");
        d.add_css_class("git-palette-detail");
        d.set_ellipsize(pango::EllipsizeMode::Middle);
        text.append(&d);
    }
    b.append(&text);
    b.append(&widgets::tag(e.kind.label(), ""));
    let r = gtk::ListBoxRow::new();
    r.set_child(Some(&b));
    r
}
fn entries(mode: Mode) -> Vec<Entry> {
    match mode {
        Mode::All => {
            let mut all = pages();
            all.extend(commands());
            all.extend(repositories());
            all.extend(inventory());
            all
        }
        Mode::Worktrees => inventory().into_iter().filter(|e| e.kind == Kind::Worktree).collect(),
        Mode::Files => Vec::new(),
    }
}
fn tracked_files(root: PathBuf) -> anyhow::Result<Vec<PathBuf>> {
    nyx_git::core::files::tracked(&root, FILES)
}
fn file_entries(root: &std::path::Path, files: Vec<PathBuf>) -> Vec<Entry> {
    files
        .into_iter()
        .map(|file| {
            let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let (r, f) = (root.to_path_buf(), file.clone());
            let history = move || super::file_history::open(r.clone(), f.clone(), None);
            let (r, f) = (root.to_path_buf(), file.clone());
            let mut e = entry(Kind::File, "text-x-generic-symbolic", name, file.display().to_string(), history);
            e.alternate = Some(("show its blame", Rc::new(move || super::blame::open(r.clone(), f.clone()))));
            e
        })
        .collect()
}
pub fn open() {
    show(Mode::All);
}
pub fn show(mode: Mode) {
    let Some(ui) = current() else { return };
    let (title, placeholder) = match mode {
        Mode::All => ("Go to", "Pages, commands, repositories, branches, worktrees"),
        Mode::Worktrees => ("Worktrees", "Open a worktree of this repository"),
        Mode::Files => ("Files", "Tracked files: Enter shows history · Alt+Enter blame"),
    };
    let (dialog, card) = widgets::dialog(title, 640);
    dialog.set_default_height(520);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(placeholder));
    card.append(&search);
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("git-palette");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .vexpand(true)
        .min_content_height(360)
        .child(&list)
        .build();
    card.append(&scroll);
    let hint = widgets::label("Enter opens · Alt+Enter for the other choice · Esc closes", "dim-label");
    hint.add_css_class("git-palette-detail");
    card.append(&hint);
    let all = Rc::new(RefCell::new(entries(mode)));
    let shown: Rc<RefCell<Vec<Entry>>> = Rc::default();
    let refresh: Rc<dyn Fn()> = {
        let (all, shown, list, search, hint) = (all.clone(), shown.clone(), list.clone(), search.clone(), hint.clone());
        Rc::new(move || {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let ranked = rank(&all.borrow(), &search.text(), mode);
            for e in &ranked {
                list.append(&row(e));
            }
            if ranked.is_empty() {
                let none = gtk::ListBoxRow::new();
                none.set_child(Some(&widgets::label(
                    if mode == Mode::Files && all.borrow().is_empty() { "Listing tracked files…" } else { "Nothing found." },
                    "dim-label",
                )));
                none.set_selectable(false);
                none.set_activatable(false);
                list.append(&none);
            }
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
            let alternate = ranked.first().and_then(|e| e.alternate.as_ref()).map(|(what, _)| *what);
            hint.set_text(&match alternate {
                Some(what) => format!("Enter opens · Alt+Enter to {what} · Esc closes"),
                None => "Enter opens · Esc closes".into(),
            });
            *shown.borrow_mut() = ranked;
        })
    };
    refresh();
    if matches!(mode, Mode::All | Mode::Files)
        && let Some(root) = ui.root()
    {
        let (all, refresh, d) = (all.clone(), refresh.clone(), dialog.clone());
        nyx_core::cmd::background(
            move || tracked_files(root.clone()).map(|files| (root, files)),
            move |result| {
                if !d.is_visible() {
                    return;
                }
                match result {
                    Ok((root, files)) => {
                        all.borrow_mut().extend(file_entries(&root, files));
                        refresh();
                    }
                    Err(e) if mode == Mode::Files => {
                        if let Some(ui) = current() {
                            ui.error(&format!("{e:#}"));
                        }
                    }
                    Err(_) => {}
                }
            },
        );
    }
    let go: Rc<dyn Fn(usize, bool)> = {
        let (shown, dialog) = (shown.clone(), dialog.clone());
        Rc::new(move |index, alternate| {
            let Some(e) = shown.borrow().get(index).cloned() else { return };
            let run = match (&e.alternate, alternate) {
                (Some((_, run)), true) => run.clone(),
                _ => e.run.clone(),
            };
            dialog.close();
            run();
        })
    };
    let g = go.clone();
    list.connect_row_activated(move |_, r| g(r.index() as usize, false));
    let r = refresh.clone();
    search.connect_search_changed(move |_| r());
    let (g, l) = (go.clone(), list.clone());
    search.connect_activate(move |_| g(l.selected_row().map_or(0, |r| r.index() as usize), false));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (l, sc) = (list.clone(), scroll.clone());
    keys.connect_key_pressed(move |_, key, _, mods| {
        let index = l.selected_row().map_or(0, |r| r.index());
        match key {
            gdk::Key::Down | gdk::Key::Up => {
                let next = if key == gdk::Key::Down { index + 1 } else { (index - 1).max(0) };
                // Typing carries on in the search box; the list scrolls by hand.
                if let Some(r) = l.row_at_index(next) {
                    l.select_row(Some(&r));
                    if let Some(p) = r.compute_point(&l, &gtk::graphene::Point::new(0.0, 0.0)) {
                        let adj = sc.vadjustment();
                        let (top, h) = (p.y() as f64, r.height() as f64);
                        if top < adj.value() {
                            adj.set_value(top);
                        } else if top + h > adj.value() + adj.page_size() {
                            adj.set_value(top + h - adj.page_size());
                        }
                    }
                }
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter if mods.contains(gdk::ModifierType::ALT_MASK) => {
                go(index as usize, true);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    dialog.add_controller(keys);
    dialog.present();
    search.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;
    fn e(kind: Kind, title: &str, detail: &str) -> Entry {
        entry(kind, "", title, detail, || {})
    }
    #[test]
    fn ranks_title_matches_before_details_and_narrows_by_mode() {
        let entries = vec![
            e(Kind::File, "main.rs", "apps/git/src/main.rs"),
            e(Kind::Branch, "● main", "Tracks origin/main"),
            e(Kind::Command, "Fetch all remotes", "Ctrl+Shift+F"),
            e(Kind::Page, "History", "The commit graph"),
        ];
        let got: Vec<_> = rank(&entries, "main", Mode::All).into_iter().map(|e| e.title).collect();
        assert_eq!(got, ["● main", "main.rs"]);
        let got: Vec<_> = rank(&entries, "graph commit", Mode::All).into_iter().map(|e| e.title).collect();
        assert_eq!(got, ["History"]);
        assert_eq!(rank(&entries, "", Mode::All).len(), 2);
        assert_eq!(rank(&entries, "", Mode::Files).len(), 1);
    }
}
