//! The new read paths (Flow, Activity, previews, blame) on awkward repositories: bare,
//! unborn, detached, shallow, many branches, many worktrees, a submodule, mid-merge.
use nyx_git::core::{
    activity,
    advanced::{Action, ResetMode},
    files, flow,
    preview::{self, Subject},
};
use std::{fs, path::Path, process::Command, time::Instant};

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ALLOW_PROTOCOL", "file")
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().into()
}
fn init(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    for (k, v) in [("user.name", "Fixture"), ("user.email", "fixture@example.invalid"), ("commit.gpgsign", "false"), ("core.hooksPath", "/dev/null"), ("protocol.file.allow", "always")] {
        git(root, &["config", k, v]);
    }
}
fn commit(root: &Path, file: &str, text: &str, message: &str) {
    fs::write(root.join(file), text).unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-q", "-m", message]);
}
fn common(root: &Path) -> std::path::PathBuf {
    git(root, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).into()
}

#[test]
fn unborn_and_detached_repositories_read_without_errors() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("unborn");
    init(&root);
    let f = flow::read(&root, &root, vec![]).unwrap();
    assert!(f.branches.is_empty());
    assert!(activity::read(&root, &common(&root)).unwrap().is_empty());
    commit(&root, "a", "a\n", "one");
    commit(&root, "a", "b\n", "two");
    let first = git(&root, &["rev-parse", "HEAD~1"]);
    git(&root, &["switch", "-q", "--detach", &first]);
    let f = flow::read(&root, &root, vec![]).unwrap();
    assert_eq!(f.trunk.as_deref(), Some("main"));
    assert!(f.worktrees[0].branch.is_none(), "detached");
    assert!(flow::nodes(&f, false).iter().any(|(id, _)| id.starts_with("worktree:")), "a detached worktree gets its own card");
    let p = preview::preview(&root, &Subject::Advanced(Action::Reset { target: first, mode: ResetMode::Soft })).unwrap();
    assert_eq!(p.refs[0].name, "HEAD (detached)");
    let lines = files::blame(&root, Path::new("a"), None).unwrap();
    assert_eq!(lines.len(), 1);
}

#[test]
fn bare_shallow_and_submodule_repositories_are_read_as_they_are() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    init(&source);
    for i in 0..5 {
        commit(&source, "f", &format!("{i}\n"), &format!("c{i}"));
    }
    let bare = dir.path().join("bare.git");
    git(dir.path(), &["clone", "-q", "--bare", &source.to_string_lossy(), &bare.to_string_lossy()]);
    let f = flow::read(&bare, &bare, vec![]).unwrap();
    assert_eq!(f.trunk.as_deref(), Some("main"));
    assert!(f.worktrees.iter().all(|w| w.dirty.is_none()), "a bare repository has no working tree to count");
    let shallow = dir.path().join("shallow");
    git(dir.path(), &["clone", "-q", "--depth=2", &format!("file://{}", source.display()), &shallow.to_string_lossy()]);
    for (k, v) in [("user.name", "Fixture"), ("user.email", "fixture@example.invalid")] {
        git(&shallow, &["config", k, v]);
    }
    let f = flow::read(&shallow, &shallow, vec![]).unwrap();
    assert_eq!(f.trunk.as_deref(), Some("main"));
    let events = activity::read(&shallow, &common(&shallow)).unwrap();
    assert!(events.iter().any(|e| e.kind == activity::Kind::Fetch || e.title.to_lowercase().contains("clone")));
    // A submodule: the parent's Flow and blame aren't confused by it.
    let parent = dir.path().join("parent");
    init(&parent);
    commit(&parent, "top", "top\n", "top");
    git(&parent, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", &source.to_string_lossy(), "child"]);
    git(&parent, &["commit", "-q", "-m", "add child"]);
    let f = flow::read(&parent, &parent, vec![]).unwrap();
    assert_eq!(f.worktrees.len(), 1);
    assert_eq!(f.worktrees[0].dirty, Some(0));
    assert!(files::blame(&parent, Path::new("child"), None).is_err(), "a submodule isn't a regular file");
}

#[test]
fn many_branches_and_worktrees_stay_bounded_and_quick() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    init(&root);
    commit(&root, "base", "base\n", "base");
    // 300 branches, most of them merged (same commit), 40 with their own commit.
    let mut script = String::new();
    for i in 0..300 {
        script.push_str(&format!("create refs/heads/b{i:03} HEAD\n"));
    }
    let mut child = Command::new("git").arg("-C").arg(&root).args(["update-ref", "--stdin"]).stdin(std::process::Stdio::piped()).spawn().unwrap();
    use std::io::Write;
    child.stdin.as_mut().unwrap().write_all(script.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
    for i in 0..40 {
        git(&root, &["switch", "-q", &format!("b{i:03}")]);
        commit(&root, &format!("f{i}"), "x\n", &format!("work {i}"));
    }
    git(&root, &["switch", "-q", "main"]);
    for i in 0..6 {
        git(&root, &["worktree", "add", "-q", &dir.path().join(format!("wt{i}")).to_string_lossy(), &format!("b{i:03}")]);
    }
    let started = Instant::now();
    let f = flow::read(&root, &root, vec![]).unwrap();
    let elapsed = started.elapsed();
    assert_eq!(f.branches.len(), 301);
    assert_eq!(f.worktrees.len(), 7);
    let active = f.branches.iter().filter(|b| b.active && !b.trunk).count();
    assert_eq!(active, 40, "merged branches without a worktree are inactive");
    assert!(f.branches.iter().filter(|b| b.divergence.is_some()).count() <= flow::ACTIVE_LIMIT);
    let nodes = flow::nodes(&f, false);
    let places = flow::layout(&f, &nodes);
    assert_eq!(places.len(), nodes.len());
    assert!(elapsed.as_secs() < 30, "Flow read took {elapsed:?}");
}

#[test]
fn a_merge_in_progress_shows_in_previews_and_activity() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    init(&root);
    commit(&root, "f", "base\n", "base");
    git(&root, &["switch", "-q", "-c", "topic"]);
    commit(&root, "f", "topic\n", "topic");
    git(&root, &["switch", "-q", "main"]);
    commit(&root, "f", "main\n", "main");
    let out = Command::new("git").arg("-C").arg(&root).args(["merge", "topic"]).env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap();
    assert!(!out.status.success());
    let p = preview::preview(&root, &Subject::Advanced(Action::Abort)).unwrap();
    assert_eq!(p.at_risk, vec![std::path::PathBuf::from("f")]);
    assert!(p.irreversible);
    let f = flow::read(&root, &root, vec![]).unwrap();
    assert!(f.worktrees[0].dirty.unwrap() >= 1);
}
