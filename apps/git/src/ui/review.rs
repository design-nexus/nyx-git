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
    detail: gtk::Box,
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
        let bar = widgets::hbox(6);
        bar.add_css_class("git-history-search");
        let all = Rc::new(Cell::new(false));
        let a = all.clone();
        let states = widgets::segmented(&widgets::opts(&[("open", "Open"), ("all", "All")]), "open", move |id| {
            a.set(id == "all");
            page(|p| p.load(true));
        });
        bar.append(&states);
        let heading = widgets::label("", "dim-label");
        heading.set_hexpand(true);
        heading.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        bar.append(&heading);
        let create = gtk::Button::with_label("New request…");
        create.connect_clicked(|_| page(|p| {
            if let Some(d) = p.data.borrow().as_ref() {
                super::providers::new_request(d.account.clone(), d.repo.clone());
            }
        }));
        bar.append(&create);
        let accounts = widgets::bar_button("system-users-symbolic", "Hosting accounts…");
        accounts.connect_clicked(|_| super::providers::show());
        bar.append(&accounts);
        let refresh = widgets::bar_button("view-refresh-symbolic", "Read requests and checks again");
        refresh.connect_clicked(|_| page(|p| p.load(true)));
        bar.append(&refresh);
        root.append(&bar);
        let list = gtk::ListBox::new();
        list.add_css_class("git-palette");
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
        let detail = widgets::vbox(8);
        detail.add_css_class("git-details");
        let detail_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(super::scrollbars::policy())
            .hexpand(true)
            .child(&detail)
            .build();
        let body = gtk::Paned::new(gtk::Orientation::Horizontal);
        body.set_start_child(Some(&list_scroll));
        body.set_end_child(Some(&detail_scroll));
        body.set_resize_start_child(false);
        body.set_shrink_start_child(false);
        body.set_shrink_end_child(false);
        super::layout::persist(&body, "review.list", 340, false);
        let empty = widgets::vbox(12);
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
        self.empty.append(&widgets::label(title, "section-title"));
        let l = widgets::label(text, "dim-label");
        l.set_wrap(true);
        l.set_max_width_chars(60);
        l.set_justify(gtk::Justification::Center);
        self.empty.append(&l);
        if connect {
            let b = gtk::Button::with_label("Hosting accounts…");
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
            let b = widgets::vbox(2);
            let t = widgets::label(&format!("#{} {}", r.number, r.title), "");
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            b.append(&t);
            let d = widgets::label(
                &format!("{} → {} · {}{} · {}", r.source_branch, r.target, r.state, if r.draft { " · draft" } else { "" }, badge(*state)),
                if *state == Some(CheckState::Failure) { "git-error" } else { "dim-label" },
            );
            d.add_css_class("git-palette-detail");
            d.set_ellipsize(gtk::pango::EllipsizeMode::End);
            b.append(&d);
            row.set_child(Some(&b));
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
        while let Some(c) = self.detail.first_child() {
            self.detail.remove(&c);
        }
        let add = |text: &str, class: &str| {
            let l = widgets::label(text, class);
            l.set_wrap(true);
            l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            l.set_selectable(true);
            self.detail.append(&l);
        };
        let Some((account, repo, _)) = self.request(r.number) else { return };
        let caps = account.kind.capabilities();
        add(&format!("#{} {}", r.number, r.title), "section-title");
        add(&format!("{} → {} · {}{} · {}", r.source_branch, r.target, r.state, if r.draft { " · draft" } else { "" }, r.status), "dim-label");
        // The local side: is the source branch here, and where is it?
        let local = current().and_then(|ui| ui.destinations.flow_branch(&r.source_branch));
        match &local {
            Some((tip, worktree)) => add(
                &format!(
                    "Local branch {}{} · {}",
                    r.source_branch,
                    worktree.as_ref().map(|w| format!(" in {}", w.display())).unwrap_or_default(),
                    if *tip == r.head { "same commit as the request".to_string() } else { format!("at {}, the request is at {}", &tip[..tip.len().min(8)], &r.head[..r.head.len().min(8)]) }
                ),
                "",
            ),
            None => add("No local branch by this name.", "dim-label"),
        }
        let actions = gtk::FlowBox::new();
        actions.set_selection_mode(gtk::SelectionMode::None);
        actions.set_max_children_per_line(4);
        actions.set_column_spacing(6);
        actions.set_row_spacing(6);
        let open = gtk::Button::with_label("Open on the web");
        let (a, url) = (account.clone(), r.web_url.clone());
        open.connect_clicked(move |_| super::providers::link(&a, &url));
        actions.insert(&open, -1);
        if local.is_none() && !r.head.is_empty() {
            let checkout = gtk::Button::with_label("Check out for review…");
            let (a, rp, rq) = (account.clone(), repo.clone(), r.clone());
            checkout.connect_clicked(move |_| super::providers::checkout_request(a.clone(), rp.clone(), rq.clone()));
            actions.insert(&checkout, -1);
        } else if local.is_some() {
            let show = gtk::Button::with_label("Show in Flow");
            let branch = r.source_branch.clone();
            show.connect_clicked(move |_| {
                if let Some(ui) = current() {
                    ui.show_view("flow");
                    ui.destinations.flow.focus_branch(&branch);
                }
            });
            actions.insert(&show, -1);
        }
        let open_state = r.state == "open" || r.state == "opened";
        if caps.merge && open_state && !r.head.is_empty() {
            let merge = gtk::Button::with_label("Merge…");
            let (a, rp, rq) = (account.clone(), repo.clone(), r.clone());
            let failing = details.as_ref().and_then(|d| review::summary(&d.checks)) == Some(CheckState::Failure);
            merge.connect_clicked(move |_| merge_dialog(a.clone(), rp.clone(), rq.clone(), failing));
            actions.insert(&merge, -1);
        }
        self.detail.append(&actions);
        let Some(d) = details else {
            add("Reading checks, reviews, commits and files…", "dim-label");
            return;
        };
        add("CHECKS", "group-title");
        if d.checks.is_empty() {
            add("No checks reported for the head commit.", "dim-label");
        }
        let mut checks = d.checks.clone();
        checks.sort_by_key(|c| c.state);
        for c in &checks {
            let class = match c.state {
                CheckState::Failure => "git-error",
                CheckState::Pending => "git-warning",
                _ => "",
            };
            add(&format!("{} · {}", c.name, c.state.word()), class);
        }
        add("REVIEWS", "group-title");
        if d.reviews.is_empty() {
            add(if account.kind == nyx_git::providers::Kind::Gitlab { "No approvals yet. Discussions are on the web." } else { "No reviews yet." }, "dim-label");
        }
        for rv in &d.reviews {
            add(&format!("{} · {}", rv.author, rv.state.to_lowercase().replace('_', " ")), "");
            if !rv.body.is_empty() {
                add(&rv.body, "dim-label");
            }
        }
        add(&format!("COMMITS · {}", d.commits.len()), "group-title");
        for c in d.commits.iter().take(50) {
            add(&format!("{} {} · {}", &c.id[..c.id.len().min(8)], c.subject, c.author), "");
        }
        add(&format!("FILES · {}", d.files.len()), "group-title");
        for f in d.files.iter().take(200) {
            let counts = match (f.additions, f.deletions) {
                (Some(a), Some(r)) => format!(" +{a} −{r}"),
                _ => String::new(),
            };
            add(&format!("{} · {}{counts}", f.path, f.status), "dim-label");
        }
        if !caps.line_counts {
            add(&format!("{} doesn't report line counts here.", account.kind.name()), "dim-label");
        }
        for e in &d.errors {
            add(e, "git-error");
        }
    }
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
