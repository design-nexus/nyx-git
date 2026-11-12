//! The destinations beyond History and Changes, each a page of its own that loads when
//! first shown and again when the repository changes.
use std::{cell::RefCell, rc::Rc};

pub struct Pages {
    pub flow: Rc<super::flow::FlowPage>,
    pub rebase: Rc<super::rebase_studio::RebaseStudio>,
    pub conflicts: Rc<super::conflict_studio::ConflictStudio>,
    pub activity: Rc<super::activity::Activity>,
    pub review: Rc<super::review::ReviewPage>,
    visible: RefCell<String>,
}
impl Pages {
    pub fn new() -> Self {
        Self {
            flow: super::flow::FlowPage::new(),
            rebase: super::rebase_studio::RebaseStudio::new(),
            conflicts: super::conflict_studio::ConflictStudio::new(),
            activity: super::activity::Activity::new(),
            review: super::review::ReviewPage::new(),
            visible: RefCell::new(String::new()),
        }
    }
    /// The destinations the window offers, in header order.
    pub fn ids(&self) -> Vec<&'static str> {
        let mut ids = vec!["flow", "changes", "history", "review", "activity"];
        ids.sort_by_key(|id| super::destinations::key(id));
        ids
    }
    pub fn add_to(&self, stack: &gtk::Stack) {
        stack.add_named(&self.flow.root, Some("flow"));
        stack.add_named(&self.rebase.root, Some("rebase"));
        stack.add_named(&self.conflicts.root, Some("conflicts"));
        stack.add_named(&self.activity.root, Some("activity"));
        stack.add_named(&self.review.root, Some("review"));
    }
    /// `name` became the visible page.
    pub fn shown(&self, name: &str) {
        *self.visible.borrow_mut() = name.into();
        match name {
            "activity" => self.activity.load(false),
            "flow" => self.flow.load(false),
            "review" => self.review.load(false),
            _ => {}
        }
    }
    /// A local branch's tip and worktree, if Flow has read it, else straight from Git.
    pub fn flow_branch(&self, name: &str) -> Option<(String, Option<std::path::PathBuf>)> {
        self.flow.branch(name)
    }
    /// The repository was read again (it changed, or another one opened).
    pub fn refresh(&self) {
        match self.visible.borrow().as_str() {
            "activity" => self.activity.load(true),
            "flow" => self.flow.load(true),
            // Network reads only when asked for, or for another repository.
            "review" => self.review.load(false),
            _ => {}
        }
    }
}
/// Studios: pages opened for a task, not listed in the header.
pub fn is_studio(name: &str) -> bool {
    matches!(name, "rebase" | "conflicts")
}
/// History and Changes share the window's details pane; the other pages lay out their own.
pub fn has_details(name: &str) -> bool {
    matches!(name, "history" | "changes")
}
