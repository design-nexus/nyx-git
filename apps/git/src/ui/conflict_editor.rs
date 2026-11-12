//! Opening conflicts: everything happens in the Conflict Studio.
use super::window::current;
use std::path::PathBuf;

pub fn choose() {
    let conflicted = current().is_some_and(|ui| ui.snapshot.borrow().as_ref().is_some_and(|s| s.status.files.iter().any(|f| f.conflicted)));
    if !conflicted {
        if let Some(ui) = current() {
            ui.toast("No conflicted files. Refresh if Git changed externally.");
        }
        return;
    }
    super::conflict_studio_show(None);
}
pub fn open(file: PathBuf) {
    super::conflict_studio_show(Some(file));
}
