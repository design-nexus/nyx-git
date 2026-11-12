//! The operation journal: what Nyx Git did to a repository and which references it moved,
//! kept under `$XDG_STATE_HOME/nyx-git/journal/`, one JSON line per operation. It records
//! facts; it is not an undo stack. Git's own reflog stays the authority for HEAD.
use super::operations;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Start journaling this process's operations. Off for library users and tests unless asked.
pub fn enable() {
    ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}
pub fn enabled() -> bool {
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}
/// Records read back at most.
pub const READ_LIMIT: usize = 1000;
/// A journal file is trimmed to its newer half beyond this size.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefChange {
    pub name: String,
    pub before: Option<String>,
    pub after: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub operation: String,
    /// The worktree it ran in.
    pub root: PathBuf,
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// HEAD's branch before and after (`refs/heads/…`), None when detached.
    #[serde(default)]
    pub branch: (Option<String>, Option<String>),
    /// HEAD and every reference that changed, with the commit before and after.
    pub changes: Vec<RefChange>,
}
impl Entry {
    pub fn head(&self) -> Option<&RefChange> {
        self.changes.iter().find(|c| c.name == "HEAD")
    }
}
/// References and HEAD at one moment.
#[derive(Clone, Debug, Default)]
pub struct Refs {
    pub common_dir: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub refs: BTreeMap<String, String>,
}
fn git(root: &Path, label: &str, args: &[&str], allow: bool) -> Result<Vec<u8>> {
    operations::invoke(root, label, args.iter().map(OsString::from).collect(), None, &[], allow)
}
pub fn refs(root: &Path) -> Result<Refs> {
    let common = git(root, "Read Git directory", &["rev-parse", "--path-format=absolute", "--git-common-dir"], false)?;
    let common_dir = PathBuf::from(String::from_utf8_lossy(&common).trim());
    let head = git(root, "Read HEAD", &["rev-parse", "--verify", "--quiet", "HEAD"], true)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .filter(|s| !s.is_empty());
    let branch = git(root, "Read HEAD branch", &["symbolic-ref", "--quiet", "HEAD"], true)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .filter(|s| !s.is_empty());
    let listing = git(root, "Read references", &["for-each-ref", "--format=%(objectname) %(refname)"], false)?;
    let refs = String::from_utf8_lossy(&listing)
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(id, name)| (name.to_string(), id.to_string())))
        .collect();
    Ok(Refs { common_dir, head, branch, refs })
}
/// What changed between two reads, HEAD first.
pub fn changes(before: &Refs, after: &Refs) -> Vec<RefChange> {
    let mut out = Vec::new();
    if before.head != after.head {
        out.push(RefChange { name: "HEAD".into(), before: before.head.clone(), after: after.head.clone() });
    }
    let names: std::collections::BTreeSet<&String> = before.refs.keys().chain(after.refs.keys()).collect();
    for name in names {
        let (b, a) = (before.refs.get(name), after.refs.get(name));
        if b != a {
            out.push(RefChange { name: name.clone(), before: b.cloned(), after: a.cloned() });
        }
    }
    out
}
pub fn directory() -> PathBuf {
    if let Some(dir) = std::env::var_os("NYX_GIT_STATE_DIR") {
        return PathBuf::from(dir).join("journal");
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"));
    state.join("nyx-git/journal")
}
/// One file per repository (its common Git directory), so linked worktrees share it.
pub fn file(common_dir: &Path) -> PathBuf {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in common_dir.as_os_str().as_encoded_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    directory().join(format!("{hash:016x}.jsonl"))
}
fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}
/// Record an operation that ran between `before` and now. Failures to write the journal are
/// logged and never fail the operation itself.
pub fn record(root: &Path, operation: &str, before: &Refs, error: Option<String>, always: bool) {
    let after = refs(root).unwrap_or_else(|_| before.clone());
    let changes = changes(before, &after);
    if changes.is_empty() && error.is_none() && !always {
        return;
    }
    let entry = Entry {
        time: now(),
        operation: operation.into(),
        root: root.into(),
        ok: error.is_none(),
        error: error.map(|e| e.chars().take(2000).collect()),
        branch: (before.branch.clone(), after.branch.clone()),
        changes,
    };
    if let Err(e) = append(&before.common_dir, &entry) {
        tracing::warn!(%e, "Could not write the operation journal");
    }
}
pub fn append(common_dir: &Path, entry: &Entry) -> Result<()> {
    let path = file(common_dir);
    fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let mut line = serde_json::to_vec(entry)?;
    line.push(b'\n');
    let mut f = fs::OpenOptions::new().create(true).append(true).open(&path)?;
    f.write_all(&line)?;
    if f.metadata()?.len() > MAX_BYTES {
        let text = fs::read(&path)?;
        let half = text.len() / 2;
        let start = text[half..].iter().position(|b| *b == b'\n').map_or(text.len(), |i| half + i + 1);
        let tmp = path.with_extension("jsonl.tmp");
        fs::write(&tmp, &text[start..])?;
        fs::rename(tmp, &path)?;
    }
    Ok(())
}
/// The journal for a repository, newest first. Unreadable lines are skipped.
pub fn read(common_dir: &Path) -> Vec<Entry> {
    let Ok(text) = fs::read_to_string(file(common_dir)) else { return Vec::new() };
    let mut entries: Vec<Entry> = text.lines().rev().filter_map(|l| serde_json::from_str(l).ok()).take(READ_LIMIT).collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.time));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changes_list_moved_created_and_deleted_refs_with_head_first() {
        let before = Refs {
            head: Some("a".into()),
            refs: [("refs/heads/main".into(), "a".into()), ("refs/heads/old".into(), "b".into())].into(),
            ..Default::default()
        };
        let after = Refs {
            head: Some("c".into()),
            refs: [("refs/heads/main".into(), "c".into()), ("refs/heads/new".into(), "c".into())].into(),
            ..Default::default()
        };
        let c = changes(&before, &after);
        assert_eq!(c[0].name, "HEAD");
        assert_eq!(c.len(), 4);
        assert!(c.iter().any(|r| r.name == "refs/heads/old" && r.after.is_none()));
        assert!(c.iter().any(|r| r.name == "refs/heads/new" && r.before.is_none()));
    }
}
