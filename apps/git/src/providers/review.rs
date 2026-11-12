//! Review data from a hosting provider: a request's checks, files, commits and reviews, and
//! merging it. Providers differ; what one doesn't offer is reported as unavailable, never
//! made up.
use super::{Client, Kind, Repository, number, optional, parse_repository, required};
use anyhow::{Context, Result, ensure};
use reqwest::Method;
use serde_json::{Value, json};

/// What a provider can show and do for a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub checks: bool,
    pub reviews: bool,
    /// File line counts (GitLab lists changed files without them).
    pub line_counts: bool,
    pub merge: bool,
    /// Merge methods besides a merge commit.
    pub squash: bool,
    pub rebase: bool,
}
impl Kind {
    pub fn capabilities(self) -> Capabilities {
        match self {
            Kind::Github => Capabilities { checks: true, reviews: true, line_counts: true, merge: true, squash: true, rebase: true },
            // Merge methods are a project setting on GitLab; the API merges as the project says.
            Kind::Gitlab => Capabilities { checks: true, reviews: true, line_counts: false, merge: true, squash: false, rebase: false },
            Kind::Forgejo | Kind::Gitea => Capabilities { checks: true, reviews: true, line_counts: true, merge: true, squash: true, rebase: true },
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckState {
    Failure,
    Pending,
    Success,
    /// Neutral, skipped or cancelled.
    Other,
}
impl CheckState {
    pub fn word(self) -> &'static str {
        match self {
            Self::Failure => "failing",
            Self::Pending => "pending",
            Self::Success => "passing",
            Self::Other => "skipped",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub state: CheckState,
    pub url: String,
}
/// One word for a set of checks: failing if any fail, pending if any run, passing if all pass.
pub fn summary(checks: &[Check]) -> Option<CheckState> {
    if checks.is_empty() {
        return None;
    }
    checks.iter().map(|c| c.state).filter(|s| *s != CheckState::Other).min().or(Some(CheckState::Other))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub status: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestCommit {
    pub id: String,
    pub subject: String,
    pub author: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub author: String,
    /// APPROVED, CHANGES_REQUESTED, COMMENTED…, as the provider says.
    pub state: String,
    pub body: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}
fn state_of(word: &str) -> CheckState {
    match word.to_lowercase().as_str() {
        "success" | "passed" | "completed_success" => CheckState::Success,
        "failure" | "failed" | "error" | "timed_out" | "action_required" | "startup_failure" => CheckState::Failure,
        "pending" | "running" | "queued" | "in_progress" | "created" | "waiting_for_resource" | "preparing" | "scheduled" | "requested" | "waiting" | "manual" => CheckState::Pending,
        _ => CheckState::Other,
    }
}
fn array(v: &Value) -> Result<&Vec<Value>> {
    v.as_array().context("Expected a list from the provider")
}
pub fn parse_checks(kind: Kind, runs: Option<&Value>, statuses: &Value) -> Result<Vec<Check>> {
    let mut out = Vec::new();
    if let Some(runs) = runs.and_then(|r| r.get("check_runs")).and_then(Value::as_array) {
        for r in runs {
            let state = match (optional(r, "status").as_str(), r.get("conclusion").and_then(Value::as_str)) {
                ("completed", Some(c)) => state_of(c),
                ("completed", None) => CheckState::Other,
                _ => CheckState::Pending,
            };
            out.push(Check { name: required(r, "name")?, state, url: optional(r, "html_url") });
        }
    }
    let list = if kind == Kind::Github { statuses.get("statuses").unwrap_or(&Value::Null) } else { statuses };
    if let Some(list) = list.as_array() {
        for s in list {
            let (name, state, url) = match kind {
                Kind::Gitlab => (optional(s, "name"), optional(s, "status"), optional(s, "target_url")),
                Kind::Github => (optional(s, "context"), optional(s, "state"), optional(s, "target_url")),
                _ => (optional(s, "context"), optional(s, "status"), optional(s, "target_url")),
            };
            // Providers list a status once per update; keep the newest per name.
            if !out.iter().any(|c: &Check| c.name == name) {
                out.push(Check { name, state: state_of(&state), url });
            }
        }
    }
    Ok(out)
}
pub fn parse_files(kind: Kind, v: &Value) -> Result<Vec<FileChange>> {
    if kind == Kind::Gitlab {
        let list = v.get("changes").unwrap_or(v);
        return array(list)?
            .iter()
            .map(|f| {
                let status = if f.get("new_file").and_then(Value::as_bool) == Some(true) {
                    "added"
                } else if f.get("deleted_file").and_then(Value::as_bool) == Some(true) {
                    "removed"
                } else if f.get("renamed_file").and_then(Value::as_bool) == Some(true) {
                    "renamed"
                } else {
                    "modified"
                };
                Ok(FileChange { path: required(f, "new_path")?, status: status.into(), additions: None, deletions: None })
            })
            .collect();
    }
    array(v)?
        .iter()
        .map(|f| {
            Ok(FileChange {
                path: required(f, "filename")?,
                status: optional(f, "status"),
                additions: f.get("additions").and_then(Value::as_u64),
                deletions: f.get("deletions").and_then(Value::as_u64),
            })
        })
        .collect()
}
pub fn parse_commits(kind: Kind, v: &Value) -> Result<Vec<RequestCommit>> {
    array(v)?
        .iter()
        .map(|c| {
            Ok(if kind == Kind::Gitlab {
                RequestCommit { id: required(c, "id")?, subject: optional(c, "title"), author: optional(c, "author_name") }
            } else {
                let message = c.pointer("/commit/message").and_then(Value::as_str).unwrap_or_default();
                RequestCommit {
                    id: required(c, "sha")?,
                    subject: message.lines().next().unwrap_or_default().into(),
                    author: c.pointer("/commit/author/name").and_then(Value::as_str).unwrap_or_default().into(),
                }
            })
        })
        .collect()
}
pub fn parse_reviews(kind: Kind, v: &Value) -> Result<Vec<Review>> {
    if kind == Kind::Gitlab {
        let approved = v.get("approved_by").and_then(Value::as_array).cloned().unwrap_or_default();
        return Ok(approved
            .iter()
            .map(|a| Review {
                author: a.pointer("/user/username").and_then(Value::as_str).unwrap_or_default().into(),
                state: "APPROVED".into(),
                body: String::new(),
            })
            .collect());
    }
    Ok(array(v)?
        .iter()
        .map(|r| Review {
            author: r.pointer("/user/login").and_then(Value::as_str).unwrap_or_default().into(),
            state: optional(r, "state"),
            body: optional(r, "body").chars().take(2000).collect(),
        })
        .collect())
}
fn requests(kind: Kind) -> &'static str {
    if kind == Kind::Gitlab { "merge_requests" } else { "pulls" }
}
impl Client {
    pub fn kind(&self) -> Kind {
        self.account.kind
    }
    /// The provider's repository at `path` (as in its URL: `owner/name`, `group/sub/name`).
    pub fn find_repository(&self, path: &str) -> Result<Repository> {
        let path = path.trim_matches('/').trim_end_matches(".git");
        ensure!(!path.is_empty() && path.split('/').all(|s| !s.is_empty() && s != "." && s != ".."), "Invalid repository path");
        let v = if self.account.kind == Kind::Gitlab {
            // One segment, '/' escaped: the project's full path.
            self.call(Method::GET, &["projects", path], &[], None)?
        } else {
            let parts: Vec<&str> = path.split('/').collect();
            ensure!(parts.len() == 2, "Expected owner/name");
            self.call(Method::GET, &["repos", parts[0], parts[1]], &[], None)?
        };
        parse_repository(self.account.kind, &v.value)
    }
    pub fn checks(&self, repo: &Repository, sha: &str) -> Result<Vec<Check>> {
        crate::core::operations::valid_id(sha)?;
        let kind = self.account.kind;
        match kind {
            Kind::Github => {
                let runs = self.repo_call(Method::GET, repo, &["commits", sha, "check-runs"], &[("per_page", "100".into())], None)?;
                let statuses = self.repo_call(Method::GET, repo, &["commits", sha, "status"], &[], None)?;
                parse_checks(kind, Some(&runs.value), &statuses.value)
            }
            Kind::Gitlab => {
                let statuses = self.repo_call(Method::GET, repo, &["repository", "commits", sha, "statuses"], &[], None)?;
                parse_checks(kind, None, &statuses.value)
            }
            _ => {
                let statuses = self.repo_call(Method::GET, repo, &["commits", sha, "statuses"], &[("limit", "50".into())], None)?;
                parse_checks(kind, None, &statuses.value)
            }
        }
    }
    pub fn request_files(&self, repo: &Repository, n: u64) -> Result<Vec<FileChange>> {
        ensure!(n > 0, "Invalid request number");
        let kind = self.account.kind;
        let tail = if kind == Kind::Gitlab { "changes" } else { "files" };
        let v = self.repo_call(Method::GET, repo, &[requests(kind), &n.to_string(), tail], &[], None)?;
        parse_files(kind, &v.value)
    }
    pub fn request_commits(&self, repo: &Repository, n: u64) -> Result<Vec<RequestCommit>> {
        ensure!(n > 0, "Invalid request number");
        let kind = self.account.kind;
        let v = self.repo_call(Method::GET, repo, &[requests(kind), &n.to_string(), "commits"], &[], None)?;
        parse_commits(kind, &v.value)
    }
    pub fn reviews(&self, repo: &Repository, n: u64) -> Result<Vec<Review>> {
        ensure!(n > 0, "Invalid request number");
        let kind = self.account.kind;
        let tail = if kind == Kind::Gitlab { "approvals" } else { "reviews" };
        let v = self.repo_call(Method::GET, repo, &[requests(kind), &n.to_string(), tail], &[], None)?;
        parse_reviews(kind, &v.value)
    }
    /// Merge through the provider, only if the request's head is still `head`.
    pub fn merge(&self, repo: &Repository, n: u64, head: &str, method: MergeMethod) -> Result<()> {
        ensure!(n > 0, "Invalid request number");
        crate::core::operations::valid_id(head)?;
        let kind = self.account.kind;
        let caps = kind.capabilities();
        ensure!(caps.merge, "{} can't merge requests through its API", kind.name());
        ensure!(
            method == MergeMethod::Merge || (method == MergeMethod::Squash && caps.squash) || (method == MergeMethod::Rebase && caps.rebase),
            "{} merges with the method set for the project",
            kind.name()
        );
        let word = match method {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        };
        let number = n.to_string();
        match kind {
            Kind::Github => {
                self.repo_call(Method::PUT, repo, &["pulls", &number, "merge"], &[], Some(json!({"sha": head, "merge_method": word})))?;
            }
            Kind::Gitlab => {
                self.repo_call(Method::PUT, repo, &["merge_requests", &number, "merge"], &[], Some(json!({"sha": head})))?;
            }
            _ => {
                self.repo_call(Method::POST, repo, &["pulls", &number, "merge"], &[], Some(json!({"Do": word, "head_commit_id": head})))?;
            }
        }
        Ok(())
    }
}
/// Which connected account hosts a repository, from its remotes' URLs (`origin` first):
/// the account and the repository's path on it. An instance under a path (`/git/`) matches
/// only remotes under that path.
pub fn locate(accounts: &[super::Account], remotes: &[(String, String)]) -> Option<(super::Account, String)> {
    let mut ordered: Vec<&(String, String)> = remotes.iter().collect();
    ordered.sort_by_key(|(name, _)| name != "origin");
    for (_, url) in ordered {
        let Ok((host, path)) = crate::core::hosting::transport(url) else { continue };
        for account in accounts {
            let Ok(base) = account.base() else { continue };
            let Some(base_host) = base.host_str() else { continue };
            let base_host = match base.port() {
                Some(p) => format!("{base_host}:{p}"),
                None => base_host.to_lowercase(),
            };
            // SSH remotes name the host without the web port.
            let host_only = host.split(':').next().unwrap_or(&host);
            if host != base_host && host_only != base_host.split(':').next().unwrap_or_default() {
                continue;
            }
            let prefix = base.path().trim_matches('/');
            let path = if prefix.is_empty() { Some(path.as_str()) } else { path.strip_prefix(prefix).map(|p| p.trim_start_matches('/')) };
            if let Some(path) = path.filter(|p| !p.is_empty()) {
                return Some((account.clone(), path.to_string()));
            }
        }
    }
    None
}
/// The provider number a JSON request object carries, for callers that parse their own.
pub fn request_number(kind: Kind, v: &Value) -> Result<u64> {
    number(v, if kind == Kind::Gitlab { "iid" } else { "number" })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checks_from_each_provider_are_summarised_without_guessing() {
        let runs = json!({"check_runs":[{"name":"build","status":"completed","conclusion":"success","html_url":"https://x/1"},{"name":"lint","status":"in_progress","conclusion":null}]});
        let statuses = json!({"statuses":[{"context":"ci/legacy","state":"failure","target_url":"https://x/2"}]});
        let gh = parse_checks(Kind::Github, Some(&runs), &statuses).unwrap();
        assert_eq!(gh.len(), 3);
        assert_eq!(summary(&gh), Some(CheckState::Failure));
        let gl = parse_checks(Kind::Gitlab, None, &json!([{"name":"test","status":"running"},{"name":"test","status":"failed"}])).unwrap();
        assert_eq!(gl.len(), 1, "the newest status per name");
        assert_eq!(summary(&gl), Some(CheckState::Pending));
        let fj = parse_checks(Kind::Forgejo, None, &json!([{"context":"woodpecker","status":"success"}])).unwrap();
        assert_eq!(summary(&fj), Some(CheckState::Success));
        assert_eq!(summary(&[]), None, "no checks is not passing");
        let files = parse_files(Kind::Gitlab, &json!({"changes":[{"new_path":"a.rs","new_file":true},{"new_path":"b.rs"}]})).unwrap();
        assert_eq!((files[0].status.as_str(), files[1].additions), ("added", None));
        let reviews = parse_reviews(Kind::Gitlab, &json!({"approved_by":[{"user":{"username":"ada"}}]})).unwrap();
        assert_eq!(reviews[0].state, "APPROVED");
        assert!(!Kind::Gitlab.capabilities().line_counts && Kind::Github.capabilities().squash);
    }
    #[test]
    fn remotes_find_their_account_and_path() {
        use super::super::{Account, CredentialSource};
        let gh = Account::new(Kind::Github, "https://github.com", CredentialSource::GitHelper).unwrap();
        let forge = Account::new(Kind::Forgejo, "https://forge.example/git", CredentialSource::GitHelper).unwrap();
        let accounts = vec![gh.clone(), forge.clone()];
        let remotes = vec![
            ("fork".to_string(), "git@forge.example:git/me/tool.git".to_string()),
            ("origin".to_string(), "https://github.com/design-nexus/nyx-git.git".to_string()),
        ];
        let (account, path) = locate(&accounts, &remotes).unwrap();
        assert_eq!((account.kind, path.as_str()), (Kind::Github, "design-nexus/nyx-git"));
        let (account, path) = locate(&accounts, &remotes[..1]).unwrap();
        assert_eq!((account.kind, path.as_str()), (Kind::Forgejo, "me/tool"));
        assert!(locate(&accounts, &[("origin".into(), "https://forge.example/other/x.git".into())]).is_none(), "outside the instance path");
        assert!(locate(&accounts, &[("origin".into(), "/srv/git/local.git".into())]).is_none());
    }
}
