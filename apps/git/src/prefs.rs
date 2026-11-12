pub use nyx_theme::ThemeMode;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub mode: ThemeMode,
    pub theme: String,
    pub glow: bool,
    pub reduce_motion: bool,
    pub show_scrollbars: bool,
    pub clone_dir: String,
    pub fetch_minutes: u64,
    pub sidebar_collapsed: bool,
    pub recent: Vec<PathBuf>,
    pub last_repository: Option<PathBuf>,
    pub details_position: i32,
    pub graph_zoom: f64,
    pub last_view: String,
    pub accounts: Vec<nyx_git::providers::Account>,
    pub workspace: nyx_git::core::workspace::Workspace,
}
impl Default for Prefs {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Omarchy,
            theme: nyx_theme::DEFAULT_THEME.into(),
            glow: true,
            reduce_motion: false,
            show_scrollbars: false,
            clone_dir: String::new(),
            fetch_minutes: 0,
            sidebar_collapsed: false,
            recent: Vec::new(),
            last_repository: None,
            details_position: 560,
            graph_zoom: 1.0,
            last_view: "history".into(),
            accounts: Vec::new(),
            workspace: Default::default(),
        }
    }
}
nyx_core::prefs_store!(Prefs);
