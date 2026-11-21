use std::borrow::Cow;

use gpui::{svg, AssetSource, Result, SharedString, Styled, Svg};

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// The application's bundled assets, addressed as `icons/<tabler-name>.svg`.
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
