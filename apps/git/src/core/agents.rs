//! Nyx Agents' live sessions, read from the snapshot it publishes
//! (`$XDG_RUNTIME_DIR/nyx-agents/hub.json`, version 1). Nyx Git only reads it: sessions are
//! started, stopped and kept by Nyx Agents. Without the file, there are no agents.
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub harness: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub activity: String,
    #[serde(default)]
    pub blocked_on: Option<String>,
    #[serde(default)]
    pub started: u64,
    #[serde(default)]
    pub status_since: u64,
    /// Opens the session in Nyx Agents, as an argv.
    #[serde(default)]
    pub open: Vec<String>,
}
impl Session {
    /// The harness's name as people say it.
    pub fn agent(&self) -> String {
        match self.harness.as_str() {
            "claude" => "Claude Code".into(),
            "codex" => "Codex".into(),
            "cursor" => "Cursor".into(),
            "grok" => "Grok".into(),
            "shell" => "Shell".into(),
            "" => "Agent".into(),
            other => {
                let mut c = other.chars();
                c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
            }
        }
    }
    /// The activity in words: working, blocked on a permission, done…
    pub fn state(&self) -> String {
        match (self.activity.as_str(), self.blocked_on.as_deref()) {
            ("blocked", Some(on)) => format!("blocked on {on}"),
            ("unknown", _) | ("", _) => "running".into(),
            (a, _) => a.into(),
        }
    }
}
#[derive(Debug, Default, Deserialize)]
struct Hub {
    version: u32,
    #[serde(default)]
    sessions: Vec<Session>,
}
pub fn hub_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NYX_AGENTS_HUB") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("nyx-agents/hub.json"))
}
/// Live sessions; none when Nyx Agents isn't running or the file is of another version.
pub fn sessions() -> Vec<Session> {
    let Some(path) = hub_path() else { return Vec::new() };
    let Ok(bytes) = std::fs::read(&path) else { return Vec::new() };
    if bytes.len() > 4 * 1024 * 1024 {
        return Vec::new();
    }
    parse(&bytes)
}
pub fn parse(bytes: &[u8]) -> Vec<Session> {
    match serde_json::from_slice::<Hub>(bytes) {
        Ok(hub) if hub.version == 1 => hub.sessions,
        _ => Vec::new(),
    }
}
/// The session's worktree among `worktrees`: the deepest one its working directory is in.
pub fn worktree_of<'a>(session: &Session, worktrees: &'a [PathBuf]) -> Option<&'a PathBuf> {
    let cwd = Path::new(&session.cwd);
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    worktrees
        .iter()
        .filter(|w| {
            let w = w.canonicalize().unwrap_or_else(|_| w.to_path_buf());
            cwd.starts_with(&w)
        })
        .max_by_key(|w| w.components().count())
}
/// Show the session in Nyx Agents by running the hub's own argv, without a shell.
pub fn open(session: &Session) -> anyhow::Result<()> {
    let (program, args) = session.open.split_first().ok_or_else(|| anyhow::anyhow!("Nyx Agents gave no way to open this session"))?;
    anyhow::ensure!(program == "nyx-agents", "Nyx Agents' open command is unexpected");
    let mut child = std::process::Command::new(program).args(args).stdin(std::process::Stdio::null()).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_version_one_and_matches_sessions_to_the_deepest_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("repo");
        let linked = main.join("nested-worktree");
        std::fs::create_dir_all(linked.join("src")).unwrap();
        let hub = format!(
            r#"{{"version":1,"future":true,"sessions":[{{"id":"a1","harness":"claude","title":"Fix","cwd":"{}","branch":"agents/fix","activity":"blocked","blocked_on":"permission","open":["nyx-agents","--open","session/a1"]}}]}}"#,
            linked.join("src").display()
        );
        let sessions = parse(hub.as_bytes());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent(), "Claude Code");
        assert_eq!(sessions[0].state(), "blocked on permission");
        let trees = vec![main.clone(), linked.clone()];
        assert_eq!(worktree_of(&sessions[0], &trees), Some(&linked));
        assert!(parse(br#"{"version":2,"sessions":[{"id":"x"}]}"#).is_empty());
        assert!(parse(b"not json").is_empty());
    }
}
