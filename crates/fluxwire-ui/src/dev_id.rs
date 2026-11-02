//! `dev_tag`: tags a view element with a stable id, role, label and enabled state for the
//! semantic UI tree (M9.5, ADR-018). With `dev-mcp` off it is an identity function over borrowed
//! arguments, so render paths gain no code and no allocation. With it on, the tag records the
//! element's bounds, clip and focus each frame in a registry owned by the window.

use gpui::{FocusHandle, IntoElement};

pub use crate::dev_tree::{Meta, Role};

#[cfg(not(feature = "dev-mcp"))]
#[inline(always)]
pub fn dev_tag(el: impl IntoElement, _id: &str, _role: Role, _label: &str) -> impl IntoElement {
    el
}

#[cfg(not(feature = "dev-mcp"))]
#[inline(always)]
pub fn dev_tag_state(
    el: impl IntoElement,
    _id: &str,
    _role: Role,
    _label: &str,
    _enabled: bool,
    _focus: Option<&FocusHandle>,
) -> impl IntoElement {
    el
}

/// Adds `on_click` to `el` and registers it as what `ui.click_element` runs for this element.
/// Take the handler by value: with `dev-mcp` off this is just `el.on_click(on_click)`.
#[cfg(not(feature = "dev-mcp"))]
#[inline(always)]
pub fn dev_click<E, F>(
    el: E,
    _id: &str,
    _role: Role,
    _label: &str,
    _enabled: bool,
    on_click: F,
) -> E
where
    E: gpui::StatefulInteractiveElement,
    F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
{
    el.on_click(on_click)
}

#[cfg(not(feature = "dev-mcp"))]
#[inline(always)]
pub fn begin_frame(_cx: &mut gpui::App) {}

#[cfg(feature = "dev-mcp")]
pub use on::*;

#[cfg(feature = "dev-mcp")]
mod on {
    use super::*;
    use crate::dev_tree::{Frame, Meta, Record, Rect};
    use gpui::{
        AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, LayoutId,
        Pixels, Window,
    };

    /// The tagged elements of the frame being painted, and the open-parent stack.
    #[derive(Default)]
    pub struct Registry {
        /// The completed previous frame, which `ui.tree` reads.
        pub last: Frame,
        building: Frame,
        stack: Vec<usize>,
        handlers: std::collections::HashMap<String, Handler>,
        last_handlers: std::collections::HashMap<String, Handler>,
        /// When `begin_frame` last ran, for the frame interval.
        last_begin: Option<std::time::Instant>,
        /// The first input since the last frame, for input-to-paint latency.
        input_at: Option<std::time::Instant>,
        pub latency: crate::dev_audit::Latency,
    }

    /// Notes that input arrived; the next frame measures how long it took to paint.
    pub fn note_input(cx: &mut App) {
        let reg = cx.default_global::<Registry>();
        reg.input_at.get_or_insert_with(std::time::Instant::now);
    }

    /// The latest frame interval and input-to-paint latency.
    pub fn latency(cx: &App) -> crate::dev_audit::Latency {
        cx.try_global::<Registry>()
            .map(|r| r.latency)
            .unwrap_or_default()
    }

    pub type Handler = std::rc::Rc<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App)>;

    /// The click handler registered for `id` in the latest frame.
    pub fn click_handler(cx: &App, id: &str) -> Option<Handler> {
        let r = cx.try_global::<Registry>()?;
        let m = if r.building.records.is_empty() {
            &r.last_handlers
        } else {
            &r.handlers
        };
        m.get(id).cloned()
    }

    impl gpui::Global for Registry {}

    /// The latest complete frame. Requests run on the UI thread between draws, so the frame
    /// under construction is whole by then; `last` covers a window that has not redrawn.
    pub fn last_frame(cx: &App) -> Frame {
        cx.try_global::<Registry>()
            .map(|r| {
                if r.building.records.is_empty() {
                    r.last.clone()
                } else {
                    r.building.clone()
                }
            })
            .unwrap_or_default()
    }

    /// Starts a new frame; the previous one becomes readable.
    pub fn begin_frame(cx: &mut App) {
        let reg = cx.default_global::<Registry>();
        let now = std::time::Instant::now();
        if let Some(prev) = reg.last_begin.replace(now) {
            let ms = now.duration_since(prev).as_secs_f32() * 1000.;
            // A long gap is an idle window, not a slow frame.
            if ms < 250. {
                reg.latency.frame_ms = Some(ms);
            }
        }
        if let Some(at) = reg.input_at.take() {
            reg.latency.input_to_paint_ms = Some(now.duration_since(at).as_secs_f32() * 1000.);
        }
        let done = std::mem::take(&mut reg.building);
        let handlers = std::mem::take(&mut reg.handlers);
        if !done.records.is_empty() {
            reg.last = done;
            reg.last_handlers = handlers;
        }
        reg.stack.clear();
    }

    fn rect(b: Bounds<Pixels>) -> Rect {
        Rect::new(
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height),
        )
    }

    /// An invisible child that fills its parent and records the parent's frame data.
    pub fn marker(
        id: &str,
        role: Role,
        label: &str,
        enabled: bool,
        click: Option<Handler>,
        meta: Meta,
    ) -> impl IntoElement {
        use gpui::{Styled, canvas};
        let (id, label) = (id.to_string(), label.to_string());
        canvas(
            move |bounds, window, cx| {
                let record = Record {
                    id,
                    role,
                    label,
                    enabled,
                    bounds: rect(bounds),
                    clip: rect(window.content_mask().bounds),
                    focused: false,
                    parent: cx.default_global::<Registry>().stack.last().copied(),
                    meta,
                };
                let reg = cx.default_global::<Registry>();
                let idx = reg.building.push(record);
                if let Some(h) = click {
                    let id = reg.building.records[idx].id.clone();
                    reg.handlers.insert(id, h);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    pub fn dev_tag(el: impl IntoElement, id: &str, role: Role, label: &str) -> impl IntoElement {
        dev_tag_state(el, id, role, label, true, None)
    }

    pub fn dev_click<E, F>(
        el: E,
        id: &str,
        role: Role,
        label: &str,
        enabled: bool,
        on_click: F,
    ) -> impl IntoElement + use<E, F>
    where
        E: gpui::StatefulInteractiveElement + IntoElement,
        F: Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    {
        let shared: Handler = std::rc::Rc::new(on_click);
        let for_el = shared.clone();
        Tagged {
            click: Some(shared),
            ..tagged(
                el.on_click(move |e, w, c| for_el(e, w, c)),
                id,
                role,
                label,
                enabled,
                None,
            )
        }
    }

    pub fn dev_tag_state(
        el: impl IntoElement,
        id: &str,
        role: Role,
        label: &str,
        enabled: bool,
        focus: Option<&FocusHandle>,
    ) -> impl IntoElement {
        tagged(el, id, role, label, enabled, focus)
    }

    fn tagged(
        el: impl IntoElement,
        id: &str,
        role: Role,
        label: &str,
        enabled: bool,
        focus: Option<&FocusHandle>,
    ) -> Tagged {
        Tagged {
            click: None,
            child: el.into_any_element(),
            id: id.to_string(),
            role,
            label: label.to_string(),
            enabled,
            focus: focus.cloned(),
            meta: Meta::default(),
        }
    }

    struct Tagged {
        child: AnyElement,
        id: String,
        role: Role,
        label: String,
        enabled: bool,
        focus: Option<FocusHandle>,
        click: Option<Handler>,
        meta: Meta,
    }

    impl IntoElement for Tagged {
        type Element = Self;
        fn into_element(self) -> Self {
            self
        }
    }

    impl Element for Tagged {
        type RequestLayoutState = ();
        type PrepaintState = ();

        fn id(&self) -> Option<ElementId> {
            None
        }

        fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
            None
        }

        fn request_layout(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            window: &mut Window,
            cx: &mut App,
        ) -> (LayoutId, ()) {
            (self.child.request_layout(window, cx), ())
        }

        fn prepaint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            bounds: Bounds<Pixels>,
            _: &mut (),
            window: &mut Window,
            cx: &mut App,
        ) {
            let mut meta = self.meta.clone();
            if self.focus.is_some() && meta.focusable.is_none() {
                meta.focusable = Some(true);
            }
            let record = Record {
                id: self.id.clone(),
                role: self.role,
                label: self.label.clone(),
                enabled: self.enabled,
                bounds: rect(bounds),
                clip: rect(window.content_mask().bounds),
                focused: self.focus.as_ref().is_some_and(|f| f.is_focused(window)),
                parent: cx.default_global::<Registry>().stack.last().copied(),
                meta,
            };
            let reg = cx.default_global::<Registry>();
            let idx = reg.building.push(record);
            if let Some(h) = self.click.clone() {
                let id = reg.building.records[idx].id.clone();
                reg.handlers.insert(id, h);
            }
            reg.stack.push(idx);
            self.child.prepaint(window, cx);
            cx.default_global::<Registry>().stack.pop();
        }

        fn paint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            _: Bounds<Pixels>,
            _: &mut (),
            _: &mut (),
            window: &mut Window,
            cx: &mut App,
        ) {
            self.child.paint(window, cx);
        }
    }
}

/// Drop-in for `.on_click(handler)` anywhere in a chain: `el.dev_click(id, role, label, enabled,
/// handler)` adds the handler and, with `dev-mcp` on, an invisible absolutely positioned marker
/// child that records the element's bounds, clip and handler. Off, it is exactly `.on_click`.
pub trait DevClick: gpui::StatefulInteractiveElement + gpui::ParentElement + Sized {
    #[cfg(not(feature = "dev-mcp"))]
    #[inline(always)]
    fn dev_click<F>(self, _id: &str, _role: Role, _label: &str, _enabled: bool, on_click: F) -> Self
    where
        F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    {
        self.on_click(on_click)
    }

    #[cfg(feature = "dev-mcp")]
    fn dev_click<F>(self, id: &str, role: Role, label: &str, enabled: bool, on_click: F) -> Self
    where
        F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    {
        let shared: Handler = std::rc::Rc::new(on_click);
        let for_el = shared.clone();
        self.on_click(move |e, w, c| for_el(e, w, c))
            .child(on::marker(
                id,
                role,
                label,
                enabled,
                Some(shared),
                Meta::default(),
            ))
    }
}

impl<T: gpui::StatefulInteractiveElement + gpui::ParentElement> DevClick for T {}

/// Tags a non-clickable element (an input, a list, a dock) without wrapping it.
pub trait DevTag: gpui::ParentElement + Sized {
    #[cfg(not(feature = "dev-mcp"))]
    #[inline(always)]
    fn dev_tag(self, _id: &str, _role: Role, _label: &str, _enabled: bool) -> Self {
        self
    }

    #[cfg(feature = "dev-mcp")]
    fn dev_tag(self, id: &str, role: Role, label: &str, enabled: bool) -> Self {
        self.child(on::marker(id, role, label, enabled, None, Meta::default()))
    }
}

impl<T: gpui::ParentElement> DevTag for T {}

/// Like [`DevClick::dev_click`] and [`DevTag::dev_tag`] but also records [`Meta`] for the audit
/// (M9.7). Off, they are exactly `.on_click` and the identity.
pub trait DevMeta: gpui::StatefulInteractiveElement + gpui::ParentElement + Sized {
    #[cfg(not(feature = "dev-mcp"))]
    #[inline(always)]
    fn dev_click_meta<F>(
        self,
        _id: &str,
        _role: Role,
        _label: &str,
        _enabled: bool,
        _meta: Meta,
        on_click: F,
    ) -> Self
    where
        F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    {
        self.on_click(on_click)
    }

    #[cfg(feature = "dev-mcp")]
    fn dev_click_meta<F>(
        self,
        id: &str,
        role: Role,
        label: &str,
        enabled: bool,
        meta: Meta,
        on_click: F,
    ) -> Self
    where
        F: Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    {
        let shared: Handler = std::rc::Rc::new(on_click);
        let for_el = shared.clone();
        self.on_click(move |e, w, c| for_el(e, w, c))
            .child(on::marker(id, role, label, enabled, Some(shared), meta))
    }
}

impl<T: gpui::StatefulInteractiveElement + gpui::ParentElement> DevMeta for T {}

/// Tags a non-clickable element and records its [`Meta`].
pub trait DevTagMeta: gpui::ParentElement + Sized {
    #[cfg(not(feature = "dev-mcp"))]
    #[inline(always)]
    fn dev_tag_meta(
        self,
        _id: &str,
        _role: Role,
        _label: &str,
        _enabled: bool,
        _meta: Meta,
    ) -> Self {
        self
    }

    #[cfg(feature = "dev-mcp")]
    fn dev_tag_meta(self, id: &str, role: Role, label: &str, enabled: bool, meta: Meta) -> Self {
        self.child(on::marker(id, role, label, enabled, None, meta))
    }
}

impl<T: gpui::ParentElement> DevTagMeta for T {}
