//! Optional hosting APIs. Tokens never enter settings, URLs or Git command arguments.
pub mod credentials;
pub mod review;
use anyhow::{Context, Result, bail, ensure};
use reqwest::{
    Method,
    blocking::Client as Http,
    header::{AUTHORIZATION, HeaderValue},
    redirect,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::Read, time::Duration};
use url::Url;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    Github,
    Gitlab,
    Forgejo,
    Gitea,
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Github => "GitHub",
            Self::Gitlab => "GitLab",
            Self::Forgejo => "Forgejo",
            Self::Gitea => "Gitea",
        }
    }
    pub fn default_url(self) -> &'static str {
        match self {
            Self::Github => "https://github.com",
            Self::Gitlab => "https://gitlab.com",
            Self::Forgejo => "https://codeberg.org",
            Self::Gitea => "https://gitea.com",
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CredentialSource {
    GitHelper,
    SecretService,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub kind: Kind,
    pub base_url: String,
    #[serde(default)]
    pub credential_username: String,
    pub username: String,
    pub source: CredentialSource,
}
impl Account {
    pub fn new(kind: Kind, base: &str, source: CredentialSource) -> Result<Self> {
        let base = validate_base(base)?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        Ok(Self {
            id: format!("{stamp}-{}", std::process::id()),
            kind,
            base_url: base.to_string().trim_end_matches('/').into(),
            credential_username: String::new(),
            username: String::new(),
            source,
        })
    }
    pub fn base(&self) -> Result<Url> {
        validate_base(&self.base_url)
    }
    pub fn token_page(&self) -> Result<Url> {
        let mut url = self.base()?;
        url.path_segments_mut().map_err(|_| anyhow::anyhow!("Invalid host URL"))?.pop_if_empty().extend(match self.kind {
            Kind::Github => vec!["settings", "personal-access-tokens"],
            Kind::Gitlab => vec!["-", "user_settings", "personal_access_tokens"],
            _ => vec!["user", "settings", "applications"],
        });
        Ok(url)
    }
}
pub fn validate_base(value: &str) -> Result<Url> {
    let mut u = Url::parse(value.trim()).context("Enter a complete HTTPS instance URL")?;
    ensure!(u.scheme() == "https" && u.host_str().is_some(), "Hosting accounts require HTTPS");
    ensure!(
        u.username().is_empty() && u.password().is_none() && u.query().is_none() && u.fragment().is_none(),
        "Use an instance URL without credentials, query or fragment"
    );
    ensure!(!u.path_segments().is_some_and(|mut p| p.any(|s| s == "." || s == "..")), "Invalid URL path");
    u.set_path(&format!("{}/", u.path().trim_end_matches('/')));
    Ok(u)
}
#[derive(Clone, Debug)]
pub struct Repository {
    pub key: String,
    pub name: String,
    pub web_url: String,
    pub https_url: String,
    pub ssh_url: String,
    pub default_branch: String,
}
#[derive(Clone, Debug)]
pub struct Request {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub draft: bool,
    pub web_url: String,
    pub head: String,
    pub source: String,
    /// The source branch's name alone (GitHub's `source` is `owner:branch`).
    pub source_branch: String,
    pub target: String,
    pub status: String,
}
#[derive(Clone, Debug)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub web_url: String,
}
#[derive(Clone, Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<u32>,
}
struct ApiResponse {
    value: Value,
    more: Option<bool>,
}
fn next_page(more: Option<bool>, count: usize, page: u32) -> Option<u32> {
    if more.unwrap_or(count > 0) { page.checked_add(1) } else { None }
}
pub struct Client {
    account: Account,
    http: Http,
    token: credentials::Secret,
    api: Url,
}
impl Client {
    pub fn new(account: Account, token: credentials::Secret) -> Result<Self> {
        let base = account.base()?;
        Self::build(account, token, base)
    }
    fn build(account: Account, token: credentials::Secret, mut api: Url) -> Result<Self> {
        if account.kind == Kind::Github && api.host_str() == Some("github.com") && api.port().is_none() && api.path() == "/" {
            api = Url::parse("https://api.github.com/")?;
        } else {
            api.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid API URL"))?
                .pop_if_empty()
                .extend(match account.kind {
                    Kind::Github => vec!["api", "v3"],
                    Kind::Gitlab => vec!["api", "v4"],
                    _ => vec!["api", "v1"],
                })
                .push("");
        }
        let http = Http::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(redirect::Policy::none())
            .user_agent("Nyx-Git/0.1")
            .build()?;
        Ok(Self { account, http, token, api })
    }
    fn call(&self, method: Method, segments: &[&str], query: &[(&str, String)], body: Option<Value>) -> Result<ApiResponse> {
        let mut url = self.api.clone();
        url.path_segments_mut().map_err(|_| anyhow::anyhow!("Invalid API URL"))?.pop_if_empty().extend(segments);
        let mut auth = HeaderValue::from_str(&format!(
            "{} {}",
            if matches!(self.account.kind, Kind::Forgejo | Kind::Gitea) { "token" } else { "Bearer" },
            self.token.expose()
        ))
        .map_err(|_| anyhow::anyhow!("Invalid credential"))?;
        auth.set_sensitive(true);
        let mut request =
            self.http.request(method, url).header(AUTHORIZATION, auth).header("Accept", "application/json").query(query);
        if self.account.kind == Kind::Github {
            request = request.header("X-GitHub-Api-Version", "2022-11-28");
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        // Responses may contain echoed credentials or private diagnostics; errors report status only.
        let response = request.send().map_err(|_| {
            anyhow::anyhow!("Could not reach {}. Check the network, TLS trust and instance URL.", self.account.kind.name())
        })?;
        let status = response.status();
        if !status.is_success() {
            bail!(
                "{} API returned HTTP {}. {}",
                self.account.kind.name(),
                status.as_u16(),
                match status.as_u16() {
                    401 => "Reconnect with a valid token.",
                    403 => "Check token permissions, organization authorization or rate limits.",
                    404 => "Check repository access and instance URL.",
                    429 => "Rate limited; wait before retrying.",
                    300..=399 => "Redirect refused; enter the final HTTPS instance URL.",
                    _ => "The server refused the request. Check its status and permissions before retrying.",
                }
            );
        }
        let more = if let Some(value) = response.headers().get("x-next-page") {
            Some(value.to_str().ok().is_some_and(|v| !v.trim().is_empty() && v.trim() != "0"))
        } else {
            response
                .headers()
                .get("link")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("rel=\"next\"") || v.contains("rel=next"))
        };
        let mut bytes = Vec::new();
        response
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow::anyhow!("Could not read the API response"))?;
        ensure!(bytes.len() <= 8 * 1024 * 1024, "API response exceeds 8 MiB");
        let value =
            serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("The provider returned an invalid JSON response"))?;
        Ok(ApiResponse { value, more })
    }
    pub fn save_token(&self) -> Result<()> {
        credentials::save(&self.account, &self.token)
    }
    pub fn verify(&self) -> Result<String> {
        let user = self.call(Method::GET, &["user"], &[], None)?;
        required(&user.value, if self.account.kind == Kind::Gitlab { "username" } else { "login" })
    }
    pub fn repositories(&self, page: u32) -> Result<Page<Repository>> {
        ensure!(page > 0, "Invalid page");
        let (path, mut query) = match self.account.kind {
            Kind::Gitlab => (vec!["projects"], vec![("membership", "true".into()), ("order_by", "last_activity_at".into())]),
            _ => (vec!["user", "repos"], vec![("sort", "updated".into())]),
        };
        query.extend(pagination(self.account.kind, page));
        let value = self.call(Method::GET, &path, &query, None)?;
        let array = value.value.as_array().context("Expected a repository list")?;
        let items = array.iter().map(|v| parse_repository(self.account.kind, v)).collect::<Result<_>>()?;
        Ok(Page { items, next: next_page(value.more, array.len(), page) })
    }
    fn repo_path(&self, repo: &Repository) -> Result<Vec<String>> {
        if self.account.kind == Kind::Gitlab {
            ensure!(repo.key.parse::<u64>().is_ok(), "Invalid project ID");
            Ok(vec!["projects".into(), repo.key.clone()])
        } else {
            let parts = repo.key.split('/').collect::<Vec<_>>();
            ensure!(
                parts.len() == 2 && parts.iter().all(|s| !s.is_empty() && *s != "." && *s != ".."),
                "Invalid repository name"
            );
            Ok(vec!["repos".into(), parts[0].into(), parts[1].into()])
        }
    }
    fn repo_call(
        &self,
        method: Method,
        repo: &Repository,
        tail: &[&str],
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<ApiResponse> {
        let mut path = self.repo_path(repo)?;
        path.extend(tail.iter().map(|s| s.to_string()));
        self.call(method, &path.iter().map(String::as_str).collect::<Vec<_>>(), query, body)
    }
    pub fn requests(&self, repo: &Repository, page: u32, all: bool) -> Result<Page<Request>> {
        ensure!(page > 0, "Invalid page");
        let kind = self.account.kind;
        let query = [
            vec![(
                "state",
                if all {
                    "all"
                } else if kind == Kind::Gitlab {
                    "opened"
                } else {
                    "open"
                }
                .into(),
            )],
            pagination(kind, page),
        ]
        .concat();
        let value =
            self.repo_call(Method::GET, repo, &[if kind == Kind::Gitlab { "merge_requests" } else { "pulls" }], &query, None)?;
        let a = value.value.as_array().context("Expected a request list")?;
        Ok(Page {
            items: a.iter().map(|v| parse_request(kind, v)).collect::<Result<_>>()?,
            next: next_page(value.more, a.len(), page),
        })
    }
    pub fn request(&self, repo: &Repository, number: u64) -> Result<Request> {
        ensure!(number > 0, "Invalid request number");
        let v = self.repo_call(
            Method::GET,
            repo,
            &[if self.account.kind == Kind::Gitlab { "merge_requests" } else { "pulls" }, &number.to_string()],
            &[],
            None,
        )?;
        parse_request(self.account.kind, &v.value)
    }
    pub fn issues(&self, repo: &Repository, page: u32) -> Result<Page<Issue>> {
        ensure!(page > 0, "Invalid page");
        let value = self.repo_call(Method::GET, repo, &["issues"], &pagination(self.account.kind, page), None)?;
        let a = value.value.as_array().context("Expected an issue list")?;
        Ok(Page {
            items: a
                .iter()
                .filter(|v| v.get("pull_request").is_none_or(Value::is_null))
                .map(|v| {
                    Ok(Issue {
                        number: number(v, if self.account.kind == Kind::Gitlab { "iid" } else { "number" })?,
                        title: required(v, "title")?,
                        web_url: required(v, if self.account.kind == Kind::Gitlab { "web_url" } else { "html_url" })?,
                    })
                })
                .collect::<Result<_>>()?,
            next: next_page(value.more, a.len(), page),
        })
    }
    pub fn create_request(
        &self,
        repo: &Repository,
        title: &str,
        description: &str,
        source: &str,
        target: &str,
        draft: bool,
    ) -> Result<Request> {
        ensure!(
            !title.trim().is_empty() && !source.trim().is_empty() && !target.trim().is_empty(),
            "Enter a title and source/target branches"
        );
        ensure!(title.len() <= 1000 && description.len() <= 100_000, "Request text is too long");
        let kind = self.account.kind;
        let body = if kind == Kind::Gitlab {
            json!({"title":if draft{format!("Draft: {}",title.trim())}else{title.trim().into()},"description":description,"source_branch":source,"target_branch":target})
        } else if kind == Kind::Github {
            json!({"title":title,"body":description,"head":source,"base":target,"draft":draft})
        } else {
            json!({"title":if draft{format!("WIP: {}",title.trim())}else{title.trim().into()},"body":description,"head":source,"base":target})
        };
        let v = self.repo_call(
            Method::POST,
            repo,
            &[if kind == Kind::Gitlab { "merge_requests" } else { "pulls" }],
            &[],
            Some(body),
        )?;
        parse_request(kind, &v.value)
    }
    pub fn checkout_reference(&self, number: u64) -> Result<String> {
        ensure!(number > 0, "Invalid request number");
        Ok(if self.account.kind == Kind::Gitlab {
            format!("refs/merge-requests/{number}/head")
        } else {
            format!("refs/pull/{number}/head")
        })
    }
    pub fn browser_url(&self, value: &str) -> Result<Url> {
        let url = validate_web(value)?;
        let base = self.account.base()?;
        ensure!(url.origin() == base.origin(), "The provider returned a link to another origin");
        Ok(url)
    }
}
fn validate_web(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("Invalid provider link")?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none(),
        "Provider links must be HTTPS without embedded credentials"
    );
    Ok(url)
}
fn required(v: &Value, key: &str) -> Result<String> {
    v.get(key).and_then(Value::as_str).map(String::from).with_context(|| format!("Provider response is missing {key}"))
}
fn number(v: &Value, key: &str) -> Result<u64> {
    v.get(key).and_then(Value::as_u64).filter(|n| *n > 0).with_context(|| format!("Provider response is missing {key}"))
}
fn optional(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().into()
}
fn pagination(kind: Kind, page: u32) -> Vec<(&'static str, String)> {
    vec![
        (if matches!(kind, Kind::Forgejo | Kind::Gitea) { "limit" } else { "per_page" }, "50".into()),
        ("page", page.to_string()),
    ]
}
pub fn parse_repository(kind: Kind, v: &Value) -> Result<Repository> {
    Ok(if kind == Kind::Gitlab {
        Repository {
            key: number(v, "id")?.to_string(),
            name: required(v, "path_with_namespace")?,
            web_url: required(v, "web_url")?,
            https_url: required(v, "http_url_to_repo")?,
            ssh_url: optional(v, "ssh_url_to_repo"),
            default_branch: optional(v, "default_branch"),
        }
    } else {
        Repository {
            key: required(v, "full_name")?,
            name: required(v, "full_name")?,
            web_url: required(v, "html_url")?,
            https_url: required(v, "clone_url")?,
            ssh_url: optional(v, "ssh_url"),
            default_branch: optional(v, "default_branch"),
        }
    })
}
pub fn parse_request(kind: Kind, v: &Value) -> Result<Request> {
    let gitlab = kind == Kind::Gitlab;
    let head =
        if gitlab { optional(v, "sha") } else { v.pointer("/head/sha").and_then(Value::as_str).unwrap_or_default().into() };
    let merged = v.get("merged").and_then(Value::as_bool).unwrap_or(false) || v.get("merged_at").is_some_and(|v| !v.is_null());
    let state = if merged { "merged".into() } else { required(v, "state")? };
    Ok(Request {
        number: number(v, if gitlab { "iid" } else { "number" })?,
        title: required(v, "title")?,
        state,
        draft: v.get("draft").or(v.get("work_in_progress")).and_then(Value::as_bool).unwrap_or_else(|| {
            v.get("title").and_then(Value::as_str).is_some_and(|s| s.starts_with("WIP:") || s.starts_with("Draft:"))
        }),
        web_url: required(v, if gitlab { "web_url" } else { "html_url" })?,
        head,
        source: if gitlab {
            optional(v, "source_branch")
        } else {
            v.pointer("/head/label").or(v.pointer("/head/ref")).and_then(Value::as_str).unwrap_or_default().into()
        },
        source_branch: if gitlab {
            optional(v, "source_branch")
        } else {
            v.pointer("/head/ref").and_then(Value::as_str).unwrap_or_default().into()
        },
        target: if gitlab {
            optional(v, "target_branch")
        } else {
            v.pointer("/base/ref").and_then(Value::as_str).unwrap_or_default().into()
        },
        status: if gitlab {
            optional(v, "detailed_merge_status")
        } else {
            let state = optional(v, "mergeable_state");
            if !state.is_empty() {
                state
            } else {
                match v.get("mergeable").and_then(Value::as_bool) {
                    Some(true) => "Mergeable".into(),
                    Some(false) => "Blocked or conflicting".into(),
                    None => "Mergeability pending".into(),
                }
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::credentials::Secret;
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    struct Expected {
        method: &'static str,
        path: String,
        response: Value,
        body: Option<Value>,
    }
    fn server(kind: Kind, expected: Vec<Expected>) -> (Client, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            for e in expected {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut b = [0; 4096];
                    let n = stream.read(&mut b).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&b[..n]);
                    if let Some(i) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let header = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let first = header.lines().next().unwrap().split_whitespace().collect::<Vec<_>>();
                assert_eq!(first[0], e.method);
                assert_eq!(first[1], e.path);
                let lower = header.to_lowercase();
                assert!(lower.contains(if matches!(kind, Kind::Gitea | Kind::Forgejo) {
                    "authorization: token fake-unit-token"
                } else {
                    "authorization: bearer fake-unit-token"
                }));
                assert!(!first[1].contains("token"));
                let len = lower
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: ").and_then(|l| l.parse::<usize>().ok()))
                    .unwrap_or(0);
                while bytes.len() < header_end + len {
                    let mut b = [0; 4096];
                    let n = stream.read(&mut b).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&b[..n]);
                }
                if let Some(body) = e.body {
                    let got: Value = serde_json::from_slice(&bytes[header_end..header_end + len]).unwrap();
                    assert_eq!(got, body);
                }
                let body = serde_json::to_vec(&e.response).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let account = Account::new(kind, "https://forge.example/subdir", CredentialSource::GitHelper).unwrap();
        let client = Client::build(
            account,
            credentials::Secret::new("fake-unit-token".into()).unwrap(),
            Url::parse(&format!("http://127.0.0.1:{port}/subdir/")).unwrap(),
        )
        .unwrap();
        (client, handle)
    }
    #[test]
    fn four_provider_endpoints_paging_and_request_payloads() {
        for kind in [Kind::Github, Kind::Gitlab, Kind::Forgejo, Kind::Gitea] {
            let gl = kind == Kind::Gitlab;
            let prefix = match kind {
                Kind::Github => "/subdir/api/v3",
                Kind::Gitlab => "/subdir/api/v4",
                _ => "/subdir/api/v1",
            };
            let repo = if gl {
                json!({"id":73,"path_with_namespace":"team/sub/project","web_url":"https://forge.example/team/sub/project","http_url_to_repo":"https://forge.example/team/sub/project.git","ssh_url_to_repo":"git@forge.example:team/sub/project.git","default_branch":"main"})
            } else {
                json!({"full_name":"team/project","html_url":"https://forge.example/team/project","clone_url":"https://forge.example/team/project.git","ssh_url":"git@forge.example:team/project.git","default_branch":"main"})
            };
            let request = if gl {
                json!({"iid":7,"title":"Draft: Change","state":"opened","web_url":"https://forge.example/p/merge_requests/7","sha":"a".repeat(40),"source_branch":"feature","target_branch":"main","draft":true,"detailed_merge_status":"checking"})
            } else {
                json!({"number":7,"title":"Change","state":"open","html_url":"https://forge.example/p/pull/7","head":{"sha":"a".repeat(40),"label":"team:feature"},"base":{"ref":"main"},"draft":true,"mergeable":true})
            };
            let repo_path = if gl { "projects/73" } else { "repos/team/project" };
            let pulls = if gl { "merge_requests" } else { "pulls" };
            let per_page = if matches!(kind, Kind::Forgejo | Kind::Gitea) { "limit" } else { "per_page" };
            let repositories = if gl {
                format!("{prefix}/projects?membership=true&order_by=last_activity_at&{per_page}=50&page=1")
            } else {
                format!("{prefix}/user/repos?sort=updated&{per_page}=50&page=1")
            };
            let body = if gl {
                json!({"title":"Draft: Change","description":"Details","source_branch":"feature","target_branch":"main"})
            } else if kind == Kind::Github {
                json!({"title":"Change","body":"Details","head":"feature","base":"main","draft":true})
            } else {
                json!({"title":"WIP: Change","body":"Details","head":"feature","base":"main"})
            };
            let issue = if gl {
                json!({"iid":2,"title":"Bug","web_url":"https://forge.example/p/issues/2"})
            } else {
                json!({"number":2,"title":"Bug","html_url":"https://forge.example/p/issues/2","pull_request":null})
            };
            let (client, server) = server(
                kind,
                vec![
                    Expected {
                        method: "GET",
                        path: format!("{prefix}/user"),
                        response: if gl { json!({"username":"ken"}) } else { json!({"login":"ken"}) },
                        body: None,
                    },
                    Expected { method: "GET", path: repositories, response: Value::Array(vec![repo; 50]), body: None },
                    Expected {
                        method: "GET",
                        path: format!(
                            "{prefix}/{repo_path}/{pulls}?state={}&{per_page}=50&page=1",
                            if gl { "opened" } else { "open" }
                        ),
                        response: json!([request.clone()]),
                        body: None,
                    },
                    Expected {
                        method: "GET",
                        path: format!("{prefix}/{repo_path}/{pulls}/7"),
                        response: request.clone(),
                        body: None,
                    },
                    Expected {
                        method: "GET",
                        path: format!("{prefix}/{repo_path}/issues?{per_page}=50&page=1"),
                        response: json!([issue]),
                        body: None,
                    },
                    Expected {
                        method: "POST",
                        path: format!("{prefix}/{repo_path}/{pulls}"),
                        response: request,
                        body: Some(body),
                    },
                ],
            );
            assert_eq!(client.verify().unwrap(), "ken");
            let repos = client.repositories(1).unwrap();
            assert_eq!(repos.next, Some(2));
            let repo = &repos.items[0];
            let requests = client.requests(repo, 1, false).unwrap();
            assert_eq!(requests.items[0].number, 7);
            assert_eq!(client.request(repo, 7).unwrap().target, "main");
            assert_eq!(client.issues(repo, 1).unwrap().items[0].number, 2);
            assert_eq!(client.create_request(repo, "Change", "Details", "feature", "main", true).unwrap().number, 7);
            assert_eq!(client.checkout_reference(7).unwrap(), if gl { "refs/merge-requests/7/head" } else { "refs/pull/7/head" });
            server.join().unwrap();
        }
    }
    #[test]
    fn review_endpoints_for_each_provider() {
        let sha = "b".repeat(40);
        for kind in [Kind::Github, Kind::Gitlab, Kind::Forgejo, Kind::Gitea] {
            let gl = kind == Kind::Gitlab;
            let prefix = match kind {
                Kind::Github => "/subdir/api/v3",
                Kind::Gitlab => "/subdir/api/v4",
                _ => "/subdir/api/v1",
            };
            let repo_json = if gl {
                json!({"id":73,"path_with_namespace":"team/sub/project","web_url":"https://forge.example/team/sub/project","http_url_to_repo":"https://forge.example/team/sub/project.git"})
            } else {
                json!({"full_name":"team/project","html_url":"https://forge.example/team/project","clone_url":"https://forge.example/team/project.git"})
            };
            let (find, repo_path, pulls) =
                if gl { (format!("{prefix}/projects/team%2Fsub%2Fproject"), "projects/73", "merge_requests") } else { (format!("{prefix}/repos/team/project"), "repos/team/project", "pulls") };
            let mut expected = vec![Expected { method: "GET", path: find, response: repo_json, body: None }];
            match kind {
                Kind::Github => {
                    expected.push(Expected { method: "GET", path: format!("{prefix}/{repo_path}/commits/{sha}/check-runs?per_page=100"), response: json!({"check_runs":[]}), body: None });
                    expected.push(Expected { method: "GET", path: format!("{prefix}/{repo_path}/commits/{sha}/status"), response: json!({"statuses":[{"context":"ci","state":"success"}]}), body: None });
                }
                Kind::Gitlab => expected.push(Expected { method: "GET", path: format!("{prefix}/{repo_path}/repository/commits/{sha}/statuses"), response: json!([{"name":"ci","status":"success"}]), body: None }),
                _ => expected.push(Expected { method: "GET", path: format!("{prefix}/{repo_path}/commits/{sha}/statuses?limit=50"), response: json!([{"context":"ci","status":"success"}]), body: None }),
            }
            let files = if gl { json!({"changes":[{"new_path":"a.rs"}]}) } else { json!([{"filename":"a.rs","status":"modified","additions":1,"deletions":2}]) };
            expected.push(Expected { method: "GET", path: format!("{prefix}/{repo_path}/{pulls}/7/{}", if gl { "changes" } else { "files" }), response: files, body: None });
            let commits = if gl { json!([{"id":sha.clone(),"title":"Change","author_name":"Ada"}]) } else { json!([{"sha":sha.clone(),"commit":{"message":"Change\n\nBody","author":{"name":"Ada"}}}]) };
            expected.push(Expected { method: "GET", path: format!("{prefix}/{repo_path}/{pulls}/7/commits"), response: commits, body: None });
            let reviews = if gl { json!({"approved_by":[]}) } else { json!([{"user":{"login":"bo"},"state":"APPROVED","body":"ok"}]) };
            expected.push(Expected { method: "GET", path: format!("{prefix}/{repo_path}/{pulls}/7/{}", if gl { "approvals" } else { "reviews" }), response: reviews, body: None });
            let (method, body) = match kind {
                Kind::Github => ("PUT", json!({"sha":sha.clone(),"merge_method":"merge"})),
                Kind::Gitlab => ("PUT", json!({"sha":sha.clone()})),
                _ => ("POST", json!({"Do":"merge","head_commit_id":sha.clone()})),
            };
            expected.push(Expected { method, path: format!("{prefix}/{repo_path}/{pulls}/7/merge"), response: json!({}), body: Some(body) });
            let (client, server) = server(kind, expected);
            let repo = client.find_repository(if gl { "team/sub/project" } else { "team/project.git" }).unwrap();
            let checks = client.checks(&repo, &sha).unwrap();
            assert_eq!(review::summary(&checks), Some(review::CheckState::Success));
            assert_eq!(client.request_files(&repo, 7).unwrap()[0].path, "a.rs");
            assert_eq!(client.request_commits(&repo, 7).unwrap()[0].subject, "Change");
            assert_eq!(client.reviews(&repo, 7).unwrap().len(), if gl { 0 } else { 1 });
            client.merge(&repo, 7, &sha, review::MergeMethod::Merge).unwrap();
            if gl {
                assert!(client.merge(&repo, 7, &sha, review::MergeMethod::Squash).is_err(), "GitLab merges as the project is set");
            }
            server.join().unwrap();
        }
    }
    #[test]
    fn host_validation_secret_redaction_and_safe_failure_responses() {
        for bad in [
            "http://example.com",
            "https://user:token@example.com",
            "file:///tmp/secret",
            "https://example.com/?token=oops",
            "https://example.com/#bad",
        ] {
            assert!(validate_base(bad).is_err(), "{bad}");
        }
        let a = Account::new(Kind::Gitlab, "https://example.com/prefix/", CredentialSource::SecretService).unwrap();
        assert!(a.token_page().unwrap().as_str().starts_with("https://example.com/prefix/"));
        assert!(!serde_json::to_string(&a).unwrap().contains("private-token"));
        assert!(!format!("{:?}", Secret::new("private-token".into()).unwrap()).contains("private-token"));
        for status in [401, 302] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0; 4096];
                let _ = s.read(&mut buf);
                let body = "private-token echoed";
                write!(s,"HTTP/1.1 {status} Test\r\nLocation: http://127.0.0.1:1/never\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            });
            let c = Client::build(
                a.clone(),
                Secret::new("private-token".into()).unwrap(),
                Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
            )
            .unwrap();
            let e = c.verify().unwrap_err().to_string();
            assert!(e.contains(&status.to_string()));
            assert!(!e.contains("private-token"));
            assert!(c.browser_url("https://other.example/path").is_err());
            server.join().unwrap();
        }
    }
}
