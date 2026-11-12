use nyx_git::core::{
    activity::{self, Kind, Source},
    advanced::{Action, ResetMode},
    journal,
    operations::{self, Operation},
    preview::{self, Subject},
};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().into()
}
fn repo(dir: &Path) {
    git(dir, &["init", "-b", "main"]);
    for (k, v) in [("user.name", "Safety"), ("user.email", "safety@example.invalid"), ("commit.gpgsign", "false"), ("core.hooksPath", "/dev/null")] {
        git(dir, &["config", k, v]);
    }
}
fn commit(dir: &Path, file: &str, text: &str, message: &str) {
    fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "--all"]);
    git(dir, &["commit", "-m", message]);
}

#[test]
fn journal_activity_and_previews_report_what_operations_do() {
    let state = tempfile::tempdir().unwrap();
    // Only this test in this binary touches the environment.
    unsafe { std::env::set_var("NYX_GIT_STATE_DIR", state.path()) };
    journal::enable();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let remote = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--bare", "-b", "main"]);
    repo(root);
    commit(root, "a.txt", "one\n", "First");
    commit(root, "a.txt", "two\n", "Second");
    git(root, &["remote", "add", "origin", &remote.path().to_string_lossy()]);
    git(root, &["push", "-q", "origin", "main"]);
    git(root, &["fetch", "-q", "origin"]);
    commit(root, "a.txt", "three\n", "Third, unpublished");
    let first = git(root, &["rev-parse", "HEAD~2"]);
    let head = git(root, &["rev-parse", "HEAD"]);

    // A hard reset: two commits leave main, one of them already on origin; the dirty
    // tracked file is at risk; untracked files aren't listed.
    fs::write(root.join("a.txt"), "edited\n").unwrap();
    fs::write(root.join("new.txt"), "untracked\n").unwrap();
    let p = preview::preview(root, &Subject::Advanced(Action::Reset { target: first.clone(), mode: ResetMode::Hard })).unwrap();
    assert_eq!(p.commits_total, 2);
    assert_eq!(p.published, 1);
    assert_eq!(p.at_risk, vec![std::path::PathBuf::from("a.txt")]);
    assert!(p.irreversible);
    assert_eq!(p.refs[0].name, "main");
    assert_eq!(p.refs[0].before.as_deref(), Some(head.as_str()));
    // Previewing changes nothing.
    assert_eq!(git(root, &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "edited\n");
    fs::write(root.join("a.txt"), "three\n").unwrap();
    fs::remove_file(root.join("new.txt")).unwrap();

    // Both sides change the same file: the rebase preview names it as a likely conflict.
    git(root, &["switch", "-q", "-c", "topic", &first]);
    commit(root, "a.txt", "topic\n", "Topic edit");
    commit(root, "b.txt", "only topic\n", "Topic only");
    let p = preview::preview(root, &Subject::Advanced(Action::Rebase(head.clone()))).unwrap();
    assert_eq!(p.overlap, vec![std::path::PathBuf::from("a.txt")]);
    assert_eq!((p.commits_total, p.published), (2, 0));

    // Deleting an unmerged branch lists the commits found nowhere else.
    git(root, &["switch", "-q", "main"]);
    let topic = git(root, &["rev-parse", "topic"]);
    let p = preview::preview(root, &Subject::Advanced(Action::DeleteBranch { branch: "topic".into(), expected: topic })).unwrap();
    assert_eq!(p.commits_total, 2);
    assert_eq!(p.refs[0].after, "deleted");

    // Operations through the core are journaled with the references they moved; staging isn't.
    // Create branch switches to it, so the commit lands on feature.
    operations::run(root, Operation::CreateBranch("feature".into())).unwrap();
    fs::write(root.join("c.txt"), "c\n").unwrap();
    operations::run(root, Operation::StageAll).unwrap();
    operations::run(root, Operation::Commit { message: "Journaled commit".into(), amend: false }).unwrap();
    assert!(operations::run(root, Operation::SwitchBranch("missing-branch".into())).is_err());
    let common = std::path::PathBuf::from(git(root, &["rev-parse", "--path-format=absolute", "--git-common-dir"]));
    let entries = journal::read(&common);
    let labels: Vec<_> = entries.iter().map(|e| e.operation.as_str()).collect();
    assert_eq!(labels, ["Switch branch", "Create commit", "Create branch"], "newest first, staging left out");
    assert!(!entries[0].ok && entries[0].error.is_some());
    let commit_entry = &entries[1];
    assert!(commit_entry.head().is_some_and(|h| h.before.as_deref() == Some(head.as_str())));
    assert!(commit_entry.changes.iter().any(|c| c.name == "refs/heads/feature"));
    assert_eq!(commit_entry.branch.1.as_deref(), Some("refs/heads/feature"));
    assert!(entries[2].changes.iter().any(|c| c.name == "refs/heads/feature" && c.before.is_none()));
    assert_eq!(entries[2].branch, (Some("refs/heads/main".into()), Some("refs/heads/feature".into())));

    // Activity: the journal's commit isn't repeated from the reflog; the CLI commits are.
    let events = activity::read(root, &common).unwrap();
    let journaled = events.iter().filter(|e| e.source == Source::Journal).count();
    assert_eq!(journaled, 3);
    assert_eq!(events.iter().filter(|e| e.title == "Commit" && e.detail == "Journaled commit").count(), 0);
    assert!(events.iter().any(|e| e.source == Source::Reflog && e.kind == Kind::Commit && e.detail == "Third, unpublished"));
    assert!(events.windows(2).all(|w| w[0].time >= w[1].time));
    unsafe { std::env::remove_var("NYX_GIT_STATE_DIR") };
}
