use nyx_git::core::{
    advanced::{self, Action, Active, Guard, Pick},
    conflicts::{self, Choice, Resolution, Shape},
    operations::{self, Operation},
};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().into()
}
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let p = d.path();
    git(p, &["init", "-q", "-b", "main"]);
    for (k, v) in [
        ("user.name", "Studio"),
        ("user.email", "studio@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
        ("rerere.enabled", "false"),
    ] {
        git(p, &["config", k, v]);
    }
    commit(p, "file", "base\n", "base");
    d
}
fn commit(root: &Path, file: &str, text: &str, msg: &str) {
    fs::write(root.join(file), text).unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-q", "-m", msg]);
}
fn apply(root: &Path, a: Action) -> anyhow::Result<Vec<u8>> {
    let guard = Guard::capture(root)?;
    advanced::run(root, a, &guard)
}
fn editor() -> std::path::PathBuf {
    env!("CARGO_BIN_EXE_nyx-git").into()
}
fn rebasing(root: &Path) -> bool {
    root.join(".git/rebase-merge").is_dir()
}

#[test]
fn studio_rebase_rewords_splits_and_skips_with_git_semantics() {
    let d = fixture();
    let p = d.path();
    let base = git(p, &["rev-parse", "HEAD"]);
    commit(p, "one", "one\n", "one");
    fs::write(p.join("a"), "a\n").unwrap();
    commit(p, "b", "b\n", "two parts");
    commit(p, "three", "three\n", "three");
    let mut steps = advanced::plan(p, &base).unwrap();
    assert_eq!(steps.len(), 3);
    steps[0].action = Pick::Reword;
    steps[0].message = Some("one, reworded\n\nWith a body.".into());
    steps[1].action = Pick::Split;
    let proposed = advanced::proposed(&steps);
    assert_eq!(proposed.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["one, reworded", "two parts", "three"]);
    // The rebase stops at the split commit with its changes unstaged.
    let out = apply(p, Action::Interactive { base: base.clone(), steps, editor: editor() }).unwrap();
    assert!(String::from_utf8_lossy(&out).contains("split"), "{}", String::from_utf8_lossy(&out));
    assert!(rebasing(p));
    assert_eq!(git(p, &["log", "-1", "--format=%B", "HEAD"]), "one, reworded\n\nWith a body.");
    let status = git(p, &["status", "--porcelain"]);
    assert!(status.contains("?? a") && status.contains("?? b"), "{status}");
    // Commit it in two pieces, then continue.
    operations::run(p, Operation::Stage(vec!["a".into()])).unwrap();
    operations::run(p, Operation::Commit { message: "part a".into(), amend: false }).unwrap();
    operations::run(p, Operation::Stage(vec!["b".into()])).unwrap();
    operations::run(p, Operation::Commit { message: "part b".into(), amend: false }).unwrap();
    apply(p, Action::Continue).unwrap();
    assert!(!rebasing(p));
    assert!(!p.join(".git/nyx-rebase").exists(), "studio state is tidied up");
    assert_eq!(git(p, &["log", "--format=%s", &format!("{base}..HEAD")]), "three\npart b\npart a\none, reworded");

    // A reordered plan whose commits touch the same file is flagged, and Skip leaves one out.
    let base = git(p, &["rev-parse", "HEAD"]);
    commit(p, "file", "x\n", "x");
    commit(p, "file", "y\n", "y");
    let original = advanced::plan(p, &base).unwrap();
    let mut steps = original.clone();
    steps.swap(0, 1);
    let facts = advanced::facts(p, &base).unwrap();
    assert_eq!(advanced::reorder_warnings(&original, &steps, &facts.files).len(), 1);
    assert!(apply(p, Action::Interactive { base: base.clone(), steps, editor: editor() }).is_err(), "y can't apply before x");
    assert!(rebasing(p));
    apply(p, Action::Skip).unwrap();
    // x now conflicts with y's content; skip it too and the rebase ends with y alone.
    if rebasing(p) {
        apply(p, Action::Skip).unwrap();
    }
    assert!(!rebasing(p));
    assert_eq!(git(p, &["log", "--format=%s", &format!("{base}..HEAD")]).lines().count(), 1);
}

#[test]
fn conflict_blocks_resolve_one_at_a_time_and_staging_waits_for_mark_resolved() {
    let d = fixture();
    let p = d.path();
    fs::write(p.join("file"), "top\n1\n2\n3\n4\n5\nbottom\n").unwrap();
    git(p, &["commit", "-qam", "three lines"]);
    git(p, &["switch", "-q", "-c", "topic"]);
    fs::write(p.join("file"), "top theirs\n1\n2\n3\n4\n5\nbottom theirs\n").unwrap();
    git(p, &["commit", "-qam", "theirs"]);
    git(p, &["switch", "-q", "main"]);
    fs::write(p.join("file"), "top ours\n1\n2\n3\n4\n5\nbottom ours\n").unwrap();
    git(p, &["commit", "-qam", "ours"]);
    let topic = git(p, &["rev-parse", "topic"]);
    assert!(apply(p, Action::Merge(topic)).is_err());
    let c = conflicts::load(p, Path::new("file")).unwrap();
    assert_eq!(c.shape(), Shape::Content);
    let working = String::from_utf8(c.working.clone().unwrap()).unwrap();
    let blocks = conflicts::blocks(&working);
    assert_eq!(blocks.len(), 2);
    // Choosing sides only edits the result; the index still says conflicted.
    let text = conflicts::apply(&working, 0, Choice::Ours).unwrap();
    let text = conflicts::apply(&text, 0, Choice::Theirs).unwrap();
    assert_eq!(text, "top ours\n1\n2\n3\n4\n5\nbottom theirs\n");
    assert!(git(p, &["status", "--porcelain"]).starts_with("UU"));
    // A result with markers left is refused.
    assert!(conflicts::resolve(p, &c, Resolution::Text(working.clone())).is_err());
    conflicts::resolve(p, &conflicts::load(p, Path::new("file")).unwrap(), Resolution::Text(text)).unwrap();
    assert!(git(p, &["status", "--porcelain"]).starts_with("M "));
    assert_eq!(advanced::active(&p.join(".git")), Some(Active::Merge));
    apply(p, Action::Continue).unwrap();
    assert_eq!(advanced::active(&p.join(".git")), None);
    assert_eq!(fs::read_to_string(p.join("file")).unwrap(), "top ours\n1\n2\n3\n4\n5\nbottom theirs\n");

    // Modify/delete is told apart from content conflicts.
    git(p, &["switch", "-q", "-c", "gone"]);
    git(p, &["rm", "-q", "file"]);
    git(p, &["commit", "-qm", "delete"]);
    git(p, &["switch", "-q", "main"]);
    fs::write(p.join("file"), "changed again\n").unwrap();
    git(p, &["commit", "-qam", "change"]);
    let gone = git(p, &["rev-parse", "gone"]);
    assert!(apply(p, Action::Merge(gone)).is_err());
    let c = conflicts::load(p, Path::new("file")).unwrap();
    assert_eq!(c.shape(), Shape::DeletedByThem);
    conflicts::resolve(p, &c, Resolution::Ours).unwrap();
    apply(p, Action::Continue).unwrap();
    assert_eq!(fs::read_to_string(p.join("file")).unwrap(), "changed again\n");
}
