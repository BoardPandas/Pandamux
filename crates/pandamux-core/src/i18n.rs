//! Minimal localization scaffolding, ported from the Electron `site/i18n.js`
//! language switching. A [`Localizer`] holds the current [`Locale`] and looks up
//! UI strings by key, falling back to English and then to the key itself when a
//! translation is missing.

use serde::{Deserialize, Serialize};

/// A supported UI locale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locale {
    #[default]
    En,
    Fr,
    Ar,
    Ja,
}

impl Locale {
    /// Parse an ISO-ish language code ("en", "fr", "ar", "ja"); None if unknown.
    pub fn parse(value: &str) -> Option<Locale> {
        match value {
            "en" => Some(Locale::En),
            "fr" => Some(Locale::Fr),
            "ar" => Some(Locale::Ar),
            "ja" => Some(Locale::Ja),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Fr => "fr",
            Locale::Ar => "ar",
            Locale::Ja => "ja",
        }
    }
}

/// Look up `key` in `locale`'s catalog. Returns None if `locale` has no entry
/// for `key` (the caller falls back to English, then to the key itself).
fn catalog_lookup(locale: Locale, key: &str) -> Option<&'static str> {
    match (locale, key) {
        (Locale::En, "new_session") => Some("New Session"),
        (Locale::En, "settings") => Some("Settings"),
        (Locale::En, "notifications") => Some("Notifications"),
        (Locale::En, "find") => Some("Find"),
        (Locale::En, "sessions") => Some("Sessions"),
        (Locale::En, "command_palette") => Some("Command Palette"),

        (Locale::Fr, "new_session") => Some("Nouvelle session"),
        (Locale::Fr, "settings") => Some("Paramètres"),
        (Locale::Fr, "notifications") => Some("Notifications"),
        (Locale::Fr, "find") => Some("Rechercher"),
        (Locale::Fr, "sessions") => Some("Sessions"),
        (Locale::Fr, "command_palette") => Some("Palette de commandes"),

        (Locale::Ar, "settings") => Some("الإعدادات"),
        (Locale::Ar, "notifications") => Some("الإشعارات"),

        (Locale::Ja, "settings") => Some("設定"),
        (Locale::Ja, "notifications") => Some("通知"),

        _ => None,
    }
}

/// Holds the active locale and resolves catalog keys to display strings.
#[derive(Clone, Copy, Debug)]
pub struct Localizer {
    locale: Locale,
}

impl Default for Localizer {
    fn default() -> Self {
        Self::new(Locale::En)
    }
}

impl Localizer {
    pub fn new(locale: Locale) -> Self {
        Self { locale }
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    pub fn set_locale(&mut self, locale: Locale) {
        self.locale = locale;
    }

    /// Look up `key` in the current locale's catalog, falling back to English,
    /// then to the key itself when no translation exists.
    pub fn t(&self, key: &str) -> String {
        catalog_lookup(self.locale, key)
            .or_else(|| catalog_lookup(Locale::En, key))
            .unwrap_or(key)
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn french_translation_differs_from_english() {
        let en = Localizer::new(Locale::En);
        let fr = Localizer::new(Locale::Fr);
        assert_eq!(fr.t("settings"), "Paramètres");
        assert_ne!(fr.t("settings"), en.t("settings"));
    }

    #[test]
    fn unknown_key_returns_the_key_itself() {
        let localizer = Localizer::new(Locale::En);
        assert_eq!(localizer.t("does_not_exist"), "does_not_exist");
    }

    #[test]
    fn missing_translation_falls_back_to_english() {
        let localizer = Localizer::new(Locale::Ja);
        // "find" has no Japanese entry, so it falls back to English.
        assert_eq!(localizer.t("find"), "Find");
    }

    #[test]
    fn locale_parse_round_trips_known_codes() {
        assert_eq!(Locale::parse("fr"), Some(Locale::Fr));
        assert_eq!(Locale::parse("ja"), Some(Locale::Ja));
        assert!(Locale::parse("xx").is_none());
    }
}
