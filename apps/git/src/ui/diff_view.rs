use gtk::prelude::*;
use sourceview5::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub struct DiffView {
    pub root: gtk::ScrolledWindow,
    pub buffer: sourceview5::Buffer,
    /// The file's own line numbers ("old new") for each line of the diff, for the gutter.
    numbers: Rc<RefCell<Vec<String>>>,
    gutter: sourceview5::GutterRendererText,
    /// Old and new side by side, built by [`DiffView::switchable`].
    split: RefCell<Option<Split>>,
    side_by_side: Cell<bool>,
}
/// One side of a side-by-side diff.
struct Side {
    buffer: sourceview5::Buffer,
    numbers: Rc<RefCell<Vec<String>>>,
    gutter: sourceview5::GutterRendererText,
}
struct Split {
    stack: gtk::Stack,
    left: Side,
    right: Side,
}
fn text_view(buffer: &sourceview5::Buffer, label: &str) -> (sourceview5::View, sourceview5::GutterRendererText, Rc<RefCell<Vec<String>>>) {
    let view = sourceview5::View::with_buffer(buffer);
    view.set_editable(false);
    view.upcast_ref::<gtk::TextView>().update_property(&[gtk::accessible::Property::Label(label)]);
    view.set_cursor_visible(true);
    view.set_monospace(true);
    view.set_left_margin(10);
    view.set_right_margin(10);
    view.add_css_class("git-diff");
    // The patch's own line numbers mean nothing to a reader; show the file's.
    let numbers = Rc::new(RefCell::new(Vec::<String>::new()));
    let gutter = sourceview5::GutterRendererText::new();
    gutter.set_xalign(1.0);
    gutter.set_xpad(4);
    let lines = numbers.clone();
    gutter.connect_query_data(move |renderer, _, line| {
        if let Some(renderer) = renderer.downcast_ref::<sourceview5::GutterRendererText>() {
            renderer.set_text(lines.borrow().get(line as usize).map_or("", String::as_str));
        }
    });
    sourceview5::prelude::ViewExt::gutter(&view, gtk::TextWindowType::Left).insert(&gutter, 0);
    (view, gutter, numbers)
}
fn create_tags(buffer: &sourceview5::Buffer) {
    for name in ["addition", "deletion", "hunk", "meta", "word-add", "word-del", "filler"] {
        buffer.create_tag(Some(name), &[]);
    }
}
fn show_numbers(gutter: &sourceview5::GutterRendererText, store: &RefCell<Vec<String>>, numbers: Vec<String>) {
    let width = numbers.iter().map(String::len).max().unwrap_or(0);
    *store.borrow_mut() = numbers;
    let (w, _) = sourceview5::prelude::GutterRendererTextExt::measure(gutter, &"0".repeat(width.max(1)));
    gutter.set_size_request(if width == 0 { 0 } else { w + 8 }, -1);
    gutter.set_visible(width > 0);
    gutter.queue_draw();
}
fn tag_lines(buffer: &sourceview5::Buffer, kinds: &[Option<&str>], words: &[(usize, usize, usize, &str)]) {
    for (line, kind) in kinds.iter().enumerate() {
        let Some(kind) = kind else { continue };
        if let Some(start) = buffer.iter_at_line(line as i32) {
            let mut end = start;
            end.forward_to_line_end();
            buffer.apply_tag_by_name(kind, &start, &end);
        }
    }
    for (line, from, to, kind) in words {
        if let (Some(start), Some(end)) = (buffer.iter_at_line_offset(*line as i32, *from as i32), buffer.iter_at_line_offset(*line as i32, *to as i32)) {
            buffer.apply_tag_by_name(kind, &start, &end);
        }
    }
}
impl DiffView {
    pub fn new() -> Self {
        let buffer = sourceview5::Buffer::new(None);
        buffer.set_highlight_syntax(false);
        let (view, gutter, numbers) = text_view(&buffer, "File diff");
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(super::scrollbars::policy())
            .vscrollbar_policy(super::scrollbars::policy())
            .hexpand(true)
            .vexpand(true)
            .min_content_height(100)
            .child(&view)
            .build();
        create_tags(&buffer);
        let me = Self { root, buffer, numbers, gutter, split: RefCell::new(None), side_by_side: Cell::new(false) };
        me.recolor();
        me
    }
    /// The unified view and a side-by-side one in a stack, for pages that offer both.
    pub fn switchable(&self) -> gtk::Widget {
        if let Some(split) = self.split.borrow().as_ref() {
            return split.stack.clone().upcast();
        }
        let side = |label: &str| {
            let buffer = sourceview5::Buffer::new(None);
            buffer.set_highlight_syntax(false);
            create_tags(&buffer);
            let (view, gutter, numbers) = text_view(&buffer, label);
            (Side { buffer, numbers, gutter }, view)
        };
        let (left, left_view) = side("Old version");
        let (right, right_view) = side("New version");
        let scroll = |view: &sourceview5::View| {
            gtk::ScrolledWindow::builder()
                .hscrollbar_policy(super::scrollbars::policy())
                .vscrollbar_policy(super::scrollbars::policy())
                .hexpand(true)
                .vexpand(true)
                .min_content_height(100)
                .child(view)
                .build()
        };
        let (l, r) = (scroll(&left_view), scroll(&right_view));
        r.set_vadjustment(Some(&l.vadjustment()));
        let pair = gtk::Paned::new(gtk::Orientation::Horizontal);
        pair.set_start_child(Some(&l));
        pair.set_end_child(Some(&r));
        pair.set_shrink_start_child(false);
        pair.set_shrink_end_child(false);
        pair.set_resize_start_child(true);
        pair.set_resize_end_child(true);
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        stack.set_hhomogeneous(false);
        stack.add_named(&self.root, Some("unified"));
        stack.add_named(&pair, Some("split"));
        // Keep the halves even as the pane resizes.
        pair.connect_max_position_notify(|p| p.set_position(p.max_position() / 2));
        *self.split.borrow_mut() = Some(Split { stack: stack.clone(), left, right });
        self.recolor();
        stack.upcast()
    }
    pub fn side_by_side(&self) -> bool {
        self.side_by_side.get()
    }
    pub fn set_side_by_side(&self, on: bool) {
        let Some(split) = self.split.borrow().as_ref().map(|s| s.stack.clone()) else { return };
        self.side_by_side.set(on);
        split.set_visible_child_name(if on { "split" } else { "unified" });
        if on {
            let text = self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), false);
            self.render_split(&text);
        }
    }
    fn render_split(&self, text: &str) {
        let split = self.split.borrow();
        let Some(split) = split.as_ref() else { return };
        let columns = columns(text);
        for (side, lines) in [(&split.left, &columns.left), (&split.right, &columns.right)] {
            let body: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
            side.buffer.set_text(&body.join("\n"));
            let kinds: Vec<Option<&str>> = lines.iter().map(|l| l.kind).collect();
            let words: Vec<(usize, usize, usize, &str)> =
                lines.iter().enumerate().filter_map(|(i, l)| l.word.map(|(a, b, k)| (i, a, b, k))).collect();
            tag_lines(&side.buffer, &kinds, &words);
            show_numbers(&side.gutter, &side.numbers, lines.iter().map(|l| l.number.map(|n| n.to_string()).unwrap_or_default()).collect());
        }
    }
    pub fn recolor(&self) {
        let palette = nyx_theme::palette();
        let mut buffers = vec![self.buffer.clone()];
        if let Some(split) = self.split.borrow().as_ref() {
            buffers.push(split.left.buffer.clone());
            buffers.push(split.right.buffer.clone());
        }
        for buffer in &buffers {
            self.recolor_buffer(buffer, &palette);
        }
    }
    fn recolor_buffer(&self, buffer: &sourceview5::Buffer, palette: &nyx_theme::Palette) {
        if let Some(scheme) = scheme(palette) {
            buffer.set_style_scheme(Some(&scheme));
        }
        let rgba = |c: &str| gtk::gdk::RGBA::parse(c).unwrap_or(gtk::gdk::RGBA::BLACK);
        let (bg, accent, danger) = (rgba(&palette.bg), rgba(&palette.accent), rgba(&palette.danger));
        let tint = |c: &gtk::gdk::RGBA, t: f32| {
            gtk::gdk::RGBA::new(
                bg.red() * (1.0 - t) + c.red() * t,
                bg.green() * (1.0 - t) + c.green() * t,
                bg.blue() * (1.0 - t) + c.blue() * t,
                1.0,
            )
        };
        let table = buffer.tag_table();
        if let Some(tag) = table.lookup("addition") {
            tag.set_foreground_rgba(Some(&accent));
            tag.set_paragraph_background_rgba(Some(&tint(&accent, 0.10)));
        }
        if let Some(tag) = table.lookup("deletion") {
            tag.set_foreground_rgba(Some(&danger));
            tag.set_paragraph_background_rgba(Some(&tint(&danger, 0.10)));
        }
        // The words that changed within a changed line, a step stronger.
        if let Some(tag) = table.lookup("word-add") {
            tag.set_background_rgba(Some(&tint(&accent, 0.30)));
        }
        if let Some(tag) = table.lookup("word-del") {
            tag.set_background_rgba(Some(&tint(&danger, 0.30)));
        }
        if let Some(tag) = table.lookup("filler") {
            tag.set_paragraph_background_rgba(Some(&tint(&rgba(&palette.dim_text), 0.05)));
        }
        for name in ["hunk", "meta"] {
            if let Some(tag) = table.lookup(name) {
                tag.set_foreground(Some(&palette.dim_text));
            }
        }
    }
    pub fn set_text(&self, text: &str) {
        let current = self.buffer.text(&self.buffer.start_iter(), &self.buffer.end_iter(), false);
        // The same diff again (a refresh) keeps the scroll position and the selection.
        if current == text {
            return;
        }
        let adjustment = self.root.vadjustment();
        let scroll = adjustment.value();
        let keep = !current.is_empty() && lines_match(&current, text);
        self.buffer.set_text(text);
        let (kinds, numbers) = annotate(text);
        let words: Vec<(usize, usize, usize, &str)> =
            word_spans(text).into_iter().map(|(line, from, to, add)| (line, from, to, if add { "word-add" } else { "word-del" })).collect();
        tag_lines(&self.buffer, &kinds, &words);
        show_numbers(&self.gutter, &self.numbers, numbers);
        if self.side_by_side.get() {
            self.render_split(text);
        }
        // A changed diff of the same file stays where the reader was.
        if keep {
            gtk::glib::idle_add_local_once(move || adjustment.set_value(scroll));
        }
    }
    pub fn range(&self) -> Option<(usize, usize)> {
        self.buffer
            .selection_bounds()
            .map(|(start, end)| (start.line() as usize, end.line() as usize + usize::from(end.line_offset() > 0)))
    }
    pub fn cursor_line(&self) -> usize {
        self.buffer.iter_at_offset(self.buffer.cursor_position()).line() as usize
    }
}

/// Two diffs of the same file: their first lines (`diff --git a/… b/…`) agree.
fn lines_match(a: &str, b: &str) -> bool {
    a.lines().next() == b.lines().next()
}

/// Each line's tag, and its old and new line numbers in the file, as narrow as they fit.
fn annotate(text: &str) -> (Vec<Option<&'static str>>, Vec<String>) {
    let mut kinds = Vec::new();
    let mut lines: Vec<Option<(Option<u64>, Option<u64>)>> = Vec::new();
    let (mut old, mut new) = (0u64, 0u64);
    let mut in_hunk = false;
    let mut header = false;
    let mut any = false;
    for line in text.lines() {
        let (kind, number) = if line.starts_with("diff ") {
            in_hunk = false;
            header = true;
            (Some("meta"), None)
        } else if line.starts_with("@@") {
            // @@ -old,count +new,count @@
            let mut parts = line.split_whitespace().skip(1);
            let start = |p: Option<&str>| p.and_then(|p| p.get(1..)?.split(',').next()?.parse().ok()).unwrap_or(0);
            old = start(parts.next());
            new = start(parts.next());
            in_hunk = true;
            any = true;
            (Some("hunk"), None)
        } else if !in_hunk {
            header |= line.starts_with("--- ");
            (header.then_some("meta"), None)
        } else if line.starts_with('+') {
            new += 1;
            (Some("addition"), Some((None, Some(new - 1))))
        } else if line.starts_with('-') {
            old += 1;
            (Some("deletion"), Some((Some(old - 1), None)))
        } else if line.starts_with('\\') {
            (Some("meta"), None)
        } else {
            old += 1;
            new += 1;
            (None, Some((Some(old - 1), Some(new - 1))))
        };
        kinds.push(kind);
        lines.push(number);
    }
    if !any {
        return (kinds, Vec::new());
    }
    let digits = (old.max(new).max(1) as f64).log10().floor() as usize + 1;
    let column = |n: Option<u64>| n.map_or_else(|| " ".repeat(digits), |n| format!("{n:>digits$}"));
    let numbers =
        lines.into_iter().map(|n| n.map_or_else(String::new, |(o, n)| format!("{} {}", column(o), column(n)))).collect();
    (kinds, numbers)
}

/// The changed part of `new` against `old`, as char ranges in each: what's left after
/// their common start and end. None when the lines share too little for it to help.
fn changed_span(old: &str, new: &str) -> Option<((usize, usize), (usize, usize))> {
    let a: Vec<char> = old.chars().collect();
    let b: Vec<char> = new.chars().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
    // Lines that share only a letter or two were rewritten, not edited.
    if a == b || prefix + suffix < 3 || (prefix + suffix) * 4 < a.len().min(b.len()) {
        return None;
    }
    Some(((prefix, a.len() - suffix), (prefix, b.len() - suffix)))
}
/// Pairs each run of removed lines with the added lines after it, line by line, and
/// marks the words that changed: (line, from, to, added), in chars from the line's start.
fn word_spans(text: &str) -> Vec<(usize, usize, usize, bool)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let removed = |l: &str| l.starts_with('-') && !l.starts_with("---");
    let added = |l: &str| l.starts_with('+') && !l.starts_with("+++");
    while i < lines.len() {
        if !removed(lines[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && removed(lines[i]) {
            i += 1;
        }
        let middle = i;
        while i < lines.len() && added(lines[i]) {
            i += 1;
        }
        for k in 0..(middle - start).min(i - middle) {
            let (old, new) = (start + k, middle + k);
            if let Some(((a0, a1), (b0, b1))) = changed_span(&lines[old][1..], &lines[new][1..]) {
                if a1 > a0 {
                    out.push((old, a0 + 1, a1 + 1, false));
                }
                if b1 > b0 {
                    out.push((new, b0 + 1, b1 + 1, true));
                }
            }
        }
    }
    out
}
struct Column {
    text: String,
    kind: Option<&'static str>,
    number: Option<u64>,
    /// The changed words: (from, to, tag), in chars.
    word: Option<(usize, usize, &'static str)>,
}
#[derive(Default)]
struct Columns {
    left: Vec<Column>,
    right: Vec<Column>,
}
/// A unified diff laid out as old (left) and new (right), lines aligned: removals face
/// the additions that replaced them, and blank filler keeps unchanged lines level.
fn columns(text: &str) -> Columns {
    let mut out = Columns::default();
    let lines: Vec<&str> = text.lines().collect();
    let (mut old, mut new) = (0u64, 0u64);
    let mut in_hunk = false;
    let mut i = 0;
    let both = |out: &mut Columns, text: &str, kind: Option<&'static str>, o: Option<u64>, n: Option<u64>| {
        out.left.push(Column { text: text.into(), kind, number: o, word: None });
        out.right.push(Column { text: text.into(), kind, number: n, word: None });
    };
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("diff ") {
            in_hunk = false;
            both(&mut out, line, Some("meta"), None, None);
        } else if line.starts_with("@@") {
            let mut parts = line.split_whitespace().skip(1);
            let start = |p: Option<&str>| p.and_then(|p| p.get(1..)?.split(',').next()?.parse().ok()).unwrap_or(0);
            old = start(parts.next());
            new = start(parts.next());
            in_hunk = true;
            both(&mut out, line, Some("hunk"), None, None);
        } else if !in_hunk || line.starts_with('\\') {
            both(&mut out, line, Some("meta"), None, None);
        } else if line.starts_with('-') || line.starts_with('+') {
            let start = i;
            while i < lines.len() && lines[i].starts_with('-') {
                i += 1;
            }
            let middle = i;
            while i < lines.len() && lines[i].starts_with('+') {
                i += 1;
            }
            let (removed, added) = (&lines[start..middle], &lines[middle..i]);
            for k in 0..removed.len().max(added.len()) {
                let span = match (removed.get(k), added.get(k)) {
                    (Some(a), Some(b)) => changed_span(&a[1..], &b[1..]),
                    _ => None,
                };
                match removed.get(k) {
                    Some(text) => {
                        old += 1;
                        let word = span.filter(|s| s.0.1 > s.0.0).map(|s| (s.0.0, s.0.1, "word-del"));
                        out.left.push(Column { text: text[1..].into(), kind: Some("deletion"), number: Some(old - 1), word });
                    }
                    None => out.left.push(Column { text: String::new(), kind: Some("filler"), number: None, word: None }),
                }
                match added.get(k) {
                    Some(text) => {
                        new += 1;
                        let word = span.filter(|s| s.1.1 > s.1.0).map(|s| (s.1.0, s.1.1, "word-add"));
                        out.right.push(Column { text: text[1..].into(), kind: Some("addition"), number: Some(new - 1), word });
                    }
                    None => out.right.push(Column { text: String::new(), kind: Some("filler"), number: None, word: None }),
                }
            }
            continue;
        } else {
            old += 1;
            new += 1;
            let body = line.get(1..).unwrap_or_default();
            both(&mut out, body, None, Some(old - 1), Some(new - 1));
        }
        i += 1;
    }
    out
}

// GtkSourceView paints its gutter from a style scheme, independently of GTK CSS.
// Cache a scheme generated from the same semantic palette as the window.
fn scheme(palette: &nyx_theme::Palette) -> Option<sourceview5::StyleScheme> {
    use std::cell::RefCell;
    thread_local! {static LAST: RefCell<Option<(nyx_theme::Palette, sourceview5::StyleScheme)>> = const {RefCell::new(None)};}
    LAST.with(|last| {
        if let Some((old, scheme)) = last.borrow().as_ref()
            && old == palette
        {
            return Some(scheme.clone());
        }
        let directory = nyx_core::paths::cache_dir().join("sourceview");
        std::fs::create_dir_all(&directory).ok()?;
        let bg = gtk::glib::markup_escape_text(&palette.bg);
        let text = gtk::glib::markup_escape_text(&palette.text);
        let dim = gtk::glib::markup_escape_text(&palette.dim_text);
        let accent = gtk::glib::markup_escape_text(&palette.accent);
        let bg_rgba = gtk::gdk::RGBA::parse(&palette.bg).ok()?;
        let accent_rgba = gtk::gdk::RGBA::parse(&palette.accent).ok()?;
        let channel = |a: f32, b: f32| ((a * 0.82 + b * 0.18) * 255.0).round() as u8;
        let selection = format!(
            "#{:02x}{:02x}{:02x}",
            channel(bg_rgba.red(), accent_rgba.red()),
            channel(bg_rgba.green(), accent_rgba.green()),
            channel(bg_rgba.blue(), accent_rgba.blue())
        );
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<style-scheme id="nyx-git-diff" name="Git" version="1.0">
<style name="text" foreground="{text}" background="{bg}"/>
<style name="line-numbers" foreground="{dim}" background="{bg}"/>
<style name="current-line-number" foreground="{accent}" background="{bg}"/>
<style name="current-line" background="{bg}"/>
<style name="selection" foreground="{text}" background="{selection}"/>
<style name="cursor" foreground="{accent}"/>
</style-scheme>"#
        );
        nyx_core::cmd::atomic_write(&directory.join("nyx-git-diff.xml"), &xml).ok()?;
        let manager = sourceview5::StyleSchemeManager::new();
        manager.append_search_path(&directory.to_string_lossy());
        manager.force_rescan();
        let scheme = manager.scheme("nyx-git-diff")?;
        *last.borrow_mut() = Some((palette.clone(), scheme.clone()));
        Some(scheme)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_lines_from_hunk_headers() {
        let (kinds, numbers) = annotate("diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -10,3 +10,3 @@\n a\n-b\n+c\n");
        assert_eq!(kinds, [Some("meta"), Some("meta"), Some("meta"), Some("hunk"), None, Some("deletion"), Some("addition")]);
        assert_eq!(numbers[4].split_whitespace().collect::<Vec<_>>(), ["10", "10"]);
        assert_eq!(numbers[5].trim(), "11");
        assert_eq!(numbers[6].trim(), "11");
        assert!(annotate("Select a file.").1.is_empty());
    }
    #[test]
    fn pairs_changed_lines_and_marks_their_changed_words() {
        let diff = "@@ -1,3 +1,3 @@\n keep\n-let x = 1;\n+let x = 2;\n-gone\n";
        assert_eq!(word_spans(diff), [(2, 9, 10, false), (3, 9, 10, true)]);
        let c = columns(diff);
        assert_eq!(c.left.len(), c.right.len());
        assert_eq!(c.left.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["@@ -1,3 +1,3 @@", "keep", "let x = 1;", "gone"]);
        assert_eq!(c.right.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["@@ -1,3 +1,3 @@", "keep", "let x = 2;", ""]);
        assert_eq!(c.right[3].kind, Some("filler"));
        assert_eq!((c.left[2].number, c.right[2].number), (Some(2), Some(2)));
        assert_eq!(c.left[2].word, Some((8, 9, "word-del")));
        assert!(changed_span("abc", "xyz").is_none());
    }
}
