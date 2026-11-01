//! The shared single-line text input (M9.02, SPEC §17.11).
//!
//! One entity renders every text field. Printable text arrives only through the platform input
//! handler (`EntityInputHandler`), so IME composition, dead keys, press-and-hold accents and the
//! emoji picker work. Everything else is a `TextInput`-context action from `input_keys`.
//! Enter, Escape and Tab are not consumed; the owner of the field decides what they do.

use crate::actions::KeymapState;
use crate::input::TextBuffer;
use crate::input_keys::*;
use crate::theme::Theme;
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, Corner, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyDownEvent, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, ShapedLine,
    SharedString, Style, Subscription, TextRun, UTF16Selection, UnderlineStyle, Window,
    anchored as anchor_at, deferred, div, fill, point, prelude::*, px, relative, size,
};
use std::ops::Range;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;

const MASK: &str = "•";
const CARET_MARGIN: f32 = 4.;
const BLINK: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    /// The text changed (typing, paste, undo, `set_text` is silent).
    Changed,
    /// Backspace pressed in an empty field (owners use it to step back).
    BackspaceOnEmpty,
    Focus,
    Blur,
    /// An owner-appended context-menu item was chosen.
    MenuItem(usize),
    /// A paste the owner asked to handle itself (`set_paste_intercept`); nothing was inserted.
    PasteIntercepted(String),
}

impl EventEmitter<TextInputEvent> for TextInput {}

/// What a keyboard action or menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Left(bool),
    Right(bool),
    WordLeft(bool),
    WordRight(bool),
    LineStart(bool),
    LineEnd(bool),
    Up(bool),
    Down(bool),
    DocStart(bool),
    DocEnd(bool),
    SelectAll,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    Backspace,
    Delete,
    DeleteWordBack,
    DeleteWordForward,
    DeleteToLineStart,
    ContextMenu,
}

/// A styled span over the text, for `{{var}}` highlighting and unresolved underlines.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoration {
    /// Byte range of the text.
    pub range: Range<usize>,
    pub color: Option<Hsla>,
    pub background: Option<Hsla>,
    pub underline: Option<Hsla>,
    /// Shown near the pointer while it is over the span.
    pub tip: Option<SharedString>,
}

struct Drag {
    /// Word range under the initial double click: the drag extends by words.
    word: Option<(usize, usize)>,
    last: Point<Pixels>,
}

struct Menu {
    at: Point<Pixels>,
    selected: usize,
}

struct MenuItem {
    label: &'static str,
    op: Option<Op>,
    enabled: bool,
    shortcut: Option<String>,
    separator_before: bool,
    extra: Option<usize>,
}

pub struct TextInput {
    focus: FocusHandle,
    menu_focus: FocusHandle,
    buf: TextBuffer,
    placeholder: SharedString,
    read_only: bool,
    disabled: bool,
    masked: bool,
    invalid: Option<SharedString>,
    consume_vertical: bool,
    /// Extra key-context identifier, so the owner's bindings still match (GPUI matches an
    /// identifier against the deepest context only).
    owner_context: Option<&'static str>,
    decorations: Vec<Decoration>,
    strip_whitespace: bool,
    paste_intercept: Option<fn(&str) -> bool>,
    hover_tip: Option<(SharedString, Point<Pixels>)>,
    extra_menu: Vec<&'static str>,
    marked: Option<Range<usize>>,
    scroll_x: Pixels,
    layout: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    drag: Option<Drag>,
    caret_visible: bool,
    blink_epoch: usize,
    menu: Option<Menu>,
    wired: bool,
    _subs: Vec<Subscription>,
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// A text input whose text is mirrored into its owner: `on_change` runs on every edit.
/// For filters and search boxes, where the owner stores a plain `String`.
pub fn bound_input<V: 'static>(
    placeholder: &'static str,
    cx: &mut Context<V>,
    on_change: impl Fn(&mut V, String, &mut Context<V>) + 'static,
) -> Entity<TextInput> {
    let input = cx.new(|cx| TextInput::new(placeholder, cx));
    cx.subscribe(&input, move |this, input, ev: &TextInputEvent, cx| {
        if *ev == TextInputEvent::Changed {
            let text = input.read(cx).text().to_string();
            on_change(this, text, cx);
        }
    })
    .detach();
    input
}

/// Slot for an input created on first use (state structs built without a context).
pub type LazyInput = std::cell::RefCell<Option<Entity<TextInput>>>;

/// The slot's input, created (and wired to `on_change`) on first call.
pub fn lazy_input<V: 'static>(
    slot: &LazyInput,
    placeholder: &'static str,
    cx: &mut Context<V>,
    on_change: impl Fn(&mut V, String, &mut Context<V>) + 'static,
) -> Entity<TextInput> {
    if let Some(i) = slot.borrow().clone() {
        return i;
    }
    let i = bound_input(placeholder, cx, on_change);
    *slot.borrow_mut() = Some(i.clone());
    i
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

impl TextInput {
    /// Creates the input. Focus subscriptions need a `Window`, so they are registered on the
    /// first render; owners can create inputs anywhere they have a context.
    pub fn new(placeholder: &'static str, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let menu_focus = cx.focus_handle();
        Self {
            focus,
            menu_focus,
            buf: TextBuffer::default(),
            placeholder: placeholder.into(),
            read_only: false,
            disabled: false,
            masked: false,
            invalid: None,
            consume_vertical: true,
            owner_context: None,
            decorations: Vec::new(),
            strip_whitespace: false,
            paste_intercept: None,
            hover_tip: None,
            extra_menu: Vec::new(),
            marked: None,
            scroll_x: px(0.),
            layout: None,
            bounds: None,
            drag: None,
            caret_visible: false,
            blink_epoch: 0,
            menu: None,
            wired: false,
            _subs: Vec::new(),
        }
    }

    fn wire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.wired = true;
        self._subs = vec![
            cx.on_focus(&self.focus, window, |this, _, cx| {
                this.reset_blink(cx);
                cx.emit(TextInputEvent::Focus);
                cx.notify();
            }),
            cx.on_blur(&self.focus, window, |this, window, cx| {
                this.handle_blur(window, cx)
            }),
            cx.on_blur(&self.menu_focus, window, |this, window, cx| {
                if this.menu.is_some() && !this.focus.is_focused(window) {
                    this.menu = None;
                    cx.notify();
                }
            }),
        ];
    }

    fn handle_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        self.buf.break_group();
        self.caret_visible = false;
        self.blink_epoch += 1;
        if self.menu.is_some() && !self.menu_focus.is_focused(window) {
            self.menu = None;
        }
        cx.emit(TextInputEvent::Blur);
        cx.notify();
    }

    // ---- public API ----

    pub fn text(&self) -> &str {
        self.buf.text()
    }
    pub fn buffer(&self) -> &TextBuffer {
        &self.buf
    }
    pub fn selection(&self) -> (usize, usize) {
        self.buf.selection()
    }
    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
    }
    pub fn focus(&self, window: &mut Window) {
        if !self.disabled {
            window.focus(&self.focus);
        }
    }
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }
    pub fn scroll_x(&self) -> Pixels {
        self.scroll_x
    }
    pub fn last_bounds(&self) -> Option<Bounds<Pixels>> {
        self.bounds
    }
    pub fn x_for_index(&self, byte: usize) -> Option<Pixels> {
        Some(self.layout.as_ref()?.x_for_index(self.to_disp(byte)) - self.scroll_x)
    }

    /// Replaces the text without emitting `Changed`; the caret goes to the end.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.buf.set_text(text);
        self.marked = None;
        self.scroll_x = px(0.);
        cx.notify();
    }
    /// Replaces the text and emits `Changed` (a programmatic edit the owner should see).
    pub fn replace_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.set_text(text, cx);
        cx.emit(TextInputEvent::Changed);
    }
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.buf.select_all();
        cx.notify();
    }
    pub fn set_selection(&mut self, anchor: usize, head: usize, cx: &mut Context<Self>) {
        self.buf.select(anchor, head);
        cx.notify();
    }
    pub fn set_placeholder(&mut self, p: &'static str, cx: &mut Context<Self>) {
        self.placeholder = p.into();
        cx.notify();
    }
    pub fn set_read_only(&mut self, v: bool, cx: &mut Context<Self>) {
        self.read_only = v;
        cx.notify();
    }
    pub fn set_disabled(&mut self, v: bool, cx: &mut Context<Self>) {
        self.disabled = v;
        cx.notify();
    }
    /// Secret mode: one bullet per grapheme; Copy and Cut are refused; typing and paste work.
    pub fn set_masked(&mut self, v: bool, cx: &mut Context<Self>) {
        self.masked = v;
        cx.notify();
    }
    pub fn is_invalid(&self) -> bool {
        self.invalid.is_some()
    }
    pub fn is_masked(&self) -> bool {
        self.masked
    }
    pub fn set_invalid(&mut self, message: Option<&str>, cx: &mut Context<Self>) {
        self.invalid = message.map(|m| m.to_string().into());
        cx.notify();
    }
    /// When false, Up/Down propagate to the owner (table navigation, modal lists).
    pub fn set_consume_vertical(&mut self, v: bool) {
        self.consume_vertical = v;
    }
    /// The owner's key context (`Modal`, `RequestEditor`…), added next to `TextInput`.
    pub fn set_owner_context(&mut self, ctx: &'static str) {
        self.owner_context = Some(ctx);
    }
    /// Styled spans over the current text (`{{var}}` chips, unresolved underlines).
    pub fn set_decorations(&mut self, d: Vec<Decoration>, cx: &mut Context<Self>) {
        if self.decorations != d {
            self.decorations = d;
            cx.notify();
        }
    }
    /// Drop whitespace from typed and pasted text (URLs).
    pub fn set_strip_whitespace(&mut self, v: bool) {
        self.strip_whitespace = v;
    }
    /// Pastes for which `f` is true are reported as `PasteIntercepted` instead of inserted.
    pub fn set_paste_intercept(&mut self, f: fn(&str) -> bool) {
        self.paste_intercept = Some(f);
    }

    fn clean(&self, s: &str) -> String {
        if self.strip_whitespace {
            s.chars().filter(|c| !c.is_whitespace()).collect()
        } else {
            s.to_string()
        }
    }
    /// Items appended to the context menu below a separator; reported as `MenuItem(index)`.
    pub fn set_menu_extras(&mut self, labels: Vec<&'static str>) {
        self.extra_menu = labels;
    }

    // ---- display mapping (masked fields draw bullets) ----

    fn display(&self) -> SharedString {
        if self.masked {
            MASK.repeat(self.buf.grapheme_count()).into()
        } else {
            self.buf.text().to_string().into()
        }
    }
    fn to_disp(&self, byte: usize) -> usize {
        if self.masked {
            self.buf.text()[..byte].graphemes(true).count() * MASK.len()
        } else {
            byte
        }
    }
    fn disp_to_byte(&self, d: usize) -> usize {
        if self.masked {
            self.buf
                .text()
                .grapheme_indices(true)
                .nth(d / MASK.len())
                .map_or(self.buf.text().len(), |(i, _)| i)
        } else {
            d
        }
    }

    // ---- UTF-16 conversion (the platform speaks UTF-16) ----

    fn to_utf16(&self, byte: usize) -> usize {
        utf16_len(&self.buf.text()[..byte.min(self.buf.text().len())])
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
    fn range_utf16_to_bytes(&self, r: &Range<usize>) -> Range<usize> {
        self.utf16_to_byte(r.start)..self.utf16_to_byte(r.end)
    }
    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.to_utf16(r.start)..self.to_utf16(r.end)
    }

    // ---- blink ----

    fn reset_blink(&mut self, cx: &mut Context<Self>) {
        self.blink_epoch += 1;
        self.caret_visible = true;
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

    // ---- actions ----

    fn can(&self, op: Op, cx: &App) -> bool {
        let sel = self.buf.has_selection();
        match op {
            Op::Undo => !self.read_only && self.buf.can_undo(),
            Op::Redo => !self.read_only && self.buf.can_redo(),
            Op::Cut => !self.read_only && !self.masked && sel,
            Op::Copy => !self.masked && sel,
            Op::Paste => {
                !self.read_only
                    && cx
                        .read_from_clipboard()
                        .and_then(|i| i.text())
                        .is_some_and(|t| !t.is_empty())
            }
            Op::SelectAll => !self.buf.is_empty(),
            Op::Backspace | Op::Delete => !self.read_only,
            _ => true,
        }
    }

    pub fn run(&mut self, op: Op, window: &mut Window, cx: &mut Context<Self>) {
        let rev = self.buf.revision();
        let mut changed = false;
        let editable = !self.read_only && !self.disabled;
        match op {
            Op::Left(e) => self.buf.left(e),
            Op::Right(e) => self.buf.right(e),
            Op::WordLeft(e) => self.buf.word_left_by(e),
            Op::WordRight(e) => self.buf.word_right_by(e),
            Op::LineStart(e) => self.buf.to_line_start(e),
            Op::LineEnd(e) => self.buf.to_line_end(e),
            Op::DocStart(e) => self.buf.to_doc_start(e),
            Op::DocEnd(e) => self.buf.to_doc_end(e),
            Op::Up(e) | Op::Down(e) => {
                if !self.consume_vertical {
                    cx.propagate();
                    return;
                }
                if matches!(op, Op::Up(_)) {
                    self.buf.to_doc_start(e)
                } else {
                    self.buf.to_doc_end(e)
                }
            }
            Op::SelectAll => self.buf.select_all(),
            Op::Copy => {
                if !self.masked
                    && let Some(s) = self.buf.copy()
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(s));
                }
            }
            Op::Cut => {
                if editable
                    && !self.masked
                    && let Some(s) = self.buf.cut()
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(s));
                    changed = true;
                }
            }
            Op::Paste => {
                if editable && let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    if self.paste_intercept.is_some_and(|f| f(&t)) {
                        cx.emit(TextInputEvent::PasteIntercepted(t));
                    } else {
                        changed = self.buf.paste(&self.clean(&t));
                    }
                }
            }
            Op::Undo => changed = editable && self.buf.undo(),
            Op::Redo => changed = editable && self.buf.redo(),
            Op::Backspace => {
                if editable && self.buf.is_empty() {
                    cx.emit(TextInputEvent::BackspaceOnEmpty);
                }
                changed = editable && self.buf.backspace()
            }
            Op::Delete => changed = editable && self.buf.delete(),
            Op::DeleteWordBack => changed = editable && self.buf.delete_word_back(),
            Op::DeleteWordForward => changed = editable && self.buf.delete_word_forward(),
            Op::DeleteToLineStart => changed = editable && self.buf.delete_to_line_start(),
            Op::ContextMenu => {
                let at = self
                    .bounds
                    .map(|b| point(b.left() + self.caret_x(), b.bottom()))
                    .unwrap_or_default();
                self.open_menu(at, window, cx);
            }
        }
        self.marked = None;
        if changed && self.buf.revision() != rev {
            cx.emit(TextInputEvent::Changed);
        }
        self.reset_blink(cx);
        cx.notify();
    }

    fn caret_x(&self) -> Pixels {
        self.x_for_index(self.buf.head()).unwrap_or_default()
    }

    // ---- mouse ----

    fn index_at(&self, pos: Point<Pixels>) -> usize {
        let (Some(b), Some(line)) = (self.bounds, self.layout.as_ref()) else {
            return 0;
        };
        let x = pos.x - b.left() + self.scroll_x;
        let d = if self.buf.is_empty() {
            0
        } else {
            line.closest_index_for_x(x.max(px(0.)))
        };
        self.buf.snap(self.disp_to_byte(d))
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.menu = None;
        window.focus(&self.focus);
        let idx = self.index_at(e.position);
        match e.click_count {
            1 if e.modifiers.shift => self.buf.move_to(idx, true),
            1 => self.buf.move_to(idx, false),
            2 => self.buf.select_word_at(idx),
            _ => self.buf.select_all(),
        }
        let word = (e.click_count == 2).then(|| self.buf.selection());
        self.drag = Some(Drag {
            word,
            last: e.position,
        });
        self.marked = None;
        self.reset_blink(cx);
        self.start_autoscroll(window, cx);
        cx.notify();
    }

    fn on_right_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        window.focus(&self.focus);
        let idx = self.index_at(e.position);
        let (a, b) = self.buf.selection();
        if idx < a || idx > b || a == b {
            self.buf.move_to(idx, false);
        }
        self.open_menu(e.position, window, cx);
    }

    fn on_hover_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.drag.is_some() || self.decorations.iter().all(|d| d.tip.is_none()) {
            return;
        }
        let idx = self.index_at(e.position);
        // `index_at` snaps to the nearest boundary, so check the character under the pointer.
        let tip = self
            .decorations
            .iter()
            .find(|d| d.range.contains(&idx) && d.tip.is_some())
            .and_then(|d| d.tip.clone())
            .map(|t| (t, e.position));
        if tip.as_ref().map(|t| &t.0) != self.hover_tip.as_ref().map(|t| &t.0) {
            self.hover_tip = tip;
            cx.notify();
        }
    }

    fn render_tip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (text, at) = self.hover_tip.clone()?;
        let t = cx.global::<Theme>();
        let (bg, border, fg) = (t.elevated, t.border, t.text);
        Some(
            deferred(
                anchor_at()
                    .position(point(at.x, at.y + px(16.)))
                    .anchor(Corner::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .bg(bg)
                            .border_1()
                            .border_color(border)
                            .text_size(px(12.))
                            .text_color(fg)
                            .child(text),
                    ),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    fn drag_to(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(b) = self.bounds else { return };
        let word = match self.drag.as_mut() {
            Some(d) => {
                d.last = pos;
                d.word
            }
            None => return,
        };
        // Auto-scroll while the pointer is outside the field.
        let over = if pos.x < b.left() {
            pos.x - b.left()
        } else if pos.x > b.right() {
            pos.x - b.right()
        } else {
            px(0.)
        };
        if over != px(0.) {
            self.scroll_x = (self.scroll_x + over.clamp(px(-24.), px(24.)) / 2.).max(px(0.));
        }
        let idx = self.index_at(pos);
        match word {
            Some((s, e)) => {
                let (ws, we) = self.buf.word_range(idx);
                if idx >= s {
                    self.buf.select(s, we.max(e));
                } else {
                    self.buf.select(e, ws);
                }
            }
            None => self.buf.move_to(idx, true),
        }
        cx.notify();
    }

    fn start_autoscroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
                let keep = this
                    .update(cx, |t, cx| {
                        let Some(d) = t.drag.as_ref() else {
                            return false;
                        };
                        let last = d.last;
                        t.drag_to(last, cx);
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

    // ---- context menu ----

    fn menu_items(&self, cx: &App) -> Vec<MenuItem> {
        let keys = cx.try_global::<KeymapState>();
        let sc = |name: &str| keys.and_then(|k| k.label_for(name));
        let mk = |label, op: Op, name: &str, sep| MenuItem {
            label,
            op: Some(op),
            enabled: self.can(op, cx),
            shortcut: sc(name),
            separator_before: sep,
            extra: None,
        };
        let mut v = vec![
            mk("Undo", Op::Undo, "fluxwire::InputUndo", false),
            mk("Redo", Op::Redo, "fluxwire::InputRedo", false),
            mk("Cut", Op::Cut, "fluxwire::InputCut", true),
            mk("Copy", Op::Copy, "fluxwire::InputCopy", false),
            mk("Paste", Op::Paste, "fluxwire::InputPaste", false),
            MenuItem {
                label: "Delete",
                op: Some(Op::Delete),
                enabled: !self.read_only && self.buf.has_selection(),
                shortcut: None,
                separator_before: false,
                extra: None,
            },
            mk(
                "Select All",
                Op::SelectAll,
                "fluxwire::InputSelectAll",
                true,
            ),
        ];
        if self.read_only {
            v.retain(|m| matches!(m.op, Some(Op::Copy | Op::SelectAll)));
            if let Some(f) = v.first_mut() {
                f.separator_before = false;
            }
        }
        for (i, label) in self.extra_menu.iter().enumerate() {
            v.push(MenuItem {
                label,
                op: None,
                enabled: true,
                shortcut: None,
                separator_before: i == 0,
                extra: Some(i),
            });
        }
        v
    }

    fn open_menu(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let first = self
            .menu_items(cx)
            .iter()
            .position(|m| m.enabled)
            .unwrap_or(0);
        self.menu = Some(Menu {
            at,
            selected: first,
        });
        window.focus(&self.menu_focus);
        // A mouse-down that opened the menu may still hand focus back to the field afterwards.
        cx.defer_in(window, |this, window, _| {
            if this.menu.is_some() {
                window.focus(&this.menu_focus);
            }
        });
        cx.notify();
    }

    pub fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            window.focus(&self.focus);
            cx.notify();
        }
    }

    fn activate_menu_item(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.menu_items(cx);
        let Some(item) = items.get(i) else { return };
        if !item.enabled {
            return;
        }
        let (op, extra) = (item.op, item.extra);
        self.close_menu(window, cx);
        if let Some(op) = op {
            self.run(op, window, cx);
        }
        if let Some(x) = extra {
            cx.emit(TextInputEvent::MenuItem(x));
        }
    }

    fn on_menu_key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sel) = self.menu.as_ref().map(|m| m.selected) else {
            return;
        };
        let items = self.menu_items(cx);
        let step = |from: usize, dir: isize| {
            let n = items.len() as isize;
            let mut i = from as isize;
            for _ in 0..n {
                i = (i + dir).rem_euclid(n);
                if items[i as usize].enabled {
                    return i as usize;
                }
            }
            from
        };
        match e.keystroke.key.as_str() {
            "down" => {
                if let Some(m) = self.menu.as_mut() {
                    m.selected = step(sel, 1);
                }
            }
            "up" => {
                if let Some(m) = self.menu.as_mut() {
                    m.selected = step(sel, -1);
                }
            }
            "enter" => self.activate_menu_item(sel, window, cx),
            "escape" => self.close_menu(window, cx),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let t = cx.global::<Theme>();
        let (bg, border, text, muted, hover, sel_bg) = (
            t.elevated,
            t.border,
            t.text,
            t.text_muted,
            t.element_hover,
            t.element_selected,
        );
        let items = self.menu_items(cx);
        let mut col = div()
            .track_focus(&self.menu_focus)
            .on_key_down(
                cx.listener(|this, e: &KeyDownEvent, window, cx| this.on_menu_key(e, window, cx)),
            )
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_menu(window, cx)))
            .occlude()
            .min_w(px(180.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(bg)
            .border_1()
            .border_color(border)
            .text_size(px(13.))
            .text_color(text)
            .flex()
            .flex_col();
        for (i, m) in items.into_iter().enumerate() {
            if m.separator_before {
                col = col.child(div().my(px(4.)).h(px(1.)).bg(border));
            }
            let enabled = m.enabled;
            col = col.child(
                div()
                    .id(("input-menu", i))
                    .flex()
                    .flex_row()
                    .justify_between()
                    .gap_4()
                    .px(px(10.))
                    .py(px(3.))
                    .when(menu.selected == i, |d| d.bg(sel_bg))
                    .when(enabled, |d| d.hover(move |s| s.bg(hover)).cursor_pointer())
                    .when(!enabled, |d| d.text_color(muted).opacity(0.5))
                    .child(m.label)
                    .when_some(m.shortcut, |d, s| d.child(div().text_color(muted).child(s)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_menu_item(i, window, cx)
                    })),
            );
        }
        Some(
            deferred(
                anchor_at()
                    .position(menu.at)
                    .anchor(Corner::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(col),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

// ---- platform text input ----

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.range_utf16_to_bytes(&range);
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
        // Enter reaches the input handler as a lone newline on some platforms; it is the
        // owner's key (submit), never text, in a single-line field.
        if self.read_only || self.disabled || matches!(text, "\n" | "\r" | "\r\n") {
            return;
        }
        let r = range
            .map(|r| self.range_utf16_to_bytes(&r))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| {
                let (a, b) = self.buf.selection();
                a..b
            });
        let text = self.clean(text);
        let rev = self.buf.revision();
        self.buf.replace_range_with(r.start, r.end, &text);
        self.marked = None;
        if self.buf.revision() != rev {
            cx.emit(TextInputEvent::Changed);
        }
        self.reset_blink(cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Enter reaches the input handler as a lone newline on some platforms; it is the
        // owner's key (submit), never text, in a single-line field.
        if self.read_only || self.disabled || matches!(text, "\n" | "\r" | "\r\n") {
            return;
        }
        let r = range
            .map(|r| self.range_utf16_to_bytes(&r))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| {
                let (a, b) = self.buf.selection();
                a..b
            });
        let start = self.buf.snap(r.start.min(r.end));
        let before = self.buf.text().len();
        let text = &self.clean(text);
        self.buf.replace_range_with(r.start, r.end, text);
        let len = text.len().min(self.buf.head() - start.min(self.buf.head()));
        let _ = before;
        self.marked = (len > 0).then(|| start..start + len);
        if let Some(sel) = selected {
            let base = self.to_utf16(start);
            let a = self.utf16_to_byte(base + sel.start);
            let b = self.utf16_to_byte(base + sel.end);
            self.buf.select(a, b);
        }
        cx.emit(TextInputEvent::Changed);
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
        let line = self.layout.as_ref()?;
        let r = self.range_utf16_to_bytes(&range);
        Some(Bounds::from_corners(
            point(
                bounds.left() + line.x_for_index(self.to_disp(r.start)) - self.scroll_x,
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index(self.to_disp(r.end)) - self.scroll_x,
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.bounds?.localize(&p)?;
        Some(self.to_utf16(self.index_at(p)))
    }
}

// ---- element ----

struct TextElement {
    input: Entity<TextInput>,
}

struct Prepaint {
    line: ShapedLine,
    caret: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    scroll: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// Splits `len` bytes of text into runs, applying decorations and the marked underline.
fn build_runs(
    len: usize,
    base: &TextRun,
    decorations: &[Decoration],
    marked: Option<&Range<usize>>,
) -> Vec<TextRun> {
    let mut cuts = vec![0, len];
    for d in decorations {
        cuts.push(d.range.start.min(len));
        cuts.push(d.range.end.min(len));
    }
    if let Some(m) = marked {
        cuts.push(m.start.min(len));
        cuts.push(m.end.min(len));
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| {
            let mut run = TextRun {
                len: w[1] - w[0],
                ..base.clone()
            };
            if let Some(d) = decorations
                .iter()
                .find(|d| d.range.start <= w[0] && w[1] <= d.range.end)
            {
                if let Some(c) = d.color {
                    run.color = c;
                }
                run.background_color = d.background;
                if let Some(u) = d.underline {
                    run.underline = Some(UnderlineStyle {
                        color: Some(u),
                        thickness: px(1.),
                        wavy: false,
                    });
                }
            }
            if marked.is_some_and(|m| m.start <= w[0] && w[1] <= m.end) {
                run.underline = Some(UnderlineStyle {
                    color: Some(run.color),
                    thickness: px(1.),
                    wavy: false,
                });
            }
            run
        })
        .collect()
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let input = self.input.read(cx);
        let t = cx.global::<Theme>();
        let (text_color, placeholder_color, accent) = (t.text, t.text_placeholder, t.accent);
        let focused = input.focus.is_focused(window);
        let sel_bg = if focused {
            t.accent_bg()
        } else {
            Hsla {
                a: t.accent_bg().a * 0.5,
                ..t.accent
            }
        };
        let style = window.text_style();
        let empty = input.buf.is_empty();
        let display = if empty {
            input.placeholder.clone()
        } else {
            input.display()
        };
        let base = TextRun {
            len: display.len(),
            font: style.font(),
            color: if empty { placeholder_color } else { text_color },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if empty || input.masked {
            vec![base]
        } else {
            build_runs(
                display.len(),
                &base,
                &input.decorations,
                input.marked.as_ref(),
            )
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display, font_size, &runs, None);

        let head = if empty {
            0
        } else {
            input.to_disp(input.buf.head())
        };
        let caret_x = line.x_for_index(head);
        let margin = px(CARET_MARGIN);
        let width = bounds.size.width;
        let mut scroll = input.scroll_x;
        if caret_x - scroll < margin {
            scroll = caret_x - margin;
        }
        if caret_x - scroll > width - margin {
            scroll = caret_x - (width - margin);
        }
        let text_w = if empty { px(0.) } else { line.width };
        scroll = scroll.clamp(px(0.), (text_w + margin * 2. - width).max(px(0.)));

        let (sel, caret) = if input.buf.has_selection() && !empty {
            let (a, b) = input.buf.selection();
            let (xa, xb) = (
                line.x_for_index(input.to_disp(a)),
                line.x_for_index(input.to_disp(b)),
            );
            (
                Some(fill(
                    Bounds::from_corners(
                        point(bounds.left() + xa - scroll, bounds.top()),
                        point(bounds.left() + xb - scroll, bounds.bottom()),
                    ),
                    sel_bg,
                )),
                None,
            )
        } else {
            let show = focused && input.caret_visible;
            (
                None,
                show.then(|| {
                    fill(
                        Bounds::new(
                            point(bounds.left() + caret_x - scroll, bounds.top()),
                            size(px(1.), bounds.size.height),
                        ),
                        accent,
                    )
                }),
            )
        };
        Prepaint {
            line,
            caret,
            selection: sel,
            scroll,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        pre: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if self.input.read(cx).drag.is_some() {
            let e = self.input.clone();
            window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                if ev.pressed_button == Some(MouseButton::Left) {
                    e.update(cx, |t, cx| t.drag_to(ev.position, cx));
                }
            });
            let e = self.input.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                e.update(cx, |t, _| t.drag = None);
            });
        }
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            if let Some(q) = pre.selection.take() {
                window.paint_quad(q);
            }
            let origin = point(bounds.left() - pre.scroll, bounds.top());
            let _ = pre.line.paint(origin, window.line_height(), window, cx);
            if let Some(q) = pre.caret.take() {
                window.paint_quad(q);
            }
        });
        let line = pre.line.clone();
        let scroll = pre.scroll;
        self.input.update(cx, |t, _| {
            t.layout = Some(line);
            t.bounds = Some(bounds);
            t.scroll_x = scroll;
        });
    }
}

macro_rules! bind_ops {
    ($el:expr, $cx:expr; $($ty:ident => $op:expr),* $(,)?) => {{
        let mut el = $el;
        $( el = el.on_action($cx.listener(|t, _: &$ty, w, cx| t.run($op, w, cx))); )*
        el
    }};
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.wired {
            self.wire(window, cx);
        }
        let t = cx.global::<Theme>();
        let (error, border) = (t.deleted, t.border);
        let _ = border;
        let focused = self.focus.is_focused(window);
        let menu = self.render_menu(cx);
        let invalid = self.invalid.clone();
        let context = match self.owner_context {
            Some(o) => format!("{CONTEXT} {o}"),
            None => CONTEXT.to_string(),
        };
        let tip = self.render_tip(cx);
        let row = div()
            .id("text-input")
            .key_context(context.as_str())
            .when(!self.disabled, |d| d.track_focus(&self.focus))
            .when(self.disabled, |d| d.opacity(0.5))
            .w_full()
            .overflow_hidden()
            .cursor(CursorStyle::IBeam)
            .when(invalid.is_some(), |d| d.border_b_1().border_color(error))
            .on_mouse_move(cx.listener(Self::on_hover_move))
            .on_hover(cx.listener(|t, hovered: &bool, _, cx| {
                if !*hovered && t.hover_tip.take().is_some() {
                    cx.notify();
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_down))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|t, _: &MouseUpEvent, _, cx| {
                    t.drag = None;
                    cx.notify();
                }),
            );
        let row = bind_ops!(row, cx;
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
            InputContextMenu => Op::ContextMenu,
        );
        let _ = focused;
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(row.child(TextElement { input: cx.entity() }))
            .when_some(invalid, |d, m| {
                d.child(div().text_size(px(11.)).text_color(error).child(m))
            })
            .children(menu)
            .children(tip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};

    struct Harness {
        input: Entity<TextInput>,
        other: FocusHandle,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(div().w(px(200.)).child(self.input.clone()))
                .child(div().id("other").track_focus(&self.other).size(px(10.)))
        }
    }

    fn setup<'a>(
        cx: &'a mut TestAppContext,
        text: &str,
    ) -> (Entity<TextInput>, &'a mut VisualTestContext) {
        let (input, _, cx) = setup_full(cx, text);
        (input, cx)
    }

    fn setup_full<'a>(
        cx: &'a mut TestAppContext,
        text: &str,
    ) -> (Entity<TextInput>, FocusHandle, &'a mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(Theme::one_dark());
            crate::actions::install(cx);
        });
        let (h, vcx) = cx.add_window_view(|_, cx| Harness {
            input: cx.new(|cx| TextInput::new("placeholder", cx)),
            other: cx.focus_handle(),
        });
        let input = h.read_with(vcx, |h, _| h.input.clone());
        let other = h.read_with(vcx, |h, _| h.other.clone());
        let text = text.to_string();
        vcx.update(|window, cx| {
            input.update(cx, |i, cx| {
                i.set_text(&text, cx);
                i.focus(window);
            })
        });
        vcx.run_until_parked();
        (input, other, vcx)
    }

    fn text(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> String {
        input.read_with(cx, |i, _| i.text().to_string())
    }

    fn sel(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> (usize, usize) {
        input.read_with(cx, |i, _| i.selection())
    }

    const WORD: &str = if cfg!(target_os = "macos") {
        "alt"
    } else {
        "ctrl"
    };

    #[gpui::test]
    fn typing_goes_through_the_input_handler_once(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "");
        cx.simulate_input("héllo");
        assert_eq!(text(&input, cx), "héllo");
    }

    #[gpui::test]
    fn selection_and_word_movement(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "foo bar.baz");
        cx.simulate_keystrokes(&format!("{WORD}-left"));
        assert_eq!(sel(&input, cx), (8, 8));
        cx.simulate_keystrokes(&format!("{WORD}-shift-left"));
        assert_eq!(sel(&input, cx), (4, 8));
        cx.simulate_keystrokes("left");
        assert_eq!(sel(&input, cx), (4, 4));
        cx.simulate_keystrokes("shift-right shift-right");
        assert_eq!(sel(&input, cx), (4, 6));
        cx.simulate_keystrokes("secondary-a");
        assert_eq!(sel(&input, cx), (0, 11));
        cx.simulate_keystrokes("home");
        assert_eq!(sel(&input, cx), (0, 0));
        cx.simulate_keystrokes("end");
        assert_eq!(sel(&input, cx), (11, 11));
    }

    #[gpui::test]
    fn clipboard_round_trip_and_empty_selection_copy(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "abc");
        cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
        cx.simulate_keystrokes("secondary-c");
        assert_eq!(
            cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
            Some("sentinel"),
            "copy with no selection does nothing"
        );
        cx.simulate_keystrokes("secondary-a secondary-c end secondary-v");
        assert_eq!(text(&input, cx), "abcabc");
        cx.simulate_keystrokes("secondary-a secondary-x");
        assert_eq!(text(&input, cx), "");
        cx.simulate_keystrokes("secondary-v");
        assert_eq!(text(&input, cx), "abcabc");
    }

    #[gpui::test]
    fn paste_collapses_newlines_in_single_line(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "");
        cx.write_to_clipboard(ClipboardItem::new_string("a\r\nb\n\nc".into()));
        cx.simulate_keystrokes("secondary-v");
        assert_eq!(text(&input, cx), "a b c");
    }

    #[gpui::test]
    fn masked_refuses_copy_and_cut_but_accepts_paste(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "secret");
        input.update(cx, |i, cx| i.set_masked(true, cx));
        cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
        cx.simulate_keystrokes("secondary-a secondary-c secondary-x");
        assert_eq!(text(&input, cx), "secret");
        assert_eq!(
            cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
            Some("sentinel")
        );
        cx.simulate_keystrokes("secondary-v");
        assert_eq!(text(&input, cx), "sentinel");
        input.read_with(cx, |i, cx| {
            assert!(!i.can(Op::Copy, cx) && !i.can(Op::Cut, cx));
            assert_eq!(i.display().chars().count(), "sentinel".len());
        });
    }

    #[gpui::test]
    fn read_only_allows_select_and_copy_only(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "fixed");
        input.update(cx, |i, cx| i.set_read_only(true, cx));
        cx.write_to_clipboard(ClipboardItem::new_string("x".into()));
        cx.simulate_keystrokes("secondary-v backspace");
        cx.simulate_input("zz");
        assert_eq!(text(&input, cx), "fixed");
        cx.simulate_keystrokes("secondary-a secondary-c");
        assert_eq!(
            cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
            Some("fixed")
        );
    }

    #[gpui::test]
    fn undo_redo_stay_inside_the_input(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "");
        cx.simulate_input("abc");
        cx.simulate_keystrokes("secondary-z");
        assert_eq!(text(&input, cx), "");
        let redo = if cfg!(target_os = "macos") {
            "cmd-shift-z"
        } else {
            "ctrl-y"
        };
        cx.simulate_keystrokes(redo);
        assert_eq!(text(&input, cx), "abc");
    }

    #[gpui::test]
    fn ime_commits_once_and_blur_clears_marking(cx: &mut TestAppContext) {
        let (input, other, cx) = setup_full(cx, "ab");
        cx.simulate_keystrokes("left");
        input.update_in(cx, |i, w, cx| {
            i.replace_and_mark_text_in_range(None, "に", None, w, cx);
        });
        assert_eq!(text(&input, cx), "anib".replace("ni", "に"));
        assert!(input.read_with(cx, |i, _| i.marked_range().is_some()));
        input.update_in(cx, |i, w, cx| {
            i.replace_text_in_range(None, "日本", w, cx);
        });
        assert_eq!(text(&input, cx), "a日本b");
        assert!(input.read_with(cx, |i, _| i.marked_range().is_none()));
        input.update_in(cx, |i, w, cx| {
            i.replace_and_mark_text_in_range(None, "x", None, w, cx);
        });
        // The test window is never "active", so GPUI does not dispatch focus events; call the
        // handler the `on_blur` subscription runs.
        let _ = &other;
        input.update_in(cx, |i, w, cx| i.handle_blur(w, cx));
        assert!(input.read_with(cx, |i, _| i.marked_range().is_none()));
    }

    #[gpui::test]
    fn mouse_click_drag_shift_double_triple(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "hello world foo");
        let (b, xs) = input.read_with(cx, |i, _| {
            let b = i.last_bounds().unwrap();
            let xs: Vec<Pixels> = (0..=15)
                .map(|idx| b.left() + i.x_for_index(idx).unwrap())
                .collect();
            (b, xs)
        });
        let x = |idx: usize| xs[idx];
        let y = b.center().y;
        let at = |idx| point(x(idx) + px(0.5), y);
        cx.simulate_click(at(3), Modifiers::default());
        assert_eq!(sel(&input, cx), (3, 3));
        cx.simulate_mouse_down(at(2), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(at(7), Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(at(7), MouseButton::Left, Modifiers::default());
        assert_eq!(sel(&input, cx), (2, 7));
        cx.simulate_click(
            at(9),
            Modifiers {
                shift: true,
                ..Default::default()
            },
        );
        assert_eq!(sel(&input, cx), (2, 9));
        cx.simulate_event(MouseDownEvent {
            position: at(7),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 2,
            first_mouse: false,
        });
        cx.simulate_mouse_up(at(7), MouseButton::Left, Modifiers::default());
        assert_eq!(sel(&input, cx), (6, 11), "double click selects the word");
        cx.simulate_event(MouseDownEvent {
            position: at(7),
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 3,
            first_mouse: false,
        });
        cx.simulate_mouse_up(at(7), MouseButton::Left, Modifiers::default());
        assert_eq!(sel(&input, cx), (0, 15));
    }

    #[gpui::test]
    fn context_menu_opens_moves_caret_and_escape_restores_focus(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "hello world");
        let b = input.read_with(cx, |i, _| i.last_bounds().unwrap());
        input.update(cx, |i, cx| i.set_selection(0, 5, cx));
        let outside = point(b.left() + px(150.), b.center().y);
        cx.simulate_mouse_down(outside, MouseButton::Right, Modifiers::default());
        cx.simulate_mouse_up(outside, MouseButton::Right, Modifiers::default());
        assert!(input.read_with(cx, |i, _| i.menu_open()));
        assert_eq!(
            sel(&input, cx),
            (11, 11),
            "caret moved outside the selection"
        );
        input.read_with(cx, |i, cx| {
            let items = i.menu_items(cx);
            let en = |l: &str| items.iter().find(|m| m.label == l).unwrap().enabled;
            assert!(en("Undo") == i.buf.can_undo());
            assert!(!en("Cut") && !en("Copy") && !en("Delete"));
            assert!(en("Select All"));
            assert_eq!(
                items.iter().map(|m| m.label).collect::<Vec<_>>(),
                [
                    "Undo",
                    "Redo",
                    "Cut",
                    "Copy",
                    "Paste",
                    "Delete",
                    "Select All"
                ]
            );
        });
        cx.simulate_keystrokes("escape");
        assert!(!input.read_with(cx, |i, _| i.menu_open()));
        assert!(cx.update(|w, app| input.read(app).is_focused(w)));
        cx.simulate_input("!");
        assert_eq!(text(&input, cx), "hello world!");
    }

    #[gpui::test]
    fn context_menu_keyboard_activates_select_all(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "abc");
        input.update_in(cx, |i, w, cx| i.run(Op::ContextMenu, w, cx));
        assert!(input.read_with(cx, |i, _| i.menu_open()));
        // Undo, Redo, Cut, Copy, Delete are disabled; Paste may be enabled, Select All last.
        for _ in 0..8 {
            cx.simulate_keystrokes("down");
        }
        let all = input.read_with(cx, |i, cx| {
            i.menu_items(cx)
                .iter()
                .position(|m| m.label == "Select All")
        });
        assert_eq!(
            input.read_with(cx, |i, _| i.menu.as_ref().map(|m| m.selected)),
            all
        );
        cx.simulate_keystrokes("enter");
        assert!(!input.read_with(cx, |i, _| i.menu_open()));
        assert_eq!(sel(&input, cx), (0, 3));
    }

    #[gpui::test]
    fn long_value_keeps_the_caret_visible(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, &"abcdefghij".repeat(50));
        cx.run_until_parked();
        input.read_with(cx, |i, _| {
            let b = i.last_bounds().unwrap();
            assert!(i.scroll_x() > px(0.), "scrolled");
            let x = i.x_for_index(i.buffer().head()).unwrap();
            assert!(
                x >= px(0.) && x <= b.size.width,
                "caret at {x:?} in {:?}",
                b.size.width
            );
        });
        cx.simulate_keystrokes("home");
        cx.run_until_parked();
        assert_eq!(input.read_with(cx, |i, _| i.scroll_x()), px(0.));
    }

    #[gpui::test]
    fn up_down_propagate_when_the_owner_wants_them(cx: &mut TestAppContext) {
        let (input, cx) = setup(cx, "abc");
        cx.simulate_keystrokes("home");
        cx.simulate_keystrokes("down");
        assert_eq!(sel(&input, cx), (3, 3), "down goes to the end");
        input.update(cx, |i, _| i.set_consume_vertical(false));
        cx.simulate_keystrokes("up");
        assert_eq!(sel(&input, cx), (3, 3), "owner consumes up");
    }
}
