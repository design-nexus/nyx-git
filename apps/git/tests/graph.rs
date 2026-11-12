use nyx_git::{
    core::{
        history::{Cursor, Event, Request, Scope, Session},
        operations::{self, Operation},
    },
    graph::{
        layout::Layout,
        model::{Commit, Query},
        virtualization::{MAX_CACHE, visible_range},
    },
};
use std::{
    collections::HashMap,
    fs,
    path::Path,
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
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
fn dated(root: &Path, args: &[&str], timestamp: i64) {
    let date = format!("{timestamp} +0000");
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
fn node(id: &str, parents: &[&str]) -> Commit {
    Commit {
        id: id.into(),
        parents: parents.iter().map(|s| s.to_string()).collect(),
        subject: id.into(),
        author: "Nyx".into(),
        email: "test@example.invalid".into(),
        timestamp: 1700000000,
        refs: Vec::new(),
        shallow: false,
        collapsed: 0,
    }
}
#[test]
fn stable_lanes_preserve_merges_octopus_edges_and_page_boundaries() {
    let commits = vec![
        node("merge", &["left", "right", "third"]),
        node("left", &["root"]),
        node("third", &["root"]),
        node("right", &["root"]),
        node("root", &[]),
    ];
    let mut layout = Layout::default();
    let rows: Vec<_> = commits.into_iter().map(|c| layout.push(c)).collect();
    assert_eq!(rows[0].edges.iter().filter(|e| e.top == 1).count(), 3);
    assert_eq!(rows[0].lane, rows[1].lane);
    assert_ne!(rows[1].lane, rows[2].lane);
    for pair in rows.windows(2) {
        let mut below: Vec<_> = pair[0].edges.iter().filter(|e| e.bottom == 2).map(|e| (e.to, e.color)).collect();
        below.sort();
        below.dedup();
        let mut above: Vec<_> = pair[1].edges.iter().filter(|e| e.top == 0).map(|e| (e.from, e.color)).collect();
        above.sort();
        above.dedup();
        assert_eq!(below, above, "Edges must meet identically at row/page boundaries");
    }
    assert!(!rows.last().unwrap().edges.iter().any(|e| e.bottom == 2));
    assert_eq!(visible_range(360000.0, 800.0, 36.0, 100000).len(), 72);
}
#[test]
fn real_skewed_merge_history_labels_details_and_shallow_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "--initial-branch=main"]);
    git(root, &["config", "user.name", "Nyx Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["config", "core.hooksPath", "/dev/null"]);
    fs::write(root.join("root file"), "base\n").unwrap();
    git(root, &["add", "."]);
    dated(root, &["commit", "-m", "Root"], 1700000200);
    git(root, &["switch", "-c", "side"]);
    fs::write(root.join("side"), "side\n").unwrap();
    git(root, &["add", "."]);
    dated(root, &["commit", "-m", "Side"], 1700000100);
    git(root, &["switch", "main"]);
    fs::write(root.join("main"), "main\n").unwrap();
    git(root, &["add", "."]);
    dated(root, &["commit", "-m", "Main"], 1700000300);
    dated(root, &["merge", "--no-ff", "side", "-m", "Merge"], 1699990000);
    git(root, &["tag", "-a", "release", "-m", "Release"]);
    let head = git(root, &["rev-parse", "HEAD"]);
    git(root, &["update-ref", "refs/remotes/origin/main", &head]);
    let mut repo = gix::discover(root).unwrap();
    repo.object_cache_size_if_unset(1024 * 1024);
    let mut cursor = Cursor::new(&repo, Scope::All, Arc::new(AtomicBool::new(false))).unwrap();
    cursor.fill(2).unwrap();
    let first = cursor.store.read(0).unwrap();
    cursor.fill(100).unwrap();
    assert!(cursor.done);
    assert_eq!(cursor.store.len(), 4);
    assert_eq!(first.commit.id, cursor.store.read(0).unwrap().commit.id);
    assert!(first.commit.refs.iter().any(|r| r.name == "HEAD"));
    assert!(first.commit.refs.iter().any(|r| r.name == "release"));
    assert!(first.commit.refs.iter().any(|r| r.name == "origin/main"));
    assert_eq!(first.color, 0);
    let rows = cursor.store.page(0, MAX_CACHE + 100).unwrap();
    let index: HashMap<_, _> = rows.iter().enumerate().map(|(i, r)| (r.commit.id.clone(), i)).collect();
    for (i, row) in rows.iter().enumerate() {
        for parent in &row.commit.parents {
            assert!(index[parent] > i, "Parents must always follow their children");
        }
    }
    // First-parent history collapses the merged side branch into the merge commit.
    let mut collapsed = Cursor::with_options(&repo, Scope::Head, true, Arc::new(AtomicBool::new(false))).unwrap();
    collapsed.fill(100).unwrap();
    let subjects: Vec<_> = (0..collapsed.store.len()).map(|i| collapsed.store.read(i).unwrap().commit.subject).collect();
    assert_eq!(subjects, ["Merge", "Main", "Root"]);
    let merge = collapsed.store.read(0).unwrap();
    assert_eq!((merge.commit.parents.len(), merge.commit.collapsed), (1, 1));
    assert!(collapsed.store.read(2).unwrap().edges.iter().all(|e| e.bottom != 2), "no edge leads to a hidden parent");
    let details = nyx_git::core::history::details(root, &head).unwrap();
    assert_eq!(details.parents.len(), 2);
    let files =
        operations::run(root, Operation::CommitFiles { id: head.clone(), parent: details.parents.first().cloned() }).unwrap();
    assert!(files.windows(4).any(|w| w == b"side"));
    let root_id = git(root, &["rev-list", "--max-parents=0", "HEAD"]);
    let root_diff = operations::run(root, Operation::CommitDiff { id: root_id, parent: None, file: "root file".into() }).unwrap();
    assert!(String::from_utf8_lossy(&root_diff).contains("+base"));
    // Git's native shallow marker is a read-only traversal boundary, even when
    // the old parent objects still happen to be present in the local store.
    fs::write(root.join(".git/shallow"), format!("{head}\n")).unwrap();
    let shallow_repo = gix::discover(root).unwrap();
    let mut shallow = Cursor::new(&shallow_repo, Scope::Head, Arc::new(AtomicBool::new(false))).unwrap();
    shallow.fill(100).unwrap();
    assert_eq!(shallow.store.len(), 1);
    let row = shallow.store.read(0).unwrap();
    assert!(row.commit.shallow);
    assert!(row.commit.parents.is_empty());
}
#[test]
fn query_filters_and_actor_cancellation_remain_bounded() {
    let c = node("fix", &[]);
    assert!(Query::parse("fix author:nyx after:2020-01-01 before:2030-01-01").unwrap().matches(&c));
    assert!(!Query::parse("author:someone").unwrap().matches(&c));
    assert!(Query::parse("after:invalid-date").is_err());
    let dir = tempfile::tempdir().unwrap();
    gix::init(dir.path()).unwrap();
    let session = Session::start(dir.path().into(), Scope::All);
    session.request(Request::Page { start: 0, count: MAX_CACHE * 2 });
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut empty = false;
    while Instant::now() < deadline {
        if let Some(event) = session.poll() {
            match event {
                Event::Page { rows, done, .. } => {
                    assert!(rows.is_empty());
                    assert!(done);
                    empty = true;
                    break;
                }
                Event::Error(e) => panic!("{e}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(empty);
    drop(session);
    let repo = gix::discover(dir.path()).unwrap();
    let cancelled = Arc::new(AtomicBool::new(true));
    let mut cursor = Cursor::new(&repo, Scope::All, cancelled).unwrap();
    assert!(cursor.fill(1).is_err());
}
