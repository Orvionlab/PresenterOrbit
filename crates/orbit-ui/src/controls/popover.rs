use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    anchored, deferred, div, prelude::*, AnyElement, App, Bounds, ElementId, Pixels, Point, Window,
};
use gpui_base::actions::Cancel;
use gpui_base::{ElementExt, Popup};

use super::{child_id, Handler};
use crate::ActiveTheme;

/// A panel that opens below its trigger. The owner keeps whether it is open and closes it in `on_dismiss`, which
/// runs on a press outside the panel and its trigger (so a press on the trigger can toggle). Keys belong to the content.
#[derive(IntoElement)]
pub struct Popover {
    id: ElementId,
    trigger: AnyElement,
    content: Option<AnyElement>,
    on_dismiss: Option<Handler<()>>,
    anchor: gpui::Anchor,
}

impl Popover {
    pub fn new(id: impl Into<ElementId>, trigger: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            trigger: trigger.into_any_element(),
            content: None,
            on_dismiss: None,
            anchor: gpui::Anchor::TopLeft,
        }
    }

    /// The panel, shown while the owner is open. Leave it out to keep the popover closed.
    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    /// Which corner of the panel meets the trigger: a top corner opens below it, a bottom corner above it.
    pub fn anchor(mut self, anchor: gpui::Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn((), &mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let trigger = window
            .use_keyed_state(child_id(&self.id, "trigger-bounds"), cx, |_, _| {
                Rc::new(Cell::new(Bounds::<Pixels>::default()))
            })
            .read(cx)
            .clone();
        let placed = trigger.clone();
        let escape = self.on_dismiss.clone();
        let viewport = window.viewport_size();
        let backdrop = match (self.content.is_some(), self.on_dismiss) {
            (true, Some(dismiss)) => {
                // A nested select or color picker can extend outside the panel. Its hitbox
                // blocks clicks on this backdrop, which sits beneath all popup surfaces.
                Some(
                    deferred(
                        anchored().position(Point::default()).child(
                            div()
                                .id(child_id(&self.id, "dismiss"))
                                .w(viewport.width)
                                .h(viewport.height)
                                .capture_any_mouse_down(move |event, window, cx| {
                                    if !window.has_active_prompt()
                                        && !trigger.get().contains(&event.position)
                                    {
                                        dismiss((), window, cx);
                                    }
                                }),
                        ),
                    )
                    .priority(gpui_base::POPUP_PRIORITY - 1),
                )
            }
            _ => None,
        };
        let popup = Popup::new(
            child_id(&self.id, "popup"),
            div()
                .w_full()
                .on_prepaint(move |bounds, _, _| placed.set(bounds))
                .child(self.trigger),
        )
        .anchor(self.anchor)
        .offset(theme.metrics.space_2xs)
        .when_some(self.content, |this, content| {
            this.content(
                deferred(
                    div()
                        .occlude()
                        // Escape closes the popover and not the dialog around it, and Enter and Space reach the
                        // content's own key handler instead of an enclosing dialog's default action.
                        .key_context("PopoverContent")
                        .on_action(move |_: &Cancel, window, cx| {
                            if let Some(dismiss) = &escape {
                                dismiss((), window, cx);
                            }
                        })
                        .child(content),
                )
                .priority(gpui_base::POPUP_PRIORITY),
            )
        });
        div().child(popup).children(backdrop)
    }
}
