use gpui::App;
use orbit_ui::{set_appearance_settings, AppearanceSettings, Translator};
use serde_json::Value;

/// Applies a host event to the theme of every window. Only `settings-changed` carries appearance.
pub fn apply_host_event(cx: &mut App, event: &str, payload: &Value) -> Result<(), String> {
    if event == "settings-changed" {
        set_appearance_settings(cx, AppearanceSettings::from_settings(payload)?);
        let language = payload["generalSettings"]["appLanguage"]
            .as_str()
            .ok_or("The settings have no valid generalSettings.appLanguage.")?;
        cx.set_global(Translator::for_setting(Some(language)));
        // Views read the translator while rendering, so every window draws again with the new language.
        cx.refresh_windows();
    }
    Ok(())
}
