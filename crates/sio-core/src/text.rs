//! Localized strings that travel *inside* the catalog.
//!
//! Catalog entries carry their own translations rather than referencing keys in the
//! frontend's i18n bundle. That is deliberate: the catalog updates independently of
//! app releases, so a newly added app must be able to describe itself in all three
//! languages without shipping a new binary.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The locale used when a requested one is missing. English is the most likely
/// language for a contributor adding an entry, so it is the most likely to be present.
pub const FALLBACK_LOCALE: &str = "en";

/// A string with per-locale variants, e.g. `{"en": "Browser", "ru": "Браузер"}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LocalizedText(BTreeMap<String, String>);

impl LocalizedText {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, locale: impl Into<String>, text: impl Into<String>) -> &mut Self {
        self.0.insert(locale.into(), text.into());
        self
    }

    /// Resolve for a locale, degrading gracefully rather than ever returning nothing.
    ///
    /// Order: exact match → base language (`ru-RU` → `ru`) → English → any available
    /// translation → empty. The "any available" step matters because a half-translated
    /// catalog entry should still render *something* readable instead of a blank row.
    pub fn get(&self, locale: &str) -> &str {
        if let Some(v) = self.0.get(locale) {
            return v;
        }
        if let Some((base, _)) = locale.split_once(['-', '_']) {
            if let Some(v) = self.0.get(base) {
                return v;
            }
        }
        if let Some(v) = self.0.get(FALLBACK_LOCALE) {
            return v;
        }
        self.0.values().next().map(String::as_str).unwrap_or("")
    }

    /// Locales this entry has been translated into.
    pub fn locales(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for LocalizedText {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LocalizedText {
        LocalizedText::from_iter([("en", "Browser"), ("ru", "Браузер"), ("uk", "Браузер")])
    }

    #[test]
    fn exact_locale_wins() {
        assert_eq!(sample().get("ru"), "Браузер");
        assert_eq!(sample().get("en"), "Browser");
    }

    #[test]
    fn regional_locale_falls_back_to_base_language() {
        assert_eq!(sample().get("ru-RU"), "Браузер");
        assert_eq!(sample().get("uk_UA"), "Браузер");
    }

    #[test]
    fn missing_locale_falls_back_to_english() {
        assert_eq!(sample().get("de"), "Browser");
    }

    #[test]
    fn missing_english_falls_back_to_any_translation() {
        let partial = LocalizedText::from_iter([("ru", "Браузер")]);
        assert_eq!(
            partial.get("de"),
            "Браузер",
            "a half-translated entry must still render"
        );
    }

    #[test]
    fn empty_text_yields_empty_string_rather_than_panicking() {
        assert_eq!(LocalizedText::new().get("en"), "");
    }

    #[test]
    fn serialises_as_a_plain_object() {
        let json = serde_json::to_string(&LocalizedText::from_iter([("en", "Hi")])).unwrap();
        assert_eq!(json, r#"{"en":"Hi"}"#);
    }
}
