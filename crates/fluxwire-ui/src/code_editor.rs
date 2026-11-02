//! The reusable multi-line code editor (M9.06, SPEC §17.3, §17.11).
//!
//! `CodeEditor` is an entity with its own focus, scroll and caret. Text arrives through the
//! platform input handler (`EntityInputHandler`), so IME composition, dead keys and the emoji
//! picker work. Movement, clipboard and undo reuse the `TextInput` actions (the key context is
//! `TextInput CodeEditor`), so `keymap.toml` rebinding applies; Enter, Tab and paging are read
//! from key events; the editor-only actions live in `actions.rs` (`editor: …`).
//!
//! The owner of the document keeps the text: it calls `set_text` when its copy changes and
//! listens for `CodeEditorEvent::Changed`; `revision` lets it tell its own echoes apart.
//! Only the rows in view are shaped and painted (`code_editor_element.rs`).

use crate::code_editor_ops::{self as ops, Plan, Tabs};
use crate::completion::{
    CompletionProvider, Ranked, Trigger, expand, is_ident_char, rank, word_start,
};
use crate::display_map::DisplayMap;
use crate::highlight::{Highlighter, Lang, Spans, Tok, highlight};
use crate::input::TextBuffer;
use crate::input_keys::*;
use crate::language_config::{Context as LangContext, LanguageConfig};
use fluxwire_format::{EditorConfig, SoftWrap, Whitespace};
use gpui::anchored as anchor_at;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point, Render,
    ScrollWheelEvent, SharedString, Subscription, UTF16Selection, Window, div, point, prelude::*,
    px,
};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

const BLINK: Duration = Duration::from_millis(500);
pub(crate) const CARET_ROWS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeEditorEvent {
    /// A chip was clicked; the payload is its `tag`.
    ChipClicked(usize),
    Changed,
    Focused,
    Blurred,
    CaretMoved,
}

/// A right-aligned label on the row holding `offset` (Docs: what a protected block is and
/// where to edit it). Clicking it emits `ChipClicked(tag)`.
#[derive(Clone, Debug)]
pub struct Chip {
    pub offset: usize,
    pub label: SharedString,
    pub tag: usize,
}

impl EventEmitter<CodeEditorEvent> for CodeEditor {}

/// The open completion menu.
pub(crate) struct Menu {
    pub items: Vec<Ranked>,
    pub selected: usize,
    /// Start of the replaced word, for anchoring.
    pub anchor: usize,
}

pub struct CodeEditor {
    pub(crate) focus: FocusHandle,
    pub(crate) buf: TextBuffer,
    lang: Lang,
    lang_cfg: LanguageConfig,
    pub(crate) cfg: EditorConfig,
    pub(crate) placeholder: SharedString,
    pub(crate) read_only: bool,
    /// Ranges the user cannot change (Docs editor); the caret may pass through them.
    read_only_ranges: Vec<Range<usize>>,
    pub(crate) marked: Option<Range<usize>>,
    goal_col: Option<usize>,
    // layout state shared with the element
    pub(crate) top_row: f32,
    pub(crate) scroll_x: Pixels,
    pub(crate) bounds: Option<Bounds<Pixels>>,
    pub(crate) char_w: Pixels,
    pub(crate) line_h: Pixels,
    pub(crate) gutter_w: Pixels,
    pub(crate) wrap_cols: Option<usize>,
    dm: DisplayMap,
    dm_key: (u64, Option<usize>, usize),
    spans: Spans,
    spans_rev: u64,
    /// Incremental parse tree for single-grammar languages; `None` uses the full path.
    hl: Option<Highlighter>,
    hl_rev: u64,
    reveal: bool,
    pub(crate) caret_visible: bool,
    blink_epoch: usize,
    drag: bool,
    has_focus: bool,
    provider: Option<Rc<dyn CompletionProvider>>,
    pub(crate) menu: Option<Menu>,
    var_check: Option<VarCheck>,
    chips: Vec<Chip>,
    /// Where each chip was last painted, for clicks.
    pub(crate) chip_rects: Vec<(Bounds<Pixels>, usize)>,
    notice: Option<SharedString>,
    wired: bool,
    _subs: Vec<Subscription>,
}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

impl CodeEditor {
    pub fn new(lang: Lang, cx: &mut Context<Self>) -> Self {
        let mut cfg = EditorConfig::default();
        if lang == Lang::FluxMd {
            cfg.soft_wrap = SoftWrap::None;
        }
        Self {
            focus: cx.focus_handle().tab_stop(true),
            buf: TextBuffer::new("", true),
            lang,
            lang_cfg: LanguageConfig::for_lang(lang),
            cfg,
            placeholder: SharedString::default(),
            read_only: false,
            read_only_ranges: Vec::new(),
            marked: None,
            goal_col: None,
            top_row: 0.,
            scroll_x: px(0.),
            bounds: None,
            char_w: px(8.),
            line_h: px(20.),
            gutter_w: px(44.),
            wrap_cols: None,
            dm: DisplayMap::new("", None, 2),
            dm_key: (u64::MAX, None, 0),
            spans: Vec::new(),
            spans_rev: u64::MAX,
            hl: Highlighter::supports(lang).then(|| Highlighter::new(lang)),
            hl_rev: u64::MAX,
            reveal: false,
            caret_visible: false,
            blink_epoch: 0,
            drag: false,
            has_focus: false,
            provider: None,
            menu: None,
            var_check: None,
            chips: Vec::new(),
            chip_rects: Vec::new(),
            notice: None,
            wired: false,
            _subs: Vec::new(),
        }
    }

    // ---- builder-style configuration ----

    pub fn language(mut self, lang: Lang) -> Self {
        self.set_language(lang);
        self
    }
    pub fn config(mut self, cfg: EditorConfig) -> Self {
        self.cfg = cfg;
        self
    }
    pub fn line_numbers(mut self, on: bool) -> Self {
        self.cfg.line_numbers = on;
        self
    }
    pub fn soft_wrap(mut self, w: SoftWrap) -> Self {
        self.cfg.soft_wrap = w;
        self
    }
    pub fn tab_size(mut self, size: usize, hard_tabs: bool) -> Self {
        self.cfg.tab_size = size.max(1);
        self.cfg.hard_tabs = hard_tabs;
        self
    }
    pub fn show_whitespace(mut self, w: Whitespace) -> Self {
        self.cfg.show_whitespace = w;
        self
    }
    pub fn indent_guides(mut self, on: bool) -> Self {
        self.cfg.indent_guides = on;
        self
    }
    pub fn auto_close(mut self, on: bool) -> Self {
        self.cfg.auto_close = on;
        self
    }
    pub fn completion_provider(mut self, p: Rc<dyn CompletionProvider>) -> Self {
        self.provider = Some(p);
        self
    }
    pub fn read_only(mut self, on: bool) -> Self {
        self.read_only = on;
        self
    }
    pub fn placeholder(mut self, p: impl Into<SharedString>) -> Self {
        self.placeholder = p.into();
        self
    }
    pub fn read_only_ranges(mut self, r: Vec<Range<usize>>) -> Self {
        self.read_only_ranges = r;
        self
    }

    pub fn set_language(&mut self, lang: Lang) {
        self.lang = lang;
        self.lang_cfg = LanguageConfig::for_lang(lang);
        self.spans_rev = u64::MAX;
        self.hl = Highlighter::supports(lang).then(|| Highlighter::new(lang));
        self.hl_rev = u64::MAX;
    }
    /// Takes the `[editor]` defaults for this editor's language (creation and settings reload).
    pub fn apply_settings(&mut self, s: &fluxwire_format::EditorSettings, cx: &mut Context<Self>) {
        let cfg = s.for_language(settings_key(self.lang));
        self.set_config(cfg, cx);
        self.dm_key = (u64::MAX, None, 0);
    }
    pub fn set_config(&mut self, cfg: EditorConfig, cx: &mut Context<Self>) {
        self.cfg = cfg;
        cx.notify();
    }
    pub fn set_placeholder(&mut self, p: impl Into<SharedString>) {
        self.placeholder = p.into();
    }
    pub(crate) fn protected_ranges(&self) -> &[Range<usize>] {
        &self.read_only_ranges
    }
    pub(crate) fn var_check(&self) -> Option<VarCheck> {
        self.var_check.clone()
    }
    pub(crate) fn chips(&self) -> &[Chip] {
        &self.chips
    }
    /// Colours `{{name}}` references: accent when `check(name)` is true, error colour and
    /// underline when not.
    pub fn set_var_check(&mut self, check: Option<VarCheck>) {
        self.var_check = check;
    }
    /// Right-aligned labels drawn on the row holding the offset (Docs: what a block is and
    /// where to edit it).
    pub fn set_chips(&mut self, chips: Vec<Chip>) {
        self.chips = chips;
    }
    pub fn set_read_only_ranges(&mut self, r: Vec<Range<usize>>) {
        self.read_only_ranges = r;
    }
    pub fn lang(&self) -> Lang {
        self.lang
    }

    // ---- state ----

    pub fn text(&self) -> &str {
        self.buf.text()
    }
    pub fn revision(&self) -> u64 {
        self.buf.revision()
    }
    pub fn selection(&self) -> (usize, usize) {
        self.buf.selection()
    }
    pub fn cursor(&self) -> usize {
        self.buf.head()
    }
    pub fn has_focus(&self) -> bool {
        self.has_focus
    }
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }
    pub fn notice(&self) -> Option<&SharedString> {
        self.notice.as_ref()
    }
    pub fn focus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    /// The owner's text changed (not an edit made here): replaces the buffer silently and
    /// keeps the caret where it was, clamped.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.buf.text() == text {
            return;
        }
        let head = self.buf.head();
        self.buf.set_text(text);
        self.buf.move_to(head, false);
        self.marked = None;
        cx.notify();
    }

    /// Replaces the whole text as one undo step (format body); the caret goes to the start.
    pub fn replace_all(&mut self, text: &str, cx: &mut Context<Self>) {
        let len = self.buf.text().len();
        self.apply_plan(
            Plan {
                range: 0..len,
                text: text.to_string(),
                anchor: 0,
                head: 0,
            },
            cx,
        );
    }

    pub fn set_cursor(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.buf.move_to(offset, false);
        self.reveal = true;
        cx.emit(CodeEditorEvent::CaretMoved);
        cx.notify();
    }

    /// 1-based line and column of the caret.
    pub fn line_col(&self) -> (usize, usize) {
        let t = self.buf.text();
        let h = self.buf.head();
        let ls = t[..h].rfind('\n').map_or(0, |i| i + 1);
        (
            t[..ls].matches('\n').count() + 1,
            t[ls..h].chars().count() + 1,
        )
    }

    /// Moves to `line[:column]` (1-based; clamped). Returns false for text that is not a
    /// location.
    pub fn go_to(&mut self, spec: &str, cx: &mut Context<Self>) -> bool {
        let mut it = spec.trim().splitn(2, ':');
        let Some(line) = it.next().and_then(|l| l.trim().parse::<usize>().ok()) else {
            return false;
        };
        let col = it
            .next()
            .and_then(|c| c.trim().parse::<usize>().ok())
            .unwrap_or(1);
        let t = self.buf.text();
        let mut start = 0;
        for _ in 1..line.max(1) {
            match t[start..].find('\n') {
                Some(i) => start += i + 1,
                None => break,
            }
        }
        let end = t[start..].find('\n').map_or(t.len(), |i| start + i);
        let off = t[start..end]
            .char_indices()
            .nth(col.saturating_sub(1))
            .map_or(end, |(i, _)| start + i);
        self.set_cursor(off, cx);
        true
    }

    pub fn toggle_soft_wrap(&mut self, cx: &mut Context<Self>) {
        self.cfg.soft_wrap = match self.cfg.soft_wrap {
            SoftWrap::None => SoftWrap::EditorWidth,
            SoftWrap::EditorWidth => SoftWrap::None,
        };
        self.dm_key = (u64::MAX, None, 0);
        cx.notify();
    }

    // ---- derived layout ----

    pub(crate) fn display(&mut self) -> &DisplayMap {
        let key = (self.buf.revision(), self.wrap_cols, self.cfg.tab_size);
        if self.dm_key != key {
            self.dm = DisplayMap::new(self.buf.text(), self.wrap_cols, self.cfg.tab_size);
            self.dm_key = key;
        }
        &self.dm
    }

    /// Brings the incremental tree up to date with the buffer.
    fn sync_hl(&mut self) {
        if let Some(h) = self.hl.as_mut()
            && self.hl_rev != self.buf.revision()
        {
            h.update(self.buf.text());
            self.hl_rev = self.buf.revision();
        }
    }

    /// Highlight spans intersecting `range` (absolute offsets). Single-grammar languages
    /// re-parse incrementally and compute only this range; the others (`.flux.md`, plain) cache
    /// a whole-document pass per revision.
    pub(crate) fn spans_in(&mut self, range: Range<usize>) -> Spans {
        let t = self.buf.text();
        let (mut s, mut e) = (range.start.min(t.len()), range.end.min(t.len()));
        while !t.is_char_boundary(s) {
            s -= 1;
        }
        while !t.is_char_boundary(e) {
            e += 1;
        }
        if self.hl.is_some() {
            self.sync_hl();
            return self.hl.as_ref().map(|h| h.spans(s..e)).unwrap_or_default();
        }
        if self.spans_rev != self.buf.revision() {
            self.spans = highlight(self.buf.text(), self.lang);
            self.spans_rev = self.buf.revision();
        }
        self.spans
            .iter()
            .filter(|(a, b, _)| *a < e && *b > s)
            .cloned()
            .collect()
    }

    fn context_at(&mut self, off: usize) -> LangContext {
        // The token holding the character before the caret, found in a small window.
        let (lo, hi) = (off.saturating_sub(4096), off);
        let spans = self.spans_in(lo..hi);
        match spans.iter().find(|(s, e, _)| *s < off && off <= *e) {
            Some((_, _, Tok::String)) => LangContext::String,
            Some((_, _, Tok::Comment)) => LangContext::Comment,
            _ => LangContext::Code,
        }
    }

    fn tabs(&self) -> Tabs {
        Tabs {
            size: self.cfg.tab_size,
            hard: self.cfg.hard_tabs,
        }
    }

    fn range_touches_read_only(&self, r: &Range<usize>) -> bool {
        self.read_only_ranges.iter().any(|ro| {
            if r.is_empty() {
                r.start > ro.start && r.start < ro.end
            } else {
                r.start < ro.end && r.end > ro.start
            }
        })
    }

    fn editable(&self) -> bool {
        !self.read_only
    }

    fn utf16_to_byte(&self, u: usize) -> usize {
        let mut n = 0;
        for (i, c) in self.buf.text().char_indices() {
            if n >= u {
                return i;
            }
            n += c.len_utf16();
        }
        self.buf.text().len()
    }
    fn to_utf16(&self, b: usize) -> usize {
        utf16_len(&self.buf.text()[..b.min(self.buf.text().len())])
    }
    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.to_utf16(r.start)..self.to_utf16(r.end)
    }
    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.utf16_to_byte(r.start)..self.utf16_to_byte(r.end)
    }

    // ---- blink ----

    pub(crate) fn reset_blink(&mut self, cx: &mut Context<Self>) {
        self.blink_epoch += 1;
        self.caret_visible = true;
        if !self.cfg.cursor_blink {
            return;
        }
        let epoch = self.blink_epoch;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK).await;
                let keep = this
                    .update(cx, |t, cx| {
                        if t.blink_epoch != epoch {
                            return false;
                        }
                        t.caret_visible = !t.caret_visible;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    fn wire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.wired = true;
        self._subs = vec![
            cx.on_focus(&self.focus, window, |this, _, cx| {
                this.has_focus = true;
                this.reset_blink(cx);
                cx.emit(CodeEditorEvent::Focused);
                cx.notify();
            }),
            cx.on_blur(&self.focus, window, |this, _, cx| {
                this.has_focus = false;
                this.marked = None;
                this.blink_epoch += 1;
                this.caret_visible = false;
                cx.emit(CodeEditorEvent::Blurred);
                cx.notify();
            }),
        ];
    }

    // ---- editing ----

    fn finish(&mut self, rev: u64, cx: &mut Context<Self>) {
        self.marked = None;
        self.reveal = true;
        self.goal_col = None;
        if self.buf.revision() != rev {
            cx.emit(CodeEditorEvent::Changed);
        }
        cx.emit(CodeEditorEvent::CaretMoved);
        self.reset_blink(cx);
        cx.notify();
    }

    /// Applies a plan as one undo step. Refused (notice, no change) when it would touch a
    /// read-only range.
    pub(crate) fn apply_plan(&mut self, plan: Plan, cx: &mut Context<Self>) {
        let rev = self.buf.revision();
        if plan.is_move() {
            self.buf.select(plan.anchor, plan.head);
            return self.finish(rev, cx);
        }
        if !self.editable() || self.range_touches_read_only(&plan.range) {
            return;
        }
        self.buf.break_group();
        self.buf
            .replace_range_with(plan.range.start, plan.range.end, &plan.text);
        self.buf.select(plan.anchor, plan.head);
        self.buf.break_group();
        self.finish(rev, cx);
    }

    /// Inserts `text` over the selection (or the marked range), applying auto-close and
    /// surround for a single typed character.
    fn insert_text(&mut self, range: Option<Range<usize>>, text: &str, cx: &mut Context<Self>) {
        if !self.editable() {
            return;
        }
        let r = range.or_else(|| self.marked.clone()).unwrap_or_else(|| {
            let (a, b) = self.buf.selection();
            a..b
        });
        if self.range_touches_read_only(&r) {
            return;
        }
        let mut chars = text.chars();
        let typed = match (chars.next(), chars.next(), self.marked.as_ref()) {
            (Some(c), None, None) => Some(c),
            _ => None,
        };
        if let Some(c) = typed
            && self.lang == Lang::FluxMd
            && self.cfg.auto_close
            && let Some(e) = crate::markdown_edit::surround(self.buf.text(), r.clone(), c)
        {
            let plan = Plan {
                anchor: e.range.start + 1,
                head: e.caret,
                range: e.range,
                text: e.text,
            };
            self.apply_plan(plan, cx);
            return self.after_typing(typed, cx);
        }
        if let Some(c) = typed {
            let ctx = self.context_at(r.start);
            if let Some(plan) = ops::type_char(
                &self.lang_cfg,
                self.buf.text(),
                r.clone(),
                c,
                ctx,
                self.cfg.auto_close,
            ) {
                self.apply_plan(plan, cx);
                return self.after_typing(typed, cx);
            }
        }
        let rev = self.buf.revision();
        self.buf.replace_range_with(r.start, r.end, text);
        self.finish(rev, cx);
        self.after_typing(typed, cx);
    }

    // ---- completions ----

    pub fn set_completion_provider(&mut self, p: Option<Rc<dyn CompletionProvider>>) {
        self.provider = p;
        if self.provider.is_none() {
            self.menu = None;
        }
    }

    pub fn completion_open(&self) -> bool {
        self.menu.is_some()
    }

    pub fn completion_labels(&self) -> Vec<String> {
        self.menu
            .as_ref()
            .map(|m| m.items.iter().map(|r| r.item.label.clone()).collect())
            .unwrap_or_default()
    }

    fn close_completion(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// After text was typed: open the menu on a trigger or identifier character, refresh an
    /// open one, close it on anything else.
    fn after_typing(&mut self, typed: Option<char>, cx: &mut Context<Self>) {
        let Some(p) = self.provider.clone() else {
            return;
        };
        let trigger = match typed {
            Some(c) if p.is_trigger_char(c) => Trigger::Char(c),
            Some(c) if is_ident_char(c) => Trigger::Typing,
            _ => return self.close_completion(cx),
        };
        self.refresh_completion(trigger, cx);
    }

    /// Re-queries the provider; opens, updates or closes the menu.
    pub(crate) fn refresh_completion(&mut self, trigger: Trigger, cx: &mut Context<Self>) {
        let Some(p) = self.provider.clone() else {
            return;
        };
        let (a, b) = self.buf.selection();
        if !self.editable() || self.marked.is_some() || a != b {
            return self.close_completion(cx);
        }
        let head = self.buf.head();
        let typed_word = word_start(self.buf.text(), head) < head;
        let wanted = matches!(trigger, Trigger::Manual | Trigger::Char(_))
            || self.menu.is_some()
            || typed_word;
        if !wanted {
            return;
        }
        let items = p.completions(self.buf.text(), head, trigger);
        let Some(first) = items.first() else {
            return self.close_completion(cx);
        };
        let (start, end) = (first.replace.start, first.replace.end);
        if end != head || start > head {
            return self.close_completion(cx);
        }
        let query = self.buf.text()[start..head].to_string();
        let ranked = rank(items, &query);
        // A menu that only repeats the word already typed adds nothing.
        if ranked.is_empty() || (ranked.len() == 1 && ranked[0].item.label == query) {
            return self.close_completion(cx);
        }
        let keep = self
            .menu
            .as_ref()
            .filter(|m| m.anchor == start)
            .and_then(|m| m.items.get(m.selected))
            .map(|r| r.item.label.clone());
        let selected = keep
            .and_then(|l| ranked.iter().position(|r| r.item.label == l))
            .unwrap_or(0);
        self.menu = Some(Menu {
            items: ranked,
            selected,
            anchor: start,
        });
        cx.notify();
    }

    pub(crate) fn menu_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(m) = self.menu.as_mut() {
            let n = m.items.len() as isize;
            m.selected = (m.selected as isize + delta).clamp(0, n - 1) as usize;
            cx.notify();
        }
    }

    /// Replaces the item's range with its expanded text as one undo step.
    pub(crate) fn accept_completion(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let Some(m) = self.menu.take() else { return };
        let Some(r) = m.items.get(index.unwrap_or(m.selected)) else {
            return;
        };
        let ex = expand(&r.item.insert_text);
        let start = r.item.replace.start;
        let plan = Plan {
            range: r.item.replace.clone(),
            anchor: start + ex.selection.start,
            head: start + ex.selection.end,
            text: ex.text,
        };
        self.apply_plan(plan, cx);
        cx.notify();
    }

    pub(crate) fn dismiss_completion(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.menu.is_some();
        self.close_completion(cx);
        open
    }

    fn enter(&mut self, cx: &mut Context<Self>) {
        let (a, b) = self.buf.selection();
        let plan = if self.lang == Lang::FluxMd {
            // prose: continue lists and quotes
            (a == b)
                .then(|| crate::markdown_edit::enter(self.buf.text(), a))
                .flatten()
                .map(|e| Plan {
                    anchor: e.caret,
                    head: e.caret,
                    range: e.range,
                    text: e.text,
                })
        } else {
            None
        };
        let plan =
            plan.unwrap_or_else(|| ops::enter(&self.lang_cfg, self.buf.text(), a..b, self.tabs()));
        self.apply_plan(plan, cx);
    }

    fn tab(&mut self, outdent: bool, cx: &mut Context<Self>) {
        let (a, b) = self.buf.selection();
        if self.lang == Lang::FluxMd && a == b {
            // A list item nests (Tab) or un-nests (Shift+Tab); ordered lists renumber.
            let unit = crate::language_config::indent_unit(self.cfg.tab_size, self.cfg.hard_tabs);
            if let Some(e) = crate::markdown_edit::indent_item(self.buf.text(), a, outdent, &unit) {
                let plan = Plan {
                    anchor: e.caret,
                    head: e.caret,
                    range: e.range,
                    text: e.text,
                };
                self.apply_plan(plan, cx);
                let renumbered = crate::markdown_edit::renumber(self.buf.text());
                if renumbered != self.buf.text() {
                    let caret = self.buf.head();
                    let len = self.buf.text().len();
                    self.apply_plan(
                        Plan {
                            range: 0..len,
                            text: renumbered,
                            anchor: caret,
                            head: caret,
                        },
                        cx,
                    );
                }
                return;
            }
        }
        let plan = ops::tab(self.buf.text(), a..b, outdent, self.tabs());
        self.apply_plan(plan, cx);
    }

    pub(crate) fn toggle_comment(&mut self, cx: &mut Context<Self>) {
        let (a, b) = self.buf.selection();
        if let Some(p) = ops::toggle_comment(&self.lang_cfg, self.buf.text(), a..b) {
            self.apply_plan(p, cx);
        }
    }

    pub(crate) fn line_op(
        &mut self,
        f: impl Fn(&str, Range<usize>) -> Option<Plan>,
        cx: &mut Context<Self>,
    ) {
        let (a, b) = self.buf.selection();
        if let Some(p) = f(self.buf.text(), a..b) {
            self.apply_plan(p, cx);
        }
    }

    /// The bracket next to the caret and its partner, found outside strings and comments
    /// (decided from the highlight spans).
    pub(crate) fn bracket_pair(&mut self) -> Option<(usize, usize)> {
        let head = self.buf.head();
        if self.hl.is_some() {
            self.sync_hl();
            let h = self.hl.as_ref()?;
            let skip = |o: usize| h.in_string_or_comment(o);
            return ops::matching_bracket(&self.lang_cfg, self.buf.text(), head, &skip);
        }
        let spans = self.spans_in(0..self.buf.text().len());
        let skip = move |o: usize| {
            spans
                .iter()
                .any(|(s, e, t)| *s <= o && o < *e && matches!(t, Tok::String | Tok::Comment))
        };
        ops::matching_bracket(&self.lang_cfg, self.buf.text(), head, &skip)
    }

    pub(crate) fn jump_to_bracket(&mut self, cx: &mut Context<Self>) {
        if let Some((a, b)) = self.bracket_pair() {
            let to = if self.buf.head() == a || self.buf.head() == a + 1 {
                b
            } else {
                a
            };
            self.set_cursor(to, cx);
        }
    }

    // ---- movement ----

    fn move_vertical(&mut self, rows: isize, extend: bool) {
        let head = self.buf.head();
        let goal = match self.goal_col {
            Some(g) => g,
            None => {
                let c = self.display().col_of(head);
                self.goal_col = Some(c);
                c
            }
        };
        let to = self.display().vertical(head, rows, goal);
        self.buf.move_to(to, extend);
        self.reveal = true;
    }

    fn page_rows(&self) -> isize {
        let h = self.bounds.map_or(px(400.), |b| b.size.height);
        ((f32::from(h) / f32::from(self.line_h)) as isize - 1).max(1)
    }

    pub(crate) fn run(&mut self, op: crate::text_input::Op, cx: &mut Context<Self>) {
        use crate::text_input::Op;
        if self.menu.is_some() {
            match op {
                Op::Up(false) => return self.menu_move(-1, cx),
                Op::Down(false) => return self.menu_move(1, cx),
                Op::Backspace
                | Op::Delete
                | Op::DeleteWordBack
                | Op::DeleteWordForward
                | Op::DeleteToLineStart => {}
                _ => self.menu = None,
            }
        }
        let rev = self.buf.revision();
        let editable = self.editable();
        let mut keep_goal = false;
        match op {
            Op::Left(e) => self.buf.left(e),
            Op::Right(e) => self.buf.right(e),
            Op::WordLeft(e) => self.buf.word_left_by(e),
            Op::WordRight(e) => self.buf.word_right_by(e),
            Op::LineStart(e) => {
                let to = ops::home(self.buf.text(), self.buf.head());
                self.buf.move_to(to, e);
            }
            Op::LineEnd(e) => self.buf.to_line_end(e),
            Op::DocStart(e) => self.buf.to_doc_start(e),
            Op::DocEnd(e) => self.buf.to_doc_end(e),
            Op::Up(e) => {
                keep_goal = true;
                self.move_vertical(-1, e)
            }
            Op::Down(e) => {
                keep_goal = true;
                self.move_vertical(1, e)
            }
            Op::SelectAll => self.buf.select_all(),
            Op::Copy => {
                if let Some(s) = self.buf.copy() {
                    cx.write_to_clipboard(ClipboardItem::new_string(s));
                }
            }
            Op::Cut => {
                let (a, b) = self.buf.selection();
                if editable
                    && a != b
                    && !self.range_touches_read_only(&(a..b))
                    && let Some(s) = self.buf.cut()
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(s));
                }
            }
            Op::Paste => {
                if editable && let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    let (a, b) = self.buf.selection();
                    if !self.range_touches_read_only(&(a..b)) {
                        if self.lang == Lang::FluxMd
                            && let Some(e) =
                                crate::markdown_edit::paste_link(self.buf.text(), a..b, &t)
                        {
                            return self.apply_plan(
                                Plan {
                                    range: e.range,
                                    text: e.text,
                                    anchor: e.caret,
                                    head: e.caret,
                                },
                                cx,
                            );
                        }
                        self.buf.paste(&t);
                    }
                }
            }
            Op::Undo => {
                if editable {
                    self.buf.undo();
                }
            }
            Op::Redo => {
                if editable {
                    self.buf.redo();
                }
            }
            Op::Backspace
            | Op::Delete
            | Op::DeleteWordBack
            | Op::DeleteWordForward
            | Op::DeleteToLineStart => {
                if editable {
                    let (a, b) = self.buf.selection();
                    let head = self.buf.head();
                    let (lo, hi) = match op {
                        Op::Backspace if a == b => (self.buf.prev_boundary(head), head),
                        Op::Delete if a == b => (head, self.buf.next_boundary(head)),
                        Op::DeleteWordBack if a == b => (self.buf.word_left(head), head),
                        Op::DeleteWordForward if a == b => (head, self.buf.word_right(head)),
                        Op::DeleteToLineStart if a == b => (self.buf.line_start(head), head),
                        _ => (a, b),
                    };
                    if self.range_touches_read_only(&(lo..hi)) {
                        return;
                    }
                    if matches!(op, Op::Backspace)
                        && a == b
                        && self.cfg.auto_close
                        && let Some(p) = ops::backspace_pair(&self.lang_cfg, self.buf.text(), head)
                    {
                        return self.apply_plan(p, cx);
                    }
                    match op {
                        Op::Backspace => self.buf.backspace(),
                        Op::Delete => self.buf.delete(),
                        Op::DeleteWordBack => self.buf.delete_word_back(),
                        Op::DeleteWordForward => self.buf.delete_word_forward(),
                        _ => self.buf.delete_to_line_start(),
                    };
                }
            }
            Op::ContextMenu => {}
        }
        self.reveal = true;
        self.finish_keep(rev, keep_goal, cx);
        if self.menu.is_some() {
            self.refresh_completion(Trigger::Typing, cx);
        }
    }

    fn finish_keep(&mut self, rev: u64, keep_goal: bool, cx: &mut Context<Self>) {
        let goal = self.goal_col;
        self.finish(rev, cx);
        if keep_goal {
            self.goal_col = goal;
        }
    }

    // ---- keys the actions table does not own ----

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if m.control || m.alt || m.platform || m.function && k.key != "enter" {
            return;
        }
        if self.menu.is_some() {
            let handled = match k.key.as_str() {
                "enter" | "tab" if !m.shift => {
                    self.accept_completion(None, cx);
                    true
                }
                "pageup" => {
                    self.menu_move(-10, cx);
                    true
                }
                "pagedown" => {
                    self.menu_move(10, cx);
                    true
                }
                _ => false,
            };
            if handled {
                cx.stop_propagation();
                return;
            }
        }
        match k.key.as_str() {
            "enter" if !m.shift => {
                if self.editable() {
                    self.enter(cx);
                    cx.stop_propagation();
                }
            }
            "tab" => {
                if self.editable() {
                    self.tab(m.shift, cx);
                    cx.stop_propagation();
                }
            }
            "pageup" | "pagedown" => {
                let rows = self.page_rows() * if k.key == "pageup" { -1 } else { 1 };
                self.move_vertical(rows, m.shift);
                self.top_row = (self.top_row + rows as f32).max(0.);
                self.finish_keep(self.buf.revision(), true, cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    // ---- mouse ----

    pub(crate) fn offset_at(&mut self, pos: Point<Pixels>) -> usize {
        let Some(b) = self.bounds else { return 0 };
        let rows = self.display().row_count();
        let y = f32::from(pos.y - b.top()).max(0.);
        let row = ((y / f32::from(self.line_h)) + self.top_row) as usize;
        let x = (pos.x - b.left() - self.gutter_w + self.scroll_x).max(px(0.));
        let col = (f32::from(x) / f32::from(self.char_w)).round() as usize;
        self.display()
            .offset_at(row.min(rows.saturating_sub(1)), col)
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        if let Some((_, tag)) = self
            .chip_rects
            .iter()
            .find(|(b, _)| b.contains(&e.position))
        {
            cx.emit(CodeEditorEvent::ChipClicked(*tag));
            return;
        }
        let idx = self.offset_at(e.position);
        let idx = self.buf.snap(idx);
        match e.click_count {
            1 if e.modifiers.shift => self.buf.move_to(idx, true),
            1 => self.buf.move_to(idx, false),
            2 => self.buf.select_word_at(idx),
            _ => self.buf.select_line_at(idx),
        }
        self.drag = true;
        self.marked = None;
        self.goal_col = None;
        self.reset_blink(cx);
        cx.emit(CodeEditorEvent::CaretMoved);
        cx.notify();
    }

    pub(crate) fn drag_to(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let idx = self.offset_at(pos);
        let idx = self.buf.snap(idx);
        self.buf.move_to(idx, true);
        // auto-scroll past an edge
        if let Some(b) = self.bounds {
            if pos.y < b.top() {
                self.top_row = (self.top_row - 1.).max(0.);
            } else if pos.y > b.bottom() {
                self.top_row += 1.;
            }
        }
        cx.notify();
    }

    fn on_scroll(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let d = e.delta.pixel_delta(self.line_h);
        let rows = self.display().row_count() as f32;
        self.top_row =
            (self.top_row - f32::from(d.y) / f32::from(self.line_h)).clamp(0., (rows - 1.).max(0.));
        if self.cfg.soft_wrap == SoftWrap::None {
            self.scroll_x = (self.scroll_x - d.x).max(px(0.));
        }
        cx.notify();
    }

    /// Keeps the caret `CARET_ROWS` rows inside the viewport; called from prepaint.
    pub(crate) fn reveal_caret(&mut self, visible_rows: usize) {
        if !self.reveal {
            return;
        }
        self.reveal = false;
        let head = self.buf.head();
        let row = self.display().row_of(head);
        let margin = CARET_ROWS.min(visible_rows.saturating_sub(1) / 2);
        let top = self.top_row;
        if (row as f32) < top + margin as f32 {
            self.top_row = (row as f32 - margin as f32).max(0.);
        } else if (row + margin + 1) as f32 > top + visible_rows as f32 {
            self.top_row = (row + margin + 1) as f32 - visible_rows as f32;
        }
        if self.cfg.soft_wrap == SoftWrap::None {
            let col = self.display().col_of(head);
            let x = self.char_w * col as f32;
            let w = self
                .bounds
                .map_or(px(400.), |b| b.size.width - self.gutter_w);
            if x < self.scroll_x {
                self.scroll_x = (x - self.char_w * 4.).max(px(0.));
            } else if x + self.char_w * 2. > self.scroll_x + w {
                self.scroll_x = x + self.char_w * 4. - w;
            }
        } else {
            self.scroll_x = px(0.);
        }
    }
}

/// Language name used by `[editor.languages.<lang>]`.
pub fn settings_key(lang: Lang) -> &'static str {
    match lang {
        Lang::Json => "json",
        Lang::Xml => "xml",
        Lang::Yaml => "yaml",
        Lang::Ts => "ts",
        Lang::Js => "js",
        Lang::FluxMd => "markdown",
        Lang::Plain => "text",
    }
}

/// Tells whether a `{{name}}` reference resolves.
pub type VarCheck = Rc<dyn Fn(&str) -> bool>;

const MENU_ROWS: usize = 10;

impl CodeEditor {
    /// The completion menu, anchored under the word being completed.
    fn render_menu(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let b = self.bounds?;
        let (items, selected, anchor) = {
            let m = self.menu.as_ref()?;
            (m.items.clone(), m.selected, m.anchor)
        };
        let (lh, cw, gw, sx, top) = (
            self.line_h,
            self.char_w,
            self.gutter_w,
            self.scroll_x,
            self.top_row,
        );
        let dm = self.display();
        let (row, col) = (dm.row_of(anchor), dm.col_of(anchor));
        let at = point(
            b.left() + gw + cw * col as f32 - sx,
            b.top() + lh * (row as f32 - top + 1.),
        );
        let t = cx.global::<crate::theme::Theme>().clone();
        let first = (selected + 1).saturating_sub(MENU_ROWS);
        let shown: Vec<(usize, Ranked)> = items
            .iter()
            .cloned()
            .enumerate()
            .skip(first)
            .take(MENU_ROWS)
            .collect();
        let doc = items.get(selected).map(|r| {
            let mut s = r.item.detail.clone();
            if !r.item.documentation.is_empty() {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str(&r.item.documentation);
            }
            s
        });
        let mut list = div()
            .flex()
            .flex_col()
            .min_w(px(260.))
            .max_w(px(420.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(t.elevated)
            .border_1()
            .border_color(t.border)
            .text_size(px(12.))
            .font_family(crate::assets::BUFFER_FONT)
            .text_color(t.text);
        for (i, r) in shown {
            let matched = r.matched.clone();
            let label = r.item.label.chars().enumerate().map(move |(ci, c)| {
                div()
                    .when(matched.contains(&(ci as u32)), |d| {
                        d.text_color(t.accent).font_weight(gpui::FontWeight::BOLD)
                    })
                    .child(c.to_string())
            });
            list = list.child(
                div()
                    .id(("completion", i))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px(px(8.))
                    .h(px(22.))
                    .cursor_pointer()
                    .when(i == selected, |d| d.bg(t.element_selected))
                    .hover({
                        let h = t.element_hover;
                        move |s| s.bg(h)
                    })
                    .child(crate::ui::icon(r.item.kind.icon(), 12., t.text_muted))
                    .child(div().flex().flex_row().children(label))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_color(t.text_muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(r.item.detail.clone()),
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.accept_completion(Some(i), cx)),
                    ),
            );
        }
        let side = doc.filter(|d| !d.is_empty()).map(|d| {
            div()
                .ml_1()
                .p_2()
                .max_w(px(280.))
                .rounded(px(6.))
                .bg(t.elevated)
                .border_1()
                .border_color(t.border)
                .text_size(px(12.))
                .text_color(t.text_muted)
                .child(d)
        });
        Some(
            gpui::deferred(
                anchor_at()
                    .position(at)
                    .anchor(gpui::Corner::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_start()
                            .occlude()
                            .child(list)
                            .children(side),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.range_from_utf16(&range);
        adjusted.replace(self.range_to_utf16(&r));
        Some(self.buf.text()[r].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let (a, b) = self.buf.selection();
        Some(UTF16Selection {
            range: self.range_to_utf16(&(a..b)),
            reversed: self.buf.head() < self.buf.anchor(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range.map(|r| self.range_from_utf16(&r));
        // Enter arrives as a lone newline on some platforms; `on_key_down` owns it.
        if matches!(text, "\n" | "\r" | "\r\n") && self.marked.is_none() {
            return;
        }
        self.insert_text(r, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable() {
            return;
        }
        let r = range
            .map(|r| self.range_from_utf16(&r))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| {
                let (a, b) = self.buf.selection();
                a..b
            });
        if self.range_touches_read_only(&r) {
            return;
        }
        let start = self.buf.snap(r.start.min(r.end));
        let rev = self.buf.revision();
        self.buf.replace_range_with(r.start, r.end, text);
        let len = text.len().min(self.buf.head().saturating_sub(start));
        self.marked = (len > 0).then(|| start..start + len);
        if let Some(sel) = selected {
            let base = self.to_utf16(start);
            let a = self.utf16_to_byte(base + sel.start);
            let b = self.utf16_to_byte(base + sel.end);
            self.buf.select(a, b);
        }
        self.reveal = true;
        if self.buf.revision() != rev {
            cx.emit(CodeEditorEvent::Changed);
        }
        self.reset_blink(cx);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let r = self.range_from_utf16(&range);
        let (lh, cw, gw, sx, top) = (
            self.line_h,
            self.char_w,
            self.gutter_w,
            self.scroll_x,
            self.top_row,
        );
        let dm = self.display();
        let row = dm.row_of(r.start);
        let col = dm.col_of(r.start);
        let end_col = dm.col_of(r.end).max(col + 1);
        let y = bounds.top() + lh * (row as f32 - top);
        Some(Bounds::from_corners(
            point(bounds.left() + gw + cw * col as f32 - sx, y),
            point(bounds.left() + gw + cw * end_col as f32 - sx, y + lh),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.bounds?.localize(&p)?;
        let o = self.offset_at(p);
        Some(self.to_utf16(o))
    }
}

macro_rules! bind_ops {
    ($el:expr, $cx:expr; $($ty:ident => $op:expr),* $(,)?) => {{
        let mut el = $el;
        $( el = el.on_action($cx.listener(|t, _: &$ty, _, cx| t.run($op, cx))); )*
        el
    }};
}

impl Render for CodeEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::text_input::Op;
        if !self.wired {
            self.wire(window, cx);
        }
        let menu = self.render_menu(cx);
        let el = div()
            .id("code-editor")
            .key_context("TextInput CodeEditor")
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .cursor(CursorStyle::IBeam)
            .font_family(crate::assets::BUFFER_FONT)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_scroll_wheel(cx.listener(Self::on_scroll));
        let el = bind_ops!(el, cx;
            InputLeft => Op::Left(false), InputRight => Op::Right(false),
            InputWordLeft => Op::WordLeft(false), InputWordRight => Op::WordRight(false),
            InputLineStart => Op::LineStart(false), InputLineEnd => Op::LineEnd(false),
            InputUp => Op::Up(false), InputDown => Op::Down(false),
            InputDocStart => Op::DocStart(false), InputDocEnd => Op::DocEnd(false),
            InputSelectLeft => Op::Left(true), InputSelectRight => Op::Right(true),
            InputSelectWordLeft => Op::WordLeft(true), InputSelectWordRight => Op::WordRight(true),
            InputSelectLineStart => Op::LineStart(true), InputSelectLineEnd => Op::LineEnd(true),
            InputSelectUp => Op::Up(true), InputSelectDown => Op::Down(true),
            InputSelectDocStart => Op::DocStart(true), InputSelectDocEnd => Op::DocEnd(true),
            InputSelectAll => Op::SelectAll, InputCopy => Op::Copy, InputCut => Op::Cut,
            InputPaste => Op::Paste, InputUndo => Op::Undo, InputRedo => Op::Redo,
            InputBackspace => Op::Backspace, InputDelete => Op::Delete,
            InputDeleteWordBack => Op::DeleteWordBack,
            InputDeleteWordForward => Op::DeleteWordForward,
            InputDeleteToLineStart => Op::DeleteToLineStart,
        );
        use crate::actions::*;
        el.on_action(cx.listener(|t, _: &EditorToggleComment, _, cx| t.toggle_comment(cx)))
            .on_action(cx.listener(|t, _: &EditorMoveLineUp, _, cx| {
                t.line_op(|s, r| ops::move_lines(s, r, true), cx)
            }))
            .on_action(cx.listener(|t, _: &EditorMoveLineDown, _, cx| {
                t.line_op(|s, r| ops::move_lines(s, r, false), cx)
            }))
            .on_action(cx.listener(|t, _: &EditorDuplicateLine, _, cx| {
                t.line_op(|s, r| Some(ops::duplicate(s, r)), cx)
            }))
            .on_action(cx.listener(|t, _: &EditorDeleteLine, _, cx| {
                t.line_op(|s, r| Some(ops::delete_lines(s, r)), cx)
            }))
            .on_action(cx.listener(|t, _: &EditorJumpToBracket, _, cx| t.jump_to_bracket(cx)))
            .on_action(cx.listener(|t, _: &EditorToggleSoftWrap, _, cx| t.toggle_soft_wrap(cx)))
            .on_action(cx.listener(|t, _: &EditorShowCompletions, _, cx| {
                t.refresh_completion(Trigger::Manual, cx)
            }))
            .on_action(cx.listener(|t, _: &EditorDismissCompletions, _, cx| {
                if !t.dismiss_completion(cx) {
                    cx.propagate();
                }
            }))
            .child(crate::code_editor_element::CodeEditorElement {
                editor: cx.entity(),
            })
            .children(menu)
    }
}

/// Creates an editor entity for `lang` with the given initial text.
pub fn new_editor(lang: Lang, text: &str, cx: &mut App) -> Entity<CodeEditor> {
    cx.new(|cx| {
        let mut e = CodeEditor::new(lang, cx);
        e.buf = TextBuffer::new(text, true);
        e
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    struct Harness {
        editor: Entity<CodeEditor>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.editor.clone())
        }
    }

    fn setup<'a>(
        cx: &'a mut TestAppContext,
        lang: Lang,
        text: &str,
    ) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(Theme::one_dark());
            crate::actions::install(cx);
        });
        let text = text.to_string();
        let (h, vcx) = cx.add_window_view(|_, cx| Harness {
            editor: cx.new(|cx| CodeEditor::new(lang, cx)),
        });
        let ed = h.read_with(vcx, |h, _| h.editor.clone());
        vcx.update(|window, cx| {
            ed.update(cx, |e, cx| {
                e.set_text(&text, cx);
                e.set_cursor(text.len(), cx);
                e.focus(window);
            })
        });
        vcx.run_until_parked();
        (ed, vcx)
    }

    fn text(e: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
        e.read_with(cx, |e, _| e.text().to_string())
    }

    use crate::theme::Theme;

    #[gpui::test]
    fn typing_goes_through_the_input_handler(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Plain, "");
        cx.simulate_input("héllo");
        assert_eq!(text(&e, cx), "héllo");
    }

    #[gpui::test]
    fn json_brace_enter_opens_an_indented_block(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Json, "");
        cx.simulate_input("{");
        assert_eq!(text(&e, cx), "{}");
        cx.simulate_keystrokes("enter");
        assert_eq!(text(&e, cx), "{\n  \n}");
        assert_eq!(e.read_with(cx, |e, _| e.cursor()), 4);
    }

    #[gpui::test]
    fn backspace_in_empty_pair_deletes_both_and_quote_in_string_is_plain(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Json, "");
        cx.simulate_input("[");
        cx.simulate_keystrokes("backspace");
        assert_eq!(text(&e, cx), "");
        cx.simulate_input("\"");
        assert_eq!(text(&e, cx), "\"\"");
        cx.simulate_input("a");
        cx.simulate_input("\"");
        assert_eq!(text(&e, cx), "\"a\"");
        assert_eq!(e.read_with(cx, |e, _| e.cursor()), 3);
    }

    #[gpui::test]
    fn tab_and_shift_tab_indent_selected_lines(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Plain, "a\nb");
        cx.simulate_keystrokes("secondary-a tab");
        assert_eq!(text(&e, cx), "  a\n  b");
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(text(&e, cx), "a\nb");
    }

    #[gpui::test]
    fn comment_toggle_works_in_scripts_only(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Ts, "let a;");
        cx.simulate_keystrokes("secondary-/");
        assert_eq!(text(&e, cx), "// let a;");
        let (j, cx) = setup(cx, Lang::Json, "{}");
        cx.simulate_keystrokes("secondary-/");
        assert_eq!(text(&j, cx), "{}");
    }

    #[gpui::test]
    fn alt_up_down_move_lines(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Plain, "a\nb");
        cx.simulate_keystrokes("alt-up");
        assert_eq!(text(&e, cx), "b\na");
    }

    #[gpui::test]
    fn up_down_keep_the_goal_column(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Plain, "abcd\nx\nabcd");
        cx.simulate_keystrokes("up up");
        assert_eq!(e.read_with(cx, |e, _| e.cursor()), 4);
        cx.simulate_keystrokes("down down");
        assert_eq!(e.read_with(cx, |e, _| e.cursor()), 11);
    }

    #[gpui::test]
    fn ime_marks_then_commits(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Plain, "ab");
        cx.simulate_keystrokes("left");
        e.update_in(cx, |e, w, cx| {
            e.replace_and_mark_text_in_range(None, "に", None, w, cx)
        });
        assert_eq!(text(&e, cx), "aにb");
        assert!(e.read_with(cx, |e, _| e.marked_range().is_some()));
        e.update_in(cx, |e, w, cx| e.replace_text_in_range(None, "日本", w, cx));
        assert_eq!(text(&e, cx), "a日本b");
        assert!(e.read_with(cx, |e, _| e.marked_range().is_none()));
    }

    #[gpui::test]
    fn read_only_ranges_refuse_edits_but_allow_the_caret(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Plain, "ab\nCD\nef");
        e.update(cx, |e, _| {
            e.set_read_only_ranges(std::iter::once(3..5).collect())
        });
        e.update(cx, |e, cx| e.set_cursor(4, cx));
        let rev = e.read_with(cx, |e, _| e.revision());
        cx.simulate_input("x");
        cx.simulate_keystrokes("backspace");
        assert_eq!(text(&e, cx), "ab\nCD\nef");
        assert_eq!(e.read_with(cx, |e, _| e.revision()), rev);
        e.update(cx, |e, cx| e.set_cursor(2, cx));
        cx.simulate_input("x");
        assert_eq!(text(&e, cx), "abx\nCD\nef");
    }

    #[gpui::test]
    fn read_only_editor_never_changes(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Plain, "a");
        e.update(cx, |e, _| e.read_only = true);
        cx.simulate_input("x");
        cx.simulate_keystrokes("enter tab");
        assert_eq!(text(&e, cx), "a");
    }

    #[gpui::test]
    fn undo_is_one_step_per_plan(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Json, "");
        cx.simulate_input("{");
        cx.simulate_keystrokes("enter");
        cx.simulate_keystrokes("secondary-z");
        assert_eq!(text(&e, cx), "{}");
    }

    #[gpui::test]
    fn set_text_is_silent_and_go_to_line_clamps(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Plain, "one\ntwo\nthree");
        e.update(cx, |e, cx| assert!(e.go_to("2:2", cx)));
        assert_eq!(e.read_with(cx, |e, _| e.cursor()), 5);
        e.update(cx, |e, cx| assert!(e.go_to("99", cx)));
        assert_eq!(e.read_with(cx, |e, _| e.line_col()), (3, 1));
        e.update(cx, |e, cx| assert!(!e.go_to("x", cx)));
        let events = std::rc::Rc::new(std::cell::Cell::new(0));
        let n = events.clone();
        let _sub = e.update(cx, |_, cx| {
            cx.subscribe_self(move |_, ev: &CodeEditorEvent, _| {
                if *ev == CodeEditorEvent::Changed {
                    n.set(n.get() + 1)
                }
            })
        });
        e.update(cx, |e, cx| e.set_text("new", cx));
        assert_eq!(events.get(), 0);
        assert_eq!(text(&e, cx), "new");
    }

    #[gpui::test]
    fn soft_wrap_toggle_changes_the_display(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Plain, &"word ".repeat(200));
        let rows = |e: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
            e.update(cx, |e, _| e.display().row_count())
        };
        assert_eq!(rows(&e, cx), 1);
        e.update(cx, |e, cx| e.toggle_soft_wrap(cx));
        cx.run_until_parked();
        assert!(rows(&e, cx) > 1);
    }

    #[gpui::test]
    fn markdown_enter_continues_lists(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::FluxMd, "- a");
        cx.simulate_keystrokes("enter");
        assert_eq!(text(&e, cx), "- a\n- ");
        cx.simulate_keystrokes("enter");
        assert_eq!(text(&e, cx), "- a\n");
    }

    fn with_script<'a>(
        cx: &'a mut TestAppContext,
        text: &str,
    ) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
        let (e, cx) = setup(cx, Lang::Ts, text);
        e.update(cx, |e, _| {
            e.set_completion_provider(Some(Rc::new(crate::completion::Composite(vec![
                Rc::new(crate::completion_providers::VariablesProvider {
                    vars: vec![("token".into(), "staging.env".into())],
                }),
                Rc::new(crate::completion_providers::ScriptProvider {
                    role: fluxwire_script::Role::Post,
                    variables: vec!["token".into(), "baseUrl".into()],
                }),
                Rc::new(crate::completion_providers::WordsProvider),
            ]))))
        });
        (e, cx)
    }

    #[gpui::test]
    fn dot_opens_the_menu_and_typing_filters_it(cx: &mut TestAppContext) {
        let (e, cx) = with_script(cx, "");
        cx.simulate_input("fw.");
        assert!(e.read_with(cx, |e, _| e.completion_open()));
        let all = e.read_with(cx, |e, _| e.completion_labels());
        assert!(all.contains(&"vars".to_string()) && all.contains(&"response".to_string()));
        cx.simulate_input("ve");
        let some = e.read_with(cx, |e, _| e.completion_labels());
        assert!(some.len() < all.len() && !some.contains(&"response".to_string()));
    }

    #[gpui::test]
    fn enter_accepts_and_is_one_undo_step(cx: &mut TestAppContext) {
        let (e, cx) = with_script(cx, "");
        cx.simulate_input("fw.sle");
        cx.simulate_keystrokes("enter");
        assert_eq!(text(&e, cx), "fw.sleep()");
        assert_eq!(e.read_with(cx, |e, _| e.cursor()), 9);
        assert!(!e.read_with(cx, |e, _| e.completion_open()));
        cx.simulate_keystrokes("secondary-z");
        assert_eq!(text(&e, cx), "fw.sle");
    }

    #[gpui::test]
    fn escape_closes_the_menu_and_arrows_move(cx: &mut TestAppContext) {
        let (e, cx) = with_script(cx, "");
        cx.simulate_input("fw.");
        cx.simulate_keystrokes("down down");
        let sel = e.read_with(cx, |e, _| e.menu.as_ref().unwrap().selected);
        assert_eq!(sel, 2);
        cx.simulate_keystrokes("escape");
        assert!(!e.read_with(cx, |e, _| e.completion_open()));
        assert_eq!(text(&e, cx), "fw.");
    }

    #[gpui::test]
    fn snippet_selects_its_placeholder(cx: &mut TestAppContext) {
        let (e, cx) = with_script(cx, "");
        e.update(cx, |e, _| e.cfg.auto_close = false);
        cx.simulate_input("tes");
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&e, cx), "fw.test(\"name\", () => {\n  \n});");
        let (a, b) = e.read_with(cx, |e, _| e.selection());
        assert_eq!(&text(&e, cx)[a..b], "name");
    }

    #[gpui::test]
    fn variables_after_braces_and_string_args(cx: &mut TestAppContext) {
        let (e, cx) = with_script(cx, "");
        e.update(cx, |e, _| e.cfg.auto_close = false);
        cx.simulate_input("{{");
        assert!(
            e.read_with(cx, |e, _| e.completion_labels())
                .contains(&"token".to_string())
        );
        cx.simulate_keystrokes("enter");
        assert_eq!(text(&e, cx), "{{token}}");
        let (e2, cx) = with_script(cx, "");
        e2.update(cx, |e, _| e.cfg.auto_close = false);
        cx.simulate_input("fw.vars.get(\"");
        let l = e2.read_with(cx, |e, _| e.completion_labels());
        assert_eq!(l[..2], ["token", "baseUrl"]);
    }

    #[gpui::test]
    fn moving_the_caret_or_a_read_only_editor_closes_or_never_opens(cx: &mut TestAppContext) {
        let (e, cx) = with_script(cx, "");
        cx.simulate_input("fw.");
        cx.simulate_keystrokes("left");
        assert!(!e.read_with(cx, |e, _| e.completion_open()));
        let (r, cx) = with_script(cx, "fw");
        r.update(cx, |e, _| e.read_only = true);
        cx.simulate_input(".");
        assert!(!r.read_with(cx, |e, _| e.completion_open()));
    }

    #[gpui::test]
    fn ctrl_space_opens_manually_and_ime_composition_never_opens(cx: &mut TestAppContext) {
        let (e, cx) = with_script(cx, "fw.");
        cx.simulate_keystrokes("ctrl-space");
        assert!(e.read_with(cx, |e, _| e.completion_open()));
        let (i, cx) = with_script(cx, "");
        i.update_in(cx, |e, w, cx| {
            e.replace_and_mark_text_in_range(None, "に", None, w, cx)
        });
        assert!(!i.read_with(cx, |e, _| e.completion_open()));
    }

    #[gpui::test]
    fn painting_with_guides_whitespace_brackets_vars_and_chips_works(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::Json, "{\n\t\"a\": [1, {{x}}],\n  \"b\": {}\n}");
        e.update(cx, |e, cx| {
            let mut c = e.cfg;
            c.show_whitespace = Whitespace::All;
            c.indent_guides = true;
            e.set_config(c, cx);
            e.set_var_check(Some(Rc::new(|n: &str| n == "x")));
            e.set_chips(vec![Chip {
                offset: 2,
                label: "http request · edit in Params/Body".into(),
                tag: 2,
            }]);
            e.set_read_only_ranges(std::iter::once(2..5).collect());
            e.set_cursor(1, cx);
        });
        cx.run_until_parked();
        let pair = e.update(cx, |e, _| e.bracket_pair());
        assert_eq!(pair, Some((0, e.read_with(cx, |e, _| e.text().len() - 1))));
        e.update(cx, |e, cx| e.set_cursor(0, cx));
        cx.run_until_parked();
    }

    #[gpui::test]
    fn markdown_star_surrounds_and_tab_nests_list_items(cx: &mut TestAppContext) {
        let (e, cx) = setup(cx, Lang::FluxMd, "a word b");
        e.update(cx, |e, cx| {
            e.buf.select(2, 6);
            cx.notify();
        });
        cx.simulate_input("*");
        assert_eq!(text(&e, cx), "a *word* b");
        let (l, cx) = setup(cx, Lang::FluxMd, "- a\n- b");
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&l, cx), "- a\n  - b");
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(text(&l, cx), "- a\n- b");
        let (n, cx) = setup(cx, Lang::FluxMd, "1. a\n1. b\n1. c");
        cx.simulate_keystrokes("tab");
        assert_eq!(text(&n, cx), "1. a\n2. b\n  1. c");
    }
}
