//! Git on the control interface (`nyx_core::control`, API 2), for `nyx git …` (and
//! `nyx repo …`), Glimpse's Git panel and Nyx Assistant. Read a repository's status,
//! branches, worktrees, changes, history, flow and requests; open it; fetch; switch or
//! create a branch; create a worktree. Nothing here commits, pushes, discards, deletes or
//! rewrites history: those stay in the window, behind its previews.
//!
//! Actions take an optional `path` (absolute, or `~/…`): without one, the repository open in
//! Git, else the last one opened. Works with no window (D-Bus starts Git as a service).
use crate::{prefs, ui::window};
use gtk::{glib, prelude::*};
use nyx_actions::{Error, ErrorKind};
use nyx_core::control::Answer;
use nyx_git::core::{
    agents, flow,
    navigation::{self, Action as NavAction, Kind as NavKind},
    operations::{self, Operation},
    repository, worktrees,
};
use serde_json::{Value, json};
use std::{cell::RefCell, path::PathBuf};

const APP: &str = "io.github.design_nexus.nyx.Git";

pub fn description(app_id: &str) -> Value {
    let path = json!({"type": "string", "description": "a repository folder (absolute or ~/…); the open or last repository when left out"});
    let read = |name: &str, title: &str, text: &str, result: &str, aliases: Value| {
        json!({"name": name, "description": text,
            "inputSchema": {"type": "object", "properties": {"path": path}, "additionalProperties": false},
            "annotations": {"title": title, "readOnlyHint": true},
            "x-nyx": {"risk": "read_only", "positional": ["path"], "aliases": aliases, "result": result}})
    };
    let mut d = json!({
        "tools": [
            read("repo.status", "Status", "A repository's branch, upstream, commits ahead and behind, and changed files.", "object", json!(["st"])),
            {"name": "repo.repos", "description": "Repositories Git knows: open tabs, favorites and recent ones.",
             "inputSchema": {"type": "object", "properties": {"prefix": {"type": "string"}}, "additionalProperties": false},
             "annotations": {"title": "Repositories", "readOnlyHint": true},
             "x-nyx": {"risk": "read_only", "positional": ["prefix"], "aliases": ["list", "ls"], "result": "list"}},
            read("repo.branches", "Branches", "Local branches, the current one first, with upstream and last commit.", "list", json!(["br"])),
            read("repo.worktrees", "Worktrees", "A repository's worktrees with their branches and how many files changed in each.", "list", json!(["wt"])),
            read("repo.changes", "Changes", "Changed, staged, untracked and conflicted files in a repository's working tree.", "list", json!(["diff"])),
            {"name": "repo.history", "description": "Recent commits on a repository's current branch: hash, author, date, subject.",
             "inputSchema": {"type": "object", "properties": {"path": path, "count": {"type": "integer", "minimum": 1, "maximum": 200}}, "additionalProperties": false},
             "annotations": {"title": "History", "readOnlyHint": true},
             "x-nyx": {"risk": "read_only", "positional": ["path"], "aliases": ["log"], "result": "list"}},
            read("repo.flow", "Flow", "Work in progress: active branches with their divergence from the trunk, worktrees, agent sessions and branches touching the same files.", "object", json!([])),
            {"name": "repo.reviews", "description": "Open pull or merge requests for a repository on its connected hosting account.",
             "inputSchema": {"type": "object", "properties": {"path": path}, "additionalProperties": false},
             "annotations": {"title": "Requests", "readOnlyHint": true, "openWorldHint": true},
             "x-nyx": {"risk": "external", "positional": ["path"], "aliases": ["prs", "requests"], "result": "list"}},
            {"name": "repo.open", "description": "Open a repository in Git, at a page (flow, changes, history, review or activity) when named.",
             "inputSchema": {"type": "object", "properties": {"path": path, "page": {"type": "string", "enum": ["flow", "changes", "history", "review", "activity"]}}, "additionalProperties": false},
             "annotations": {"title": "Open in Git"},
             "x-nyx": {"risk": "writes", "confirmation": "never", "positional": ["path"], "result": "none"}},
            {"name": "repo.fetch", "description": "Fetch all remotes of a repository (downloads; changes no branch or file).",
             "inputSchema": {"type": "object", "properties": {"path": path}, "additionalProperties": false},
             "annotations": {"title": "Fetch", "openWorldHint": true},
             "x-nyx": {"risk": "external", "positional": ["path"], "result": "message"}},
            {"name": "repo.switch_branch", "description": "Switch a repository to a local branch. Git refuses when local changes would be overwritten.",
             "inputSchema": {"type": "object", "properties": {"branch": {"type": "string", "minLength": 1}, "path": path}, "required": ["branch"], "additionalProperties": false},
             "annotations": {"title": "Switch branch"},
             "x-nyx": {"risk": "writes", "positional": ["branch", "path"], "aliases": ["switch", "checkout"], "result": "message"}},
            {"name": "repo.branch_create", "description": "Create a branch at the current commit and switch to it. Local changes come along.",
             "inputSchema": {"type": "object", "properties": {"name": {"type": "string", "minLength": 1}, "path": path}, "required": ["name"], "additionalProperties": false},
             "annotations": {"title": "Create branch"},
             "x-nyx": {"risk": "writes", "positional": ["name", "path"], "aliases": ["branch", "new"], "result": "message"}},
            {"name": "repo.worktree_create", "description": "Create a worktree for a branch (an existing one, or a new one from HEAD) in a folder beside the repository.",
             "inputSchema": {"type": "object", "properties": {"branch": {"type": "string", "minLength": 1}, "path": path}, "required": ["branch"], "additionalProperties": false},
             "annotations": {"title": "Create worktree"},
             "x-nyx": {"risk": "writes", "positional": ["branch", "path"], "aliases": ["worktree"], "result": "message"}},
        ],
        "nyx": {"api": 2, "app": app_id, "name": "Git", "icon": APP, "namespaces": ["repo"], "aliases": ["git"],
                "bus_name": app_id, "activation": "dbus", "default": "repo.status", "open": {"action": "repo.open"},
                "topics": [{"name": "status"}],
                "panel": {"id": "git", "topics": ["status"], "size": {"min": [460, 300], "preferred": [620, 400]}}}
    });
    // Launchers pick icons from verbs otherwise ("switch" would be a power button).
    for tool in d["tools"].as_array_mut().into_iter().flatten() {
        let icon = match tool["name"].as_str().unwrap_or_default() {
            "repo.status" => "document-properties-symbolic",
            "repo.repos" => "folder-symbolic",
            "repo.branches" => "media-playlist-consecutive-symbolic",
            "repo.worktrees" => "folder-open-symbolic",
            "repo.changes" => "document-edit-symbolic",
            "repo.history" => "document-open-recent-symbolic",
            "repo.flow" => "network-workgroup-symbolic",
            "repo.reviews" => "object-select-symbolic",
            "repo.open" => "go-jump-symbolic",
            "repo.fetch" => "network-receive-symbolic",
            "repo.switch_branch" => "media-playlist-consecutive-symbolic",
            "repo.branch_create" => "list-add-symbolic",
            "repo.worktree_create" => "folder-new-symbolic",
            _ => continue,
        };
        tool["x-nyx"]["icon"] = json!(icon);
    }
    d
}

thread_local! {
    static SERVICE: RefCell<Option<nyx_core::control::Service>> = const { RefCell::new(None) };
    static TIMER: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
}
/// Register the interface (from `connect_startup`). The `status` topic is read again every
/// 10 s while watched, and whenever the window reads its repository.
pub fn serve(app: &gtk::Application) {
    let id = app.application_id().map(|s| s.to_string()).unwrap_or_else(|| APP.to_string());
    let window_app = app.clone();
    let Some(service) =
        nyx_core::control::serve_with(app, description(&id), move |request, answer| handle(&window_app, &request.action, &request.arguments, answer))
    else {
        return;
    };
    service.on_watch(|_, on| {
        TIMER.with(|t| {
            if let Some(id) = t.borrow_mut().take() {
                id.remove();
            }
            if on {
                publish();
                *t.borrow_mut() = Some(glib::timeout_add_seconds_local(10, || {
                    publish();
                    glib::ControlFlow::Continue
                }));
            }
        });
    });
    SERVICE.with(|s| *s.borrow_mut() = Some(service));
}
/// Tell watchers (Glimpse) the repository changed.
pub fn changed() {
    publish();
}
fn publish() {
    let Some(service) = SERVICE.with(|s| s.borrow().clone()) else { return };
    if !service.watched("status") {
        return;
    }
    let Ok(root) = resolve(None) else {
        service.publish("status", json!({"repository": null}));
        return;
    };
    nyx_core::cmd::background(move || panel_state(&root), move |state| service.publish("status", state.unwrap_or_else(|e| json!({"error": format!("{e:#}")}))));
}
fn failed(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Failed, message)
}
fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidArguments, message)
}
/// The repository an action is about.
fn resolve(path: Option<&str>) -> Result<PathBuf, Error> {
    if let Some(p) = path.filter(|p| !p.trim().is_empty()) {
        let p = nyx_core::control::home_path(p.trim()).map_err(invalid)?;
        if !p.is_dir() {
            return Err(invalid(format!("{} isn't a folder", p.display())));
        }
        return Ok(p);
    }
    if let Some(root) = window::current().and_then(|ui| ui.root()) {
        return Ok(root);
    }
    let p = prefs::get();
    p.workspace.active.or(p.last_repository).filter(|r| r.is_dir()).ok_or_else(|| invalid("No repository is open; name one with a path"))
}
fn status(root: &std::path::Path) -> anyhow::Result<Value> {
    let s = repository::load_reusing(root, None)?;
    let (ahead, behind) = divergence(s.status.divergence.as_deref());
    let files = &s.status.files;
    let count = |f: fn(&nyx_git::core::status::FileStatus) -> bool| files.iter().filter(|x| f(x)).count();
    Ok(json!({
        "repository": s.repository.root.to_string_lossy(),
        "name": s.repository.root.file_name().unwrap_or_default().to_string_lossy(),
        "branch": s.repository.branch,
        "upstream": s.status.upstream,
        "ahead": ahead,
        "behind": behind,
        "changed": files.len(),
        "staged": count(|f| f.index != '.' && f.index != '?' && !f.conflicted),
        "untracked": count(|f| f.index == '?'),
        "conflicts": count(|f| f.conflicted),
        "operation": s.progress.map(|p| p.name()),
        "untrusted_content": true,
    }))
}
fn divergence(text: Option<&str>) -> (u64, u64) {
    let mut n = text.unwrap_or_default().split_whitespace().map(|n| n.trim_start_matches(['+', '-']).parse().unwrap_or(0));
    (n.next().unwrap_or(0), n.next().unwrap_or(0))
}
fn changes(root: &std::path::Path) -> anyhow::Result<Value> {
    let s = repository::load_reusing(root, None)?;
    let items: Vec<Value> = s
        .status
        .files
        .iter()
        .take(500)
        .map(|f| {
            let path = f.path.to_string_lossy();
            json!({"title": path, "value": path, "subtitle": f.kind(), "state": f.short_kind(), "conflicted": f.conflicted})
        })
        .collect();
    Ok(json!({"items": items, "untrusted_content": true}))
}
fn branches(root: &std::path::Path) -> anyhow::Result<Value> {
    let inventory = navigation::read(root)?;
    let mut items = Vec::new();
    for g in inventory.groups.iter().filter(|g| g.kind == NavKind::Local) {
        for item in &g.items {
            if let NavAction::Reference { current, .. } = item.action {
                let detail = item.detail.lines().skip(1).collect::<Vec<_>>().join(" · ");
                items.push((current, json!({"title": item.name, "value": item.name, "subtitle": if current { format!("current{}", if detail.is_empty() { String::new() } else { format!(" · {detail}") }) } else { detail }, "current": current})));
            }
        }
    }
    items.sort_by_key(|(c, _)| !c);
    Ok(json!({"items": items.into_iter().map(|(_, v)| v).collect::<Vec<_>>(), "untrusted_content": true}))
}
fn worktree_list(root: &std::path::Path) -> anyhow::Result<Value> {
    let items: Vec<Value> = worktrees::list(root)?
        .into_iter()
        .map(|w| {
            let dirty = (!w.bare && w.path.exists())
                .then(|| operations::run(&w.path, Operation::Status).ok().and_then(|b| nyx_git::core::status::parse(&b).ok()).map(|s| s.files.len()))
                .flatten();
            let branch = w.branch.as_deref().map(|b| b.trim_start_matches("refs/heads/").to_string()).unwrap_or_else(|| "detached".into());
            json!({"title": branch, "value": w.path.to_string_lossy(), "path": w.path.to_string_lossy(),
                   "subtitle": format!("{}{}", w.path.display(), dirty.map(|n| if n == 0 { " · clean".to_string() } else { format!(" · {n} changed") }).unwrap_or_default()),
                   "main": w.main, "locked": w.locked.is_some(), "changed": dirty})
        })
        .collect();
    Ok(json!({"items": items, "untrusted_content": true}))
}
fn history(root: &std::path::Path, count: u64) -> anyhow::Result<Value> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["--no-pager", "log", &format!("--max-count={count}"), "--format=%H%x00%an%x00%aI%x00%s%x00", "--"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    let fields: Vec<String> = out.stdout.split(|b| *b == 0).map(|f| String::from_utf8_lossy(f).trim().to_string()).collect();
    let items: Vec<Value> = fields
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| json!({"title": c[3], "value": c[0], "subtitle": format!("{} · {} · {}", &c[0][..c[0].len().min(8)], c[1], c[2]), "hash": c[0], "author": c[1], "date": c[2]}))
        .collect();
    Ok(json!({"items": items, "untrusted_content": true}))
}
fn flow_summary(root: &std::path::Path) -> anyhow::Result<Value> {
    let f = flow::read(root, root, agents::sessions())?;
    let branches: Vec<Value> = f
        .branches
        .iter()
        .filter(|b| b.active)
        .map(|b| {
            let worktree = f.worktrees.iter().find(|w| w.branch.as_deref() == Some(b.name.as_str()));
            json!({"name": b.name, "trunk": b.trunk, "current": b.current, "builds_on": b.parent, "ahead": b.divergence.map(|d| d.0), "behind": b.divergence.map(|d| d.1),
                   "upstream": b.upstream, "worktree": worktree.map(|w| w.path.to_string_lossy()), "changed": worktree.and_then(|w| w.dirty), "subject": b.subject})
        })
        .collect();
    let agents: Vec<Value> =
        f.agents.iter().map(|(s, w)| json!({"agent": s.agent(), "title": s.title, "state": s.state(), "worktree": w.as_ref().map(|w| w.to_string_lossy())})).collect();
    let overlaps: Vec<Value> = f.overlaps.iter().map(|o| json!({"branches": [o.a, o.b], "files": o.files.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>()})).collect();
    Ok(json!({"trunk": f.trunk, "branches": branches, "agents": agents, "shared_files": overlaps, "untrusted_content": true}))
}
fn reviews(root: &std::path::Path) -> anyhow::Result<Value> {
    use nyx_git::providers::{Client, credentials, review};
    let accounts = prefs::get().accounts;
    let snapshot = nyx_git::core::remotes::read(root)?;
    let remotes: Vec<(String, String)> = snapshot.remotes.iter().flat_map(|r| r.fetch.iter().map(|u| (r.name.clone(), u.clone()))).collect();
    let (account, path) = review::locate(&accounts, &remotes).ok_or_else(|| anyhow::anyhow!("None of this repository's remotes is on a connected hosting account"))?;
    let c = Client::new(account.clone(), credentials::load(&account)?)?;
    let repo = c.find_repository(&path)?;
    let items: Vec<Value> = c
        .requests(&repo, 1, false)?
        .items
        .into_iter()
        .map(|r| json!({"title": format!("#{} {}", r.number, r.title), "value": r.web_url, "number": r.number, "subtitle": format!("{} → {} · {}", r.source_branch, r.target, r.status), "url": r.web_url}))
        .collect();
    Ok(json!({"items": items, "untrusted_content": true}))
}
/// What Glimpse's panel shows: status, recent commits, branches and worktrees together.
fn panel_state(root: &std::path::Path) -> anyhow::Result<Value> {
    let mut state = status(root)?;
    state["recent"] = history(root, 5).map(|v| v["items"].clone()).unwrap_or_default();
    state["branches"] = branches(root).map(|v| Value::Array(v["items"].as_array().cloned().unwrap_or_default().into_iter().take(12).collect())).unwrap_or_default();
    state["worktrees"] = worktree_list(root).map(|v| v["items"].clone()).unwrap_or_default();
    Ok(state)
}
fn background(answer: Answer, work: impl FnOnce() -> anyhow::Result<Value> + Send + 'static) {
    nyx_core::cmd::background(work, move |r| answer(r.map_err(|e| failed(format!("{e:#}")))));
}
/// Run a mutation through the window when it shows the repository (so it stays in step),
/// else directly through the core, which journals it.
fn mutate(root: PathBuf, operation: Operation, done: String, answer: Answer) {
    if let Some(ui) = window::current()
        && ui.root().as_ref() == Some(&root)
    {
        return ui.execute_at_then(root, operation, move |r| {
            answer(r.map(|_| json!({"message": done})).map_err(failed));
            publish();
        });
    }
    nyx_core::cmd::background(
        move || operations::run(&root, operation).map(|_| ()),
        move |r| {
            answer(r.map(|_| json!({"message": done})).map_err(|e| failed(format!("{e:#}"))));
            publish();
        },
    );
}
fn handle(app: &gtk::Application, action: &str, args: &Value, answer: Answer) {
    let path = args["path"].as_str();
    if action == "repo.repos" {
        let prefix = args["prefix"].as_str().unwrap_or_default().to_lowercase();
        let p = prefs::get();
        let mut paths: Vec<PathBuf> = p.workspace.open.clone();
        paths.extend(p.workspace.repositories.iter().filter(|r| r.favorite).map(|r| r.root.clone()));
        paths.extend(p.recent.clone());
        let mut seen = std::collections::HashSet::new();
        let items: Vec<Value> = paths
            .into_iter()
            .filter(|r| seen.insert(r.clone()) && r.is_dir())
            .filter(|r| r.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with(&prefix)))
            .map(|r| {
                let open = p.workspace.open.contains(&r);
                json!({"title": r.file_name().unwrap_or_default().to_string_lossy(), "value": r.to_string_lossy(), "path": r.to_string_lossy(),
                       "subtitle": format!("{}{}", nyx_core::paths::pretty(&r), if open { " · open" } else { "" })})
            })
            .collect();
        return answer(Ok(json!({"items": items})));
    }
    let root = match resolve(path) {
        Ok(r) => r,
        Err(e) => return answer(Err(e)),
    };
    match action {
        "repo.status" => background(answer, move || status(&root)),
        "repo.changes" => background(answer, move || changes(&root)),
        "repo.branches" => background(answer, move || branches(&root)),
        "repo.worktrees" => background(answer, move || worktree_list(&root)),
        "repo.history" => {
            let count = args["count"].as_u64().unwrap_or(20).clamp(1, 200);
            background(answer, move || history(&root, count))
        }
        "repo.flow" => background(answer, move || flow_summary(&root)),
        "repo.reviews" => background(answer, move || reviews(&root)),
        "repo.open" => {
            let page = args["page"].as_str().map(String::from);
            let ui = window::present(app);
            ui.open(root);
            if let Some(page) = page {
                ui.show_view(&page);
            }
            answer(Ok(Value::Null))
        }
        "repo.fetch" => mutate(root, Operation::Fetch, "Fetched all remotes".into(), answer),
        "repo.switch_branch" => {
            let Some(branch) = args["branch"].as_str().map(str::trim).filter(|b| !b.is_empty()) else { return answer(Err(invalid("Name a branch"))) };
            mutate(root, Operation::SwitchBranch(branch.into()), format!("Switched to {branch}"), answer)
        }
        "repo.branch_create" => {
            let Some(name) = args["name"].as_str().map(str::trim).filter(|b| !b.is_empty()) else { return answer(Err(invalid("Name the branch"))) };
            if let Err(e) = nyx_git::core::advanced::valid_ref("refs/heads/", name) {
                return answer(Err(invalid(format!("{e:#}"))));
            }
            mutate(root, Operation::CreateBranch(name.into()), format!("Created {name} and switched to it"), answer)
        }
        "repo.worktree_create" => {
            let Some(branch) = args["branch"].as_str().map(|b| b.trim().to_string()).filter(|b| !b.is_empty()) else { return answer(Err(invalid("Name a branch"))) };
            nyx_core::cmd::background(
                move || -> anyhow::Result<(PathBuf, Operation, PathBuf)> {
                    nyx_git::core::advanced::valid_ref("refs/heads/", &branch)?;
                    let main = worktrees::list(&root)?.into_iter().find(|w| w.main).map(|w| w.path).unwrap_or_else(|| root.clone());
                    let slug: String = branch.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
                    let name = main.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let destination = main.parent().unwrap_or(&main).join(format!("{name}-{slug}"));
                    let existing = nyx_git::core::advanced::resolve_target(&root, &format!("refs/heads/{branch}")).ok();
                    let base = match &existing {
                        Some(tip) => tip.clone(),
                        None => nyx_git::core::advanced::resolve_target(&root, "HEAD")?,
                    };
                    let action = worktrees::Action::Create { path: destination.clone(), branch, new_branch: existing.is_none(), base };
                    Ok((root, Operation::Worktree(action), destination))
                },
                move |r| match r {
                    Ok((root, op, destination)) => mutate(root, op, format!("Created a worktree in {}", destination.display()), answer),
                    Err(e) => answer(Err(failed(format!("{e:#}")))),
                },
            );
        }
        other => answer(Err(Error::new(ErrorKind::UnknownAction, format!("unknown action {other}")))),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_description_is_api_2() {
        let d = super::description("io.github.design_nexus.nyx.Git");
        nyx_actions::Description::from_value(&d, Some("io.github.design_nexus.nyx.Git")).expect("a valid API 2 description");
        let tools = d["tools"].as_array().unwrap();
        assert!(tools.iter().all(|t| t["name"].as_str().unwrap().starts_with("repo.")));
        // Nothing offered here deletes, discards or rewrites.
        assert!(tools.iter().all(|t| !matches!(t["x-nyx"]["risk"].as_str(), Some("destructive" | "privileged"))));
        assert_eq!(d["nyx"]["aliases"][0], "git");
    }
}
