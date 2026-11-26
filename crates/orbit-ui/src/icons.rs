use std::borrow::Cow;

use crate::theme::Theme;
use gpui::{div, prelude::*, svg, AssetSource, Div, Result, SharedString, Styled, Svg};

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// The application's bundled assets, addressed as `icons/<name>.svg`.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(path
            .strip_prefix("icons/")
            .and_then(|file| ICONS.iter().find(|(name, _)| *name == file))
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .map(|(name, _)| SharedString::from(format!("icons/{name}")))
            .filter(|asset| asset.starts_with(path))
            .collect())
    }
}

/// A Tabler outline icon by its file name, such as `player-play`. Set its color with `text_color`.
pub fn icon(name: &str) -> Svg {
    svg().path(format!("icons/{name}.svg")).size_4().flex_none()
}

/// Overlapping picture and square that show the media's stacking order.
pub fn media_layer_icon(foreground: bool, theme: &Theme) -> Div {
    let square = div()
        .absolute()
        .size(theme.metrics.font_xs)
        .rounded(theme.metrics.radius_xs)
        .bg(theme.colors.text_subtle);
    let picture = div()
        .absolute()
        .size(theme.metrics.font_xs)
        .rounded(theme.metrics.radius_xs)
        .bg(theme.colors.text_strong)
        .border_1()
        .border_color(theme.colors.preview_matte_bg)
        .child(
            icon("media-layer-photo")
                .size_full()
                .text_color(theme.colors.preview_matte_bg),
        );
    let glyph = div().relative().size(theme.metrics.font_xl).flex_none();
    if foreground {
        glyph
            .child(square.right_0().bottom_0())
            .child(picture.left_0().top_0())
    } else {
        glyph
            .child(picture.right_0().bottom_0())
            .child(square.left_0().top_0())
    }
}
