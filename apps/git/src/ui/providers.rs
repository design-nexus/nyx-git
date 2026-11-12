//! Optional hosting dialogs reuse Nyx cards and background jobs.
use super::{window::current, workflow::confirm};
use crate::prefs;
use gtk::{gio, prelude::*};
use nyx_git::{
    core::{
        advanced::Guard,
        hosting,
        operations::{self, Operation},
    },
    providers::{
        Account, Client, CredentialSource, Kind, Repository, Request,
        credentials::{self, Secret},
    },
};
use nyx_ui::widgets;
use std::{path::PathBuf, rc::Rc};
fn error(e: impl std::fmt::Display) {
    if let Some(ui) = current() {
        ui.toast(&e.to_string());
    }
}
/// A failure from Git or a provider: shown until dismissed.
fn fail(e: impl std::fmt::Display) {
    if let Some(ui) = current() {
        ui.error(&e.to_string());
    }
}
fn close(dialog: &gtk::Window, body: &gtk::Box) {
    let b = gtk::Button::with_label("Close");
    b.set_halign(gtk::Align::End);
    let d = dialog.clone();
    b.connect_clicked(move |_| d.close());
    body.append(&b);
}
fn scroll(rows: &gtk::Box) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(rows)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .min_content_height(250)
        .max_content_height(500)
        .propagate_natural_height(true)
        .build()
}
fn note(body: &gtk::Box, text: &str) -> gtk::Label {
    let label = widgets::label(text, "dim-label");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    body.append(&label);
    label
}
fn client_job<T: Send + 'static>(
    account: Account,
    work: impl FnOnce(Client) -> anyhow::Result<T> + Send + 'static,
    done: impl FnOnce(anyhow::Result<T>) + 'static,
) {
    nyx_core::cmd::background(move || Client::new(account.clone(), credentials::load(&account)?).and_then(work), done);
}
pub(super) fn link(account: &Account, value: &str) {
    match account_link(account, value) {
        Ok(url) => {
            if let Some(ui) = current() {
                gtk::UriLauncher::new(url.as_str()).launch(Some(&ui.window), gio::Cancellable::NONE, |result| {
                    if let Err(e) = result {
                        fail(e);
                    }
                });
            }
        }
        Err(e) => fail(format!("{e:#}")),
    }
}
fn account_link(account: &Account, value: &str) -> anyhow::Result<url::Url> {
    let url = url::Url::parse(value)?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.origin() == account.base()?.origin(),
        "The provider returned an unsafe browser link"
    );
    Ok(url)
}
pub fn show() {
    let (dialog, body) = widgets::dialog("Hosting accounts", 680);
    crate::dialog_dismissal::add(&dialog, &body, "Close");
    note(
        &body,
        "Connect only when you want hosting features. Local Git, SSH agents and Git credential helpers work independently of these accounts.",
    );
    let add = gtk::Button::with_label("Connect account…");
    add.set_halign(gtk::Align::Start);
    let d = dialog.clone();
    add.connect_clicked(move |_| {
        d.close();
        connect();
    });
    let top = widgets::hbox(6);
    top.append(&add);
    let clone = gtk::Button::with_label("Clone URL…");
    let d = dialog.clone();
    clone.connect_clicked(move |_| {
        d.close();
        clone_url();
    });
    top.append(&clone);
    body.append(&top);
    let rows = widgets::vbox(0);
    rows.add_css_class("group-list");
    let accounts = prefs::get().accounts;
    if accounts.is_empty() {
        note(&rows, "No hosting accounts connected.");
    }
    for account in accounts {
        let actions = widgets::hbox(6);
        let browse = gtk::Button::with_label("Repositories…");
        let a = account.clone();
        let d = dialog.clone();
        browse.connect_clicked(move |_| {
            d.close();
            repositories(a.clone(), 1);
        });
        actions.append(&browse);
        let remove = gtk::Button::with_label("Disconnect…");
        let a = account.clone();
        let d = dialog.clone();
        remove.connect_clicked(move |_|{d.close();let a=a.clone();confirm("Disconnect hosting account?","Remove this account from Git and delete its dedicated token from Secret Service, if any. Existing Git helper credentials and cloned repositories remain.","Disconnect",move||{let a=a.clone();nyx_core::cmd::background(move||credentials::delete(&a).map(|_|a.id),|result|match result{Ok(id)=>{prefs::update(|p|p.accounts.retain(|a|a.id!=id));show();},Err(e)=>fail(format!("{e:#}"))});});});
        actions.append(&remove);
        rows.append(&row(
            &format!("{} · {}", account.kind.name(), account.username),
            &gtk::glib::markup_escape_text(&account.base_url),
            Some(actions.upcast_ref()),
        ));
    }
    body.append(&scroll(&rows));
    close(&dialog, &body);
    dialog.present();
}
fn connect() {
    let (dialog, body) = widgets::dialog("Connect hosting account", 620);
    let kinds = [Kind::Github, Kind::Gitlab, Kind::Forgejo, Kind::Gitea];
    let kind = gtk::DropDown::from_strings(&kinds.map(Kind::name));
    body.append(&kind);
    let instance = gtk::Entry::new();
    instance.set_text(Kind::Github.default_url());
    instance.set_placeholder_text(Some("HTTPS instance URL · custom self-hosted URLs supported"));
    body.append(&instance);
    let source = gtk::DropDown::from_strings(&["Existing Git credential helper", "API token in Secret Service"]);
    body.append(&source);
    let username = gtk::Entry::new();
    username.set_placeholder_text(Some("Git helper username (optional)"));
    body.append(&username);
    let token = gtk::PasswordEntry::new();
    token.set_show_peek_icon(true);
    token.set_placeholder_text(Some("Personal access token · stored only in the desktop keyring"));
    token.set_visible(false);
    body.append(&token);
    let p = token.clone();
    let u = username.clone();
    source.connect_selected_notify(move |s| {
        p.set_visible(s.selected() == 1);
        u.set_visible(s.selected() == 0);
    });
    let i = instance.clone();
    kind.connect_selected_notify(move |k| {
        i.set_text(kinds.get(k.selected() as usize).copied().unwrap_or(Kind::Github).default_url())
    });
    let token_page = gtk::Button::with_label("Open token settings in browser");
    let i = instance.clone();
    let k = kind.clone();
    token_page.connect_clicked(move |_| {
        match Account::new(
            kinds.get(k.selected() as usize).copied().unwrap_or(Kind::Github),
            &i.text(),
            CredentialSource::SecretService,
        )
        .and_then(|a| a.token_page().map(|u| (a, u)))
        {
            Ok((a, url)) => link(&a, url.as_str()),
            Err(e) => fail(format!("{e:#}")),
        }
    });
    body.append(&token_page);
    note(
        &body,
        "Use an API token with access to the repositories you need. Reading requests needs read access; publishing requests needs write/API permissions. GitHub organization tokens may also need organization authorization. Private Git clone/fetch continues to use your native Git credentials or SSH setup.",
    );
    let status = note(&body, "");
    status.add_css_class("git-error");
    let buttons = widgets::hbox(6);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    buttons.append(&cancel);
    let submit = gtk::Button::with_label("Verify and connect");
    submit.add_css_class("suggested-action");
    let d = dialog.clone();
    submit.connect_clicked(move |button| {
        let provider = kinds.get(kind.selected() as usize).copied().unwrap_or(Kind::Github);
        let source = if source.selected() == 0 { CredentialSource::GitHelper } else { CredentialSource::SecretService };
        let result = Account::new(provider, &instance.text(), source).and_then(|mut a| {
            a.credential_username = username.text().trim().into();
            let token =
                if source == CredentialSource::SecretService { Some(Secret::new(token.text().to_string())?) } else { None };
            Ok((a, token))
        });
        let (mut account, secret) = match result {
            Ok(v) => v,
            Err(e) => {
                status.set_text(&format!("{e:#}"));
                return;
            }
        };
        token.set_text("");
        button.set_sensitive(false);
        status.set_text("Verifying account…");
        let b = button.clone();
        let status = status.clone();
        let d = d.clone();
        nyx_core::cmd::background(
            move || -> anyhow::Result<Account> {
                let token = match secret {
                    Some(s) => s,
                    None => credentials::load(&account)?,
                };
                let client = Client::new(account.clone(), token)?;
                account.username = client.verify()?;
                if source == CredentialSource::SecretService {
                    client.save_token()?;
                }
                Ok(account)
            },
            move |result| {
                b.set_sensitive(true);
                match result {
                    Ok(account) => {
                        prefs::update(|p| p.accounts.push(account));
                        d.close();
                        show();
                    }
                    Err(e) => status.set_text(&format!("{e:#}")),
                }
            },
        );
    });
    buttons.append(&submit);
    body.append(&buttons);
    dialog.present();
}
fn repositories(account: Account, page: u32) {
    let a = account.clone();
    client_job(
        account,
        move |c| c.repositories(page),
        move |result| match result {
            Ok(result) => {
                let (dialog, body) = widgets::dialog(&format!("{} repositories · page {page}", a.kind.name()), 760);
                crate::dialog_dismissal::add(&dialog, &body, "Close");
                note(&body, &format!("{} · {}", a.username, a.base_url));
                let search = gtk::SearchEntry::new();
                search.set_placeholder_text(Some("Filter this page of repositories"));
                body.append(&search);
                let rows = widgets::vbox(0);
                rows.add_css_class("group-list");
                let mut filters = Vec::new();
                if result.items.is_empty() {
                    note(&rows, "No repositories on this page. Check account permissions if a repository is missing.");
                }
                for repo in result.items {
                    let actions = gtk::FlowBox::new();
                    actions.set_selection_mode(gtk::SelectionMode::None);
                    actions.set_max_children_per_line(2);
                    for (title, kind) in [("Open", "web"), ("Clone…", "clone"), ("Requests…", "requests"), ("Issues…", "issues")]
                    {
                        let b = gtk::Button::with_label(title);
                        let account = a.clone();
                        let repo = repo.clone();
                        let d = dialog.clone();
                        b.connect_clicked(move |_| match kind {
                            "web" => link(&account, &repo.web_url),
                            "clone" => {
                                d.close();
                                clone_dialog(repo.clone());
                            }
                            "requests" => {
                                d.close();
                                requests(account.clone(), repo.clone(), 1, false);
                            }
                            _ => {
                                d.close();
                                issues(account.clone(), repo.clone(), 1);
                            }
                        });
                        actions.insert(&b, -1);
                    }
                    let row = row(&repo.name, &gtk::glib::markup_escape_text(&repo.web_url), Some(actions.upcast_ref()));
                    filters.push((row.clone(), repo.name.to_lowercase()));
                    rows.append(&row);
                }
                search.connect_search_changed(move |s| {
                    let q = s.text().to_lowercase();
                    for (r, name) in &filters {
                        r.set_visible(name.contains(&q));
                    }
                });
                body.append(&scroll(&rows));
                let nav = widgets::hbox(6);
                if page > 1 {
                    let b = gtk::Button::with_label("Previous page");
                    let account = a.clone();
                    let d = dialog.clone();
                    b.connect_clicked(move |_| {
                        d.close();
                        repositories(account.clone(), page - 1);
                    });
                    nav.append(&b);
                }
                if let Some(next) = result.next {
                    let b = gtk::Button::with_label("Next page");
                    let account = a.clone();
                    let d = dialog.clone();
                    b.connect_clicked(move |_| {
                        d.close();
                        repositories(account.clone(), next);
                    });
                    nav.append(&b);
                }
                body.append(&nav);
                close(&dialog, &body);
                dialog.present();
            }
            Err(e) => fail(format!("{e:#}")),
        },
    );
}
fn requests(account: Account, repo: Repository, page: u32, all: bool) {
    let a = account.clone();
    let r = repo.clone();
    client_job(
        account,
        move |c| c.requests(&r, page, all),
        move |result| match result {
            Ok(result) => {
                let (dialog, body) = widgets::dialog(&format!("Requests · {}", repo.name), 760);
                crate::dialog_dismissal::add(&dialog, &body, "Close");
                let controls = widgets::hbox(6);
                let toggle = gtk::Button::with_label(if all { "Show open" } else { "Show all states" });
                let a2 = a.clone();
                let r = repo.clone();
                let d = dialog.clone();
                toggle.connect_clicked(move |_| {
                    d.close();
                    requests(a2.clone(), r.clone(), 1, !all);
                });
                controls.append(&toggle);
                let new = gtk::Button::with_label("New request…");
                let a2 = a.clone();
                let r = repo.clone();
                let d = dialog.clone();
                new.connect_clicked(move |_| {
                    d.close();
                    new_request(a2.clone(), r.clone());
                });
                controls.append(&new);
                body.append(&controls);
                let rows = widgets::vbox(0);
                rows.add_css_class("group-list");
                if result.items.is_empty() {
                    note(&rows, "No requests on this page.");
                }
                for request in result.items {
                    let actions = widgets::hbox(6);
                    let details = gtk::Button::with_label("Details…");
                    let a2 = a.clone();
                    let r = repo.clone();
                    let n = request.number;
                    let d = dialog.clone();
                    details.connect_clicked(move |_| {
                        d.close();
                        request_details(a2.clone(), r.clone(), n);
                    });
                    actions.append(&details);
                    let web = gtk::Button::with_label("Open");
                    let a2 = a.clone();
                    let url = request.web_url.clone();
                    web.connect_clicked(move |_| link(&a2, &url));
                    actions.append(&web);
                    rows.append(&row(
                        &format!("#{} {}", request.number, request.title),
                        &gtk::glib::markup_escape_text(&format!(
                            "{}{} · {} → {}",
                            request.state,
                            if request.draft { " · draft" } else { "" },
                            request.source,
                            request.target
                        )),
                        Some(actions.upcast_ref()),
                    ));
                }
                body.append(&scroll(&rows));
                let nav = widgets::hbox(6);
                for (title, p) in [("Previous page", page.checked_sub(1).filter(|p| *p > 0)), ("Next page", result.next)] {
                    if let Some(p) = p {
                        let b = gtk::Button::with_label(title);
                        let a = a.clone();
                        let r = repo.clone();
                        let d = dialog.clone();
                        b.connect_clicked(move |_| {
                            d.close();
                            requests(a.clone(), r.clone(), p, all);
                        });
                        nav.append(&b);
                    }
                }
                body.append(&nav);
                close(&dialog, &body);
                dialog.present();
            }
            Err(e) => fail(format!("{e:#}")),
        },
    );
}
fn request_details(account: Account, repo: Repository, number: u64) {
    let a = account.clone();
    let r = repo.clone();
    client_job(
        account,
        move |c| c.request(&r, number),
        move |result| match result {
            Ok(request) => {
                let (dialog, body) = widgets::dialog(&format!("Request #{}", request.number), 680);
                crate::dialog_dismissal::add(&dialog, &body, "Close");
                note(&body, &request.title);
                note(
                    &body,
                    &format!(
                        "{}{}\n{} → {}\nStatus: {}\nCommit: {}",
                        request.state,
                        if request.draft { " · draft" } else { "" },
                        request.source,
                        request.target,
                        request.status,
                        request.head
                    ),
                );
                let actions = widgets::hbox(6);
                let open = gtk::Button::with_label("Open request");
                let a2 = a.clone();
                let url = request.web_url.clone();
                open.connect_clicked(move |_| link(&a2, &url));
                actions.append(&open);
                let checkout = gtk::Button::with_label("Checkout for review…");
                checkout.set_sensitive(!request.head.is_empty());
                let a2 = a.clone();
                let r = repo.clone();
                let request2 = request.clone();
                let d = dialog.clone();
                checkout.connect_clicked(move |_| {
                    d.close();
                    checkout_request(a2.clone(), r.clone(), request2.clone());
                });
                actions.append(&checkout);
                let refresh = gtk::Button::with_label("Refresh status");
                let d = dialog.clone();
                refresh.connect_clicked(move |_| {
                    d.close();
                    request_details(a.clone(), repo.clone(), number);
                });
                actions.append(&refresh);
                body.append(&actions);
                close(&dialog, &body);
                dialog.present();
            }
            Err(e) => fail(format!("{e:#}")),
        },
    );
}
pub(super) fn checkout_request(account: Account, repo: Repository, request: Request) {
    let Some(root) = current().and_then(|ui| ui.root()) else {
        error("Open a clone of this repository before checking out requests.");
        return;
    };
    let default = format!("review/{}/{}", account.kind.name().to_lowercase(), request.number);
    let protocol = gtk::DropDown::from_strings(&["SSH · system agent/keys", "HTTPS · system Git credentials"]);
    if repo.ssh_url.is_empty() {
        protocol.set_selected(1);
    }
    let picker = protocol.clone();
    let extra = widgets::ask_text(
        "Checkout request branch",
        "Create a new local review branch from the request's exact observed commit. A matching repository remote and clean working tree are required.",
        &default,
        "Prepare",
        move |branch| {
            let root = root.clone();
            let repo = repo.clone();
            let request = request.clone();
            let kind = account.kind;
            let ssh = picker.selected() == 0;
            nyx_core::cmd::background(
                move || -> anyhow::Result<_> {
                    let guard = Guard::capture(&root)?;
                    guard.clean()?;
                    let reference = if kind == Kind::Gitlab {
                        format!("refs/merge-requests/{}/head", request.number)
                    } else {
                        format!("refs/pull/{}/head", request.number)
                    };
                    let url = if ssh { repo.ssh_url.clone() } else { repo.https_url.clone() };
                    let action = hosting::Checkout {
                        url,
                        reference,
                        expected: request.head,
                        branch,
                        allowed_remotes: vec![repo.https_url, repo.ssh_url],
                        guard: Box::new(guard),
                    };
                    Ok((root, action))
                },
                move |result| match result {
                    Ok((root, action)) => confirm(
                        "Fetch and checkout request?",
                        &format!(
                            "Fetch from {} and create local branch {}. The request must still point to the displayed commit. Existing branches are never overwritten.",
                            action.url, action.branch
                        ),
                        "Fetch and checkout",
                        move || {
                            if let Some(ui) = current() {
                                ui.execute_at(root.clone(), Operation::CheckoutRequest(action.clone()));
                            }
                        },
                    ),
                    Err(e) => fail(format!("{e:#}")),
                },
            );
        },
    );
    extra.append(&protocol);
}
fn issues(account: Account, repo: Repository, page: u32) {
    let a = account.clone();
    let r = repo.clone();
    client_job(
        account,
        move |c| c.issues(&r, page),
        move |result| match result {
            Ok(result) => {
                let (dialog, body) = widgets::dialog(&format!("Issues · {}", repo.name), 700);
                crate::dialog_dismissal::add(&dialog, &body, "Close");
                let rows = widgets::vbox(0);
                rows.add_css_class("group-list");
                if result.items.is_empty() {
                    note(&rows, "No issues on this page.");
                }
                for issue in result.items {
                    let b = gtk::Button::with_label("Open");
                    let a = a.clone();
                    b.connect_clicked(move |_| link(&a, &issue.web_url));
                    rows.append(&row(&format!("#{} {}", issue.number, issue.title), "", Some(b.upcast_ref())));
                }
                body.append(&scroll(&rows));
                let nav = widgets::hbox(6);
                for (title, p) in [("Previous page", page.checked_sub(1).filter(|p| *p > 0)), ("Next page", result.next)] {
                    if let Some(p) = p {
                        let b = gtk::Button::with_label(title);
                        let a = a.clone();
                        let r = repo.clone();
                        let d = dialog.clone();
                        b.connect_clicked(move |_| {
                            d.close();
                            issues(a.clone(), r.clone(), p);
                        });
                        nav.append(&b);
                    }
                }
                body.append(&nav);
                close(&dialog, &body);
                dialog.present();
            }
            Err(e) => fail(format!("{e:#}")),
        },
    );
}
pub(super) fn new_request(account: Account, repo: Repository) {
    let (dialog, body) = widgets::dialog("New pull / merge request", 680);
    crate::dialog_dismissal::add(&dialog, &body, "Cancel");
    note(
        &body,
        &format!(
            "Publish to {} on {}. Push the source branch with Git first; this form does not push commits.",
            repo.name, account.base_url
        ),
    );
    let title = gtk::Entry::new();
    title.set_placeholder_text(Some("Request title"));
    body.append(&title);
    let source = gtk::Entry::new();
    source.set_placeholder_text(Some("Source branch (GitHub/Gitea may use owner:branch for a fork)"));
    body.append(&source);
    let target = gtk::Entry::new();
    target.set_placeholder_text(Some("Target branch"));
    target.set_text(&repo.default_branch);
    body.append(&target);
    let description = gtk::TextView::new();
    description.set_wrap_mode(gtk::WrapMode::WordChar);
    description.set_monospace(true);
    body.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(super::scrollbars::policy())
            .vscrollbar_policy(super::scrollbars::policy())
            .child(&description)
            .min_content_height(130)
            .build(),
    );
    let draft = gtk::CheckButton::with_label("Draft request");
    draft.set_active(true);
    body.append(&draft);
    let status = note(&body, "");
    status.add_css_class("git-error");
    let publish = gtk::Button::with_label("Review and publish…");
    publish.add_css_class("suggested-action");
    let d = dialog.clone();
    publish.connect_clicked(move |button| {
        let title = title.text().trim().to_string();
        let source = source.text().trim().to_string();
        let target = target.text().trim().to_string();
        if title.is_empty() || source.is_empty() || target.is_empty() {
            status.set_text("Enter a title, source branch and target branch.");
            return;
        }
        let buffer = description.buffer();
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
        let draft = draft.is_active();
        let a = account.clone();
        let r = repo.clone();
        let b = button.clone();
        let status = status.clone();
        let d = d.clone();
        confirm(
            "Publish request?",
            &format!(
                "Create {} on {}\n{} → {}\n{}{}",
                r.name,
                a.base_url,
                source,
                target,
                title,
                if draft { "\nDraft" } else { "" }
            ),
            "Publish request",
            move || {
                b.set_sensitive(false);
                let a = a.clone();
                let r = r.clone();
                let t = title.clone();
                let s = source.clone();
                let target = target.clone();
                let text = text.clone();
                let b = b.clone();
                let status = status.clone();
                let d = d.clone();
                let account2 = a.clone();
                let repo2 = r.clone();
                client_job(
                    a,
                    move |c| c.create_request(&r, &t, &text, &s, &target, draft),
                    move |result| {
                        b.set_sensitive(true);
                        match result {
                            Ok(created) => {
                                d.close();
                                request_details(account2, repo2, created.number);
                            }
                            Err(e) => status.set_text(&format!(
                                "{e:#}\nCheck existing requests before retrying; the server may have accepted the request."
                            )),
                        }
                    },
                );
            },
        );
    });
    body.append(&publish);
    close(&dialog, &body);
    dialog.present();
}
pub fn clone_dialog(repo: Repository) {
    let (dialog, body) = widgets::dialog("Clone repository", 660);
    crate::dialog_dismissal::add(&dialog, &body, "Cancel");
    note(&body, &repo.name);
    let choices = [("SSH · system agent/keys", repo.ssh_url.clone()), ("HTTPS · system Git credentials", repo.https_url.clone())]
        .into_iter()
        .filter(|(_, url)| !url.is_empty())
        .collect::<Vec<_>>();
    if choices.is_empty() {
        error("The provider did not supply a clone URL.");
        return;
    }
    let transport = gtk::DropDown::from_strings(&choices.iter().map(|(label, _)| *label).collect::<Vec<_>>());
    body.append(&transport);
    let remote = note(&body, &choices[0].1);
    let urls = choices.clone();
    transport.connect_selected_notify(move |p| {
        if let Some((_, url)) = urls.get(p.selected() as usize) {
            remote.set_text(url);
        }
    });
    let path = gtk::Entry::new();
    path.set_placeholder_text(Some("Absolute destination path · must not exist"));
    let parent = std::path::PathBuf::from(crate::prefs::get().clone_dir);
    if parent.is_absolute() && parent.is_dir() {
        path.set_text(&parent.join(repo.name.rsplit('/').next().unwrap_or("repository")).to_string_lossy());
    }
    body.append(&path);
    let browse = gtk::Button::with_label("Choose parent directory…");
    let d = dialog.clone();
    let p = path.clone();
    let name = repo.name.rsplit('/').next().unwrap_or("repository").to_string();
    browse.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder().title("Parent directory for the new clone").build();
        let parent = std::path::PathBuf::from(crate::prefs::get().clone_dir);
        if parent.is_absolute() && parent.is_dir() { dialog.set_initial_folder(Some(&gio::File::for_path(parent))); }
        let p = p.clone();
        let name = name.clone();
        dialog.select_folder(Some(&d), gio::Cancellable::NONE, move |result| match result {
            Ok(folder) => {
                if let Some(parent) = folder.path() {
                    p.set_text(&parent.join(name).to_string_lossy());
                }
            }
            Err(e) if e.matches(gtk::DialogError::Dismissed) || e.matches(gtk::DialogError::Cancelled) => {}
            Err(e) => fail(e),
        });
    });
    body.append(&browse);
    let clone = gtk::Button::with_label("Clone and open");
    clone.add_css_class("suggested-action");
    let d = dialog.clone();
    clone.connect_clicked(move |_| {
        let Some((_, url)) = choices.get(transport.selected() as usize) else { return };
        let url = url.clone();
        let path = PathBuf::from(path.text().trim());
        if let Some(ui) = current() {
            d.close();
            clone_repository(&ui, url, path);
        }
    });
    body.append(&clone);
    close(&dialog, &body);
    dialog.present();
}
fn clone_repository(ui: &Rc<super::window::Ui>, url: String, destination: PathBuf) {
    if ui.busy.get() {
        ui.toast("Wait for the current Git operation before cloning.");
        return;
    }
    if let Err(e) = hosting::transport(&url) {
        fail(format!("{e:#}"));
        return;
    }
    let Some(parent) = destination.parent().filter(|p| p.is_absolute()).map(PathBuf::from) else {
        error("Choose an absolute destination path.");
        return;
    };
    let operation = Operation::Clone { url, destination: destination.clone() };
    let (id, control) = match ui.tasks.start(destination.clone(), destination.clone(), &operation) {
        Ok(v) => v,
        Err(e) => {
            fail(e);
            return;
        }
    };
    ui.update_tasks();
    let weak = Rc::downgrade(ui);
    nyx_core::cmd::background(
        move || nyx_git::core::jobs::controlled(control, || operations::run(&parent, operation)).map(|_| destination),
        move |result| {
            let Some(ui) = weak.upgrade() else { return };
            ui.task_finished(id, result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
            match result {
                Ok(path) => ui.open(path),
                Err(e) => {
                    ui.update_readout();
                    ui.toast(&format!(
                        "{e:#}\nA stopped or failed clone may leave a partial directory; inspect it before retrying."
                    ));
                }
            }
        },
    );
}

// Long repository paths and request titles must wrap at half-screen widths.
fn row(title: &str, desc: &str, control: Option<&gtk::Widget>) -> gtk::Box {
    let row = widgets::row(title, desc, control);
    if let Some(text) = row.first_child() {
        let mut child = text.first_child();
        while let Some(w) = child {
            if let Some(label) = w.downcast_ref::<gtk::Label>() {
                label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                label.set_max_width_chars(46);
            }
            child = w.next_sibling();
        }
    }
    row
}

/// Developer snapshot aid; never loads credentials or performs network requests.
pub fn preview() {
    connect();
}

pub fn clone_url() {
    widgets::ask_text(
        "Clone repository URL",
        "Enter an HTTPS or SSH URL. No hosting account is needed; Git uses your native credential helpers or SSH keys.",
        "",
        "Next",
        |value| match hosting::transport(&value) {
            Ok((_, name)) => {
                let https = url::Url::parse(&value).is_ok_and(|u| u.scheme() == "https");
                clone_dialog(Repository {
                    key: name.clone(),
                    name,
                    web_url: String::new(),
                    https_url: if https { value.clone() } else { String::new() },
                    ssh_url: if https { String::new() } else { value },
                    default_branch: String::new(),
                });
            }
            Err(e) => fail(format!("{e:#}")),
        },
    );
}
