//! Explicit, stale-state checked resolution of regular-file index conflicts.
use super::{advanced::Guard, operations};
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsString,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct Side {
    pub bytes: Vec<u8>,
    pub mode: u32,
}
#[derive(Clone, Debug)]
pub struct Conflict {
    pub root: PathBuf,
    pub file: PathBuf,
    pub base: Option<Side>,
    pub ours: Option<Side>,
    pub theirs: Option<Side>,
    pub working: Option<Vec<u8>>,
    guard: Guard,
    index: Vec<u8>,
}
#[derive(Clone, Debug)]
pub enum Resolution {
    Text(String),
    Ours,
    Theirs,
    Delete,
}
fn index(root: &Path, file: &Path) -> Result<Vec<u8>> {
    let mut args: Vec<OsString> = ["--literal-pathspecs", "ls-files", "--unmerged", "-z"].iter().map(OsString::from).collect();
    operations::paths(&mut args, &[file.to_path_buf()])?;
    operations::invoke(root, "Read conflict stages", args, None, &[], false)
}
fn safe_path(root: &Path, file: &Path) -> Result<PathBuf> {
    ensure!(
        !file.is_absolute() && file.components().all(|c| matches!(c, std::path::Component::Normal(_))),
        "Invalid repository path"
    );
    let path = root.join(file);
    let parent = path.parent().context("Missing parent")?.canonicalize()?;
    ensure!(parent.starts_with(root.canonicalize()?), "File parent leaves the working tree");
    if let Ok(m) = fs::symlink_metadata(&path) {
        ensure!(m.is_file() && !m.file_type().is_symlink(), "Resolve symlink and submodule conflicts with an external tool");
    }
    Ok(path)
}
pub fn load(root: &Path, file: &Path) -> Result<Conflict> {
    let guard = Guard::capture(root)?;
    let path = safe_path(&guard.root, file)?;
    let raw = index(&guard.root, file)?;
    ensure!(!raw.is_empty(), "This file is no longer conflicted");
    let repo = gix::discover(&guard.root)?;
    let mut sides: [Option<Side>; 3] = [None, None, None];
    for record in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let header = record.split(|b| *b == b'\t').next().context("Invalid index record")?;
        let fields = std::str::from_utf8(header)?.split_whitespace().collect::<Vec<_>>();
        ensure!(fields.len() == 3, "Invalid conflict stage");
        let mode = u32::from_str_radix(fields[0], 8)?;
        ensure!(matches!(mode, 0o100644 | 0o100755), "Resolve symlink and submodule conflicts with an external tool");
        let stage: usize = fields[2].parse()?;
        ensure!((1..=3).contains(&stage), "Invalid conflict stage");
        let obj = repo.find_object(gix::ObjectId::from_hex(fields[1].as_bytes())?)?;
        ensure!(obj.data.len() <= 16 * 1024 * 1024, "Conflict exceeds the 16 MiB editor limit; use an external tool");
        sides[stage - 1] = Some(Side { bytes: obj.data.clone(), mode });
    }
    let working = match fs::read(path) {
        Ok(b) => {
            ensure!(b.len() <= 16 * 1024 * 1024, "Working file exceeds 16 MiB");
            Some(b)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let [base, ours, theirs] = sides;
    Ok(Conflict { root: guard.root.clone(), file: file.to_path_buf(), base, ours, theirs, working, guard, index: raw })
}
impl Conflict {
    pub fn text(&self) -> bool {
        self.base
            .iter()
            .chain(self.ours.iter())
            .chain(self.theirs.iter())
            .all(|s| !s.bytes.contains(&0) && std::str::from_utf8(&s.bytes).is_ok())
            && self.working.as_ref().is_none_or(|b| !b.contains(&0) && std::str::from_utf8(b).is_ok())
    }
}
/// What kind of conflict a file has, from which index stages exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Both sides changed the contents.
    Content,
    /// Both sides added a file at this path.
    AddedByBoth,
    /// We deleted it, they changed it.
    DeletedByUs,
    /// We changed it, they deleted it.
    DeletedByThem,
    /// Not text: a side has to be chosen whole.
    Binary,
}
impl Conflict {
    pub fn shape(&self) -> Shape {
        match (&self.base, &self.ours, &self.theirs) {
            _ if !self.text() => Shape::Binary,
            (_, None, Some(_)) => Shape::DeletedByUs,
            (_, Some(_), None) => Shape::DeletedByThem,
            (None, Some(_), Some(_)) => Shape::AddedByBoth,
            _ => Shape::Content,
        }
    }
}
/// One conflict in a file with markers: line numbers of its `<<<<<<<` and `>>>>>>>` lines,
/// and each side's lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub start: usize,
    pub end: usize,
    pub ours: Vec<String>,
    pub base: Option<Vec<String>>,
    pub theirs: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Ours,
    Theirs,
    /// Ours, then theirs.
    Both,
    Base,
}
/// The conflict blocks in `text`, in order. Unbalanced markers end the scan.
pub fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with("<<<<<<<") {
            i += 1;
            continue;
        }
        let start = i;
        let (mut ours, mut base, mut theirs) = (Vec::new(), None::<Vec<String>>, Vec::new());
        let mut part = 0;
        i += 1;
        let mut closed = false;
        while i < lines.len() {
            let l = lines[i];
            if l.starts_with("|||||||") && part == 0 {
                part = 1;
                base = Some(Vec::new());
            } else if l.starts_with("=======") && part < 2 {
                part = 2;
            } else if l.starts_with(">>>>>>>") && part == 2 {
                closed = true;
                break;
            } else {
                match part {
                    0 => ours.push(l.to_string()),
                    1 => base.get_or_insert_with(Vec::new).push(l.to_string()),
                    _ => theirs.push(l.to_string()),
                }
            }
            i += 1;
        }
        if !closed {
            break;
        }
        out.push(Block { start, end: i, ours, base, theirs });
        i += 1;
    }
    out
}
/// `text` with conflict `index` replaced by the chosen side(s). The rest is untouched.
pub fn apply(text: &str, index: usize, choice: Choice) -> Option<String> {
    let block = blocks(text).into_iter().nth(index)?;
    let lines: Vec<&str> = text.split('\n').collect();
    let chosen: Vec<String> = match choice {
        Choice::Ours => block.ours.clone(),
        Choice::Theirs => block.theirs.clone(),
        Choice::Both => block.ours.iter().chain(&block.theirs).cloned().collect(),
        Choice::Base => block.base.clone()?,
    };
    let mut out: Vec<String> = lines[..block.start].iter().map(|s| s.to_string()).collect();
    out.extend(chosen);
    out.extend(lines[block.end + 1..].iter().map(|s| s.to_string()));
    Some(out.join("\n"))
}
pub fn has_markers(text: &str) -> bool {
    text.lines().any(|line| ["<<<<<<<", "=======", "|||||||", ">>>>>>>"].iter().any(|prefix| line.starts_with(prefix)))
}
pub fn resolve(root: &Path, conflict: &Conflict, resolution: Resolution) -> Result<Vec<u8>> {
    ensure!(root == conflict.root, "The active repository changed");
    conflict.guard.verify(root)?;
    ensure!(index(root, &conflict.file)? == conflict.index, "The conflict index changed; reopen the editor");
    let path = safe_path(root, &conflict.file)?;
    let now = match fs::read(&path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    ensure!(now == conflict.working, "The working file changed; reopen the editor");
    let side = match resolution {
        Resolution::Text(text) => {
            ensure!(!has_markers(&text), "Remove every conflict marker before marking this file resolved");
            Some(Side {
                bytes: text.into_bytes(),
                mode: conflict.ours.as_ref().or(conflict.theirs.as_ref()).map_or(0o100644, |s| s.mode),
            })
        }
        Resolution::Ours => conflict.ours.clone(),
        Resolution::Theirs => conflict.theirs.clone(),
        Resolution::Delete => None,
    };
    if let Some(side) = side {
        if let Ok(text) = std::str::from_utf8(&side.bytes) {
            ensure!(!has_markers(text), "The selected side contains conflict markers; edit the result before resolving");
        }
        let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("Missing parent")?)?;
        temp.write_all(&side.bytes)?;
        temp.as_file().set_permissions(fs::Permissions::from_mode(side.mode & 0o777))?;
        temp.persist(&path).map_err(|e| e.error)?;
    } else if path.exists() {
        fs::remove_file(&path)?;
    }
    let mut args: Vec<OsString> = ["--literal-pathspecs", "add", "--all"].iter().map(OsString::from).collect();
    operations::paths(&mut args, std::slice::from_ref(&conflict.file))?;
    match operations::invoke(root, "Mark conflict resolved", args, None, &[], false) {
        Ok(b) => Ok(b),
        Err(e) => bail!("The resolution was saved, but staging failed: {e:#}. Reopen the file to retry."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const TEXT: &str = "top\n<<<<<<< HEAD\nours 1\n||||||| base\nbase 1\n=======\ntheirs 1\n>>>>>>> topic\nmiddle\n<<<<<<< HEAD\nours 2\n=======\ntheirs 2\n>>>>>>> topic\nend\n";
    #[test]
    fn finds_blocks_with_and_without_a_base_and_resolves_one_at_a_time() {
        let b = blocks(TEXT);
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].start, b[0].end), (1, 7));
        assert_eq!(b[0].base.as_deref(), Some(&["base 1".to_string()][..]));
        assert_eq!(b[1].base, None);
        let one = apply(TEXT, 0, Choice::Theirs).unwrap();
        assert_eq!(blocks(&one).len(), 1, "only the chosen conflict changes");
        assert!(one.starts_with("top\ntheirs 1\nmiddle\n<<<<<<<"));
        let both = apply(&one, 0, Choice::Both).unwrap();
        assert_eq!(both, "top\ntheirs 1\nmiddle\nours 2\ntheirs 2\nend\n");
        assert!(!has_markers(&both));
        assert!(apply(&one, 0, Choice::Base).is_none(), "no base recorded for this block");
        assert!(blocks("<<<<<<< a\nunclosed\n").is_empty());
    }
}
