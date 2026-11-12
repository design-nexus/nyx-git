//! Bounded sidebar inventory. Reading it never scans worktree contents or contacts remotes.
use super::{operations, worktrees};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
pub const LIMIT: usize = 2000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Local,
    RemoteBranch,
    Tag,
    Stash,
    Worktree,
    Remote,
}
impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::RemoteBranch => "remote-branches",
            Self::Tag => "tags",
            Self::Stash => "stashes",
            Self::Worktree => "worktrees",
            Self::Remote => "remotes",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Local => "Local branches",
            Self::RemoteBranch => "Remote branches",
            Self::Tag => "Tags",
            Self::Stash => "Stashes",
            Self::Worktree => "Worktrees",
            Self::Remote => "Remotes",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Reference { reference: String, current: bool, commit: bool },
    Stash(String),
    Worktree(PathBuf),
    Remote(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub detail: String,
    pub action: Action,
    /// Commits ahead of and behind the upstream, for a branch that has one.
    pub track: Option<(usize, usize)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub kind: Kind,
    pub items: Vec<Item>,
    pub limited: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    pub groups: Vec<Group>,
}
fn git(root: &Path, label: &str, args: &[&str]) -> Result<Vec<u8>> {
    operations::invoke(root, label, args.iter().map(OsString::from).collect(), None, &[], false)
}
/// "ahead 2, behind 1" (`%(upstream:track,nobracket)`) as (2, 1); "gone" or nothing as (0, 0).
fn track(text: &str) -> (usize, usize) {
    let count = |word: &str| {
        text.split(", ").find_map(|part| part.strip_prefix(word)).and_then(|n| n.trim().parse().ok()).unwrap_or(0)
    };
    (count("ahead "), count("behind "))
}
pub fn read(root: &Path) -> Result<Inventory> {
    let mut groups = Vec::new();
    for (kind, prefix) in [(Kind::Local, "refs/heads/"), (Kind::RemoteBranch, "refs/remotes/"), (Kind::Tag, "refs/tags/")] {
        let bytes = git(
            root,
            "Read sidebar refs",
            &[
                "for-each-ref",
                &format!("--count={}", LIMIT + 1),
                "--format=%(refname)%00%(symref)%00%(upstream:short)%00%(HEAD)%00%(objecttype)%00%(*objecttype)%00%(upstream:track,nobracket)%00",
                prefix,
            ],
        )?;
        let fields = bytes.split(|b| *b == 0).collect::<Vec<_>>();
        let mut items = Vec::new();
        let mut limited = false;
        for row in fields.as_chunks::<7>().0 {
            if !row[1].is_empty() {
                continue;
            } // Skip symbolic origin/HEAD aliases.
            let name = match std::str::from_utf8(row[0]) {
                Ok(name) => name.trim_start_matches('\n'),
                Err(_) => {
                    limited = true;
                    continue;
                }
            };
            ensure!(name.starts_with(prefix), "Unexpected sidebar reference");
            let upstream = String::from_utf8_lossy(row[2]);
            let commit = row[4] == b"commit" || row[5] == b"commit";
            let detail = format!(
                "{name}{}{}",
                if upstream.is_empty() { String::new() } else { format!("\nTracks {upstream}") },
                if commit { "" } else { "\nPoints to a non-commit object" }
            );
            items.push(Item {
                name: name[prefix.len()..].into(),
                detail,
                action: Action::Reference { reference: name.into(), current: row[3] == b"*", commit },
                track: (!upstream.is_empty()).then(|| track(&String::from_utf8_lossy(row[6]))),
            });
        }
        limited |= items.len() > LIMIT;
        items.truncate(LIMIT);
        groups.push(Group { kind, items, limited });
    }
    let bytes = git(
        root,
        "Read sidebar stashes",
        &["stash", "list", &format!("--max-count={}", LIMIT + 1), "--format=%H%x00%gd%x00%gs"],
    )?;
    let mut items = Vec::new();
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        let fields = line.splitn(3, |b| *b == 0).collect::<Vec<_>>();
        ensure!(fields.len() == 3, "Malformed stash listing");
        let id = std::str::from_utf8(fields[0])?;
        operations::valid_id(id)?;
        items.push(Item {
            name: String::from_utf8_lossy(fields[1]).into(),
            detail: String::from_utf8_lossy(fields[2]).chars().take(500).collect(),
            action: Action::Stash(id.into()),
            track: None,
        });
    }
    let limited = items.len() > LIMIT;
    items.truncate(LIMIT);
    groups.push(Group { kind: Kind::Stash, items, limited });
    let trees = worktrees::list(root)?;
    let limited = trees.len() > LIMIT;
    let items = trees
        .into_iter()
        .take(LIMIT)
        .map(|w| Item {
            name: w
                .branch
                .as_deref()
                .map(|b| b.trim_start_matches("refs/heads/"))
                .unwrap_or(if w.bare { "Bare repository" } else { "Detached HEAD" })
                .into(),
            detail: format!(
                "{}{}{}",
                w.path.display(),
                if w.locked.is_some() { "\nLocked" } else { "" },
                if w.prunable.is_some() { "\nMissing / prunable" } else { "" }
            ),
            action: Action::Worktree(w.path),
            track: None,
        })
        .collect();
    groups.push(Group { kind: Kind::Worktree, items, limited });
    let bytes = operations::run(root, operations::Operation::Remotes)?;
    let names = std::str::from_utf8(&bytes).context("Remote names must be UTF-8")?.lines().collect::<Vec<_>>();
    let limited = names.len() > LIMIT;
    let items = names
        .into_iter()
        .take(LIMIT)
        .map(|name| Item {
            name: name.into(),
            detail: "Manage this repository’s remotes".into(),
            action: Action::Remote(name.into()),
            track: None,
        })
        .collect();
    groups.push(Group { kind: Kind::Remote, items, limited });
    Ok(Inventory { groups })
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_upstream_tracking() {
        assert_eq!(super::track("ahead 2, behind 1"), (2, 1));
        assert_eq!(super::track("behind 3"), (0, 3));
        assert_eq!(super::track("gone"), (0, 0));
        assert_eq!(super::track(""), (0, 0));
    }
}
