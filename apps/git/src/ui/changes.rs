//! Changes: the working tree's files grouped by state (Conflicts, Staged, Changes,
//! Untracked) with the composer card under them, and the selected file's diff beside them.
//! A file with staged and unstaged changes is in both Staged and Changes; which one is
//! selected decides which side of the index the diff shows.
use super::window::current;
use gtk::{gdk, gio, glib, pango, prelude::*};
use nyx_git::core::{operations::Operation, status::FileStatus};
use nyx_ui::widgets;

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub enum Group {
    Conflicts,
    Staged,
    Changes,
    Untracked,
}
impl Group {
    fn title(self) -> &'static str {
        match self {
            Self::Conflicts => "Conflicts",
            Self::Staged => "Staged",
            Self::Changes => "Changes",
            Self::Untracked => "Untracked",
        }
    }
}
/// One row of the list: a file in one of its groups.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub group: Group,
    pub file: FileStatus,
}
/// The list's rows: every file in each group it belongs to, groups in order, files in
/// `git status` order within each.
pub fn entries(files: &[FileStatus]) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for file in files {
        let mut put = |group| out.push(Entry { group, file: file.clone() });
        if file.conflicted {
            put(Group::Conflicts);
            continue;
        }
        if file.index == '?' {
            put(Group::Untracked);
            continue;
        }
        if file.index != '.' {
            put(Group::Staged);
        }
        if file.worktree != '.' {
            put(Group::Changes);
        }
    }
    out.sort_by_key(|e| e.group);
    out
}
/// The letter shown for `entry` and its chip's tint.
fn status(entry: &Entry) -> (char, &'static str) {
    let f = &entry.file;
    let code = match entry.group {
        Group::Conflicts => return ('U', "conflict"),
        Group::Untracked => return ('A', "added"),
        Group::Staged => f.index,
        Group::Changes => f.worktree,
    };
    let class = match code {
        'A' | 'C' => "added",
        'D' => "deleted",
        'R' => "renamed",
        _ => "modified",
    };
    (if code == 'T' { 'M' } else { code }, class)
}
fn words(code: char) -> &'static str {
    match code {
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'T' => "type changed",
        _ => "modified",
    }
}
pub fn entry_of(object: &glib::Object) -> Option<Entry> {
    object.downcast_ref::<glib::BoxedAnyObject>().map(|o| o.borrow::<Entry>().clone())
}

/// Whether `entry` passes the page's filter: `category` (All, Unstaged, Staged,
/// Untracked) and the search text.
pub fn matches(entry: &Entry, category: u32, query: &str) -> bool {
    let shown = match category {
        1 => entry.group != Group::Staged,
        2 => entry.group == Group::Staged,
        3 => entry.group == Group::Untracked,
        _ => true,
    };
    shown && (query.is_empty() || entry.file.path.to_string_lossy().to_lowercase().contains(&query.to_lowercase()))
}

/// Stage or unstage (by its group) the file in `entry`; a conflict opens in Conflict Studio.
fn toggle(entry: &Entry) {
    match entry.group {
        Group::Conflicts => super::conflict_studio_show(Some(entry.file.path.clone())),
        Group::Staged => super::workflow::stage_file(&entry.file, true),
        Group::Changes | Group::Untracked => super::workflow::stage_file(&entry.file, false),
    }
}

/// The rows of the list and their factory.
pub fn file_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let row = widgets::hbox(10);
        row.add_css_class("git-file-row");
        let chip = widgets::label("", "git-chip");
        chip.set_xalign(0.5);
        chip.set_valign(gtk::Align::Center);
        row.append(&chip);
        let name = widgets::label("", "git-file-name");
        name.set_ellipsize(pango::EllipsizeMode::Middle);
        row.append(&name);
        let dir = widgets::label("", "git-file-dir");
        dir.set_hexpand(true);
        dir.set_ellipsize(pango::EllipsizeMode::Start);
        row.append(&dir);
        let op = gtk::Button::new();
        op.add_css_class("git-file-op");
        op.set_valign(gtk::Align::Center);
        op.set_focusable(false);
        let it = item.downgrade();
        op.connect_clicked(move |_| {
            if let Some(entry) = it.upgrade().and_then(|i| i.item()).and_then(|o| entry_of(&o)) {
                toggle(&entry);
            }
        });
        row.append(&op);
        item.set_child(Some(&row));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(entry) = item.item().and_then(|o| entry_of(&o)) else { return };
        let row = item.child().unwrap();
        let chip = row.first_child().and_downcast::<gtk::Label>().unwrap();
        let name = chip.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let dir = name.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let op = dir.next_sibling().and_downcast::<gtk::Button>().unwrap();
        let (code, class) = status(&entry);
        chip.set_text(&code.to_string());
        for c in ["added", "modified", "deleted", "renamed", "conflict"] {
            chip.remove_css_class(c);
        }
        chip.add_css_class(class);
        let f = &entry.file;
        name.set_text(&f.path.file_name().unwrap_or_default().to_string_lossy());
        let folder = f.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        dir.set_text(&match &f.original {
            Some(o) => format!("from {}", o.to_string_lossy()),
            None => folder,
        });
        let what = match entry.group {
            Group::Conflicts => "conflicted: resolve it before committing".to_string(),
            Group::Untracked => "untracked: not in the repository yet".to_string(),
            Group::Staged => format!("staged: {}", words(f.index)),
            Group::Changes => format!("not staged: {}", words(f.worktree)),
        };
        row.set_tooltip_text(Some(&format!("{}\n{what}", f.path.to_string_lossy())));
        let (icon, tip) = match entry.group {
            Group::Conflicts => ("git-conflict-symbolic", "Resolve in Conflict Studio"),
            Group::Staged => ("git-minus-symbolic", "Unstage (Space)"),
            _ => ("git-plus-symbolic", "Stage (Space)"),
        };
        op.set_icon_name(icon);
        op.set_tooltip_text(Some(tip));
    });
    factory
}

/// A group's heading: its name and count, and what can be done to all of it.
pub fn header_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, header| {
        let header = header.downcast_ref::<gtk::ListHeader>().unwrap();
        let row = widgets::hbox(6);
        row.add_css_class("git-group-row");
        let title = widgets::label("", "git-group-title");
        row.append(&title);
        let count = widgets::label("", "git-group-count");
        count.set_hexpand(true);
        row.append(&count);
        let act = gtk::Button::new();
        act.add_css_class("git-link");
        act.set_focusable(false);
        let h = header.downgrade();
        act.connect_clicked(move |_| {
            let Some(header) = h.upgrade() else { return };
            let Some(ui) = current() else { return };
            let Some(model) = ui.selection.model() else { return };
            let group: Vec<Entry> = (header.start()..header.end()).filter_map(|i| model.item(i)).filter_map(|o| entry_of(&o)).collect();
            let Some(first) = group.first() else { return };
            let paths: Vec<_> = group.iter().flat_map(|e| std::iter::once(e.file.path.clone()).chain(e.file.original.clone())).collect();
            match first.group {
                Group::Conflicts => super::conflict_studio_show(None),
                Group::Staged => {
                    let unborn = ui.snapshot.borrow().as_ref().is_some_and(|s| s.repository.unborn);
                    ui.execute(Operation::Unstage { paths, unborn });
                }
                Group::Changes | Group::Untracked => ui.execute(Operation::Stage(paths)),
            }
        });
        row.append(&act);
        header.set_child(Some(&row));
    });
    factory.connect_bind(|_, header| {
        let header = header.downcast_ref::<gtk::ListHeader>().unwrap();
        let Some(entry) = header.item().and_then(|o| entry_of(&o)) else { return };
        let row = header.child().unwrap();
        let title = row.first_child().and_downcast::<gtk::Label>().unwrap();
        let count = title.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let act = count.next_sibling().and_downcast::<gtk::Button>().unwrap();
        title.set_text(&entry.group.title().to_uppercase());
        count.set_text(&(header.end() - header.start()).to_string());
        let (label, tip) = match entry.group {
            Group::Conflicts => ("Conflict Studio", "Resolve the conflicted files"),
            Group::Staged => ("Unstage all", "Take these out of the next commit; the files keep their changes"),
            Group::Changes => ("Stage all", "Stage every change to a tracked file"),
            Group::Untracked => ("Stage all", "Add every untracked file to the next commit"),
        };
        act.set_label(label);
        act.set_tooltip_text(Some(tip));
    });
    factory
}

/// The list over `files`, filtered by `filter`, in sections by group.
pub fn list(files: &gio::ListStore, filter: &gtk::CustomFilter) -> (gtk::ListView, gtk::SingleSelection) {
    let filtered = gtk::FilterListModel::new(Some(files.clone()), Some(filter.clone()));
    let sorted = gtk::SortListModel::new(Some(filtered), None::<gtk::Sorter>);
    sorted.set_section_sorter(Some(&gtk::CustomSorter::new(|a, b| {
        let (a, b) = (entry_of(a).map(|e| e.group), entry_of(b).map(|e| e.group));
        a.cmp(&b).into()
    })));
    let selection = gtk::SingleSelection::new(Some(sorted));
    selection.set_autoselect(false);
    let view = gtk::ListView::new(Some(selection.clone()), Some(file_factory()));
    view.set_header_factory(Some(&header_factory()));
    view.add_css_class("git-files");
    view.update_property(&[gtk::accessible::Property::Label("Changed files")]);
    // Space stages or unstages the selected file, as the row's button does.
    let keys = gtk::EventControllerKey::new();
    let s = selection.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        if key == gdk::Key::space && mods.is_empty() {
            if let Some(entry) = s.selected_item().and_then(|o| entry_of(&o)) {
                toggle(&entry);
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    view.add_controller(keys);
    (view, selection)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn f(path: &str, index: char, worktree: char) -> FileStatus {
        FileStatus { path: path.into(), original: None, index, worktree, conflicted: false, submodule: false }
    }
    #[test]
    fn a_file_is_in_each_group_it_belongs_to() {
        let files = vec![f("a", 'M', 'M'), f("b", '.', 'M'), f("c", '?', '?'), f("d", 'A', '.')];
        let mut u = f("e", 'U', 'U');
        u.conflicted = true;
        let mut all = files.clone();
        all.push(u);
        let got: Vec<_> = entries(&all).into_iter().map(|e| (e.group, e.file.path.to_string_lossy().into_owned())).collect();
        assert_eq!(
            got,
            [
                (Group::Conflicts, "e".into()),
                (Group::Staged, "a".into()),
                (Group::Staged, "d".into()),
                (Group::Changes, "a".into()),
                (Group::Changes, "b".into()),
                (Group::Untracked, "c".into()),
            ]
        );
        let e = &entries(&files)[0];
        assert!(matches(e, 2, "") && !matches(e, 1, "") && matches(e, 0, "A") && !matches(e, 0, "zz"));
    }
}
