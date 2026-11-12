//! Flow: the work in progress in a repository, as Git (and Nyx Agents) see it now. Branches
//! with their fork points and divergence, the worktrees they're checked out in, and agent
//! sessions working in those worktrees. Only refs, merge bases and counts are read, never
//! the whole history.
//!
//! Relationships are kinds of fact, kept apart: Git ancestry (a branch forked from another),
//! a worktree's checkout, an agent's working directory, and derived overlap (two lines of
//! work touching the same files). Nothing is related by being near on the canvas.
use super::{agents, operations, status, worktrees};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Branches read at most.
pub const BRANCH_LIMIT: usize = 500;
/// Branches whose fork points and divergence are worked out.
pub const ACTIVE_LIMIT: usize = 60;
/// A branch with no worktree, agent or unmerged work counts as active this long after its last commit.
pub const RECENT_DAYS: i64 = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub reference: String,
    pub tip: String,
    pub subject: String,
    pub time: i64,
    pub upstream: Option<String>,
    /// Ahead and behind its upstream.
    pub tracking: Option<(usize, usize)>,
    /// Ahead and behind the trunk; None when not worked out (inactive) or for the trunk.
    pub divergence: Option<(usize, usize)>,
    /// The commit it shares with the trunk last (or with `parent`).
    pub fork: Option<String>,
    /// The branch this one was started from: the trunk, or another branch it builds on.
    pub parent: Option<String>,
    pub current: bool,
    pub trunk: bool,
    pub active: bool,
    /// Its commits are all in the trunk.
    pub merged: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub head: Option<String>,
    /// Changed files, None when it can't be read (missing, bare).
    pub dirty: Option<usize>,
    pub main: bool,
    /// The worktree open in this window.
    pub open: bool,
    pub locked: bool,
    pub prunable: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overlap {
    pub a: String,
    pub b: String,
    pub files: Vec<PathBuf>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flow {
    pub trunk: Option<String>,
    pub branches: Vec<Branch>,
    pub worktrees: Vec<Worktree>,
    /// Sessions with the worktree they work in.
    pub agents: Vec<(agents::Session, Option<PathBuf>)>,
    pub overlaps: Vec<Overlap>,
    /// More branches exist than were read.
    pub limited: bool,
}
fn git(root: &Path, label: &str, args: &[&str], allow: bool) -> Result<Vec<u8>> {
    operations::invoke(root, label, args.iter().map(OsString::from).collect(), None, &[], allow)
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim().to_string()
}
fn counts(root: &Path, left: &str, right: &str) -> Option<(usize, usize)> {
    let out = text(git(root, "Count divergence", &["rev-list", "--left-right", "--count", &format!("{left}...{right}")], false).ok()?);
    let mut it = out.split_whitespace().map(|n| n.parse().unwrap_or(0));
    let (behind, ahead) = (it.next()?, it.next()?);
    Some((ahead, behind))
}
fn merge_base(root: &Path, a: &str, b: &str) -> Option<String> {
    git(root, "Find merge base", &["merge-base", a, b], false).ok().map(text).filter(|s| !s.is_empty())
}
fn changed(root: &Path, from: &str, to: &str) -> HashSet<PathBuf> {
    git(root, "List changed files", &["diff", "--name-only", "-z", "--no-renames", from, to, "--"], false)
        .map(|out| out.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned())).collect())
        .unwrap_or_default()
}
/// The trunk: what `origin/HEAD` names, else main, master, trunk or develop, else the current branch.
fn trunk(root: &Path, names: &HashSet<String>, current: Option<&str>) -> Option<String> {
    if let Ok(out) = git(root, "Read default branch", &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"], true) {
        let remote = text(out);
        if let Some(name) = remote.strip_prefix("origin/")
            && names.contains(name)
        {
            return Some(name.into());
        }
    }
    ["main", "master", "trunk", "develop"].iter().find(|n| names.contains(**n)).map(|n| n.to_string()).or(current.map(String::from))
}
fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Read the repository's work in progress, with `open` the worktree shown in the window.
pub fn read(root: &Path, open: &Path, sessions: Vec<agents::Session>) -> Result<Flow> {
    let listing = git(
        root,
        "Read branches",
        &[
            "for-each-ref",
            &format!("--count={}", BRANCH_LIMIT + 1),
            "--sort=-committerdate",
            "--format=%(refname)%00%(objectname)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(committerdate:unix)%00%(HEAD)%00%(contents:subject)",
            "refs/heads/",
        ],
        false,
    )?;
    let mut branches = Vec::new();
    for line in String::from_utf8_lossy(&listing).lines() {
        let f: Vec<&str> = line.splitn(7, '\0').collect();
        if f.len() < 7 {
            continue;
        }
        let tracking = (!f[2].is_empty()).then(|| {
            let (mut ahead, mut behind) = (0, 0);
            for part in f[3].split(", ") {
                if let Some(n) = part.strip_prefix("ahead ") {
                    ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix("behind ") {
                    behind = n.parse().unwrap_or(0);
                }
            }
            (ahead, behind)
        });
        branches.push(Branch {
            name: f[0].trim_start_matches("refs/heads/").into(),
            reference: f[0].into(),
            tip: f[1].into(),
            subject: f[6].chars().take(200).collect(),
            time: f[4].parse().unwrap_or(0),
            upstream: (!f[2].is_empty()).then(|| f[2].into()),
            tracking,
            divergence: None,
            fork: None,
            parent: None,
            current: f[5] == "*",
            trunk: false,
            active: false,
            merged: false,
        });
    }
    let limited = branches.len() > BRANCH_LIMIT;
    branches.truncate(BRANCH_LIMIT);
    let open_root = open.canonicalize().unwrap_or_else(|_| open.to_path_buf());
    let trees: Vec<Worktree> = worktrees::list(root)?
        .into_iter()
        .take(64)
        .map(|w| {
            let dirty = (!w.bare && w.path.exists())
                .then(|| operations::run(&w.path, operations::Operation::Status).ok().and_then(|b| status::parse(&b).ok()).map(|s| s.files.len()))
                .flatten();
            let canonical = w.path.canonicalize().unwrap_or_else(|_| w.path.clone());
            Worktree {
                open: canonical == open_root,
                branch: w.branch.as_deref().map(|b| b.trim_start_matches("refs/heads/").to_string()),
                head: w.head.clone(),
                dirty,
                main: w.main,
                locked: w.locked.is_some(),
                prunable: w.prunable.is_some(),
                path: w.path,
            }
        })
        .collect();
    let paths: Vec<PathBuf> = trees.iter().map(|w| w.path.clone()).collect();
    let agents: Vec<_> = sessions.into_iter().map(|s| {
        let w = agents::worktree_of(&s, &paths).cloned();
        (s, w)
    }).filter(|(_, w)| w.is_some()).collect();
    let names: HashSet<String> = branches.iter().map(|b| b.name.clone()).collect();
    let current = branches.iter().find(|b| b.current).map(|b| b.name.clone());
    let trunk = trunk(root, &names, current.as_deref());
    let checked_out: HashSet<&str> = trees.iter().filter_map(|w| w.branch.as_deref()).collect();
    let agent_branches: HashSet<String> = agents
        .iter()
        .filter_map(|(_, w)| w.as_ref())
        .filter_map(|w| trees.iter().find(|t| &t.path == w).and_then(|t| t.branch.clone()))
        .collect();
    let recent = now() - RECENT_DAYS * 86400;
    let trunk_tip = trunk.as_ref().and_then(|t| branches.iter().find(|b| &b.name == t)).map(|b| b.tip.clone());
    let mut worked = 0;
    for b in branches.iter_mut() {
        b.trunk = trunk.as_deref() == Some(b.name.as_str());
        if b.trunk {
            b.active = true;
            continue;
        }
        let pinned = b.current || checked_out.contains(b.name.as_str()) || agent_branches.contains(&b.name);
        if !(pinned || b.time >= recent) || worked >= ACTIVE_LIMIT {
            continue;
        }
        worked += 1;
        if let Some(t) = &trunk_tip {
            b.divergence = counts(root, t, &b.tip);
            b.fork = merge_base(root, t, &b.tip);
            b.merged = b.divergence.is_some_and(|(ahead, _)| ahead == 0);
            b.parent = trunk.clone();
        }
        b.active = pinned || !b.merged;
    }
    // A branch started from another unmerged branch builds on it: the other's tip is among its
    // own commits beyond the trunk. Of several, the nearest (the one with most commits) wins.
    if let Some(t) = &trunk_tip {
        let own: HashMap<String, HashSet<String>> = branches
            .iter()
            .filter(|b| b.active && !b.trunk && !b.merged)
            .take(ACTIVE_LIMIT)
            .map(|b| {
                let ids = git(root, "List branch commits", &["rev-list", "--max-count=2000", &b.tip, "--not", t], false)
                    .map(|o| String::from_utf8_lossy(&o).lines().map(String::from).collect())
                    .unwrap_or_default();
                (b.name.clone(), ids)
            })
            .collect();
        let tips: HashMap<String, String> = branches.iter().map(|b| (b.name.clone(), b.tip.clone())).collect();
        let mut parents: HashMap<String, String> = HashMap::new();
        for (name, commits) in &own {
            let best = own
                .iter()
                .filter(|(other, theirs)| *other != name && commits.contains(&tips[*other]) && tips[*other] != tips[name] && theirs.len() < commits.len())
                .max_by_key(|(other, theirs)| (theirs.len(), std::cmp::Reverse((*other).clone())));
            if let Some((other, _)) = best {
                parents.insert(name.clone(), other.clone());
            }
        }
        for b in branches.iter_mut() {
            if let Some(parent) = parents.remove(&b.name) {
                b.fork = Some(tips[&parent].clone());
                b.parent = Some(parent);
            }
        }
    }
    // Derived: lines of work that touch the same files (committed since their fork, plus
    // uncommitted changes in their worktrees).
    let mut touched: BTreeMap<String, HashSet<PathBuf>> = BTreeMap::new();
    for b in branches.iter().filter(|b| b.active && !b.trunk && !b.merged).take(40) {
        let mut files = b.fork.as_ref().map(|f| changed(root, f, &b.tip)).unwrap_or_default();
        if let Some(w) = trees.iter().find(|w| w.branch.as_deref() == Some(b.name.as_str()) && w.dirty.unwrap_or(0) > 0)
            && let Ok(s) = operations::run(&w.path, operations::Operation::Status).and_then(|b| status::parse(&b))
        {
            files.extend(s.files.into_iter().map(|f| f.path));
        }
        touched.insert(b.name.clone(), files);
    }
    let names: Vec<&String> = touched.keys().collect();
    let mut overlaps = Vec::new();
    for (i, a) in names.iter().enumerate() {
        for b in &names[i + 1..] {
            let mut files: Vec<PathBuf> = touched[*a].intersection(&touched[*b]).cloned().collect();
            if !files.is_empty() {
                files.sort();
                overlaps.push(Overlap { a: (*a).clone(), b: (*b).clone(), files });
            }
        }
    }
    Ok(Flow { trunk, branches, worktrees: trees, agents, overlaps, limited })
}

// ---------- Layout ----------

/// A card's place on the canvas, in canvas units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub x: f64,
    pub y: f64,
}
pub const CARD_W: f64 = 248.0;
pub const CARD_H: f64 = 112.0;
const GAP_X: f64 = 72.0;
const GAP_Y: f64 = 28.0;

/// Node ids: `branch:NAME` and `worktree:PATH` (a worktree on no branch).
pub fn branch_id(name: &str) -> String {
    format!("branch:{name}")
}
pub fn worktree_id(path: &Path) -> String {
    format!("worktree:{}", path.display())
}
/// The cards shown, with `inactive` included or not: (id, parent id).
pub fn nodes(flow: &Flow, inactive: bool) -> Vec<(String, Option<String>)> {
    let shown: Vec<&Branch> = flow.branches.iter().filter(|b| inactive || b.active).collect();
    let names: HashSet<&str> = shown.iter().map(|b| b.name.as_str()).collect();
    let mut out: Vec<(String, Option<String>)> = shown
        .iter()
        .map(|b| {
            let parent = b.parent.as_deref().filter(|p| names.contains(p) && !b.trunk).map(branch_id).or_else(|| {
                (!b.trunk).then(|| flow.trunk.as_deref().filter(|t| names.contains(t)).map(branch_id)).flatten()
            });
            (branch_id(&b.name), parent)
        })
        .collect();
    for w in flow.worktrees.iter().filter(|w| w.branch.is_none()) {
        out.push((worktree_id(&w.path), flow.trunk.as_deref().map(branch_id)));
    }
    out
}
/// A deterministic tree layout: the trunk on the left, each branch one column right of the
/// branch it was started from, siblings stacked by when they forked (oldest first). The same
/// repository state always gives the same picture.
pub fn layout(flow: &Flow, nodes: &[(String, Option<String>)]) -> BTreeMap<String, Place> {
    let time = |id: &str| -> i64 {
        id.strip_prefix("branch:")
            .and_then(|n| flow.branches.iter().find(|b| b.name == n))
            .map_or(i64::MAX, |b| if b.trunk { i64::MIN } else { b.time })
    };
    let mut children: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
    let ids: HashSet<&String> = nodes.iter().map(|(id, _)| id).collect();
    for (id, parent) in nodes {
        let parent = parent.clone().filter(|p| ids.contains(p) && p != id);
        children.entry(parent).or_default().push(id.clone());
    }
    for list in children.values_mut() {
        list.sort_by(|a, b| time(a).cmp(&time(b)).then(a.cmp(b)));
    }
    let mut places = BTreeMap::new();
    let mut row = 0usize;
    fn walk(
        id: &str,
        depth: usize,
        children: &BTreeMap<Option<String>, Vec<String>>,
        row: &mut usize,
        places: &mut BTreeMap<String, Place>,
        seen: &mut HashSet<String>,
    ) {
        if !seen.insert(id.to_string()) {
            return;
        }
        places.insert(id.to_string(), Place { x: depth as f64 * (CARD_W + GAP_X), y: *row as f64 * (CARD_H + GAP_Y) });
        let kids = children.get(&Some(id.to_string())).cloned().unwrap_or_default();
        if kids.is_empty() {
            *row += 1;
        }
        for (i, kid) in kids.iter().enumerate() {
            // The first child sits level with its parent.
            if i == 0 {
                let r = places[id].y / (CARD_H + GAP_Y);
                *row = r as usize;
            }
            walk(kid, depth + 1, children, row, places, seen);
        }
    }
    let mut seen = HashSet::new();
    for root in children.get(&None).cloned().unwrap_or_default() {
        walk(&root, 0, &children, &mut row, &mut places, &mut seen);
    }
    places
}

/// Where the reader moved cards, per repository, apart from anything in Git.
pub fn positions_file(common_dir: &Path) -> PathBuf {
    let journal = super::journal::file(common_dir);
    let name = journal.file_stem().map(|s| s.to_os_string()).unwrap_or_default();
    super::journal::directory().with_file_name("flow").join(name).with_extension("json")
}
pub fn load_positions(common_dir: &Path) -> BTreeMap<String, Place> {
    std::fs::read(positions_file(common_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}
pub fn save_positions(common_dir: &Path, places: &BTreeMap<String, Place>) -> Result<()> {
    let path = positions_file(common_dir);
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(places)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn branch(name: &str, parent: Option<&str>, time: i64, trunk: bool) -> Branch {
        Branch {
            name: name.into(),
            reference: format!("refs/heads/{name}"),
            tip: String::new(),
            subject: String::new(),
            time,
            upstream: None,
            tracking: None,
            divergence: None,
            fork: None,
            parent: parent.map(String::from),
            current: false,
            trunk,
            active: true,
            merged: false,
        }
    }
    #[test]
    fn layout_is_a_stable_tree_with_stacked_branches_one_column_right() {
        let flow = Flow {
            trunk: Some("main".into()),
            branches: vec![
                branch("main", None, 50, true),
                branch("b", Some("main"), 20, false),
                branch("a", Some("main"), 10, false),
                branch("a2", Some("a"), 30, false),
            ],
            ..Default::default()
        };
        let n = nodes(&flow, false);
        let first = layout(&flow, &n);
        assert_eq!(first, layout(&flow, &n), "deterministic");
        let (main, a, a2, b) = (first["branch:main"], first["branch:a"], first["branch:a2"], first["branch:b"]);
        assert_eq!(main.x, 0.0);
        assert_eq!(a.x, b.x);
        assert!(a2.x > a.x, "a2 builds on a");
        assert!(a.y < b.y, "older forks first");
        assert_eq!(a.y, main.y);
        assert_eq!(a2.y, a.y);
        let ys: HashSet<i64> = first.values().map(|p| (p.y * 10.0) as i64 * 1000 + p.x as i64).collect();
        assert_eq!(ys.len(), first.len(), "no two cards in one place");
    }
}
