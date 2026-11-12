//! Native Git operations with fresh-state guards and durable recovery references.
use super::{
    operations::{self, Operation},
    repository, status,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Active {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}
impl Active {
    pub fn name(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::CherryPick => "Cherry-pick",
            Self::Revert => "Revert",
        }
    }
    fn command(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }
}
pub fn active(git_dir: &Path) -> Option<Active> {
    if git_dir.join("rebase-merge").is_dir()
        || (git_dir.join("rebase-apply").is_dir() && !git_dir.join("rebase-apply/applying").exists())
    {
        Some(Active::Rebase)
    } else if git_dir.join("MERGE_HEAD").exists() {
        Some(Active::Merge)
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        Some(Active::CherryPick)
    } else if git_dir.join("REVERT_HEAD").exists() {
        Some(Active::Revert)
    } else {
        None
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    inode: u64,
    len: u64,
    mode: u32,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
fn stamp(path: &Path) -> Result<Option<Stamp>> {
    match fs::symlink_metadata(path) {
        Ok(m) => Ok(Some(Stamp {
            inode: m.ino(),
            len: m.len(),
            mode: m.mode(),
            mtime: m.mtime(),
            mtime_ns: m.mtime_nsec(),
            ctime: m.ctime(),
            ctime_ns: m.ctime_nsec(),
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guard {
    pub root: PathBuf,
    pub head: Option<String>,
    pub branch: String,
    pub progress: Option<Active>,
    pub status: Vec<u8>,
    index: Option<Stamp>,
    files: Vec<(PathBuf, Option<Stamp>)>,
}
impl Guard {
    pub fn capture(root: &Path) -> Result<Self> {
        let repo = repository::inspect(root)?;
        ensure!(!repo.bare, "This action requires a working tree");
        let bytes = operations::run(&repo.root, Operation::Status)?;
        let parsed = status::parse(&bytes)?;
        let mut paths: Vec<_> =
            parsed.files.iter().flat_map(|f| std::iter::once(f.path.clone()).chain(f.original.clone())).collect();
        paths.sort();
        paths.dedup();
        let files = paths.into_iter().map(|path| stamp(&repo.root.join(&path)).map(|s| (path, s))).collect::<Result<_>>()?;
        Ok(Self {
            root: repo.root,
            head: repo.head,
            branch: repo.branch,
            progress: active(&repo.git_dir),
            status: bytes,
            index: stamp(&repo.git_dir.join("index"))?,
            files,
        })
    }
    pub fn verify(&self, root: &Path) -> Result<()> {
        let now = Self::capture(root)?;
        ensure!(now == *self, "The repository changed after this action was prepared. Refresh and confirm the action again.");
        Ok(())
    }
    pub fn clean(&self) -> Result<()> {
        ensure!(
            status::parse(&self.status)?.files.is_empty(),
            "Commit or stash local changes, including untracked files, before this operation"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Pick {
    Pick,
    Squash,
    Fixup,
    Drop,
    /// Keep the changes with a new message (the step's `message`).
    Reword,
    /// Stop after this commit to amend it.
    Edit,
    /// Stop after this commit with its changes unstaged, to commit them in pieces.
    Split,
}
impl Pick {
    fn name(&self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Drop => "drop",
            Self::Reword => "reword",
            Self::Edit | Self::Split => "edit",
        }
    }
    /// Starts a commit of its own (rather than joining the previous one, or leaving).
    pub fn keeps(&self) -> bool {
        matches!(self, Self::Pick | Self::Reword | Self::Edit | Self::Split)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub action: Pick,
    pub subject: String,
    /// The new message, for Reword.
    #[serde(default)]
    pub message: Option<String>,
}
#[derive(Clone, Debug)]
pub enum Action {
    Merge(String),
    Rebase(String),
    CherryPick { id: String, mainline: Option<u32> },
    Revert { id: String, mainline: Option<u32> },
    Reset { target: String, mode: ResetMode },
    Interactive { base: String, steps: Vec<Step>, editor: PathBuf },
    Squash { base: String, message: String },
    CreateTag { name: String, target: String, message: String },
    DeleteTag { name: String, expected: String },
    DeleteBranch { branch: String, expected: String },
    BranchAt { name: String, target: String },
    SwitchNew { name: String, target: String },
    Checkout(String),
    ForcePush { remote: String, branch: String, lease: String },
    Discard { file: PathBuf, untracked: bool },
    Continue,
    /// Leave out the commit the operation stopped at.
    Skip,
    Abort,
}
fn args(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}
pub fn valid_ref(prefix: &str, name: &str) -> Result<()> {
    ensure!(!name.is_empty() && !name.starts_with('-'), "Enter a valid reference name");
    gix::refs::FullName::try_from(format!("{prefix}{name}")).context("Invalid reference name")?;
    Ok(())
}
impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Merge(_) => "Merge",
            Self::Rebase(_) => "Rebase",
            Self::CherryPick { .. } => "Cherry-pick",
            Self::Revert { .. } => "Revert",
            Self::Reset { .. } => "Reset",
            Self::Interactive { .. } => "Interactive rebase",
            Self::Squash { .. } => "Squash commits",
            Self::CreateTag { .. } => "Create tag",
            Self::DeleteTag { .. } => "Delete tag",
            Self::DeleteBranch { .. } => "Delete branch",
            Self::BranchAt { .. } => "Create branch",
            Self::SwitchNew { .. } => "Create and switch branch",
            Self::Checkout(_) => "Check out commit",
            Self::ForcePush { .. } => "Force push with lease",
            Self::Discard { .. } => "Discard working changes",
            Self::Continue => "Continue operation",
            Self::Skip => "Skip commit",
            Self::Abort => "Abort operation",
        }
    }
    pub fn args(&self, progress: Option<Active>) -> Result<Vec<OsString>> {
        let mut a = match self {
            Self::Merge(id) => {
                operations::valid_id(id)?;
                args(&["merge", "--no-edit", "--no-autostash", "--", id])
            }
            Self::Rebase(id) => {
                operations::valid_id(id)?;
                args(&["rebase", "--rebase-merges", "--no-autostash", "--no-update-refs", "--", id])
            }
            Self::CherryPick { id, mainline } | Self::Revert { id, mainline } => {
                operations::valid_id(id)?;
                let verb = if matches!(self, Self::CherryPick { .. }) { "cherry-pick" } else { "revert" };
                let mut a = args(&[verb, "--no-edit"]);
                if let Some(parent) = mainline {
                    ensure!(*parent > 0, "Choose a mainline parent");
                    a.extend(args(&["--mainline", &parent.to_string()]));
                }
                a.extend(args(&["--", id]));
                a
            }
            Self::Reset { target, mode } => {
                operations::valid_id(target)?;
                args(&[
                    "reset",
                    match mode {
                        ResetMode::Soft => "--soft",
                        ResetMode::Mixed => "--mixed",
                        ResetMode::Hard => "--hard",
                    },
                    target,
                    "--",
                ])
            }
            Self::Interactive { base, .. } => {
                operations::valid_id(base)?;
                args(&["rebase", "--interactive", "--no-autostash", "--no-update-refs", "--no-rebase-merges", "--", base])
            }
            Self::CreateTag { name, target, message } => {
                valid_ref("refs/tags/", name)?;
                operations::valid_id(target)?;
                ensure!(!message.trim().is_empty(), "Enter a tag message");
                args(&["tag", "--annotate", "--file=-", "--", name, target])
            }
            Self::DeleteTag { name, .. } => {
                valid_ref("refs/tags/", name)?;
                args(&["tag", "--delete", "--", name])
            }
            Self::DeleteBranch { branch, .. } => {
                valid_ref("refs/heads/", branch)?;
                args(&["branch", "--delete", "--force", "--", branch])
            }
            Self::BranchAt { name, target } => {
                valid_ref("refs/heads/", name)?;
                operations::valid_id(target)?;
                args(&["branch", "--", name, target])
            }
            // Git refuses a switch that would overwrite local changes.
            Self::SwitchNew { name, target } => {
                valid_ref("refs/heads/", name)?;
                operations::valid_id(target)?;
                args(&["switch", "--no-guess", "--create", name, "--", target])
            }
            Self::Checkout(id) => {
                operations::valid_id(id)?;
                args(&["switch", "--detach", "--", id])
            }
            Self::ForcePush { remote, branch, lease } => {
                valid_ref("refs/heads/", branch)?;
                operations::valid_id(lease)?;
                ensure!(!remote.is_empty() && !remote.starts_with('-'), "Choose a remote");
                args(&[
                    "push",
                    &format!("--force-with-lease=refs/heads/{branch}:{lease}"),
                    "--",
                    remote,
                    &format!("HEAD:refs/heads/{branch}"),
                ])
            }
            Self::Discard { file, untracked } => {
                let mut a = if *untracked {
                    args(&["--literal-pathspecs", "clean", "--force"])
                } else {
                    args(&["--literal-pathspecs", "restore", "--worktree"])
                };
                operations::paths(&mut a, std::slice::from_ref(file))?;
                a
            }
            Self::Continue | Self::Abort | Self::Skip => {
                let progress = progress.context("No merge, rebase, cherry-pick or revert is in progress")?;
                if matches!(self, Self::Continue) && progress == Active::Merge {
                    args(&["commit", "--no-edit"])
                } else if matches!(self, Self::Skip) {
                    ensure!(progress != Active::Merge, "A merge can't skip a commit; abort it instead");
                    args(&[progress.command(), "--skip"])
                } else {
                    args(&[progress.command(), if matches!(self, Self::Continue) { "--continue" } else { "--abort" }])
                }
            }
            Self::Squash { .. } => bail!("Squash uses a guarded multi-step operation"),
        };
        a.insert(0, "--no-pager".into());
        Ok(a)
    }
}
fn git(root: &Path, label: &str, a: Vec<OsString>, input: Option<Vec<u8>>, env: &[(OsString, OsString)]) -> Result<Vec<u8>> {
    operations::invoke(root, label, a, input, env, false)
}
pub(crate) fn checkpoint(root: &Path, id: &str) -> Result<String> {
    operations::valid_id(id)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let reference = format!("refs/nyx/recovery/{stamp}-{}", std::process::id());
    git(
        root,
        "Save recovery point",
        args(&["update-ref", "--create-reflog", "-m", "Nyx recovery point", &reference, id, ""]),
        None,
        &[],
    )?;
    Ok(reference)
}
pub fn resolve_target(root: &Path, text: &str) -> Result<String> {
    let repo = gix::discover(root)?;
    if text == "HEAD" {
        return Ok(repo.head_commit()?.id.to_string());
    }
    if let Ok(oid) = gix::ObjectId::from_hex(text.as_bytes()) {
        return Ok(repo.find_object(oid)?.peel_to_kind(gix::objs::Kind::Commit)?.id.to_string());
    }
    let (references, _) = super::history::references(&repo)?;
    let ids: Vec<_> = references
        .iter()
        .filter(|(_, labels)| labels.iter().any(|r| r.full_name == text || r.name == text))
        .map(|(id, _)| id.to_string())
        .collect();
    ensure!(ids.len() == 1, "Choose an unambiguous branch, tag, full ref, or full commit hash");
    Ok(ids[0].clone())
}
pub fn plan(root: &Path, base: &str) -> Result<Vec<Step>> {
    operations::valid_id(base)?;
    git(root, "Check ancestry", args(&["merge-base", "--is-ancestor", base, "HEAD"]), None, &[])?;
    let merges = git(
        root,
        "Check linear history",
        args(&["rev-list", "--max-count=1", "--min-parents=2", &format!("{base}..HEAD")]),
        None,
        &[],
    )?;
    ensure!(
        merges.is_empty(),
        "Interactive rebase and squash require a linear range. Use a normal rebase to preserve merge topology."
    );
    let ids = git(
        root,
        "Read rebase plan",
        args(&["rev-list", "--max-count=501", "--reverse", "--topo-order", &format!("{base}..HEAD")]),
        None,
        &[],
    )?;
    let repo = gix::discover(root)?;
    let mut steps = Vec::new();
    for id in String::from_utf8(ids)?.lines() {
        let object = repo.find_object(gix::ObjectId::from_hex(id.as_bytes())?)?.try_into_commit()?;
        steps.push(Step {
            id: id.into(),
            action: Pick::Pick,
            subject: object.message_raw_sloppy().to_string().lines().next().unwrap_or_default().chars().take(200).collect(),
            message: None,
        });
    }
    ensure!(!steps.is_empty(), "There are no commits after this base");
    ensure!(steps.len() <= 500, "Choose a base within 500 commits for interactive editing");
    Ok(steps)
}
fn validate_plan(original: &[Step], steps: &[Step]) -> Result<()> {
    use std::collections::BTreeSet;
    let expected: BTreeSet<_> = original.iter().map(|s| &s.id).collect();
    let actual: BTreeSet<_> = steps.iter().map(|s| &s.id).collect();
    ensure!(
        original.len() == steps.len() && expected == actual && actual.len() == steps.len(),
        "The plan must contain each original commit exactly once; use Drop to omit a commit"
    );
    ensure!(
        steps.iter().find(|s| s.action != Pick::Drop).is_some_and(|s| s.action.keeps()),
        "The first kept commit can't be squashed or fixed up into nothing"
    );
    for s in steps.iter().filter(|s| s.action == Pick::Reword) {
        ensure!(s.message.as_ref().is_some_and(|m| !m.trim().is_empty()), "Enter a message for each reworded commit");
    }
    Ok(())
}
/// The studio's state for a rebase in progress, kept in the worktree's Git directory so a
/// later Continue or Skip still rewords and splits as planned.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Studio {
    messages: std::collections::BTreeMap<String, String>,
    splits: Vec<String>,
    handled: Vec<String>,
}
fn studio_dir(git_dir: &Path) -> PathBuf {
    git_dir.join("nyx-rebase")
}
fn read_studio(git_dir: &Path) -> Option<Studio> {
    serde_json::from_slice(&fs::read(studio_dir(git_dir).join("studio.json")).ok()?).ok()
}
fn write_studio(git_dir: &Path, studio: &Studio) -> Result<()> {
    fs::write(studio_dir(git_dir).join("studio.json"), serde_json::to_vec(studio)?)?;
    Ok(())
}
/// Install the message editor for a planned rebase. Git calls it for each reword.
fn prepare_studio(git_dir: &Path, steps: &[Step], editor: &Path) -> Result<()> {
    let dir = studio_dir(git_dir);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    let link = dir.join("nyx-git-message-editor");
    std::os::unix::fs::symlink(editor, &link)?;
    let studio = Studio {
        messages: steps.iter().filter(|s| s.action == Pick::Reword).filter_map(|s| Some((s.id.clone(), s.message.clone()?))).collect(),
        splits: steps.iter().filter(|s| s.action == Pick::Split).map(|s| s.id.clone()).collect(),
        handled: Vec::new(),
    };
    write_studio(git_dir, &studio)
}
/// The step a rebase last did, from `rebase-merge/done`: (command, commit).
fn last_done(git_dir: &Path) -> Option<(String, String)> {
    let done = fs::read_to_string(git_dir.join("rebase-merge/done")).ok()?;
    let line = done.lines().rev().find(|l| !l.trim().is_empty() && !l.starts_with('#'))?;
    let mut words = line.split_whitespace();
    Some((words.next()?.to_string(), words.next()?.to_string()))
}
/// After a rebase step: unstage a commit planned for splitting where the rebase stopped at
/// it, and tidy up once the rebase is over. Returns what happened, for the reader.
fn after_rebase(root: &Path, git_dir: &Path) -> Result<Option<String>> {
    let dir = studio_dir(git_dir);
    if !dir.exists() {
        return Ok(None);
    }
    if !git_dir.join("rebase-merge").is_dir() {
        fs::remove_dir_all(&dir)?;
        return Ok(None);
    }
    let Some(mut studio) = read_studio(git_dir) else { return Ok(None) };
    let Some((command, id)) = last_done(git_dir) else { return Ok(None) };
    if !matches!(command.as_str(), "edit" | "e") {
        return Ok(None);
    }
    let Some(full) = studio.splits.iter().find(|s| s.starts_with(&id) && !studio.handled.contains(s)).cloned() else { return Ok(None) };
    studio.handled.push(full.clone());
    write_studio(git_dir, &studio)?;
    // Git writes `amend` when it stops cleanly after an edit step; a conflict stop has none.
    let clean = git_dir.join("rebase-merge/amend").exists()
        && git(root, "Check conflicts", args(&["ls-files", "--unmerged"]), None, &[]).is_ok_and(|o| o.is_empty());
    if !clean {
        return Ok(Some("The rebase stopped for a conflict before the split; resolve it, then Continue.".into()));
    }
    let parent = git(root, "Read parent", args(&["rev-parse", "--verify", "--quiet", "HEAD^"]), None, &[]).ok();
    match parent {
        Some(_) => {
            git(root, "Unstage commit for splitting", args(&["reset", "--mixed", "--quiet", "HEAD^", "--"]), None, &[])?;
            Ok(Some(format!(
                "Stopped to split {}: its changes are unstaged in Changes. Commit them in pieces, then Continue.",
                &full[..full.len().min(10)]
            )))
        }
        None => Ok(Some("A root commit can't be split here; amend it and Continue.".into())),
    }
}
/// The message editor Git runs for a reworded commit during a studio rebase.
pub fn message_editor() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    let me = PathBuf::from(args.first()?);
    if me.file_name().is_none_or(|n| n != "nyx-git-message-editor") {
        return None;
    }
    let result = (|| -> Result<()> {
        let file = PathBuf::from(args.get(1).context("Missing message file")?);
        let dir = me.parent().context("Missing studio directory")?;
        let git_dir = dir.parent().context("Missing Git directory")?;
        let Some((command, id)) = last_done(git_dir) else { return Ok(()) };
        if !matches!(command.as_str(), "reword" | "r") {
            // Squash and fixup messages, and anything else, are left as Git prepared them.
            return Ok(());
        }
        let studio = read_studio(git_dir).unwrap_or_default();
        if let Some((_, message)) = studio.messages.iter().find(|(k, _)| k.starts_with(&id)) {
            fs::write(file, format!("{}\n", message.trim_end()))?;
        }
        Ok(())
    })();
    Some(match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Nyx message editor: {e:#}");
            1
        }
    })
}
/// What the Rebase Studio shows next to a plan: the files each commit changes, which commits
/// a remote already has, the full messages, and other references inside the range (they stay
/// on the old commits).
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub branch: String,
    pub files: std::collections::HashMap<String, Vec<PathBuf>>,
    pub published: std::collections::HashSet<String>,
    pub messages: std::collections::HashMap<String, String>,
    pub other_refs: Vec<(String, String)>,
}
pub fn facts(root: &Path, base: &str) -> Result<Facts> {
    operations::valid_id(base)?;
    let range = format!("{base}..HEAD");
    // Paths are for warnings only, so Git's quoting of unusual names is fine here.
    let log = git(root, "Read changed files", args(&["log", "--format=%H", "--name-only", "--no-renames", &range]), None, &[])?;
    let mut files: std::collections::HashMap<String, Vec<PathBuf>> = std::collections::HashMap::new();
    let mut current: Option<String> = None;
    for line in String::from_utf8_lossy(&log).lines() {
        if line.len() == 40 && line.bytes().all(|b| b.is_ascii_hexdigit()) {
            current = Some(line.to_string());
            files.entry(line.to_string()).or_default();
        } else if !line.is_empty()
            && let Some(id) = &current
        {
            files.entry(id.clone()).or_default().push(PathBuf::from(line));
        }
    }
    let ids: Vec<String> = String::from_utf8_lossy(&git(root, "List range", args(&["rev-list", &range]), None, &[])?).lines().map(String::from).collect();
    let unpublished: std::collections::HashSet<String> =
        String::from_utf8_lossy(&git(root, "Check published commits", args(&["rev-list", &range, "--not", "--remotes"]), None, &[])?).lines().map(String::from).collect();
    let published = ids.iter().filter(|id| !unpublished.contains(*id)).cloned().collect();
    let raw = git(root, "Read messages", args(&["log", "--format=%H%x00%B%x00", &range]), None, &[])?;
    let mut messages = std::collections::HashMap::new();
    let mut parts = raw.split(|b| *b == 0);
    while let (Some(id), Some(body)) = (parts.next(), parts.next()) {
        let id = String::from_utf8_lossy(id).trim().to_string();
        if !id.is_empty() {
            messages.insert(id, String::from_utf8_lossy(body).trim().to_string());
        }
    }
    let branch = String::from_utf8_lossy(&git(root, "Read branch", args(&["rev-parse", "--abbrev-ref", "HEAD"]), None, &[])?).trim().to_string();
    let set: std::collections::HashSet<&String> = ids.iter().collect();
    let refs = git(root, "Read references", args(&["for-each-ref", "--format=%(objectname) %(refname:short)", "refs/heads", "refs/tags"]), None, &[])?;
    let other_refs = String::from_utf8_lossy(&refs)
        .lines()
        .filter_map(|l| l.split_once(' '))
        .filter(|(id, name)| set.contains(&id.to_string()) && *name != branch)
        .map(|(id, name)| (name.to_string(), id.to_string()))
        .collect();
    Ok(Facts { branch, files, published, messages, other_refs })
}
/// A commit of the history a plan would make: the original commits folded into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposed {
    pub ids: Vec<String>,
    pub subject: String,
    pub action: Pick,
}
/// The history `steps` would make, oldest first: squashes and fixups join the commit before,
/// drops leave.
pub fn proposed(steps: &[Step]) -> Vec<Proposed> {
    let mut out: Vec<Proposed> = Vec::new();
    for s in steps {
        match s.action {
            Pick::Drop => {}
            Pick::Squash | Pick::Fixup => {
                if let Some(last) = out.last_mut() {
                    last.ids.push(s.id.clone());
                }
            }
            _ => out.push(Proposed {
                ids: vec![s.id.clone()],
                subject: if s.action == Pick::Reword {
                    s.message.as_deref().and_then(|m| m.lines().next()).unwrap_or(&s.subject).to_string()
                } else {
                    s.subject.clone()
                },
                action: s.action.clone(),
            }),
        }
    }
    out
}
/// Where reordering may conflict: a commit moved before one it used to follow, when both
/// change the same file. A forecast, not a promise either way.
pub fn reorder_warnings(original: &[Step], steps: &[Step], files: &std::collections::HashMap<String, Vec<PathBuf>>) -> Vec<String> {
    let position: std::collections::HashMap<&str, usize> = original.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
    let kept: Vec<&Step> = steps.iter().filter(|s| s.action != Pick::Drop).collect();
    let mut out = Vec::new();
    for (j, later) in kept.iter().enumerate() {
        for earlier in &kept[..j] {
            let (Some(a), Some(b)) = (position.get(earlier.id.as_str()), position.get(later.id.as_str())) else { continue };
            if a > b {
                let shared: Vec<_> = files
                    .get(&earlier.id)
                    .into_iter()
                    .flatten()
                    .filter(|f| files.get(&later.id).is_some_and(|g| g.contains(f)))
                    .map(|f| f.display().to_string())
                    .collect();
                if !shared.is_empty() {
                    out.push(format!(
                        "{} now comes before {}; both change {}.",
                        &earlier.id[..earlier.id.len().min(8)],
                        &later.id[..later.id.len().min(8)],
                        shared.join(", ")
                    ));
                }
            }
        }
    }
    // Dropping a commit that later kept commits build on.
    for (i, s) in steps.iter().enumerate().filter(|(_, s)| s.action == Pick::Drop) {
        let touched = files.get(&s.id).cloned().unwrap_or_default();
        if steps[i + 1..].iter().any(|t| t.action != Pick::Drop && files.get(&t.id).is_some_and(|g| g.iter().any(|f| touched.contains(f)))) {
            out.push(format!("{} is dropped, but later commits change the same files.", &s.id[..s.id.len().min(8)]));
        }
    }
    out
}
/// Called only through the sequence-editor symlink, before GTK is initialized.
pub fn sequence_editor() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.first().and_then(|a| Path::new(a).file_name()).is_none_or(|n| n != "nyx-git-sequence-editor") {
        return None;
    }
    let result = (|| -> Result<()> {
        let input = std::env::var_os("NYX_GIT_REBASE_PLAN").context("Missing rebase plan")?;
        let steps: Vec<Step> = serde_json::from_slice(&fs::read(input)?)?;
        let target = PathBuf::from(args.get(1).context("Missing Git todo file")?);
        ensure!(target.file_name().is_some_and(|n| n == "git-rebase-todo"), "Unexpected editor target");
        let mut todo = String::new();
        for step in steps {
            operations::valid_id(&step.id)?;
            todo.push_str(&format!("{} {}\n", step.action.name(), step.id));
        }
        fs::write(target, todo)?;
        Ok(())
    })();
    Some(match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Nyx sequence editor: {e:#}");
            1
        }
    })
}
pub fn run(root: &Path, action: Action, guard: &Guard) -> Result<Vec<u8>> {
    guard.verify(root)?;
    if !matches!(action, Action::Continue | Action::Abort | Action::Skip) {
        ensure!(guard.progress.is_none(), "Finish or abort the current Git operation first");
    }
    if matches!(
        action,
        Action::Merge(_)
            | Action::Rebase(_)
            | Action::CherryPick { .. }
            | Action::Revert { .. }
            | Action::Interactive { .. }
            | Action::Squash { .. }
    ) {
        guard.clean()?;
    }
    if matches!(action, Action::Continue) {
        ensure!(
            status::parse(&guard.status)?.files.iter().all(|f| !f.conflicted),
            "Resolve every conflicted file before continuing"
        );
    }
    if let Action::CherryPick { id, mainline } | Action::Revert { id, mainline } = &action {
        let repo = gix::discover(root)?;
        let count = repo.find_object(gix::ObjectId::from_hex(id.as_bytes())?)?.try_into_commit()?.parent_ids().count();
        ensure!(
            if count > 1 { mainline.is_some_and(|m| m > 0 && m as usize <= count) } else { mainline.is_none() },
            "Choose a valid mainline parent for merge commits"
        );
    }
    if let Action::Discard { file, untracked } = &action {
        let state = status::parse(&guard.status)?;
        let entry = state.files.iter().find(|f| &f.path == file).context("This file is no longer changed")?;
        ensure!(
            !entry.conflicted && !entry.submodule && entry.original.is_none(),
            "Use the conflict editor or an external tool for conflicts, submodules and renames"
        );
        ensure!((entry.index == '?') == *untracked, "The file's tracked state changed; prepare this action again");
        ensure!(*untracked || entry.worktree != '.', "This file has no unstaged changes to discard");
    }
    let recovery_id = match &action {
        Action::DeleteTag { name, expected } => {
            let actual = gix::discover(root)?.find_reference(format!("refs/tags/{name}").as_str())?.id().to_string();
            ensure!(&actual == expected, "The tag changed; prepare the action again");
            Some(expected.clone())
        }
        Action::DeleteBranch { branch, expected } => {
            let actual = gix::discover(root)?.find_reference(format!("refs/heads/{branch}").as_str())?.id().to_string();
            ensure!(&actual == expected, "The branch changed; prepare the action again");
            Some(expected.clone())
        }
        Action::CreateTag { .. }
        | Action::BranchAt { .. }
        | Action::SwitchNew { .. }
        | Action::Checkout(_)
        | Action::Discard { .. }
        | Action::ForcePush { .. } => None,
        _ => guard.head.clone(),
    };
    let original_plan =
        if let Action::Interactive { base, .. } | Action::Squash { base, .. } = &action { Some(plan(root, base)?) } else { None };
    if let Action::Interactive { steps, .. } = &action {
        validate_plan(original_plan.as_ref().unwrap(), steps)?;
    }
    let recovery = recovery_id.as_deref().map(|id| checkpoint(root, id)).transpose()?;
    let git_dir = repository::inspect(root)?.git_dir;
    if let Action::Interactive { steps, editor, .. } = &action {
        prepare_studio(&git_dir, steps, editor)?;
    }
    let studio_editor = studio_dir(&git_dir).join("nyx-git-message-editor");
    let editor: OsString = if matches!(action, Action::Interactive { .. } | Action::Continue | Action::Skip) && studio_editor.exists() {
        studio_editor.into_os_string()
    } else {
        "true".into()
    };
    let env = [("GIT_EDITOR".into(), editor)];
    let result = match &action {
        Action::Interactive { steps, editor, .. } => {
            let temp = tempfile::Builder::new().prefix("nyx-git-rebase-").tempdir_in("/tmp")?;
            let helper = temp.path().join("nyx-git-sequence-editor");
            std::os::unix::fs::symlink(editor, &helper)?;
            let plan_path = temp.path().join("plan.json");
            fs::write(&plan_path, serde_json::to_vec(steps)?)?;
            let mut env = env.to_vec();
            env.push(("GIT_SEQUENCE_EDITOR".into(), helper.into_os_string()));
            env.push(("NYX_GIT_REBASE_PLAN".into(), plan_path.into_os_string()));
            git(root, action.label(), action.args(guard.progress)?, None, &env)
        }
        Action::Squash { base, message } => {
            ensure!(!message.trim().is_empty(), "Enter a squash commit message");
            let old = guard.head.as_ref().context("No HEAD to squash")?;
            git(root, "Prepare squash", args(&["reset", "--soft", base]), None, &[])?;
            match operations::run(root, Operation::Commit { message: message.clone(), amend: false }) {
                Ok(output) => Ok(output),
                Err(error) => {
                    let rollback = git(root, "Restore original HEAD", args(&["reset", "--soft", old]), None, &[]);
                    if let Err(rollback) = rollback {
                        bail!(
                            "{error:#}\nCould not restore HEAD: {rollback:#}. Recovery: {}",
                            recovery.as_deref().unwrap_or(old)
                        );
                    }
                    Err(error)
                }
            }
        }
        _ => {
            let input = if let Action::CreateTag { message, .. } = &action { Some(message.as_bytes().to_vec()) } else { None };
            git(root, action.label(), action.args(guard.progress)?, input, &env)
        }
    };
    let result = result.map_err(|e|anyhow::anyhow!("{e:#}{}",recovery.map(|r|format!("\nPre-operation commits are retained at {r}. Use Recovery to create a branch. Uncommitted file contents are not saved by this reference.")).unwrap_or_default()));
    if matches!(action, Action::Interactive { .. } | Action::Continue | Action::Skip | Action::Abort) {
        match after_rebase(root, &git_dir) {
            Ok(Some(note)) => {
                return result.map(|mut out| {
                    out.extend_from_slice(format!("\n{note}\n").as_bytes());
                    out
                });
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(%e, "Could not finish the rebase studio step"),
        }
    }
    result
}
#[derive(Clone, Debug)]
pub struct Recovery {
    pub id: String,
    pub label: String,
    pub summary: String,
}
pub fn recovery(root: &Path) -> Result<Vec<Recovery>> {
    let reflog = git(root, "Read reflog", args(&["reflog", "--max-count=100", "--format=%H%x00%gD%x00%gs%x00"]), None, &[])?;
    let points = git(
        root,
        "Read recovery points",
        args(&[
            "for-each-ref",
            "--sort=-refname",
            "--count=100",
            "--format=%(objectname)%00%(refname)%00%(contents:subject)%00",
            "refs/nyx/recovery/",
        ]),
        None,
        &[],
    )?;
    let mut records = Vec::new();
    for bytes in [&points, &reflog] {
        let parts: Vec<_> = bytes.split(|b| *b == 0).collect();
        for triple in parts.chunks(3) {
            if triple.len() == 3 {
                records.push(Recovery {
                    id: String::from_utf8_lossy(triple[0]).trim().into(),
                    label: String::from_utf8_lossy(triple[1]).into(),
                    summary: String::from_utf8_lossy(triple[2]).into(),
                });
            }
        }
    }
    Ok(records)
}
pub fn remote_tip(root: &Path, remote: &str, branch: &str) -> Result<String> {
    valid_ref("refs/heads/", branch)?;
    ensure!(!remote.is_empty() && !remote.starts_with('-'), "Choose a remote");
    let bytes =
        git(root, "Read remote tip", args(&["ls-remote", "--heads", "--", remote, &format!("refs/heads/{branch}")]), None, &[])?;
    let text = String::from_utf8(bytes)?;
    let id = text.split_whitespace().next().context("This remote branch does not exist. Use a normal push.")?;
    operations::valid_id(id)?;
    Ok(id.into())
}
