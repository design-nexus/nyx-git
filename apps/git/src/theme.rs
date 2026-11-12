use crate::prefs;
pub fn install() {
    nyx_theme::use_design_2();
    nyx_theme::install(nyx_theme::Style {
        window_class: "git-window",
        base_skip: &[],
        css: include_str!("ui/style.css"),
        no_glow_css: "",
        extra_colors: None,
        prefs: || {
            let p = prefs::get();
            nyx_theme::ThemePrefs { mode: p.mode, theme: p.theme, glow: p.glow, reduce_motion: p.reduce_motion }
        },
        set_theme: Some(|mode, theme| {
            prefs::update(|p| {
                p.mode = mode;
                p.theme = theme;
            })
        }),
    });
    install_icons();
}

/// Git's own symbolic icons (`git-*-symbolic`), written to the cache once and added to the
/// icon search path.
fn install_icons() {
    let dir = nyx_core::paths::cache_dir().join("icons");
    let _ = std::fs::create_dir_all(&dir);
    for (name, svg) in crate::ui::icons::ICONS {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*svg) {
            let _ = std::fs::write(&path, svg);
        }
    }
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(&dir);
    }
}
