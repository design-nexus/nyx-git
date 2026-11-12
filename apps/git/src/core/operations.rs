//! System Git is the only writer. Arguments are structured and paths are literal.
use anyhow::{Result, bail, ensure};
use std::{
    ffi::OsString,
    path::{Component, Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug)]
pub enum Operation {
    Status,
    Clone { url: String, destination: PathBuf },
    Initialize { destination: PathBuf, branch: String },
    DeleteRemoteBranch { listing: Box<super::remotes::BranchListing>, branch: String, expected: String },
    CheckoutRequest(super::hosting::Checkout),
    Worktree(super::worktrees::Action),
    Lfs { action: super::lfs::Action, expected: Box<super::lfs::Snapshot> },
    Submodule { action: super::submodules::Action, index: usize, expected: Box<super::submodules::Snapshot> },
    Advanced { action: Box<super::advanced::Action>, guard: Box<super::advanced::Guard> },
    ResolveConflict { conflict: Box<super::conflicts::Conflict>, resolution: super::conflicts::Resolution },
    CommitFiles { id: String, parent: Option<String> },
    CommitDiff { id: String, parent: Option<String>, file: PathBuf },
    Compare { base: String, tip: String },
    Diff { file: PathBuf, cached: bool, untracked: bool },
    Stage(Vec<PathBuf>),
    Unstage { paths: Vec<PathBuf>, unborn: bool },
    StageAll,
    UnstageAll { unborn: bool },
    Commit { message: String, amend: bool },
    LastMessage,
    Fetch,
    Pull,
    Push,
    Remotes,
    Remote { action: super::remotes::Action, expected: Box<super::remotes::Snapshot> },
    PushBranch { remote: String, branch: String },
    Branches,
    CreateBranch(String),
    SwitchBranch(String),
    RenameBranch { old: String, new: String },
    DeleteBranch(String),
    Stashes,
    StashSave(String),
    StashApply(String),
    StashPop { id: String, expected: String },
    StashDrop { id: String, expected: String },
    StashOid(String),
    StashDiff(String),
    CheckPatch(Vec<u8>),
    ApplyPatch(Vec<u8>),
    StageSelection { file: PathBuf, expected: Vec<u8>, range: (usize, usize) },
}
pub(crate) fn valid_id(id: &str) -> Result<()> {
    ensure!([40, 64].contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit()), "Expected a full commit hash");
    Ok(())
}
fn strings(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}
pub(crate) fn paths(args: &mut Vec<OsString>, files: &[PathBuf]) -> Result<()> {
    ensure!(!files.is_empty(), "Choose at least one file");
    args.push("--".into());
    for path in files {
        ensure!(
            !path.as_os_str().is_empty() && path.components().all(|p| matches!(p, Component::Normal(_))),
            "Expected a repository-relative file path"
        );
        args.push(path.as_os_str().into());
    }
    Ok(())
}
impl Operation {
    pub fn cancellable(&self) -> bool {
        matches!(self, Self::Fetch | Self::Clone { .. } | Self::Lfs { action: super::lfs::Action::Fetch(_), .. })
    }
    pub fn mutating(&self) -> bool {
        !matches!(
            self,
            Self::Status
                | Self::CommitFiles { .. }
                | Self::CommitDiff { .. }
                | Self::Compare { .. }
                | Self::Diff { .. }
                | Self::LastMessage
                | Self::Remotes
                | Self::Branches
                | Self::Stashes
                | Self::StashOid(_)
                | Self::StashDiff(_)
                | Self::CheckPatch(_)
        )
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Status => "Read status",
            Self::Clone { .. } => "Clone repository",
            Self::Initialize { .. } => "Initialize repository",
            Self::DeleteRemoteBranch { .. } => "Delete remote branch",
            Self::CheckoutRequest(_) => "Checkout request branch",
            Self::Worktree(action) => action.label(),
            Self::Lfs { action, .. } => action.label(),
            Self::Submodule { action, .. } => action.label(),
            Self::Advanced { action, .. } => action.label(),
            Self::ResolveConflict { .. } => "Resolve conflict",
            Self::CommitFiles { .. } => "Read changed files",
            Self::CommitDiff { .. } | Self::Compare { .. } => "Read commit diff",
            Self::Diff { .. } | Self::StashDiff(_) => "Read diff",
            Self::Stage(_) | Self::StageAll | Self::StageSelection { .. } | Self::ApplyPatch(_) | Self::CheckPatch(_) => {
                "Stage changes"
            }
            Self::Unstage { .. } | Self::UnstageAll { .. } => "Unstage changes",
            Self::Commit { amend: true, .. } => "Amend commit",
            Self::Commit { .. } => "Create commit",
            Self::LastMessage => "Read commit message",
            Self::Fetch => "Fetch",
            Self::Pull => "Pull",
            Self::Push | Self::PushBranch { .. } => "Push",
            Self::Remotes => "Read remotes",
            Self::Remote { action, .. } => action.label(),
            Self::Branches => "Read branches",
            Self::CreateBranch(_) => "Create branch",
            Self::SwitchBranch(_) => "Switch branch",
            Self::RenameBranch { .. } => "Rename branch",
            Self::DeleteBranch(_) => "Delete merged branch",
            Self::Stashes => "Read stashes",
            Self::StashSave(_) => "Stash changes",
            Self::StashApply(_) => "Apply stash",
            Self::StashPop { .. } => "Pop stash",
            Self::StashDrop { .. } => "Drop stash",
            Self::StashOid(_) => "Verify stash",
        }
    }
    pub fn args(&self) -> Result<Vec<OsString>> {
        let mut a = match self {
            Self::Clone { .. }
            | Self::Initialize { .. }
            | Self::DeleteRemoteBranch { .. }
            | Self::Remote { .. }
            | Self::CheckoutRequest(_)
            | Self::Worktree(..)
            | Self::Lfs { .. }
            | Self::Submodule { .. }
            | Self::Advanced { .. }
            | Self::ResolveConflict { .. } => {
                bail!("This action must run through the operation layer")
            }
            Self::CommitFiles { id, parent } => {
                valid_id(id)?;
                let mut a = strings(&["diff-tree", "--root", "--no-commit-id", "--name-status", "-z", "-r", "--no-renames"]);
                if let Some(parent) = parent {
                    valid_id(parent)?;
                    a.push(parent.into());
                }
                a.push(id.into());
                a.push("--".into());
                a
            }
            Self::CommitDiff { id, parent, file } => {
                valid_id(id)?;
                let mut a = if parent.is_some() { strings(&["diff"]) } else { strings(&["show", "--format="]) };
                a.extend(strings(&["--no-ext-diff", "--no-textconv", "--no-color", "--no-renames"]));
                if let Some(parent) = parent {
                    valid_id(parent)?;
                    a.push(parent.into());
                }
                a.push(id.into());
                paths(&mut a, std::slice::from_ref(file))?;
                a
            }
            Self::Compare { base, tip } => {
                valid_id(base)?;
                valid_id(tip)?;
                let mut a = strings(&["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--no-renames"]);
                a.push(base.into());
                a.push(tip.into());
                a.push("--".into());
                a
            }
            Self::Status => {
                strings(&["--no-optional-locks", "status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"])
            }
            Self::Diff { file, cached, untracked } => {
                let mut a = strings(&[
                    "--no-optional-locks",
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--no-renames",
                    "--full-index",
                    "--src-prefix=a/",
                    "--dst-prefix=b/",
                    "--no-relative",
                    "--unified=3",
                ]);
                if *untracked {
                    a.push("--no-index".into());
                    a.push("--".into());
                    a.push("/dev/null".into());
                    ensure!(
                        !file.is_absolute() && file.components().all(|p| matches!(p, Component::Normal(_))),
                        "Invalid file path"
                    );
                    a.push(file.as_os_str().into());
                } else {
                    if *cached {
                        a.push("--cached".into());
                    }
                    paths(&mut a, std::slice::from_ref(file))?;
                }
                a
            }
            Self::Stage(files) => {
                let mut a = strings(&["add", "--all"]);
                paths(&mut a, files)?;
                a
            }
            Self::StageAll => strings(&["add", "--all", "--", "."]),
            Self::Unstage { paths: files, unborn } => {
                let mut a = if *unborn {
                    strings(&["rm", "--cached", "--force", "--ignore-unmatch"])
                } else {
                    strings(&["restore", "--staged"])
                };
                paths(&mut a, files)?;
                a
            }
            Self::UnstageAll { unborn } => {
                if *unborn {
                    strings(&["rm", "--cached", "--force", "-r", "--ignore-unmatch", "--", "."])
                } else {
                    strings(&["restore", "--staged", "--", "."])
                }
            }
            Self::Commit { message, amend } => {
                ensure!(!message.trim().is_empty(), "Write a commit message first");
                // `whitespace`, Git's own default for a message given as a file: `strip` would
                // drop lines such as "#123 was the cause".
                let mut a = strings(&["commit", "--file=-", "--cleanup=whitespace"]);
                if *amend {
                    a.push("--amend".into());
                }
                a
            }
            Self::LastMessage => strings(&["log", "-1", "--format=%B"]),
            Self::Fetch => strings(&["fetch", "--all", "--progress"]),
            Self::Pull => strings(&["pull", "--ff-only"]),
            Self::Push => strings(&["push"]),
            Self::Remotes => strings(&["remote"]),
            Self::PushBranch { remote, branch } => {
                ensure!(!remote.is_empty() && !branch.is_empty(), "Choose a remote and branch");
                let mut a = strings(&["push", "--set-upstream", "--"]);
                a.push(remote.into());
                a.push(branch.into());
                a
            }
            Self::Branches => strings(&["for-each-ref", "--format=%(refname:short)", "refs/heads/"]),
            Self::CreateBranch(name) => {
                ensure!(!name.trim().is_empty(), "Enter a branch name");
                let mut a = strings(&["switch", "--no-guess", "--create"]);
                a.push(name.into());
                a.push("--".into());
                a
            }
            Self::SwitchBranch(name) => {
                let mut a = strings(&["switch", "--no-guess", "--"]);
                a.push(name.into());
                a
            }
            Self::RenameBranch { old, new } => {
                let mut a = strings(&["branch", "--move", "--"]);
                a.push(old.into());
                a.push(new.into());
                a
            }
            // Git refuses unmerged branches; force deletion belongs to Phase 4.
            Self::DeleteBranch(name) => {
                let mut a = strings(&["branch", "--delete", "--"]);
                a.push(name.into());
                a
            }
            Self::Stashes => strings(&["stash", "list", "--format=%H%x00%gd%x00%gs%x00"]),
            Self::StashSave(message) => {
                let mut a = strings(&["stash", "push", "--include-untracked", "--message"]);
                a.push(message.into());
                a
            }
            Self::StashApply(id)
            | Self::StashPop { id, .. }
            | Self::StashDrop { id, .. }
            | Self::StashDiff(id)
            | Self::StashOid(id) => {
                ensure!(
                    (id.starts_with("stash@{")
                        && id.ends_with('}')
                        && id.len() > 8
                        && id[7..id.len() - 1].chars().all(|c| c.is_ascii_digit()))
                        || (matches!(self, Self::StashApply(_) | Self::StashDiff(_))
                            && [40, 64].contains(&id.len())
                            && id.chars().all(|c| c.is_ascii_hexdigit())),
                    "Invalid stash identifier"
                );
                if matches!(self, Self::StashOid(_)) {
                    return Ok(strings(&["rev-parse", "--verify", id]));
                }
                let verb = match self {
                    Self::StashApply(_) => "apply",
                    Self::StashPop { .. } => "pop",
                    Self::StashDrop { .. } => "drop",
                    _ => "show",
                };
                let mut a = strings(&["stash", verb]);
                if matches!(self, Self::StashDiff(_)) {
                    a.extend(strings(&["--patch", "--include-untracked", "--no-ext-diff", "--no-textconv", "--no-color"]));
                }
                a.push(id.into());
                a
            }
            Self::CheckPatch(_) | Self::ApplyPatch(_) => {
                let mut a = strings(&["apply", "--cached", "--whitespace=nowarn"]);
                if matches!(self, Self::CheckPatch(_)) {
                    a.push("--check".into());
                }
                a.push("-".into());
                a
            }
            Self::StageSelection { .. } => bail!("Selection staging must run through the operation layer"),
        };
        // Limit literal pathspecs to file operations. Git stash internally uses
        // pathspec magic while cleaning saved untracked files.
        if matches!(
            self,
            Self::Diff { .. }
                | Self::CommitDiff { .. }
                | Self::Stage(_)
                | Self::StageAll
                | Self::Unstage { .. }
                | Self::UnstageAll { .. }
        ) {
            a.insert(0, "--literal-pathspecs".into());
        }
        a.insert(0, "--no-pager".into());
        Ok(a)
    }
}

pub fn run(root: &Path, operation: Operation) -> Result<Vec<u8>> {
    if operation.mutating() && !matches!(operation, Operation::Clone { .. } | Operation::Initialize { .. }) {
        return super::jobs::locked(root, || journaled(root, operation));
    }
    run_inner(root, operation)
}
/// Run a mutation and note it in the operation journal: the references it moved, or why it
/// failed. Staging only changes the index and isn't journaled.
fn journaled(root: &Path, operation: Operation) -> Result<Vec<u8>> {
    let staging = matches!(
        operation,
        Operation::Stage(_)
            | Operation::StageAll
            | Operation::Unstage { .. }
            | Operation::UnstageAll { .. }
            | Operation::StageSelection { .. }
            | Operation::ApplyPatch(_)
            | Operation::CheckPatch(_)
            | Operation::ResolveConflict { .. }
    );
    let before = if staging || !super::journal::enabled() { None } else { super::journal::refs(root).ok() };
    // Discarding and worktree changes move no references but are worth remembering.
    let always = match &operation {
        Operation::Worktree(_) => true,
        Operation::Advanced { action, .. } => matches!(**action, super::advanced::Action::Discard { .. } | super::advanced::Action::Abort),
        _ => false,
    };
    let label = operation.label();
    let result = run_inner(root, operation);
    if let Some(before) = before {
        super::journal::record(root, label, &before, result.as_ref().err().map(|e| format!("{e:#}")), always);
    }
    result
}
fn run_inner(root: &Path, operation: Operation) -> Result<Vec<u8>> {
    match operation {
        Operation::Clone { url, destination } => return super::hosting::clone_repository(root, &url, &destination),
        Operation::Initialize { destination, branch } => return super::initialize::run(&destination, &branch),
        Operation::DeleteRemoteBranch { listing, branch, expected } => {
            return super::remotes::delete_branch(root, &listing, &branch, &expected);
        }
        Operation::Remote { action, expected } => return super::remotes::run(root, action, &expected),
        Operation::CheckoutRequest(request) => return super::hosting::checkout(root, request),
        Operation::Worktree(action) => return super::worktrees::run(root, action),
        Operation::Lfs { action, expected } => return super::lfs::run(root, action, &expected),
        Operation::Submodule { action, index, expected } => return super::submodules::run(root, action, index, &expected),
        Operation::Advanced { action, guard } => return super::advanced::run(root, *action, &guard),
        Operation::ResolveConflict { conflict, resolution } => return super::conflicts::resolve(root, &conflict, resolution),
        _ => {}
    }
    if let Operation::StageSelection { file, expected, range } = operation {
        return super::staging::stage_selection(root, &file, &expected, range);
    }
    if let Operation::StashPop { id, expected } | Operation::StashDrop { id, expected } = &operation {
        let actual = run(root, Operation::StashOid(id.clone()))?;
        ensure!(
            String::from_utf8_lossy(&actual).trim() == expected,
            "The stash list changed. Reopen Stashes before trying again."
        );
    }
    if matches!(operation, Operation::StageAll | Operation::Stage(_)) {
        let status = super::status::parse(&run(root, Operation::Status)?)?;
        let conflicts = status.files.iter().any(|file| {
            file.conflicted
                && match &operation {
                    Operation::Stage(files) => files.contains(&file.path),
                    _ => true,
                }
        });
        ensure!(
            !conflicts,
            "Resolve conflicted files through the conflict editor before using this action. Unresolved conflicts will not be marked complete."
        );
    }
    let args = operation.args()?;
    let input: Option<&[u8]> = match &operation {
        Operation::Commit { message, .. } => Some(message.as_bytes()),
        Operation::CheckPatch(p) | Operation::ApplyPatch(p) => Some(p),
        _ => None,
    };
    let network = matches!(operation, Operation::Fetch | Operation::Pull | Operation::Push | Operation::PushBranch { .. });
    let pull = matches!(operation, Operation::Pull);
    invoke(root, operation.label(), args, input.map(Vec::from), &[], matches!(operation, Operation::Diff { untracked: true, .. }))
        .map_err(|error| match network.then(|| hint(&format!("{error:#}"), pull)).flatten() {
            Some(hint) => anyhow::anyhow!("{error:#}\n{hint}"),
            None => error,
        })
}

/// A next step for a failed transfer, only when Git's message points at one.
pub fn hint(message: &str, pull: bool) -> Option<&'static str> {
    let m = message.to_lowercase();
    if m.contains("has no upstream branch") || m.contains("no tracking information") {
        Some("Publish the branch with Push to choose a remote and set its upstream.")
    } else if pull && (m.contains("not possible to fast-forward") || m.contains("diverging branches") || m.contains("divergent")) {
        Some("Pull only fast-forwards. Merge or rebase onto the upstream from the commit's Git actions.")
    } else if m.contains("[rejected]") || m.contains("non-fast-forward") || m.contains("fetch first") {
        Some("The remote has commits you don't have. Fetch and pull (or merge) first, then push again.")
    } else if m.contains("permission denied")
        || m.contains("authentication failed")
        || m.contains("could not read username")
        || m.contains("terminal prompts disabled")
        || m.contains("host key verification failed")
    {
        Some("Git couldn't sign in. Nyx uses your system credentials: load your key into the SSH agent or set up a Git credential helper.")
    } else if m.contains("could not resolve host") || m.contains("unable to access") || m.contains("connection timed out") || m.contains("network is unreachable") {
        Some("The remote couldn't be reached. Check the network and the remote's URL.")
    } else {
        None
    }
}

/// Shared process boundary for typed Git operations, including the advanced core.
pub(crate) fn invoke(
    root: &Path,
    label: &str,
    args: Vec<OsString>,
    input: Option<Vec<u8>>,
    env: &[(OsString, OsString)],
    allow_difference: bool,
) -> Result<Vec<u8>> {
    let started = std::time::Instant::now();
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_LITERAL_PATHSPECS")
        .env_remove("GIT_GLOB_PATHSPECS")
        .env_remove("GIT_NOGLOB_PATHSPECS")
        .env_remove("GIT_ICASE_PATHSPECS")
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(env.iter().map(|(k, v)| (k, v)));
    let output = super::jobs::output(command, label, input)?;
    tracing::debug!(operation = label, elapsed_ms = started.elapsed().as_millis(), "Git finished");
    if !output.status.success() && !(allow_difference && output.status.code() == Some(1)) {
        let message = if output.stderr.is_empty() {
            String::from_utf8_lossy(&output.stdout)
        } else {
            String::from_utf8_lossy(&output.stderr)
        };
        bail!("{label} failed: {}", message.trim());
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_keep_paths_literal_and_mutations_safe() {
        let args = Operation::Stage(vec!["* weird\nname".into()]).args().unwrap();
        assert_eq!(args, strings(&["--no-pager", "--literal-pathspecs", "add", "--all", "--", "* weird\nname"]));
        assert!(Operation::Stage(vec!["../outside".into()]).args().is_err());
        assert!(Operation::Commit { message: " ".into(), amend: false }.args().is_err());
        assert!(Operation::Pull.args().unwrap().contains(&"--ff-only".into()));
        assert!(!Operation::Push.args().unwrap().iter().any(|a| a.to_string_lossy().contains("force")));
        assert!(Operation::DeleteBranch("main".into()).args().unwrap().contains(&"--delete".into()));
        assert!(Operation::Commit { message: "x".into(), amend: false }.args().unwrap().contains(&"--cleanup=whitespace".into()));
        assert!(hint("fatal: The current branch topic has no upstream branch.", false).is_some());
        assert!(hint("error: pathspec 'x' did not match", false).is_none());
    }
}
