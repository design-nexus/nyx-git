//! Review: the open repository's pull or merge requests on its hosting provider, with checks,
//! reviews, commits and files, and merging where the provider allows it. Nothing is fetched
//! until the page is shown or refreshed; without a connected account it says how to connect.
use super::window::current;
use gtk::prelude::*;
use nyx_git::providers::{
    Account, Client, Repository, Request,
    credentials,
    review::{self, Check, CheckState, FileChange, MergeMethod, RequestCommit, Review as Approval},
};
use nyx_ui::widgets;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

struct Data {
    account: Account,
    repo: Repository,
    requests: Vec<(Request, Option<CheckState>)>,
}
struct Details {
    number: u64,
    checks: Vec<Check>,
    files: Vec<FileChange>,
    commits: Vec<RequestCommit>,
    reviews: Vec<Approval>,
    errors: Vec<String>,
}
pub struct ReviewPage {
    pub root: gtk::Box,
    heading: gtk::Label,
    list: gtk::ListBox,
    detail: super::inspector::Inspector,
    stack: gtk::Stack,
    empty: gtk::Box,
    all: Rc<Cell<bool>>,
    data: RefCell<Option<Data>>,
    selected: Cell<Option<u64>>,
    loaded: RefCell<Option<PathBuf>>,
    generation: Cell<u64>,
}
fn page(run: impl FnOnce(&ReviewPage)) {
    if let Some(ui) = current() {
        run(&ui.destinations.review);
    }
}
fn client(account: &Account) -> anyhow::Result<Client> {
    Client::new(account.clone(), credentials::load(account)?)
}
fn badge(state: Option<CheckState>) -> String {
    state.map_or("no checks".into(), |s| format!("checks {}", s.word()))
}
impl ReviewPage {
    pub fn new() -> Rc<Self> {
        let root = widgets::vbox(0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        let bar = super::pagebar::PageBar::new();
        let all = Rc::new(Cell::new(false));
        let a = all.clone();
        let states = widgets::segmented(&widgets::opts(&[("open", "Open"), ("all", "All")]), "open", move |id| {
            a.set(id == "all");
            page(|p| p.load(true));
        });
        states.add_css_class("git-segments");
        bar.start(&states);
        let heading = widgets::label("", "git-bar-note");
        heading.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        bar.end(&heading);
        let create = super::pagebar::action("git-plus-symbolic", "New request…", "Open a pull or merge request from a branch", || page(|p| {
            if let Some(d) = p.data.borrow().as_ref() {
                super::providers::new_request(d.account.clone(), d.repo.clone());
            }
        }));
        create.add_css_class("git-bordered");
        bar.end(&create);
        let accounts = widgets::bar_button("git-users-symbolic", "Hosting accounts…");
        accounts.connect_clicked(|_| super::providers::show());
        bar.end(&accounts);
        let refresh = widgets::bar_button("git-refresh-symbolic", "Read requests and checks again");
        refresh.connect_clicked(|_| page(|p| p.load(true)));
        bar.end(&refresh);
        root.append(&bar.root);
        let list = gtk::ListBox::new();
        list.add_css_class("git-palette");
        list.add_css_class("git-requests");
        list.update_property(&[gtk::accessible::Property::Label("Pull and merge requests")]);
        list.connect_row_selected(|_, row| {
            let Some(n) = row.and_then(|r| r.widget_name().parse::<u64>().ok()) else { return };
            page(|p| p.select(n));
        });
        let list_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .min_content_width(260)
            .child(&list)
            .build();
        let detail = super::inspector::Inspector::new();
        detail.root.add_css_class("git-request");
        let detail_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .hexpand(true)
            .child(&detail.root)
            .build();
        let body = gtk::Paned::new(gtk::Orientation::Horizontal);
        body.set_start_child(Some(&list_scroll));
        body.set_end_child(Some(&detail_scroll));
        body.set_resize_start_child(false);
        body.set_shrink_start_child(false);
        body.set_shrink_end_child(false);
        super::layout::persist(&body, "review.list", 340, false);
        let empty = widgets::vbox(8);
        empty.add_css_class("empty-state");
        empty.set_valign(gtk::Align::Center);
        empty.set_halign(gtk::Align::Center);
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.add_named(&body, Some("requests"));
        stack.add_named(&empty, Some("empty"));
        root.append(&stack);
        Rc::new(Self {
            root,
            heading,
            list,
            detail,
            stack,
            empty,
            all,
            data: RefCell::new(None),
            selected: Cell::new(None),
            loaded: RefCell::new(None),
            generation: Cell::new(0),
        })
    }
    fn show_empty(&self, title: &str, text: &str, connect: bool) {
        while let Some(c) = self.empty.first_child() {
            self.empty.remove(&c);
        }
        let icon = gtk::Image::from_icon_name("git-review-symbolic");
        icon.set_pixel_size(40);
        icon.add_css_class("dim");
        self.empty.append(&icon);
        let t = widgets::label(title, "empty-title");
        t.set_xalign(0.5);
        self.empty.append(&t);
        let l = widgets::label(text, "dim-label");
        l.set_wrap(true);
        l.set_max_width_chars(56);
        l.set_justify(gtk::Justification::Center);
        l.set_xalign(0.5);
        self.empty.append(&l);
        if connect {
            let b = gtk::Button::with_label("Connect an account…");
            b.add_css_class("suggested-action");
            b.set_margin_top(8);
            b.set_halign(gtk::Align::Center);
            b.connect_clicked(|_| super::providers::show());
            self.empty.append(&b);
        }
        self.stack.set_visible_child_name("empty");
    }
    /// Shown, or the repository changed: read its requests (again with `force`).
    pub fn load(&self, force: bool) {
        let Some(root) = current().and_then(|ui| ui.root()) else {
            self.show_empty("No repository open", "Open a repository to review its pull or merge requests.", false);
            return;
        };
        if !force && self.loaded.borrow().as_ref() == Some(&root) {
            return;
        }
        *self.loaded.borrow_mut() = Some(root.clone());
        let accounts = crate::prefs::get().accounts;
        if accounts.is_empty() {
            self.show_empty(
                "No hosting account",
                "Connect GitHub, GitLab, Forgejo or Gitea to see this repository's requests, checks and reviews. Local Git works without one.",
                true,
            );
            return;
        }
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.heading.set_text("Reading requests…");
        let all = self.all.get();
        nyx_core::cmd::background(
            move || -> anyhow::Result<Option<Data>> {
                let snapshot = nyx_git::core::remotes::read(&root)?;
                let remotes: Vec<(String, String)> =
                    snapshot.remotes.iter().flat_map(|r| r.fetch.iter().map(|u| (r.name.clone(), u.clone()))).collect();
                let Some((account, path)) = review::locate(&accounts, &remotes) else { return Ok(None) };
                let c = client(&account)?;
                let repo = c.find_repository(&path)?;
                let requests = c.requests(&repo, 1, all)?.items;
                // Checks for the first requests, so the list and Flow can show them.
                let requests = requests
                    .into_iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let checks = (i < 20 && !r.head.is_empty()).then(|| c.checks(&repo, &r.head).ok()).flatten();
                        let state = checks.as_deref().and_then(review::summary);
                        (r, state)
                    })
                    .collect();
                Ok(Some(Data { account, repo, requests }))
            },
            move |result| {
                page(|p| {
                    if p.generation.get() != generation {
                        return;
                    }
                    match result {
                        Ok(Some(data)) => p.fill(data),
                        Ok(None) => p.show_empty(
                            "Not on a connected host",
                            "None of this repository's remotes is on a connected hosting account. Connect the host its origin points to.",
                            true,
                        ),
                        Err(e) => p.show_empty("Requests couldn't be read", &format!("{e:#}"), true),
                    }
                })
            },
        );
    }
    fn fill(&self, data: Data) {
        self.heading.set_text(&format!("{} · {} on {}", data.repo.name, data.requests.len(), data.account.kind.name()));
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        if data.requests.is_empty() {
            *self.data.borrow_mut() = Some(data);
            self.show_empty("No requests", if self.all.get() { "This repository has no pull or merge requests." } else { "No open pull or merge requests. Choose All to see closed and merged ones." }, false);
            self.publish_links();
            return;
        }
        self.stack.set_visible_child_name("requests");
        for (r, state) in &data.requests {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&r.number.to_string());
            let line = widgets::hbox(10);
            let n = widgets::label(&format!("#{}", r.number), "git-request-number");
            n.set_valign(gtk::Align::Start);
            line.append(&n);
            let b = widgets::vbox(4);
            b.set_hexpand(true);
            let t = widgets::label(&r.title, "git-request-title");
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            b.append(&t);
            let route = widgets::hbox(6);
            route.add_css_class("git-request-meta");
            route.append(&gtk::Image::from_icon_name("git-branch-symbolic"));
            let way = widgets::label(&format!("{} → {}", r.source_branch, r.target), "");
            way.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            route.append(&way);
            b.append(&route);
            let chips = widgets::hbox(6);
            chips.append(&pill(&badge(*state), match state {
                Some(CheckState::Failure) => "bad",
                Some(CheckState::Pending) => "warn",
                Some(_) => "ok",
                None => "",
            }));
            if r.draft {
                chips.append(&pill("draft", ""));
            }
            if r.state != "open" && r.state != "opened" {
                chips.append(&pill(&r.state, "acc"));
            }
            b.append(&chips);
            line.append(&b);
            row.set_child(Some(&line));
            self.list.append(&row);
        }
        let keep = self.selected.get().filter(|n| data.requests.iter().any(|(r, _)| r.number == *n));
        *self.data.borrow_mut() = Some(data);
        self.publish_links();
        let index = keep
            .and_then(|n| self.data.borrow().as_ref().and_then(|d| d.requests.iter().position(|(r, _)| r.number == n)))
            .unwrap_or(0);
        if let Some(row) = self.list.row_at_index(index as i32) {
            self.list.select_row(Some(&row));
        }
    }
    /// Requests by branch, for Flow's cards.
    fn publish_links(&self) {
        let Some(ui) = current() else { return };
        let links = self
            .data
            .borrow()
            .as_ref()
            .map(|d| {
                d.requests
                    .iter()
                    .filter(|(r, _)| r.state == "open" || r.state == "opened")
                    .map(|(r, state)| super::flow::Link {
                        number: r.number,
                        title: r.title.clone(),
                        source: r.source_branch.clone(),
                        target: r.target.clone(),
                        state: format!("{}{} · {}", r.state, if r.draft { " draft" } else { "" }, badge(*state)),
                        url: r.web_url.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        ui.destinations.flow.set_links(links);
    }
    fn request(&self, n: u64) -> Option<(Account, Repository, Request)> {
        let d = self.data.borrow();
        let d = d.as_ref()?;
        let r = d.requests.iter().find(|(r, _)| r.number == n)?.0.clone();
        Some((d.account.clone(), d.repo.clone(), r))
    }
    fn select(&self, n: u64) {
        self.selected.set(Some(n));
        let Some((account, repo, request)) = self.request(n) else { return };
        self.render(&request, None);
        nyx_core::cmd::background(
            move || {
                let mut errors = Vec::new();
                let c = match client(&account) {
                    Ok(c) => c,
                    Err(e) => return Details { number: n, checks: vec![], files: vec![], commits: vec![], reviews: vec![], errors: vec![format!("{e:#}")] },
                };
                fn take<T>(errors: &mut Vec<String>, what: &str, r: anyhow::Result<Vec<T>>) -> Vec<T> {
                    r.unwrap_or_else(|e| {
                        errors.push(format!("{what}: {e:#}"));
                        Vec::new()
                    })
                }
                let checks = if request.head.is_empty() { Vec::new() } else { take(&mut errors, "Checks", c.checks(&repo, &request.head)) };
                let files = take(&mut errors, "Files", c.request_files(&repo, n));
                let commits = take(&mut errors, "Commits", c.request_commits(&repo, n));
                let reviews = take(&mut errors, "Reviews", c.reviews(&repo, n));
                Details { number: n, checks, files, commits, reviews, errors }
            },
            move |details| {
                page(|p| {
                    if p.selected.get() != Some(details.number) {
                        return;
                    }
                    if let Some((_, _, request)) = p.request(details.number) {
                        p.render(&request, Some(details));
                    }
                })
            },
        );
    }
    fn render(&self, r: &Request, details: Option<Details>) {
        let insp = &self.detail;
        insp.clear();
        let Some((account, repo, _)) = self.request(r.number) else { return };
        let caps = account.kind.capabilities();
        let open_state = r.state == "open" || r.state == "opened";
        let top = widgets::hbox(8);
        top.append(&widgets::label(&format!("#{}", r.number), "git-request-number"));
        top.append(&pill(if r.draft { "Draft" } else { &r.state }, if open_state { "ok" } else { "acc" }));
        let route = widgets::label(&format!("{} → {}", r.source_branch, r.target), "git-insp-note");
        route.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        top.append(&route);
        insp.append(&top);
        let title = insp.title(None, &r.title);
        title.add_css_class("git-request-heading");
        if !r.status.is_empty() {
            insp.sub(&r.status);
        }
        // The local side: is the source branch here, and where is it?
        let local = current().and_then(|ui| ui.destinations.flow_branch(&r.source_branch));
        let actions = widgets::hbox(6);
        actions.add_css_class("git-request-actions");
        if caps.merge && open_state && !r.head.is_empty() {
            let merge = widgets::labeled_button("git-merge-symbolic", "Merge…");
            merge.add_css_class("suggested-action");
            let (a, rp, rq) = (account.clone(), repo.clone(), r.clone());
            let failing = details.as_ref().and_then(|d| review::summary(&d.checks)) == Some(CheckState::Failure);
            merge.connect_clicked(move |_| merge_dialog(a.clone(), rp.clone(), rq.clone(), failing));
            actions.append(&merge);
        }
        if local.is_none() && !r.head.is_empty() {
            let checkout = widgets::labeled_button("git-download-symbolic", "Check out for review…");
            let (a, rp, rq) = (account.clone(), repo.clone(), r.clone());
            checkout.connect_clicked(move |_| super::providers::checkout_request(a.clone(), rp.clone(), rq.clone()));
            actions.append(&checkout);
        } else if local.is_some() {
            let show = widgets::labeled_button("git-flow-symbolic", "Show in Flow");
            let branch = r.source_branch.clone();
            show.connect_clicked(move |_| {
                if let Some(ui) = current() {
                    ui.show_view("flow");
                    ui.destinations.flow.focus_branch(&branch);
                }
            });
            actions.append(&show);
        }
        let open = widgets::labeled_button("git-open-symbolic", &format!("Open on {}", account.kind.name()));
        open.add_css_class("git-bar-action");
        let (a, url) = (account.clone(), r.web_url.clone());
        open.connect_clicked(move |_| super::providers::link(&a, &url));
        actions.append(&open);
        insp.append(&actions);
        insp.group("Here");
        match &local {
            Some((tip, worktree)) => {
                insp.kv("Local branch", &r.source_branch, "");
                if let Some(w) = worktree {
                    insp.kv("Worktree", &nyx_core::paths::pretty(w), "");
                }
                if *tip == r.head {
                    insp.kv("Tip", "Same commit as the request", "ok");
                } else {
                    insp.kv("Tip", &format!("{} here, {} in the request", &tip[..tip.len().min(7)], &r.head[..r.head.len().min(7)]), "warn");
                }
            }
            None => {
                insp.kv("Local branch", "None by this name", "");
            }
        }
        let Some(d) = details else {
            insp.note("Reading checks, reviews, commits and files…");
            return;
        };
        insp.group(&format!("Checks  {}", d.checks.len()));
        if d.checks.is_empty() {
            insp.note("No checks reported for the head commit.");
        }
        let mut checks = d.checks.clone();
        checks.sort_by_key(|c| c.state);
        for c in &checks {
            let tone = match c.state {
                CheckState::Failure => "bad",
                CheckState::Pending => "warn",
                _ => "ok",
            };
            insp.kv(&c.name, c.state.word(), tone);
        }
        insp.group("Reviews");
        if d.reviews.is_empty() {
            insp.note(if account.kind == nyx_git::providers::Kind::Gitlab { "No approvals yet. Discussions are on the web." } else { "No reviews yet." });
        }
        for rv in &d.reviews {
            let state = rv.state.to_lowercase().replace('_', " ");
            let tone = if state.contains("approv") { "ok" } else if state.contains("change") { "warn" } else { "" };
            insp.kv(&rv.author, &state, tone);
            if !rv.body.is_empty() {
                insp.note(&rv.body);
            }
        }
        insp.group(&format!("Commits  {}", d.commits.len()));
        for c in d.commits.iter().take(50) {
            let row = widgets::hbox(10);
            row.add_css_class("git-insp-kv");
            row.append(&widgets::label(&c.id[..c.id.len().min(7)], "git-hash"));
            let subject = widgets::label(&c.subject, "");
            subject.set_ellipsize(gtk::pango::EllipsizeMode::End);
            subject.set_hexpand(true);
            subject.set_tooltip_text(Some(&format!("{} · {}", c.subject, c.author)));
            row.append(&subject);
            insp.append(&row);
        }
        insp.group(&format!("Files  {}", d.files.len()));
        for f in d.files.iter().take(200) {
            let row = widgets::hbox(10);
            row.add_css_class("git-file-row");
            let code = match f.status.as_str() {
                "added" | "new" => 'A',
                "removed" | "deleted" => 'D',
                "renamed" => 'R',
                _ => 'M',
            };
            let chip = widgets::label(&code.to_string(), "git-chip");
            chip.set_xalign(0.5);
            chip.add_css_class(match code {
                'A' => "added",
                'D' => "deleted",
                'R' => "renamed",
                _ => "modified",
            });
            row.append(&chip);
            let name = widgets::label(&f.path, "git-file-name");
            name.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            name.set_hexpand(true);
            row.append(&name);
            if let (Some(a), Some(r)) = (f.additions, f.deletions) {
                row.append(&widgets::label(&format!("+{a} −{r}"), "git-activity-time"));
            }
            insp.append(&row);
        }
        if !caps.line_counts {
            insp.note(&format!("{} doesn't report line counts here.", account.kind.name()));
        }
        for e in &d.errors {
            insp.text(e).add_css_class("bad");
        }
    }
}
/// A state word in a rounded pill; `tone` is "ok", "warn", "bad", "acc" or "".
fn pill(text: &str, tone: &str) -> gtk::Label {
    let l = widgets::label(text, "git-pill");
    if !tone.is_empty() {
        l.add_css_class(tone);
    }
    l.set_valign(gtk::Align::Center);
    l
}
fn merge_dialog(account: Account, repo: Repository, request: Request, failing: bool) {
    let caps = account.kind.capabilities();
    let (dialog, card) = widgets::dialog(&format!("Merge #{}?", request.number), 520);
    let text = format!(
        "Merge {} into {} on {}, only if the request still points to {}. Fetch afterwards to bring the result here.{}",
        request.source_branch,
        request.target,
        account.kind.name(),
        &request.head[..request.head.len().min(10)],
        if failing { "\n\nSome checks are failing." } else { "" }
    );
    let l = widgets::label(&text, "dim-label");
    l.set_wrap(true);
    card.append(&l);
    let mut options = vec![("merge", "Merge commit")];
    if caps.squash {
        options.push(("squash", "Squash"));
    }
    if caps.rebase {
        options.push(("rebase", "Rebase"));
    }
    let method = Rc::new(Cell::new(MergeMethod::Merge));
    if options.len() > 1 {
        let m = method.clone();
        card.append(&widgets::segmented(&widgets::opts(&options), "merge", move |id| {
            m.set(match id.as_str() {
                "squash" => MergeMethod::Squash,
                "rebase" => MergeMethod::Rebase,
                _ => MergeMethod::Merge,
            })
        }));
    } else {
        card.append(&widgets::label(&format!("{} merges with the method set for the project.", account.kind.name()), "dim-label"));
    }
    let row = widgets::hbox(8);
    row.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    row.append(&cancel);
    let d = dialog.clone();
    let go = widgets::two_click("Merge", "Confirm: merge on the server", move || {
        d.close();
        let (a, r, q, m) = (account.clone(), repo.clone(), request.clone(), method.get());
        nyx_core::cmd::background(
            move || client(&a).and_then(|c| c.merge(&r, q.number, &q.head, m)),
            |result: anyhow::Result<()>| {
                let Some(ui) = current() else { return };
                match result {
                    Ok(()) => {
                        ui.toast("Merged on the server. Fetch to bring it here.");
                        page(|p| p.load(true));
                    }
                    Err(e) => ui.error(&format!("{e:#}")),
                }
            },
        );
    });
    row.append(&go);
    card.append(&row);
    dialog.present();
    cancel.grab_focus();
}
