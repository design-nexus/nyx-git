//! Serialized inspection jobs; GTK widgets never enter worker threads.
use crate::{prefs, ui::window::Ui};
use gtk::prelude::*;
use nyx_git::core::{repository, watch::Plan};
use std::{path::PathBuf, rc::Rc};

impl Ui {
    /// Open a repository, or read the open one again, at the user's request.
    pub fn open(self: &Rc<Self>, path: PathBuf) {
        self.load(path, false, None);
    }
    /// Read the open repository again after a file or ref change. Unlike `open`, nothing on
    /// screen is disabled meanwhile, so typing a commit message or selecting lines carries on.
    pub fn reload(self: &Rc<Self>, path: PathBuf, plan: Option<Plan>) {
        self.load(path, true, plan);
    }
    fn load(self: &Rc<Self>, path: PathBuf, quiet: bool, plan: Option<Plan>) {
        if self.busy.replace(true) {
            let mut pending = self.pending.borrow_mut();
            if pending.back() != Some(&path) {
                pending.push_back(path);
            }
            return;
        }
        self.quiet.set(quiet);
        let generation = self.loads.get() + 1;
        self.loads.set(generation);
        if !quiet {
            self.readout.set_text("Reading repository…");
            self.workflow.sensitive(false);
            self.header.set_loading(true);
        }
        let weak = Rc::downgrade(self);
        nyx_core::cmd::background(
            move || {
                let result = repository::load_reusing(&path, plan);
                (path, result)
            },
            move |(path, result)| {
                let Some(ui) = weak.upgrade() else { return };
                ui.busy.set(false);
                ui.quiet.set(false);
                // A Git operation that started meanwhile brings its own, newer status.
                if generation == ui.loads.get() {
                    match result {
                        Ok(snapshot) => {
                            ui.failed.borrow_mut().take();
                            let root = snapshot.repository.root.clone();
                            if !quiet {
                                prefs::update(|p| {
                                    p.recent.retain(|r| r != &root);
                                    p.recent.insert(0, root.clone());
                                    p.recent.truncate(20);
                                    p.last_repository = Some(root);
                                });
                            }
                            ui.apply(snapshot);
                        }
                        Err(error) => {
                            tracing::warn!(%error, "Repository inspection failed");
                            *ui.failed.borrow_mut() = Some(path);
                            ui.readout.set_text("Repository could not be read · F5 to retry");
                            if !quiet {
                                ui.error(&format!("{error:#}"));
                            }
                        }
                    }
                }
                ui.update_tasks();
                let next = ui.pending.borrow_mut().pop_front();
                if let Some(next) = next {
                    ui.open(next);
                }
            },
        );
    }

    pub fn refresh(self: &Rc<Self>) {
        // Retry a repository that couldn't be read, rather than the one still on screen.
        let failed = self.failed.borrow().clone();
        if let Some(path) = failed {
            self.open(path);
            return;
        }
        if self.snapshot.borrow().as_ref().is_some_and(|s| self.tasks.busy(&s.repository.common_dir)) {
            self.monitor.changed(true);
            return;
        }
        self.graph.invalidate();
        self.sidebar.invalidate();
        if let Some(root) = self.root() {
            self.open(root);
        }
    }

    pub fn discover(self: &Rc<Self>, folder: PathBuf) {
        let weak = Rc::downgrade(self);
        self.readout.set_text("Looking for repositories…");
        nyx_core::cmd::background(
            move || repository::discover(&folder),
            move |result| {
                let Some(ui) = weak.upgrade() else { return };
                match result {
                    Ok(paths) => {
                        let count = paths.len();
                        prefs::update(|p| {
                            for path in paths {
                                if !p.recent.contains(&path) {
                                    p.recent.push(path);
                                }
                            }
                            p.recent.truncate(20);
                        });
                        ui.rebuild_navigation();
                        ui.toast(&match count {
                            0 => "No repositories in the folder or its immediate subfolders.".to_string(),
                            1 => "Found 1 repository. It's listed under Recent repositories.".to_string(),
                            n => format!("Found {n} repositories. They're listed under Recent repositories."),
                        });
                    }
                    Err(error) => ui.error(&format!("{error:#}")),
                }
                ui.update_readout();
            },
        );
    }
}

impl Ui {
    pub fn root(&self) -> Option<PathBuf> {
        self.snapshot.borrow().as_ref().map(|s| s.repository.root.clone())
    }
    pub fn execute(self: &Rc<Self>, operation: nyx_git::core::operations::Operation) {
        if let Some(root) = self.root() {
            self.execute_at(root, operation);
        } else {
            self.toast("Open a repository first.");
        }
    }
    /// Fetch an open repository without requiring it to be the active tab.
    pub fn background_fetch(self: &Rc<Self>, root: PathBuf) {
        let weak = Rc::downgrade(self);
        nyx_core::cmd::background(move || repository::inspect(&root), move |result| {
            let Some(ui) = weak.upgrade() else { return };
            let Ok(repo) = result else { return };
            if prefs::get().fetch_minutes == 0 || !prefs::get().workspace.open.contains(&repo.root)
                || ui.tasks.busy(&repo.common_dir) { return; }
            let operation = nyx_git::core::operations::Operation::Fetch;
            let Ok((id, control)) = ui.tasks.start(repo.root.clone(), repo.common_dir.clone(), &operation) else { return };
            ui.update_tasks();
            let weak = Rc::downgrade(&ui);
            nyx_core::cmd::background(move || {
                nyx_git::core::jobs::controlled(control, || nyx_git::core::operations::run(&repo.root, operation))
                    .map(|_| ()).map_err(|e| format!("{e:#}"))
            }, move |result| {
                let Some(ui) = weak.upgrade() else { return };
                ui.tasks.finish(id, result);
                ui.update_tasks();
                ui.refresh();
            });
        });
    }

    pub fn execute_at(self: &Rc<Self>, root: PathBuf, operation: nyx_git::core::operations::Operation) {
        self.execute_at_then(root, operation, |_| {});
    }
    pub fn execute_at_then(
        self: &Rc<Self>,
        root: PathBuf,
        operation: nyx_git::core::operations::Operation,
        done: impl FnOnce(Result<(), String>) + 'static,
    ) {
        use nyx_git::core::operations::{self, Operation};
        let mut done = Some(done);
        let mut refuse = |message: &str| {
            self.toast(message);
            if let Some(done) = done.take() {
                done(Err(message.into()));
            }
        };
        if self.root().as_ref() != Some(&root) {
            return refuse("The active repository changed. Choose the action again.");
        }
        // A background status read doesn't hold up an action; opening a repository does.
        if self.busy.get() && !self.quiet.get() {
            return refuse("Wait for the repository to finish loading.");
        }
        let (bare, conflicts) = self
            .snapshot
            .borrow()
            .as_ref()
            .map_or((false, false), |s| (s.repository.bare, s.status.files.iter().any(|f| f.conflicted)));
        if bare && !matches!(operation, Operation::Worktree(_) | Operation::Remote { .. } | Operation::DeleteRemoteBranch { .. })
        {
            return refuse("A bare repository has no working tree, so this action isn't available here.");
        }
        if conflicts && matches!(operation, Operation::StageAll | Operation::Commit { .. }) {
            return refuse("Resolve the conflicted files first: select one and choose Resolve conflict.");
        }
        self.save_repository();
        let common = self.snapshot.borrow().as_ref().map(|s| s.repository.common_dir.clone()).unwrap_or_else(|| root.clone());
        let (id, control) = match self.tasks.start(root.clone(), common.clone(), &operation) {
            Ok(v) => v,
            Err(e) => return refuse(&format!("{e:#}")),
        };
        // Any status read already under way predates this change; its result is dropped.
        self.loads.set(self.loads.get() + 1);
        self.update_tasks();
        let label = operation.label();
        let refresh_history = !matches!(
            operation,
            Operation::Stage(_)
                | Operation::StageAll
                | Operation::Unstage { .. }
                | Operation::UnstageAll { .. }
                | Operation::StageSelection { .. }
                | Operation::CheckPatch(_)
                | Operation::ApplyPatch(_)
        );
        let quiet_success = !refresh_history;
        let commit_message = match &operation {
            Operation::Commit { message, .. } => Some(message.clone()),
            _ => None,
        };
        let worker_root = root.clone();
        let plan = self.snapshot.borrow().as_ref().filter(|_| !refresh_history).map(|s| s.watch.clone());
        let weak = Rc::downgrade(self);
        nyx_core::cmd::background(
            move || {
                let result = nyx_git::core::jobs::controlled(control, || operations::run(&worker_root, operation));
                let snapshot = repository::load_reusing(&worker_root, plan);
                (result, snapshot)
            },
            move |(result, snapshot)| {
                let Some(ui) = weak.upgrade() else { return };
                let completion = result.as_ref().map(|_| ()).map_err(|e| format!("{e:#}"));
                ui.task_finished(id, completion.clone());
                let same = ui.root().as_ref() == Some(&root) && prefs::get().workspace.open.contains(&root);
                if same && !ui.tasks.busy(&common) {
                    if refresh_history {
                        ui.graph.invalidate();
                        ui.sidebar.invalidate();
                    }
                    match snapshot {
                        Ok(snapshot) => ui.apply(snapshot),
                        Err(e) => {
                            ui.readout.set_text("Status refresh failed · F5 to retry");
                            ui.error(&format!("{label}: status refresh failed: {e:#}"));
                        }
                    }
                } else if ui.snapshot.borrow().as_ref().is_some_and(|s| s.repository.common_dir == common) {
                    ui.monitor.changed(refresh_history);
                }
                match result {
                    Ok(_) => {
                        if let Some(message) = commit_message {
                            prefs::update(|p| {
                                let entry = p.workspace.entry(&root);
                                if entry.message == message {
                                    entry.message.clear();
                                    entry.amend = false;
                                }
                            });
                            if same && ui.workflow.message() == message {
                                ui.workflow.set_message("");
                                ui.workflow.amend.set_active(false);
                            }
                        }
                        // Staging shows itself in the file list; a toast for each would be noise.
                        if !quiet_success {
                            ui.toast(&format!("{label} completed · {}", root.file_name().unwrap_or_default().to_string_lossy()));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(operation = label, %e, "Git operation failed");
                        ui.error(&format!("{e:#}"));
                    }
                }
                ui.update_tasks();
                if let Some(done) = done {
                    done(completion);
                }
                let pending = ui.pending.borrow_mut().pop_front();
                if let Some(path) = pending {
                    ui.open(path);
                }
            },
        );
    }
}
