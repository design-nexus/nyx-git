//! What an operation would do, worked out before it runs: references that move, commits
//! that leave a branch or are rewritten (and whether they're published), uncommitted files
//! at risk, likely conflicts and how it can be recovered. A preview is a forecast from the
//! repository as it is now; it can't promise an operation completes without conflicts.
use super::{
    advanced::{Action, ResetMode},
    operations::{self, Operation},
    status,
};
use anyhow::Result;
use std::{
    collections::HashSet,
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Commits listed at most; the total is counted separately.
pub const LIST: usize = 50;

#[derive(Clone, Debug)]
pub enum Subject {
    Advanced(Action),
    /// A merged local branch (`git branch --delete`).
    DeleteMergedBranch(String),
    RemoveWorktree(PathBuf),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefMove {
    pub name: String,
    pub before: Option<String>,
    /// What it will point to, in words when the commit doesn't exist yet.
    pub after: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    pub refs: Vec<RefMove>,
    /// Commits that leave the branch or are rewritten: (hash, subject).
    pub commits: Vec<(String, String)>,
    pub commits_total: usize,
    /// What `commits` are, e.g. "leave main" or "are rewritten".
    pub commits_note: String,
    /// How many of `commits` a remote-tracking branch already has.
    pub published: usize,
    pub at_risk: Vec<PathBuf>,
    pub risk_note: String,
    /// Files changed on both sides since the merge base, where conflicts are likely.
    pub overlap: Vec<PathBuf>,
    pub remote: Option<String>,
    pub recovery: Vec<String>,
    /// Some of the effect can't be undone from this repository.
    pub irreversible: bool,
}
fn git(root: &Path, label: &str, args: &[&str]) -> Result<Vec<u8>> {
    operations::invoke(root, label, args.iter().map(OsString::from).collect(), None, &[], false)
}
fn lines(bytes: Vec<u8>) -> Vec<String> {
    String::from_utf8_lossy(&bytes).lines().map(str::to_string).filter(|l| !l.is_empty()).collect()
}
fn head_branch(root: &Path) -> Option<String> {
    operations::invoke(root, "Read HEAD branch", ["symbolic-ref", "--quiet", "HEAD"].map(OsString::from).into(), None, &[], true)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .filter(|s| !s.is_empty())
}
fn head(root: &Path) -> Option<String> {
    git(root, "Read HEAD", &["rev-parse", "--verify", "HEAD"]).ok().map(|b| String::from_utf8_lossy(&b).trim().to_string())
}
/// Some commits (hash, subject), how many there are in all, and all their hashes.
type Listing = (Vec<(String, String)>, usize, HashSet<String>);
/// Commits in `range` (rev-list arguments), newest first, with the total.
fn commits(root: &Path, range: &[&str]) -> Result<Listing> {
    let mut args = vec!["rev-list", "--format=%H%x00%s", "--no-commit-header"];
    args.extend_from_slice(range);
    let all = lines(git(root, "List commits", &args)?);
    let ids: HashSet<String> = all.iter().filter_map(|l| l.split('\0').next()).map(str::to_string).collect();
    let listed = all
        .iter()
        .take(LIST)
        .filter_map(|l| l.split_once('\0').map(|(id, s)| (id.to_string(), s.chars().take(200).collect())))
        .collect();
    Ok((listed, ids.len(), ids))
}
/// How many of `ids` a remote-tracking branch already contains.
fn published(root: &Path, tip: &str, ids: &HashSet<String>) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let unpublished: HashSet<String> = lines(git(root, "Check published commits", &["rev-list", tip, "--not", "--remotes"])?).into_iter().collect();
    Ok(ids.iter().filter(|id| !unpublished.contains(*id)).count())
}
fn changed_files(root: &Path, from: &str, to: &str) -> Result<HashSet<PathBuf>> {
    let out = git(root, "List changed files", &["diff", "--name-only", "-z", "--no-renames", from, to, "--"])?;
    Ok(out.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned())).collect())
}
/// Files both sides changed since their merge base.
fn overlap(root: &Path, ours: &str, theirs: &str) -> Vec<PathBuf> {
    let Ok(base) = git(root, "Find merge base", &["merge-base", ours, theirs]) else { return Vec::new() };
    let base = String::from_utf8_lossy(&base).trim().to_string();
    let (Ok(a), Ok(b)) = (changed_files(root, &base, ours), changed_files(root, &base, theirs)) else { return Vec::new() };
    let mut both: Vec<_> = a.intersection(&b).cloned().collect();
    both.sort();
    both
}
fn ancestor(root: &Path, a: &str, b: &str) -> bool {
    operations::invoke(root, "Check ancestry", ["merge-base", "--is-ancestor", a, b].map(OsString::from).into(), None, &[], false).is_ok()
}
fn dirty(root: &Path) -> Result<Vec<status::FileStatus>> {
    Ok(status::parse(&operations::run(root, Operation::Status)?)?.files)
}
fn short(id: &str) -> String {
    id.chars().take(10).collect()
}
fn branch_name(branch: &Option<String>) -> String {
    branch.as_deref().map_or("HEAD (detached)".into(), |b| b.trim_start_matches("refs/heads/").to_string())
}
const RECOVERY_POINT: &str = "Nyx Git saves a recovery point at the current tip first (Activity or Recovery points can make a branch from it).";
const REFLOG: &str = "Git's reflog also remembers where HEAD was.";

pub fn preview(root: &Path, subject: &Subject) -> Result<Preview> {
    let mut p = Preview::default();
    let branch = head_branch(root);
    let tip = head(root);
    let current = branch.clone().unwrap_or_else(|| "HEAD".into());
    match subject {
        Subject::Advanced(Action::Reset { target, mode }) => {
            p.refs.push(RefMove { name: branch_name(&branch), before: tip.clone(), after: short(target) });
            let (list, total, ids) = commits(root, &[&format!("{target}..HEAD")])?;
            p.published = published(root, "HEAD", &ids)?;
            (p.commits, p.commits_total) = (list, total);
            p.commits_note = format!("leave {}", branch_name(&branch));
            let files = dirty(root)?;
            match mode {
                ResetMode::Hard => {
                    p.at_risk = files.iter().filter(|f| f.index != '?').map(|f| f.path.clone()).collect();
                    p.risk_note = "Uncommitted changes to these files are overwritten. They can't be recovered.".into();
                    p.irreversible = !p.at_risk.is_empty();
                }
                ResetMode::Mixed => {
                    p.at_risk = files.iter().filter(|f| f.index != '.' && f.index != '?').map(|f| f.path.clone()).collect();
                    p.risk_note = "These files are unstaged; their contents stay in the working tree.".into();
                }
                ResetMode::Soft => {}
            }
            p.recovery = vec![RECOVERY_POINT.into(), REFLOG.into()];
        }
        Subject::Advanced(Action::Rebase(onto)) => {
            p.refs.push(RefMove { name: branch_name(&branch), before: tip.clone(), after: "the replayed commits".into() });
            let (list, total, ids) = commits(root, &[&format!("{onto}..HEAD")])?;
            p.published = published(root, "HEAD", &ids)?;
            (p.commits, p.commits_total) = (list, total);
            p.commits_note = "are replayed with new hashes".into();
            p.overlap = overlap(root, "HEAD", onto);
            p.recovery = vec![RECOVERY_POINT.into(), "If it stops for conflicts, Abort returns the branch to where it was.".into()];
        }
        Subject::Advanced(Action::Interactive { base, .. } | Action::Squash { base, .. }) => {
            p.refs.push(RefMove { name: branch_name(&branch), before: tip.clone(), after: "the rewritten commits".into() });
            let (list, total, ids) = commits(root, &[&format!("{base}..HEAD")])?;
            p.published = published(root, "HEAD", &ids)?;
            (p.commits, p.commits_total) = (list, total);
            p.commits_note = "are rewritten with new hashes".into();
            p.recovery = vec![RECOVERY_POINT.into(), "If it stops for conflicts, Abort returns the branch to where it was.".into()];
        }
        Subject::Advanced(Action::Merge(id)) => {
            let fast = tip.as_deref().is_some_and(|t| ancestor(root, t, id));
            p.refs.push(RefMove {
                name: branch_name(&branch),
                before: tip.clone(),
                after: if fast { format!("{} (fast-forward)", short(id)) } else { "a new merge commit".into() },
            });
            let (list, total, _) = commits(root, &[id, "--not", "HEAD"])?;
            (p.commits, p.commits_total) = (list, total);
            p.commits_note = format!("join {}", branch_name(&branch));
            if !fast {
                p.overlap = overlap(root, "HEAD", id);
            }
            p.recovery = vec![RECOVERY_POINT.into(), "If it stops for conflicts, Abort returns the branch to where it was.".into()];
        }
        Subject::Advanced(Action::CherryPick { id, .. } | Action::Revert { id, .. }) => {
            let verb = if matches!(subject, Subject::Advanced(Action::CherryPick { .. })) { "copy of" } else { "commit undoing" };
            p.refs.push(RefMove { name: branch_name(&branch), before: tip.clone(), after: format!("a new {verb} {}", short(id)) });
            if let Ok(parent) = git(root, "Read parent", &["rev-parse", "--verify", &format!("{id}^")]) {
                let parent = String::from_utf8_lossy(&parent).trim().to_string();
                let theirs = changed_files(root, &parent, id).unwrap_or_default();
                let since = tip.as_deref().and_then(|t| git(root, "Find merge base", &["merge-base", t, id]).ok());
                if let (Some(base), Some(t)) = (since, tip.as_deref()) {
                    let ours = changed_files(root, String::from_utf8_lossy(&base).trim(), t).unwrap_or_default();
                    let mut both: Vec<_> = ours.intersection(&theirs).cloned().collect();
                    both.sort();
                    p.overlap = both;
                }
            }
            p.recovery = vec![RECOVERY_POINT.into(), "If it stops for conflicts, Abort returns the branch to where it was.".into()];
        }
        Subject::Advanced(Action::DeleteBranch { branch: name, expected }) => {
            p.refs.push(RefMove { name: name.clone(), before: Some(expected.clone()), after: "deleted".into() });
            // Patterns for --branches are relative to refs/heads/.
            let exclude = format!("--exclude={name}");
            let (list, total, ids) = commits(root, &[expected, "--not", &exclude, "--branches", "--tags", "--remotes"])?;
            p.published = published(root, expected, &ids)?;
            (p.commits, p.commits_total) = (list, total);
            p.commits_note = "are on no other branch, tag or remote branch".into();
            p.recovery = vec!["Nyx Git saves a recovery point at the branch's tip first.".into()];
        }
        Subject::DeleteMergedBranch(name) => {
            let id = git(root, "Read branch", &["rev-parse", "--verify", &format!("refs/heads/{name}")])?;
            let id = String::from_utf8_lossy(&id).trim().to_string();
            p.refs.push(RefMove { name: name.clone(), before: Some(id.clone()), after: "deleted".into() });
            p.commits_note = "are on no other branch".into();
            p.recovery = vec![
                format!(
                    "Git refuses unless its commits are already in {} or its upstream, so they stay reachable there.",
                    current.trim_start_matches("refs/heads/")
                ),
                format!("Its tip was {}; a branch can be made there again.", short(&id)),
            ];
        }
        Subject::Advanced(Action::DeleteTag { name, expected }) => {
            p.refs.push(RefMove { name: format!("tag {name}"), before: Some(expected.clone()), after: "deleted".into() });
            p.remote = Some("Tags already pushed stay on remotes.".into());
            p.recovery = vec!["Nyx Git saves a recovery point at the tag's object first.".into()];
        }
        Subject::Advanced(Action::ForcePush { remote, branch: name, lease }) => {
            p.refs.push(RefMove { name: format!("{remote}/{name}"), before: Some(lease.clone()), after: tip.as_deref().map(short).unwrap_or_default() });
            let (list, total, _) = commits(root, &[lease, "--not", "HEAD"])?;
            (p.commits, p.commits_total) = (list, total);
            p.commits_note = format!("leave {remote}/{name} on the server");
            p.remote = Some(format!(
                "The server's {name} is replaced. Anyone who fetched it must reset or rebase. The push is refused if {remote}/{name} has moved since you last fetched."
            ));
            p.irreversible = total > 0;
            p.recovery = vec![format!("Your remote-tracking branch keeps {} until the next fetch.", short(lease))];
        }
        Subject::Advanced(Action::Discard { file, untracked }) => {
            p.at_risk = vec![file.clone()];
            p.risk_note = if *untracked {
                "The untracked file is deleted.".into()
            } else {
                "Its unstaged changes are lost; staged changes stay.".into()
            };
            p.irreversible = true;
            p.recovery = vec!["Nothing: Git never saved these contents.".into()];
        }
        Subject::Advanced(Action::Abort) => {
            let files = dirty(root)?;
            p.refs.push(RefMove { name: branch_name(&branch), before: tip.clone(), after: "where it was before the operation".into() });
            p.at_risk = files.iter().map(|f| f.path.clone()).collect();
            p.risk_note = "Resolutions and edits made during the operation may be lost.".into();
            p.irreversible = !p.at_risk.is_empty();
            p.recovery = vec![RECOVERY_POINT.into()];
        }
        Subject::Advanced(Action::Continue) => {
            p.refs.push(RefMove { name: branch_name(&branch), before: tip.clone(), after: "the next commit of the operation".into() });
            p.recovery = vec![RECOVERY_POINT.into()];
        }
        Subject::Advanced(Action::Checkout(id)) => {
            p.refs.push(RefMove { name: "HEAD".into(), before: tip.clone(), after: format!("{} (detached)", short(id)) });
            p.recovery = vec![format!("{} keeps its commits; switch back to it any time.", branch_name(&branch)), REFLOG.into()];
        }
        Subject::RemoveWorktree(path) => {
            p.at_risk = vec![path.clone()];
            p.risk_note = "The folder is deleted. Git refuses if it has changes or untracked files.".into();
            p.recovery = vec!["Nyx Git saves a recovery point at the worktree's HEAD first; its branch stays.".into()];
        }
        Subject::Advanced(_) => {}
    }
    Ok(p)
}
