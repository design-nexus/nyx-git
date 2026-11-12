use nyx_git::core::{agents, flow};
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
fn commit(root: &Path, file: &str, text: &str, message: &str) {
    fs::write(root.join(file), text).unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-q", "-m", message]);
}

#[test]
fn flow_reports_ancestry_worktrees_agents_and_shared_files_without_inventing_relations() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    for (k, v) in [("user.name", "Flow"), ("user.email", "flow@example.invalid"), ("commit.gpgsign", "false"), ("core.hooksPath", "/dev/null")] {
        git(&root, &["config", k, v]);
    }
    commit(&root, "shared.rs", "base\n", "Base");
    // a, then a2 started from a (stacked).
    git(&root, &["switch", "-q", "-c", "a"]);
    commit(&root, "shared.rs", "a\n", "A edits shared");
    git(&root, &["switch", "-q", "-c", "a2"]);
    commit(&root, "a2.rs", "a2\n", "A2 on top");
    // b from main, touching the same file, checked out in a linked worktree with changes.
    git(&root, &["switch", "-q", "main"]);
    git(&root, &["switch", "-q", "-c", "b"]);
    commit(&root, "b.rs", "b\n", "B");
    git(&root, &["switch", "-q", "main"]);
    let wt = dir.path().join("wt-b");
    git(&root, &["worktree", "add", "-q", &wt.to_string_lossy(), "b"]);
    fs::write(wt.join("shared.rs"), "b edits shared too\n").unwrap();
    // A merged branch, and an unrelated one that only sits near the others.
    git(&root, &["switch", "-q", "-c", "done"]);
    commit(&root, "done.rs", "done\n", "Done");
    git(&root, &["switch", "-q", "main"]);
    git(&root, &["merge", "-q", "--ff-only", "done"]);
    commit(&root, "main.rs", "main moves on\n", "Main moves on");

    let session = agents::Session {
        id: "s1".into(),
        harness: "codex".into(),
        title: "Working on b".into(),
        cwd: wt.join("sub/dir").to_string_lossy().into(),
        activity: "working".into(),
        open: vec!["nyx-agents".into(), "--open".into(), "session/s1".into()],
        ..Default::default()
    };
    let stranger = agents::Session { id: "s2".into(), cwd: "/elsewhere".into(), ..Default::default() };
    let f = flow::read(&root, &root, vec![session, stranger]).unwrap();
    let get = |n: &str| f.branches.iter().find(|b| b.name == n).unwrap();
    assert_eq!(f.trunk.as_deref(), Some("main"));
    assert!(get("main").trunk && get("main").current);
    assert_eq!(get("a2").parent.as_deref(), Some("a"), "a2 builds on a");
    assert_eq!(get("a").parent.as_deref(), Some("main"));
    assert_eq!(get("b").parent.as_deref(), Some("main"), "b doesn't build on a just because both exist");
    assert_eq!(get("a").divergence, Some((1, 2)));
    assert_eq!(get("a2").divergence, Some((2, 2)));
    assert!(get("done").merged && !get("done").active);
    // The worktree and the agent working in it.
    let w = f.worktrees.iter().find(|w| w.branch.as_deref() == Some("b")).unwrap();
    assert_eq!(w.dirty, Some(1));
    assert!(!w.open && f.worktrees.iter().any(|w| w.open && w.main));
    assert_eq!(f.agents.len(), 1, "a session outside the repository's worktrees isn't shown");
    assert_eq!(f.agents[0].1.as_deref().map(|p| p.canonicalize().unwrap()), Some(wt.canonicalize().unwrap()));
    // a and b both touch shared.rs (b only in its working tree); a2 shares it through a.
    let pair = |x: &str, y: &str| f.overlaps.iter().find(|o| (o.a == x && o.b == y) || (o.a == y && o.b == x));
    assert_eq!(pair("a", "b").map(|o| o.files.clone()), Some(vec!["shared.rs".into()]));
    // Inactive branches are left out of the cards unless asked for; every card has a place.
    let nodes = flow::nodes(&f, false);
    assert!(!nodes.iter().any(|(id, _)| id == "branch:done"));
    assert!(flow::nodes(&f, true).iter().any(|(id, _)| id == "branch:done"));
    let places = flow::layout(&f, &nodes);
    assert_eq!(places.len(), nodes.len());
    assert!(places["branch:a2"].x > places["branch:a"].x);
    // Reading changed nothing.
    assert_eq!(git(&root, &["status", "--porcelain"]), "");
    assert_eq!(git(&wt, &["status", "--porcelain"]), "M shared.rs");
}
