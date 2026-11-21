use std::collections::HashMap;
use std::sync::OnceLock;

use gpui::Global;

// The .arb files live in `locales/` and are embedded here; `scripts/check-i18n.mts` checks them.
const EN: &str = include_str!("../locales/en.arb");
const NL: &str = include_str!("../locales/nl.arb");
const DEFAULT_LANGUAGE: &str = "en";

type Messages = HashMap<String, String>;

fn parse(arb: &str) -> Messages {
    let document: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(arb).expect("the bundled .arb files are valid JSON");
    document
        .into_iter()
        .filter(|(key, _)| !key.starts_with('@'))
        .filter_map(|(key, value)| Some((key, value.as_str()?.to_string())))
        .collect()
}

fn dictionary(language: &str) -> &'static Messages {
    static TABLES: OnceLock<[Messages; 2]> = OnceLock::new();
    let [en, nl] = TABLES.get_or_init(|| [parse(EN), parse(NL)]);
    if language == "nl" {
        nl
    } else {
        en
    }
}

fn is_supported(language: &str) -> bool {
    matches!(language, "en" | "nl")
}

/// Mirrors `resolveAppLanguage`: an explicit supported language, the first supported system language for
/// `system`, and English otherwise.
pub fn resolve_language<'a>(
    setting: Option<&'a str>,
    system_languages: impl IntoIterator<Item = &'a str>,
) -> String {
    match setting {
        Some("system") => system_languages
            .into_iter()
            .filter_map(|language| language.split(['-', '_']).next())
            .map(str::to_lowercase)
            .find(|language| is_supported(language))
            .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string()),
        Some(language) if is_supported(language) => language.to_string(),
        _ => DEFAULT_LANGUAGE.to_string(),
    }
}

/// Mirrors `formatMessage`: replaces `{name}` in one pass and leaves unknown placeholders as written.
fn format_message(message: &str, values: &[(&str, &str)]) -> String {
    let mut formatted = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(open) = rest.find('{') {
        formatted.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        let value = after_open.find('}').and_then(|close| {
            let name = &after_open[..close];
            let value = values.iter().find(|(key, _)| *key == name)?.1;
            Some((value, close))
        });
        match value {
            Some((value, close)) => {
                formatted.push_str(value);
                rest = &after_open[close + 1..];
            }
            None => {
                formatted.push('{');
                rest = after_open;
            }
        }
    }
    formatted.push_str(rest);
    formatted
}

#[derive(Clone, Copy)]
pub struct Translator {
    messages: &'static Messages,
}

impl Global for Translator {}

impl Translator {
    pub fn new(language: &str) -> Self {
        Self {
            messages: dictionary(language),
        }
    }

    /// Resolves the `appLanguage` setting against the operating system's languages.
    pub fn for_setting(setting: Option<&str>) -> Self {
        let system: Vec<String> = sys_locale::get_locales().collect();
        Self::new(&resolve_language(
            setting,
            system.iter().map(String::as_str),
        ))
    }

    pub fn t(&self, key: &str) -> String {
        self.format(key, &[])
    }

    /// Falls back to English and then to the key, as the renderer's translator does.
    pub fn format(&self, key: &str, values: &[(&str, &str)]) -> String {
        let message = self
            .messages
            .get(key)
            .or_else(|| dictionary(DEFAULT_LANGUAGE).get(key))
            .map_or(key, String::as_str);
        format_message(message, values)
    }
}
