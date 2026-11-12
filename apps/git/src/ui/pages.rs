//! The destinations beyond History and Changes, each a page of its own that loads when
//! first shown and again when the repository changes.
use std::{cell::RefCell, rc::Rc};

pub struct Pages {
    pub flow: Rc<super::flow::FlowPage>,
    pub activity: Rc<super::activity::Activity>,
    visible: RefCell<String>,
}
impl Pages {
    pub fn new() -> Self {
        Self { flow: super::flow::FlowPage::new(), activity: super::activity::Activity::new(), visible: RefCell::new(String::new()) }
    }
    /// The destinations the window offers, in header order.
    pub fn ids(&self) -> Vec<&'static str> {
        let mut ids = vec!["flow", "changes", "history", "activity"];
        ids.sort_by_key(|id| super::destinations::key(id));
        ids
    }
    pub fn add_to(&self, stack: &gtk::Stack) {
        stack.add_named(&self.flow.root, Some("flow"));
        stack.add_named(&self.activity.root, Some("activity"));
    }
    /// `name` became the visible page.
    pub fn shown(&self, name: &str) {
        *self.visible.borrow_mut() = name.into();
        match name {
            "activity" => self.activity.load(false),
            "flow" => self.flow.load(false),
            _ => {}
        }
    }
    /// The repository was read again (it changed, or another one opened).
    pub fn refresh(&self) {
        match self.visible.borrow().as_str() {
            "activity" => self.activity.load(true),
            "flow" => self.flow.load(true),
            _ => {}
        }
    }
}
/// History and Changes share the window's details pane; the other pages lay out their own.
pub fn has_details(name: &str) -> bool {
    matches!(name, "history" | "changes")
}
