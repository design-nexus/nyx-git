//! Recent repository activity: operations Nyx Git recorded in its journal, and HEAD's moves
//! from Git's reflog (which includes what other tools did). Read-only.
use super::{journal, operations};
use anyhow::Result;
use std::{ffi::OsString, path::Path};

/// Reflog entries read at most.
pub const REFLOG_LIMIT: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Commit,
    Branch,
    Checkout,
    Fetch,
    Pull,
    Push,
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Reset,
    Stash,
    Worktree,
    Tag,
    Recovery,
    Discard,
    Other,
}
impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Commit => "Commit",
            Self::Branch => "Branch",
            Self::Checkout => "Checkout",
            Self::Fetch => "Fetch",
            Self::Pull => "Pull",
            Self::Push => "Push",
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::CherryPick => "Cherry-pick",
            Self::Revert => "Revert",
            Self::Reset => "Reset",
            Self::Stash => "Stash",
            Self::Worktree => "Worktree",
            Self::Tag => "Tag",
            Self::Recovery => "Recovery",
            Self::Discard => "Discard",
            Self::Other => "Other",
        }
    }
    /// From a journal label or a reflog subject's first word(s).
    pub fn classify(text: &str) -> Self {
        let t = text.to_lowercase();
        let has = |w: &str| t.contains(w);
        if has("delete") && has("branch") {
            Self::Branch
        } else if has("cherry-pick") || has("cherry pick") {
            Self::CherryPick
        } else if has("rebase") || has("squash") {
            Self::Rebase
        } else if has("revert") {
            Self::Revert
        } else if has("merge") {
            Self::Merge
        } else if has("pull") {
            Self::Pull
        } else if has("push") {
            Self::Push
        } else if has("fetch") || has("clone") {
            Self::Fetch
        } else if has("reset") {
            Self::Reset
        } else if has("stash") {
            Self::Stash
        } else if has("worktree") || has("prune") {
            Self::Worktree
        } else if has("tag") {
            Self::Tag
        } else if has("recovery") {
            Self::Recovery
        } else if has("discard") {
            Self::Discard
        } else if has("checkout") || has("switch") || has("check out") {
            Self::Checkout
        } else if has("commit") || has("amend") {
            Self::Commit
        } else if has("branch") {
            Self::Branch
        } else {
            Self::Other
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Recorded by Nyx Git when it ran the operation.
    Journal,
    /// Read from Git's HEAD reflog; may come from any Git tool.
    Reflog,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub time: i64,
    pub kind: Kind,
    pub source: Source,
    pub title: String,
    pub detail: String,
    pub ok: bool,
    /// Where HEAD (or the main reference changed) was before and after.
    pub before: Option<String>,
    pub after: Option<String>,
    pub changes: Vec<journal::RefChange>,
}
/// HEAD's reflog, newest first: (new commit, time, subject).
pub fn reflog(root: &Path, limit: usize) -> Result<Vec<(String, i64, String)>> {
    let args: Vec<OsString> =
        ["--no-pager", "reflog", "show", "--date=unix", &format!("-n{limit}"), "--format=%H%x00%gd%x00%gs", "HEAD", "--"]
            .iter()
            .map(OsString::from)
            .collect();
    let bytes = match operations::invoke(root, "Read reflog", args, None, &[], false) {
        Ok(b) => b,
        // An unborn branch has no reflog yet.
        Err(_) => return Ok(Vec::new()),
    };
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| {
            let mut f = line.splitn(3, '\0');
            let (id, selector, subject) = (f.next()?, f.next()?, f.next()?);
            let time = selector.rsplit_once('{')?.1.trim_end_matches('}').parse().ok()?;
            Some((id.to_string(), time, subject.to_string()))
        })
        .collect())
}
/// Journal and reflog together, newest first. A reflog move that a journal record already
/// explains (same new HEAD, within seconds) isn't listed twice.
pub fn read(root: &Path, common_dir: &Path) -> Result<Vec<Event>> {
    let entries = journal::read(common_dir);
    let mut events: Vec<Event> = entries
        .iter()
        .map(|e| {
            let main = e.head().or_else(|| e.changes.first());
            let worktree = e.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let refs = e
                .changes
                .iter()
                .filter(|c| c.name != "HEAD")
                .map(|c| short(&c.name))
                .take(3)
                .collect::<Vec<_>>()
                .join(", ");
            Event {
                time: e.time,
                kind: Kind::classify(&e.operation),
                source: Source::Journal,
                title: e.operation.clone(),
                detail: match (&e.error, refs.is_empty()) {
                    (Some(err), _) => format!("Failed · {}", err.lines().next().unwrap_or_default()),
                    (None, true) => format!("in {worktree}"),
                    (None, false) => format!("{refs} · in {worktree}"),
                },
                ok: e.ok,
                before: main.and_then(|c| c.before.clone()),
                after: main.and_then(|c| c.after.clone()),
                changes: e.changes.clone(),
            }
        })
        .collect();
    let log = reflog(root, REFLOG_LIMIT)?;
    for (i, (id, time, subject)) in log.iter().enumerate() {
        let explained = entries.iter().any(|e| (e.time - time).abs() <= 10 && e.head().and_then(|h| h.after.as_deref()) == Some(id));
        if explained {
            continue;
        }
        let (verb, rest) = subject.split_once(": ").unwrap_or((subject.as_str(), ""));
        events.push(Event {
            time: *time,
            kind: Kind::classify(verb),
            source: Source::Reflog,
            title: capitalize(verb),
            detail: rest.chars().take(300).collect(),
            ok: true,
            before: log.get(i + 1).map(|(id, _, _)| id.clone()),
            after: Some(id.clone()),
            changes: Vec::new(),
        });
    }
    events.sort_by_key(|e| std::cmp::Reverse(e.time));
    Ok(events)
}
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}
pub fn short(name: &str) -> String {
    for prefix in ["refs/heads/", "refs/remotes/", "refs/tags/", "refs/nyx/recovery/"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return if prefix == "refs/nyx/recovery/" { format!("recovery {rest}") } else { rest.into() };
        }
    }
    name.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_reflog_subjects_and_journal_labels() {
        assert_eq!(Kind::classify("commit (amend)"), Kind::Commit);
        assert_eq!(Kind::classify("rebase (finish)"), Kind::Rebase);
        assert_eq!(Kind::classify("checkout"), Kind::Checkout);
        assert_eq!(Kind::classify("Delete merged branch"), Kind::Branch);
        assert_eq!(Kind::classify("Discard working changes"), Kind::Discard);
        assert_eq!(Kind::classify("pull --ff-only"), Kind::Pull);
        assert_eq!(short("refs/remotes/origin/main"), "origin/main");
    }
}
