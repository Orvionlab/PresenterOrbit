//! The application's UI primitives. `docs/design-system.md` defines how they look; `gpui-base` supplies focus,
//! keyboard, editing, and accessibility behavior. Features use these and never `gpui-base` directly.

mod bars;
mod button;
mod collection_picker;
mod color_picker;
mod color_value;
mod dialog;
mod fields;
mod menu;
mod number;
mod panels;
mod popover;
mod rich_text;
mod rich_text_model;
mod segmented;
mod select;
mod tabs;
mod toggles;

pub use bars::{BarButton, BrandButton, ToolButton};
pub use button::{tooltip_text, ActionButton, ButtonVariant, IconButton};
pub use collection_picker::{CollectionAction, CollectionOption, CollectionPicker};
pub use color_picker::{fraction_within, ColorPicker, ColorPopover};
pub use color_value::{
    data_color, format_propresenter_hex, hex_alpha, hex_to_rgb, hsv_to_rgb, is_transparent,
    normalize_hex, parse_hex_input, rgb_to_hex, rgb_to_hsv, rgb_to_hue, Rgb, AVAILABLE_COLORS,
    DEFAULT_HEADER_COLOR, STANDARD_COLORS,
};
pub use dialog::{BusyDialog, Dialog};
pub use fields::{
    filter_state, growing_textarea_state, input_state, password_state, range_state,
    text_entry_focused, textarea_state, Field, FilterInput, OutputField, RangeInput, TextInput,
    Textarea,
};
pub use gpui_base::input::{Backspace, Delete, InputEvent, InputState, TextareaState};
pub use gpui_base::slider::{SliderEvent, SliderState};
pub use gpui_base::Root;
// Specialist preview controls own their rail geometry while retaining native button semantics.
pub use gpui_base::Button as NativeButton;
pub use menu::{ContextMenu, MenuEntry, MenuItem};
pub use number::{step_number, NumberInput};
pub use panels::{list_row, list_row_without_hover, tree_row, Disclosure, EmptyState, PanelHeader};
pub use popover::Popover;
pub use rich_text::{RichText, RichTextEvent};
pub use rich_text_model::RunDocument;
pub use segmented::{SegmentedControl, SegmentedOption};
pub use select::{next_enabled, Select, SelectOption};
pub use tabs::{tab_key_target, TabItem, TabList};
pub use toggles::{CheckboxField, ToggleSwitch};

use std::rc::Rc;
use std::sync::Arc;

use gpui::{point, px, App, BoxShadow, ElementId, SharedString, Styled, Window};

use crate::ActiveTheme;

pub(crate) type Handler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

pub(crate) fn child_id(parent: &ElementId, suffix: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(parent.clone()), suffix.into())
}

/// The 2 px focus outline. A shadow draws outside the box, so focus never moves the layout.
pub(crate) fn focus_ring<S: Styled>(style: S, cx: &App) -> S {
    style.shadow(vec![BoxShadow {
        color: cx.theme().colors.focus_ring.into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset: false,
    }])
}

mod angle_dial;
mod spelling;
pub use angle_dial::AngleDial;
