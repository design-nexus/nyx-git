use gtk::{cairo, gdk, pango, prelude::*};
use nyx_git::graph::model::{Query, RefKind, Row};
pub fn row_height(zoom: f64) -> f64 {
    36.0 * zoom
}
/// "19 minutes ago", "3 days ago": the History list's time column.
pub fn ago(timestamp: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_secs() as i64);
    let delta = now.saturating_sub(timestamp);
    let say = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match delta {
        d if d < 0 => "in the future".into(),
        d if d < 60 => "just now".into(),
        d if d < 3600 => say(d / 60, "minute"),
        d if d < 86400 => say(d / 3600, "hour"),
        d if d < 86400 * 14 => say(d / 86400, "day"),
        d if d < 86400 * 60 => say(d / (86400 * 7), "week"),
        d if d < 86400 * 365 => say(d / (86400 * 30), "month"),
        d => say(d / (86400 * 365), "year"),
    }
}
pub fn relative_time(timestamp: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_secs() as i64);
    let delta = now.saturating_sub(timestamp);
    if delta < 0 {
        "future".into()
    } else if delta < 60 {
        "now".into()
    } else if delta < 3600 {
        format!("{}m", delta / 60)
    } else if delta < 86400 {
        format!("{}h", delta / 3600)
    } else if delta < 86400 * 30 {
        format!("{}d", delta / 86400)
    } else if delta < 86400 * 365 {
        format!("{}mo", delta / (86400 * 30))
    } else {
        format!("{}y", delta / (86400 * 365))
    }
}
fn color(text: &str) -> gdk::RGBA {
    gdk::RGBA::parse(text).unwrap_or(gdk::RGBA::BLACK)
}
fn mix(a: &gdk::RGBA, b: &gdk::RGBA, t: f32) -> gdk::RGBA {
    gdk::RGBA::new(
        a.red() * (1.0 - t) + b.red() * t,
        a.green() * (1.0 - t) + b.green() * t,
        a.blue() * (1.0 - t) + b.blue() * t,
        1.0,
    )
}
fn source(cr: &cairo::Context, color: &gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(color.red() as f64, color.green() as f64, color.blue() as f64, alpha);
}
fn font(area: &gtk::DrawingArea, zoom: f64) -> pango::FontDescription {
    let mut font =
        area.pango_context().font_description().unwrap_or_else(|| pango::FontDescription::from_string("JetBrains Mono"));
    font.set_absolute_size(12.0 * zoom * pango::SCALE as f64);
    font
}
/// Draws lines of text in the graph's font, each ellipsized to a width and centred on a y.
struct Text<'a> {
    area: &'a gtk::DrawingArea,
    cr: &'a cairo::Context,
    font: &'a pango::FontDescription,
}
impl Text<'_> {
    fn draw(&self, value: &str, x: f64, y: f64, width: f64, color: &gdk::RGBA, alpha: f64) {
        if width < 8.0 {
            return;
        }
        let layout = self.area.create_pango_layout(Some(value));
        layout.set_font_description(Some(self.font));
        layout.set_width((width * pango::SCALE as f64) as i32);
        layout.set_single_paragraph_mode(true);
        layout.set_ellipsize(pango::EllipsizeMode::End);
        source(self.cr, color, alpha);
        self.cr.move_to(x, y - layout.pixel_size().1 as f64 / 2.0);
        pangocairo::functions::show_layout(self.cr, &layout);
    }
}
pub struct Frame<'a> {
    pub rows: &'a [Row],
    pub start: usize,
    pub offset: f64,
    pub zoom: f64,
    pub selected: Option<usize>,
    pub hover: Option<usize>,
    pub lanes: usize,
    pub query: &'a Query,
    /// The commit marked for comparison.
    pub comparison: Option<&'a str>,
    /// The branch checked out, whose chip is solid.
    pub branch: Option<&'a str>,
}
/// A rounded rectangle's path.
fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(h / 2.0).min(w / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}
struct Colors {
    bg: gdk::RGBA,
    accent: gdk::RGBA,
    fg: gdk::RGBA,
    dim: gdk::RGBA,
    lanes: [gdk::RGBA; 8],
}
fn colors() -> Colors {
    let palette = nyx_theme::palette();
    let bg = color(&palette.bg);
    let accent = color(&palette.accent);
    let danger = color(&palette.danger);
    let fg = color(&palette.text);
    let dim = color(&palette.dim_text);
    let lanes = [
        accent,
        mix(&accent, &danger, 0.6),
        mix(&accent, &fg, 0.6),
        danger,
        mix(&accent, &dim, 0.55),
        mix(&danger, &fg, 0.5),
        mix(&accent, &danger, 0.25),
        mix(&dim, &fg, 0.6),
    ];
    Colors { bg, accent, fg, dim, lanes }
}
/// Width of a commit's column in the timeline.
pub fn timeline_column(zoom: f64) -> f64 {
    52.0 * zoom
}
/// Height of a lane in the timeline.
pub fn lane_height(zoom: f64) -> f64 {
    26.0 * zoom
}
/// Space above the lanes for ref labels, and below them for dates.
const TIMELINE_TOP: f64 = 50.0;
/// The timeline: commits left to right, newest first, each branch's lane a row. Same rows
/// and edges as the graph, transposed, so the topology is the graph's exactly.
pub fn draw_timeline(area: &gtk::DrawingArea, cr: &cairo::Context, width: f64, height: f64, frame: Frame<'_>) {
    let Colors { bg, accent, fg, dim, lanes: colors } = colors();
    source(cr, &bg, 1.0);
    let _ = cr.paint();
    let col = timeline_column(frame.zoom);
    let lane_h = lane_height(frame.zoom);
    let font = font(area, frame.zoom);
    let write = Text { area, cr, font: &font };
    let top = TIMELINE_TOP * frame.zoom;
    let y = |lane: usize| top + lane_h / 2.0 + lane as f64 * lane_h;
    cr.rectangle(0.0, 0.0, width, height);
    cr.clip();
    cr.set_line_width(2.0 * frame.zoom);
    cr.set_line_cap(cairo::LineCap::Round);
    let bottom = (top + frame.lanes as f64 * lane_h).min(height - 40.0 * frame.zoom);
    let mut last_day = None;
    for (i, row) in frame.rows.iter().enumerate() {
        let index = frame.start + i;
        let left = index as f64 * col - frame.offset;
        let middle = left + col / 2.0;
        if left + col < 0.0 || left > width {
            continue;
        }
        if frame.selected == Some(index) || frame.hover == Some(index) {
            source(cr, &mix(&bg, &accent, if frame.selected == Some(index) { 0.18 } else { 0.07 }), 1.0);
            cr.rectangle(left, 0.0, col, height);
            let _ = cr.fill();
        }
        if frame.comparison == Some(row.commit.id.as_str()) {
            source(cr, &accent, 1.0);
            cr.rectangle(left + 3.0, 0.0, col - 6.0, 3.0);
            let _ = cr.fill();
        }
        for edge in &row.edges {
            let from = left + col * edge.top as f64 / 2.0;
            let to = left + col * edge.bottom as f64 / 2.0;
            source(cr, &colors[edge.color % colors.len()], 0.9);
            cr.move_to(from, y(edge.from));
            cr.curve_to((from + to) / 2.0, y(edge.from), (from + to) / 2.0, y(edge.to), to, y(edge.to));
            let _ = cr.stroke();
        }
        let is_head = row.commit.refs.iter().any(|r| r.kind == RefKind::Head);
        let alpha = if frame.query.is_empty() || frame.query.matches(&row.commit) { 1.0 } else { 0.38 };
        source(cr, &bg, 1.0);
        cr.arc(middle, y(row.lane), 6.0 * frame.zoom, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        source(cr, &colors[row.color % colors.len()], alpha);
        cr.arc(middle, y(row.lane), if is_head { 5.5 * frame.zoom } else { 4.2 * frame.zoom }, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        if row.commit.parents.len() > 1 || row.commit.collapsed > 0 || is_head || row.commit.shallow {
            source(cr, &bg, 1.0);
            cr.arc(middle, y(row.lane), 2.3 * frame.zoom, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        // A branch or tag names its tip above the lanes, with a tick down to it.
        if let Some(name) = row.commit.refs.iter().find(|r| r.kind != RefKind::Head).map(|r| r.name.as_str()) {
            source(cr, &mix(&bg, &accent, 0.5), 1.0);
            cr.set_line_width(1.0);
            cr.move_to(middle, top - 4.0);
            cr.line_to(middle, y(row.lane) - 7.0 * frame.zoom);
            let _ = cr.stroke();
            cr.set_line_width(2.0 * frame.zoom);
            // Neighbouring tips take turns between two label rows.
            let row_y = if index.is_multiple_of(2) { top * 0.3 } else { top * 0.7 };
            write.draw(name, left + 2.0, row_y, col * 2.0 - 4.0, &accent, alpha);
        }
        // The day each run of commits starts, along the bottom.
        let day = row.commit.timestamp.div_euclid(86400);
        if last_day != Some(day) {
            last_day = Some(day);
            source(cr, &dim, 0.35);
            cr.set_line_width(1.0);
            cr.move_to(left, top);
            cr.line_to(left, bottom + 6.0);
            let _ = cr.stroke();
            cr.set_line_width(2.0 * frame.zoom);
            write.draw(&relative_time(row.commit.timestamp), left + 3.0, bottom + 16.0 * frame.zoom, col * 2.5, &dim, 1.0);
        }
    }
    // The selected commit's summary at the foot.
    if let Some(row) = frame.selected.and_then(|i| i.checked_sub(frame.start)).and_then(|i| frame.rows.get(i)) {
        let text = format!("{} · {} · {}", &row.commit.id[..8], row.commit.subject, row.commit.author);
        write.draw(&text, 12.0, height - 14.0 * frame.zoom, width - 24.0, &fg, 1.0);
    }
}
pub fn draw(area: &gtk::DrawingArea, cr: &cairo::Context, width: f64, height: f64, frame: Frame<'_>) {
    let palette = nyx_theme::palette();
    let bg = color(&palette.bg);
    let accent = color(&palette.accent);
    let danger = color(&palette.danger);
    let fg = color(&palette.text);
    let dim = color(&palette.dim_text);
    let on_accent = if palette.light { color("#ffffff") } else { bg };
    let colors = [
        accent,
        mix(&accent, &danger, 0.6),
        mix(&accent, &fg, 0.6),
        danger,
        mix(&accent, &dim, 0.55),
        mix(&danger, &fg, 0.5),
        mix(&accent, &danger, 0.25),
        mix(&dim, &fg, 0.6),
    ];
    let violet = mix(&accent, &danger, 0.45);
    let tertiary = mix(&dim, &bg, 0.35);
    source(cr, &bg, 1.0);
    let _ = cr.paint();
    let row_h = row_height(frame.zoom);
    let font = font(area, frame.zoom);
    let mut small = font.clone();
    small.set_absolute_size(11.0 * frame.zoom * pango::SCALE as f64);
    small.set_weight(pango::Weight::Semibold);
    let mut mono = pango::FontDescription::from_string("JetBrains Mono");
    mono.set_absolute_size(11.5 * frame.zoom * pango::SCALE as f64);
    let write = Text { area, cr, font: &font };
    let chips = Text { area, cr, font: &small };
    let hashes = Text { area, cr, font: &mono };
    let graph_w = (frame.lanes as f64 * 16.0 * frame.zoom + 28.0).min(width * 0.4).max(52.0);
    let spacing = (16.0 * frame.zoom).min((graph_w - 28.0) / frame.lanes.max(1) as f64);
    let x = |lane: usize| 22.0 + lane as f64 * spacing;
    // Columns at the right: hash, author and time, as the window allows.
    let time_w = 112.0 * frame.zoom;
    let author_w = if width > 720.0 { 110.0 * frame.zoom } else { 0.0 };
    let hash_w = if width > 900.0 { 70.0 * frame.zoom } else { 0.0 };
    cr.rectangle(0.0, 0.0, width, height);
    cr.clip();
    cr.set_line_width(2.0 * frame.zoom);
    cr.set_line_cap(cairo::LineCap::Round);
    for (i, row) in frame.rows.iter().enumerate() {
        let index = frame.start + i;
        let top = index as f64 * row_h - frame.offset;
        let middle = top + row_h / 2.0;
        if top + row_h < 0.0 || top > height {
            continue;
        }
        // The selected commit is a soft pill; the one under the pointer, a fainter one.
        if frame.selected == Some(index) || frame.hover == Some(index) {
            source(cr, &mix(&bg, &accent, if frame.selected == Some(index) { 0.16 } else { 0.06 }), 1.0);
            rounded(cr, 6.0, top + 1.0, width - 12.0, row_h - 2.0, 7.0);
            let _ = cr.fill();
        }
        if frame.comparison == Some(row.commit.id.as_str()) {
            source(cr, &accent, 1.0);
            rounded(cr, 6.0, top + 6.0, 3.0, row_h - 12.0, 1.5);
            let _ = cr.fill();
        }
        for edge in &row.edges {
            let from = top + row_h * edge.top as f64 / 2.0;
            let to = top + row_h * edge.bottom as f64 / 2.0;
            source(cr, &colors[edge.color % colors.len()], 0.9);
            cr.move_to(x(edge.from), from);
            cr.curve_to(x(edge.from), (from + to) / 2.0, x(edge.to), (from + to) / 2.0, x(edge.to), to);
            let _ = cr.stroke();
        }
        source(cr, &bg, 1.0);
        cr.arc(x(row.lane), middle, 6.5 * frame.zoom, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        let is_head = row.commit.refs.iter().any(|r| r.kind == RefKind::Head);
        source(cr, &colors[row.color % colors.len()], 1.0);
        cr.arc(x(row.lane), middle, if is_head { 5.5 * frame.zoom } else { 4.5 * frame.zoom }, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        if row.commit.parents.len() > 1 || row.commit.collapsed > 0 || is_head || row.commit.shallow {
            source(cr, &bg, 1.0);
            cr.arc(x(row.lane), middle, 2.4 * frame.zoom, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        let alpha = if frame.query.is_empty() || frame.query.matches(&row.commit) { 1.0 } else { 0.38 };
        let mut left = graph_w + 8.0;
        let right = (width - time_w - author_w - hash_w - 16.0).max(left + 8.0);
        // Each branch, remote branch and tag at the commit: a chip tinted by its kind.
        let mut refs: Vec<_> = row.commit.refs.iter().filter(|r| r.kind != RefKind::Head).collect();
        refs.sort_by_key(|r| match r.kind {
            RefKind::Local => 0,
            RefKind::Remote => 1,
            _ => 2,
        });
        // `origin/HEAD` peels to its target's name: show each name once.
        refs.dedup_by(|a, b| a.name == b.name && a.kind == b.kind);
        for r in refs.iter().take(3) {
            let solid = r.kind == RefKind::Local && is_head && frame.branch == Some(r.name.as_str());
            let (fill, fill_alpha, text) = match r.kind {
                _ if solid => (accent, 1.0, on_accent),
                RefKind::Local => (accent, 0.16, accent),
                RefKind::Remote => (fg, 0.06, dim),
                _ => (violet, 0.18, violet),
            };
            let layout = area.create_pango_layout(Some(&r.name));
            layout.set_font_description(Some(&small));
            let w = (layout.pixel_size().0 as f64 + 14.0 * frame.zoom).min((right - left) * 0.45);
            if w < 24.0 {
                break;
            }
            let h = 19.0 * frame.zoom;
            source(cr, &fill, fill_alpha);
            rounded(cr, left, middle - h / 2.0, w, h, 5.0);
            let _ = cr.fill();
            chips.draw(&r.name, left + 7.0 * frame.zoom, middle, w - 12.0 * frame.zoom, &text, alpha);
            left += w + 5.0;
        }
        if refs.len() > 3 {
            chips.draw(&format!("+{}", refs.len() - 3), left, middle, 30.0, &dim, alpha);
            left += 30.0;
        }
        write.draw(&row.commit.subject, left + 3.0, middle, right - left - 3.0, &fg, alpha);
        let mut at = right + 8.0;
        if hash_w > 0.0 {
            hashes.draw(&row.commit.id[..row.commit.id.len().min(7)], at, middle, hash_w - 8.0, &dim, alpha);
            at += hash_w;
        }
        if author_w > 0.0 {
            write.draw(&row.commit.author, at, middle, author_w - 10.0, &dim, alpha);
            at += author_w;
        }
        let when = ago(row.commit.timestamp);
        let layout = area.create_pango_layout(Some(&when));
        layout.set_font_description(Some(&font));
        let tw = layout.pixel_size().0 as f64;
        write.draw(&when, (at + time_w - tw - 6.0).max(at), middle, time_w, &tertiary, alpha);
    }
}
