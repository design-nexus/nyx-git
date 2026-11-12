//! Flow on a synthetic repository: `cargo run --release -p nyx-git --example flow_benchmark -- 20000 400 8`
//! (commits on the trunk, branches, worktrees). Measures the projection Flow reads in the
//! background and its layout, not drawing.
use std::{io::Write, process::Command, time::Instant};

fn git(root: &std::path::Path, args: &[&str]) {
    let s = Command::new("git").arg("-C").arg(root).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").status().unwrap();
    assert!(s.success(), "{args:?}");
}
fn main() {
    let mut args = std::env::args().skip(1).map(|a| a.parse::<usize>().unwrap());
    let commits = args.next().unwrap_or(20_000);
    let branches = args.next().unwrap_or(400);
    let worktrees = args.next().unwrap_or(8);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    // The trunk and one commit per branch, written by fast-import.
    let mut import = Command::new("git").arg("-C").arg(&root).args(["fast-import", "--quiet"]).stdin(std::process::Stdio::piped()).spawn().unwrap();
    let input = import.stdin.as_mut().unwrap();
    for i in 0..commits {
        let message = format!("Commit {i}\n");
        writeln!(input, "commit refs/heads/main\nmark :{}\ncommitter Bench <b@example.invalid> {} +0000\ndata {}\n{message}", i + 1, 1_700_000_000 + i, message.len()).unwrap();
        writeln!(input, "M 644 inline file{}\ndata 2\n{}\n", i % 64, i % 10).unwrap();
    }
    for b in 0..branches {
        let base = commits - 1 - (b * 7) % commits.min(5000);
        let message = format!("Work on branch {b}\n");
        writeln!(input, "commit refs/heads/topic/{b:04}\ncommitter Bench <b@example.invalid> {} +0000\ndata {}\n{message}from :{}", 1_800_000_000 + b, message.len(), base + 1).unwrap();
        writeln!(input, "M 644 inline topic{}\ndata 2\nx\n", b % 32).unwrap();
    }
    drop(import.stdin.take());
    assert!(import.wait().unwrap().success());
    git(&root, &["checkout", "-q", "main"]);
    for w in 0..worktrees {
        git(&root, &["worktree", "add", "-q", &dir.path().join(format!("wt{w}")).to_string_lossy(), &format!("topic/{w:04}")]);
    }
    let started = Instant::now();
    let flow = nyx_git::core::flow::read(&root, &root, vec![]).unwrap();
    let read = started.elapsed();
    let started = Instant::now();
    let nodes = nyx_git::core::flow::nodes(&flow, false);
    let places = nyx_git::core::flow::layout(&flow, &nodes);
    let layout = started.elapsed();
    println!(
        "{commits} commits, {branches} branches ({} read), {worktrees} worktrees: read {:.1} ms, layout {:.2} ms, {} active cards, {} shared-file pairs",
        flow.branches.len(),
        read.as_secs_f64() * 1000.0,
        layout.as_secs_f64() * 1000.0,
        places.len(),
        flow.overlaps.len()
    );
}
