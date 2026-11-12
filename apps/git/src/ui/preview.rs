//! The confirmation before an operation that moves history or can lose work: what it does,
//! which references move, which commits leave or are rewritten, what's at risk and how it
//! can be recovered. Risky ones need a second click.
use gtk::prelude::*;
use nyx_git::core::preview::{self, Preview, Subject};
use nyx_ui::widgets;
use std::path::PathBuf;

fn short(id: &str) -> &str {
    &id[..id.len().min(10)]
}
fn section(body: &gtk::Box, title: &str) {
    let l = widgets::label(title, "group-title");
    l.set_margin_top(4);
    body.append(&l);
}
fn line(body: &gtk::Box, text: &str, class: &str) {
    let l = widgets::label(text, class);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_selectable(true);
    body.append(&l);
}
/// Work out the preview of `subject` in the background, then ask.
pub fn confirm(
    root: PathBuf,
    subject: Subject,
    title: String,
    effects: String,
    action: String,
    destructive: bool,
    run: impl Fn() + 'static,
) {
    nyx_core::cmd::background(
        move || preview::preview(&root, &subject),
        move |result| match result {
            Ok(p) => show(&title, &effects, &action, destructive, &p, run),
            // A preview that can't be worked out doesn't block the action; it says so.
            Err(e) => {
                let p = Preview { recovery: vec![format!("The preview couldn't be worked out: {e:#}")], ..Default::default() };
                show(&title, &effects, &action, destructive, &p, run)
            }
        },
    );
}
pub fn show(title: &str, effects: &str, action: &str, destructive: bool, p: &Preview, run: impl Fn() + 'static) {
    let (dialog, card) = widgets::dialog(title, 560);
    let body = widgets::vbox(8);
    line(&body, effects, "dim-label");
    if !p.refs.is_empty() {
        section(&body, "WHAT MOVES");
        for r in &p.refs {
            let before = r.before.as_deref().map(short).unwrap_or("nothing");
            line(&body, &format!("{}: {before} → {}", r.name, r.after), "");
        }
    }
    if p.commits_total > 0 {
        section(&body, &format!("{} COMMIT{} {}", p.commits_total, if p.commits_total == 1 { "" } else { "S" }, p.commits_note.to_uppercase()));
        for (id, subject) in p.commits.iter().take(8) {
            line(&body, &format!("{} {subject}", short(id)), "dim-label");
        }
        if p.commits_total > 8 {
            line(&body, &format!("… and {} more", p.commits_total - 8), "dim-label");
        }
        if p.published > 0 {
            body.append(&widgets::banner(
                &format!(
                    "{} of these {} already on a remote. Rewriting published commits means force-pushing later, and anyone who has them must reset or rebase. Nyx Git never force-pushes by itself.",
                    p.published,
                    if p.published == 1 { "is" } else { "are" }
                ),
                true,
            ));
        }
    }
    if !p.overlap.is_empty() {
        section(&body, "LIKELY CONFLICTS");
        let names: Vec<_> = p.overlap.iter().take(8).map(|f| f.display().to_string()).collect();
        line(
            &body,
            &format!(
                "Both sides changed {}{}. Git may stop for you to resolve them. A preview can't promise either way.",
                names.join(", "),
                if p.overlap.len() > 8 { format!(" and {} more", p.overlap.len() - 8) } else { String::new() }
            ),
            "",
        );
    }
    if !p.at_risk.is_empty() {
        section(&body, "AT RISK");
        let names: Vec<_> = p.at_risk.iter().take(10).map(|f| f.display().to_string()).collect();
        line(&body, &names.join("\n"), if p.irreversible { "git-error" } else { "" });
        if p.at_risk.len() > 10 {
            line(&body, &format!("… and {} more", p.at_risk.len() - 10), "dim-label");
        }
        line(&body, &p.risk_note, "dim-label");
    }
    if let Some(remote) = &p.remote {
        section(&body, "ON THE REMOTE");
        line(&body, remote, "");
    }
    if !p.recovery.is_empty() {
        section(&body, "RECOVERY");
        for r in &p.recovery {
            line(&body, r, "dim-label");
        }
    }
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(super::scrollbars::policy())
        .propagate_natural_height(true)
        .max_content_height(520)
        .child(&body)
        .build();
    card.append(&scroll);
    let row = widgets::hbox(8);
    row.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());
    row.append(&cancel);
    let d = dialog.clone();
    let go = move || {
        d.close();
        run();
    };
    let risky = destructive || p.irreversible || p.published > 0;
    let button = if risky {
        widgets::two_click(action, &format!("Confirm: {action}"), go)
    } else {
        let b = gtk::Button::with_label(action);
        b.add_css_class("suggested-action");
        b.connect_clicked(move |_| go());
        b
    };
    row.append(&button);
    card.append(&row);
    dialog.present();
    if risky {
        cancel.grab_focus();
    } else {
        button.grab_focus();
    }
}
