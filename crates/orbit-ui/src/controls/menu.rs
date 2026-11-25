use std::rc::Rc;

use gpui::{
    anchored, deferred, div, prelude::*, px, Anchor, AnyElement, App, ElementId, Entity,
    FocusHandle, MouseButton, Pixels, Point, Role, SharedString, Window,
};

use gpui_base::Button;

use super::{child_id, Handler};
use crate::{icon, ActiveTheme};

const MENU_WIDTH: Pixels = px(224.);
const ICON_COLUMN: Pixels = px(38.);
/// The vertical separator's offset inside the icon column.
const SEPARATOR_OFFSET: Pixels = px(31.);
const WINDOW_MARGIN: Pixels = px(8.);

pub struct MenuItem {
    label: SharedString,
    icon: Option<SharedString>,
    checked: bool,
    meta: Option<SharedString>,
    disabled: bool,
    on_click: Option<Handler<()>>,
    content: Option<AnyElement>,
}

impl MenuItem {
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            checked: false,
            meta: None,
            disabled: false,
            on_click: None,
            content: None,
        }
    }

    pub fn icon(mut self, name: impl Into<SharedString>) -> Self {
        self.icon = Some(name.into());
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// Secondary text at the end of the row, such as a shortcut.
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Rich row content, retaining the label for accessibility and the normal menu action.
    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    pub fn on_click(mut self, handler: impl Fn((), &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

pub enum MenuEntry {
    Header(SharedString),
    Control {
        label: SharedString,
        control: AnyElement,
    },
    Item(MenuItem),
    Submenu {
        label: SharedString,
        disabled: bool,
        entries: Vec<MenuEntry>,
    },
    SelectableSubmenu {
        item: MenuItem,
        entries: Vec<MenuEntry>,
    },
    Separator,
}

impl From<MenuItem> for MenuEntry {
    fn from(item: MenuItem) -> Self {
        Self::Item(item)
    }
}

fn fit_menu_x(x: Pixels, width: Pixels, viewport_width: Pixels) -> Pixels {
    x.max(WINDOW_MARGIN)
        .min((viewport_width - width - WINDOW_MARGIN).max(WINDOW_MARGIN))
}

/// Whether the submenu at `path` is open. Opening one replaces its siblings and their descendants.
fn path_is_open(open: &[usize], path: &[usize]) -> bool {
    open.starts_with(path)
}

struct MenuContext {
    open_path: Entity<Vec<usize>>,
    close: Handler<()>,
    viewport_width: Pixels,
    max_height: Pixels,
    submenu_width: Pixels,
}

/// One context menu at a window position. The owner keeps whether it exists and closes it in `on_close`.
#[derive(IntoElement)]
pub struct ContextMenu {
    id: ElementId,
    position: Point<Pixels>,
    entries: Vec<MenuEntry>,
    width: Pixels,
    submenu_width: Option<Pixels>,
    on_close: Handler<()>,
}

impl ContextMenu {
    pub fn new(
        id: impl Into<ElementId>,
        position: Point<Pixels>,
        entries: Vec<MenuEntry>,
        on_close: impl Fn((), &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            position,
            entries,
            width: MENU_WIDTH,
            submenu_width: None,
            on_close: Rc::new(on_close),
        }
    }

    /// A wider menu, such as the application menus that show a shortcut column.
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    pub fn submenu_width(mut self, width: Pixels) -> Self {
        self.submenu_width = Some(width);
        self
    }
}

fn row_base(id: ElementId, disabled: bool, cx: &App) -> Button {
    let theme = *cx.theme();
    Button::new(id)
        .role(Role::MenuItem)
        .disabled(disabled)
        .relative()
        .flex_none()
        .justify_start()
        .min_h(theme.metrics.control_height_compact)
        .pr(theme.metrics.space_sm)
        .text_size(theme.metrics.font_sm)
        .text_color(if disabled {
            theme.colors.disabled_text
        } else {
            theme.colors.text
        })
        .when(!disabled, |this| {
            this.cursor_pointer().hover(|style| {
                style
                    .bg(theme.colors.accent_hover)
                    .text_color(theme.colors.accent_text)
            })
        })
}

fn icon_column() -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(ICON_COLUMN)
        .pr(ICON_COLUMN - SEPARATOR_OFFSET)
}

fn panel(
    id: ElementId,
    width: Pixels,
    max_height: Pixels,
    entries: Vec<AnyElement>,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme();
    div()
        .id(id)
        .role(Role::Menu)
        .occlude()
        .relative()
        .flex()
        .flex_col()
        .w(width)
        .py(px(3.))
        .border_1()
        .border_color(theme.colors.border)
        .bg(theme.colors.popover_bg)
        .text_color(theme.colors.text)
        .shadow(theme.shadow_soft())
        .child(
            div()
                .id("menu-rows")
                .flex()
                .flex_col()
                .min_h_0()
                .max_h(max_height - px(8.))
                .overflow_y_scroll()
                .children(entries),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(SEPARATOR_OFFSET)
                .w(px(1.))
                .bg(theme.colors.menu_separator),
        )
}

fn render_entries(
    id: &ElementId,
    entries: Vec<MenuEntry>,
    path: &[usize],
    menu_x: Pixels,
    menu_width: Pixels,
    context: &Rc<MenuContext>,
    cx: &App,
) -> Vec<gpui::AnyElement> {
    let theme = *cx.theme();
    let open = context.open_path.read(cx).clone();
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let row_id = child_id(id, index.to_string());
            let mut row_path = path.to_vec();
            row_path.push(index);
            match entry {
                MenuEntry::Control { label, control } => {
                    let open_path = context.open_path.clone();
                    let parent = path.to_vec();
                    div()
                        .id(row_id)
                        .debug_selector({
                            let label = label.clone();
                            move || format!("menu-control-{label}")
                        })
                        .flex()
                        .items_center()
                        .min_h(theme.metrics.control_height_compact)
                        .pr(theme.metrics.space_sm)
                        .text_size(theme.metrics.font_sm)
                        .on_hover(move |hovered, _, cx| {
                            if *hovered && *open_path.read(cx) != parent {
                                open_path.update(cx, |open, cx| {
                                    *open = parent.clone();
                                    cx.notify();
                                });
                            }
                        })
                        .child(icon_column())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(theme.metrics.space_sm)
                                .child(label)
                                .child(control),
                        )
                        .into_any_element()
                }
                MenuEntry::Header(label) => div()
                    .flex_none()
                    .px(theme.metrics.space_sm)
                    .min_h(theme.metrics.header_height)
                    .flex()
                    .items_center()
                    .text_size(theme.metrics.font_sm)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(label)
                    .into_any_element(),
                MenuEntry::Separator => div()
                    .flex_none()
                    .h(px(1.))
                    .ml(SEPARATOR_OFFSET)
                    .my(theme.metrics.space_2xs)
                    .bg(theme.colors.menu_separator)
                    .into_any_element(),
                MenuEntry::Item(item) => {
                    let custom = item.content.is_some();
                    let label = item.label.clone();
                    let (open_path, parent) = (context.open_path.clone(), path.to_vec());
                    let row_text = if item.disabled {
                        theme.colors.disabled_text
                    } else {
                        theme.colors.text
                    };
                    let close = context.close.clone();
                    let marker = item
                        .checked
                        .then(|| icon("check").text_color(row_text).into_any_element())
                        .or_else(|| {
                            item.icon
                                .map(|name| icon(&name).text_color(row_text).into_any_element())
                        });
                    let body = match item.content {
                        Some(content) => div()
                            .flex()
                            .w_full()
                            .items_center()
                            .child(icon_column().children(marker))
                            .child(div().flex_1().min_w_0().child(content))
                            .into_any_element(),
                        None => div()
                            .flex()
                            .w_full()
                            .items_center()
                            .child(icon_column().children(marker))
                            .child(div().flex_1().min_w_0().truncate().child(item.label))
                            .when_some(item.meta, |row, meta| {
                                row.child(div().flex_none().text_xs().child(meta))
                            })
                            .into_any_element(),
                    };
                    row_base(row_id, item.disabled, cx)
                        .accessibility_label(label.clone())
                        .when(custom && item.checked, |row| row.bg(theme.colors.selected))
                        .debug_selector(move || format!("menu-item-{label}"))
                        .on_hover(move |hovered, _, cx| {
                            if *hovered {
                                open_path.update(cx, |open, cx| {
                                    if *open != parent {
                                        *open = parent.clone();
                                        cx.notify();
                                    }
                                });
                            }
                        })
                        .when(!item.disabled, |this| {
                            let on_click = item.on_click;
                            this.on_click(move |_, window, cx| {
                                close((), window, cx);
                                if let Some(handler) = &on_click {
                                    handler((), window, cx);
                                }
                            })
                        })
                        .child(body)
                        .into_any_element()
                }
                entry @ (MenuEntry::Submenu { .. } | MenuEntry::SelectableSubmenu { .. }) => {
                    let (item, entries) = match entry {
                        MenuEntry::Submenu {
                            label,
                            disabled,
                            entries,
                        } => (MenuItem::new(label).disabled(disabled), entries),
                        MenuEntry::SelectableSubmenu { item, entries } => (item, entries),
                        _ => unreachable!(),
                    };
                    let MenuItem {
                        label,
                        disabled,
                        checked,
                        on_click,
                        ..
                    } = item;
                    let opened = path_is_open(&open, &row_path);
                    let open_path = context.open_path.clone();
                    let target = row_path.clone();
                    let child = opened.then(|| {
                        let width = context.submenu_width;
                        let room_right = context.viewport_width - menu_x - menu_width;
                        let opens_left = room_right < width + WINDOW_MARGIN && menu_x > room_right;
                        let x = fit_menu_x(
                            if opens_left {
                                menu_x - width
                            } else {
                                menu_x + menu_width
                            },
                            width,
                            context.viewport_width,
                        );
                        let inner =
                            render_entries(&row_id, entries, &row_path, x, width, context, cx);
                        div()
                            .absolute()
                            .top_0()
                            .when(opens_left, |this| this.left_0())
                            .when(!opens_left, |this| this.left_full())
                            .child(
                                deferred(
                                    anchored()
                                        .anchor(if opens_left {
                                            Anchor::TopRight
                                        } else {
                                            Anchor::TopLeft
                                        })
                                        .snap_to_window_with_margin(WINDOW_MARGIN)
                                        .child(panel(
                                            child_id(&row_id, "menu"),
                                            width,
                                            context.max_height,
                                            inner,
                                            cx,
                                        )),
                                )
                                .priority(gpui_base::POPUP_PRIORITY + row_path.len()),
                            )
                    });
                    let click_path = open_path.clone();
                    let click_target = target.clone();
                    let close = context.close.clone();
                    row_base(row_id, disabled, cx)
                        .debug_selector({
                            let label = label.clone();
                            move || format!("menu-item-{label}")
                        })
                        .when(!disabled, |this| {
                            this.on_click(move |_, window, cx| {
                                if let Some(handler) = &on_click {
                                    close((), window, cx);
                                    handler((), window, cx);
                                    return;
                                }
                                click_path.update(cx, |open, cx| {
                                    *open = click_target.clone();
                                    cx.notify();
                                });
                            })
                            .on_hover(move |hovered, _, cx| {
                                if *hovered {
                                    open_path.update(cx, |open, cx| {
                                        if !open.starts_with(&target) {
                                            *open = target.clone();
                                            cx.notify();
                                        }
                                    });
                                }
                            })
                        })
                        .child(icon_column().children(checked.then(|| icon("check"))))
                        .child(div().flex_1().min_w_0().truncate().child(label))
                        .child(icon("chevron-right").text_color(if disabled {
                            theme.colors.disabled_text
                        } else {
                            theme.colors.text
                        }))
                        .children(child)
                        .into_any_element()
                }
            }
        })
        .collect()
}

impl RenderOnce for ContextMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let viewport = window.viewport_size();
        let max_width = (viewport.width - WINDOW_MARGIN * 2.).max(px(1.));
        let width = self.width.min(max_width);
        let x = fit_menu_x(self.position.x, width, viewport.width);
        let open_path =
            window.use_keyed_state(child_id(&self.id, "open-path"), cx, |_, _| Vec::new());
        let focus: FocusHandle = window
            .use_keyed_state(child_id(&self.id, "focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        if !focus.contains_focused(window, cx) {
            let handle = focus.clone();
            window.defer(cx, move |window, cx| handle.focus(window, cx));
        }
        let context = Rc::new(MenuContext {
            open_path,
            close: self.on_close.clone(),
            viewport_width: viewport.width,
            max_height: (viewport.height - WINDOW_MARGIN * 2.).max(px(16.)),
            submenu_width: self.submenu_width.unwrap_or(self.width).min(max_width),
        });
        let entries = render_entries(&self.id, self.entries, &[], x, width, &context, cx);
        let (backdrop_close, secondary_close, key_close) = (
            self.on_close.clone(),
            self.on_close.clone(),
            self.on_close.clone(),
        );
        deferred(
            div()
                .child(
                    anchored().position(Point::default()).child(
                        div()
                            .id(child_id(&self.id, "backdrop"))
                            .occlude()
                            .w(viewport.width)
                            .h(viewport.height)
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                backdrop_close((), window, cx)
                            })
                            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                                secondary_close((), window, cx)
                            }),
                    ),
                )
                .child(
                    anchored()
                        .position(gpui::point(x, self.position.y))
                        .snap_to_window_with_margin(WINDOW_MARGIN)
                        .child(
                            panel(self.id, width, context.max_height, entries, cx)
                                .track_focus(&focus)
                                .on_key_down(move |event, window, cx| {
                                    if event.keystroke.key == "escape" {
                                        cx.stop_propagation();
                                        key_close((), window, cx);
                                    }
                                }),
                        ),
                ),
        )
        .priority(gpui_base::POPUP_PRIORITY)
    }
}
