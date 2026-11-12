//! Read-only file listings and per-line authorship.
use super::operations;
use anyhow::{Result, ensure};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

/// Tracked paths in the index, at most `limit`, as raw Linux path bytes.
pub fn tracked(root: &Path, limit: usize) -> Result<Vec<PathBuf>> {
    let bytes = operations::invoke(root, "List tracked files", ["ls-files", "-z", "--cached"].map(OsString::from).into(), None, &[], false)?;
    Ok(bytes.split(|b| *b == 0).filter(|p| !p.is_empty()).take(limit).map(|p| PathBuf::from(OsStr::from_bytes(p))).collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// Full commit hash; all zeros for lines not yet committed.
    pub commit: String,
    pub author: String,
    pub time: i64,
    pub summary: String,
    pub text: String,
}
impl BlameLine {
    pub fn uncommitted(&self) -> bool {
        self.commit.bytes().all(|b| b == b'0')
    }
}
/// Largest file blamed in the app; bigger ones belong in a terminal.
pub const BLAME_LIMIT: u64 = 4 * 1024 * 1024;

/// Who last changed each line of `file` as it is in the working tree (`at` None) or at a commit.
pub fn blame(root: &Path, file: &Path, at: Option<&str>) -> Result<Vec<BlameLine>> {
    ensure!(
        !file.as_os_str().is_empty() && file.components().all(|c| matches!(c, Component::Normal(_))),
        "Expected a repository-relative file path"
    );
    if at.is_none() {
        let meta = std::fs::symlink_metadata(root.join(file))?;
        ensure!(meta.is_file(), "Only regular files can be blamed");
        ensure!(meta.len() <= BLAME_LIMIT, "The file is larger than 4 MiB; use git blame in a terminal");
    }
    let mut args: Vec<OsString> = ["--no-pager", "blame", "--porcelain", "-w", "-M"].map(OsString::from).into();
    if let Some(id) = at {
        operations::valid_id(id)?;
        args.push(id.into());
    }
    args.push("--".into());
    args.push(file.as_os_str().into());
    parse(&operations::invoke(root, "Read blame", args, None, &[], false)?)
}

/// `git blame --porcelain`: a header per line, commit details the first time a commit appears.
pub fn parse(bytes: &[u8]) -> Result<Vec<BlameLine>> {
    #[derive(Default, Clone)]
    struct Info {
        author: String,
        time: i64,
        summary: String,
    }
    let mut known: HashMap<String, Info> = HashMap::new();
    let mut lines = Vec::new();
    let mut commit: Option<String> = None;
    for raw in bytes.split(|b| *b == b'\n') {
        if let Some(text) = raw.strip_prefix(b"\t") {
            let id = commit.take().ok_or_else(|| anyhow::anyhow!("Malformed blame output"))?;
            let info = known.get(&id).cloned().unwrap_or_default();
            lines.push(BlameLine {
                commit: id,
                author: info.author,
                time: info.time,
                summary: info.summary,
                text: String::from_utf8_lossy(text).into_owned(),
            });
            continue;
        }
        let line = String::from_utf8_lossy(raw);
        if commit.is_none() {
            let id = line.split(' ').next().unwrap_or_default();
            if id.len() >= 40 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                known.entry(id.to_string()).or_default();
                commit = Some(id.to_string());
            }
            continue;
        }
        let Some(id) = commit.as_ref() else { continue };
        let info = known.entry(id.clone()).or_default();
        if let Some(v) = line.strip_prefix("author ") {
            info.author = v.chars().take(256).collect();
        } else if let Some(v) = line.strip_prefix("author-time ") {
            info.time = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("summary ") {
            info.summary = v.chars().take(512).collect();
        }
    }
    Ok(lines)
}

/// The commit message template from `commit.template`, if one is configured and readable.
pub fn commit_template(root: &Path) -> Option<String> {
    let bytes = operations::invoke(
        root,
        "Read commit template",
        ["config", "--path", "--get", "commit.template"].map(OsString::from).into(),
        None,
        &[],
        true,
    )
    .ok()?;
    let path = PathBuf::from(OsStr::from_bytes(bytes.strip_suffix(b"\n").unwrap_or(&bytes)));
    if path.as_os_str().is_empty() {
        return None;
    }
    let path = if path.is_relative() { root.join(path) } else { path };
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_porcelain_blame_with_repeated_commits() {
        let a = "a".repeat(40);
        let z = "0".repeat(40);
        let out = format!(
            "{a} 1 1 2\nauthor Ada\nauthor-time 100\nsummary First\nfilename f\n\tone\n{a} 2 2\n\ttwo\n{z} 3 3 1\nauthor Not Committed Yet\nauthor-time 200\nsummary Version of f from f\nfilename f\n\tthree\n"
        );
        let lines = parse(out.as_bytes()).unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!((lines[1].author.as_str(), lines[1].summary.as_str(), lines[1].text.as_str()), ("Ada", "First", "two"));
        assert!(lines[2].uncommitted() && !lines[0].uncommitted());
        assert_eq!(lines[0].time, 100);
    }
}
