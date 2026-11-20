//! Launcher translations.
//!
//! The English source strings are the keys. Catalogs live beside the executable
//! as plain JSON so a translator can review or extend them without rebuilding
//! the Rust code. A missing entry falls back to the English key, never to a
//! blank label, and English itself never parses a catalog.

use eframe::egui::RichText;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "it")]
    Italian,
    #[serde(rename = "pt")]
    Portuguese,
    #[serde(rename = "ru")]
    Russian,
    #[serde(rename = "tr")]
    Turkish,
    #[serde(rename = "zh")]
    Chinese,
    #[serde(rename = "ja")]
    Japanese,
}

impl Language {
    pub const ALL: [Language; 10] = [
        Language::English,
        Language::Spanish,
        Language::German,
        Language::French,
        Language::Italian,
        Language::Portuguese,
        Language::Russian,
        Language::Turkish,
        Language::Chinese,
        Language::Japanese,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Spanish => "es",
            Language::German => "de",
            Language::French => "fr",
            Language::Italian => "it",
            Language::Portuguese => "pt",
            Language::Russian => "ru",
            Language::Turkish => "tr",
            Language::Chinese => "zh",
            Language::Japanese => "ja",
        }
    }

    /// The name shown in the language picker, in that language.
    pub fn native_label(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Spanish => "Español",
            Language::German => "Deutsch",
            Language::French => "Français",
            Language::Italian => "Italiano",
            Language::Portuguese => "Português",
            Language::Russian => "Русский",
            Language::Turkish => "Türkçe",
            Language::Chinese => "简体中文",
            Language::Japanese => "日本語",
        }
    }

    pub fn english_label(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Spanish => "Spanish",
            Language::German => "German",
            Language::French => "French",
            Language::Italian => "Italian",
            Language::Portuguese => "Portuguese",
            Language::Russian => "Russian",
            Language::Turkish => "Turkish",
            Language::Chinese => "Chinese (Simplified)",
            Language::Japanese => "Japanese",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn catalog(self) -> Option<&'static HashMap<String, String>> {
        let index = self.index();
        if index == 0 {
            return None;
        }
        catalogs().get(index - 1)
    }

    fn lowercase_catalog(self) -> Option<&'static HashMap<String, String>> {
        let index = self.index();
        if index == 0 {
            return None;
        }
        lowercase_catalogs().get(index - 1)
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);
static CATALOGS: OnceLock<Vec<HashMap<String, String>>> = OnceLock::new();
static LOWERCASE_CATALOGS: OnceLock<Vec<HashMap<String, String>>> = OnceLock::new();

fn source(language: Language) -> &'static str {
    match language {
        Language::English => "{}",
        Language::Spanish => include_str!("../assets/i18n/es.json"),
        Language::German => include_str!("../assets/i18n/de.json"),
        Language::French => include_str!("../assets/i18n/fr.json"),
        Language::Italian => include_str!("../assets/i18n/it.json"),
        Language::Portuguese => include_str!("../assets/i18n/pt.json"),
        Language::Russian => include_str!("../assets/i18n/ru.json"),
        Language::Turkish => include_str!("../assets/i18n/tr.json"),
        Language::Chinese => include_str!("../assets/i18n/zh.json"),
        Language::Japanese => include_str!("../assets/i18n/ja.json"),
    }
}

fn catalogs() -> &'static Vec<HashMap<String, String>> {
    CATALOGS.get_or_init(|| {
        Language::ALL
            .iter()
            .skip(1)
            .map(|language| serde_json::from_str(source(*language)).unwrap_or_default())
            .collect()
    })
}

/// Headings and chips are often written in capitals while the catalog holds
/// the sentence-case source. A lowercase index lets those labels find the same
/// translation without duplicating every key in every catalog.
fn lowercase_catalogs() -> &'static Vec<HashMap<String, String>> {
    LOWERCASE_CATALOGS.get_or_init(|| {
        catalogs()
            .iter()
            .map(|catalog| {
                catalog
                    .iter()
                    // The few curated all-caps action labels are looked up
                    // exactly; keeping them out of this index prevents a
                    // sentence-case key from shadowing them.
                    .filter(|(source, _)| !is_all_caps(source))
                    .map(|(source, translated)| (source.to_lowercase(), translated.clone()))
                    .collect()
            })
            .collect()
    })
}

fn is_all_caps(source: &str) -> bool {
    let mut letters = source.chars().filter(|c| c.is_alphabetic()).peekable();
    letters.peek().is_some() && letters.all(|c| c.is_uppercase())
}

pub fn current() -> Language {
    Language::ALL
        .get(CURRENT.load(Ordering::Relaxed) as usize)
        .copied()
        .unwrap_or_default()
}

pub fn set_language(language: Language) {
    CURRENT.store(language.index() as u8, Ordering::Relaxed);
}

/// Translate one English source string. The exact key wins, then the trimmed
/// key, then the sentence-case index used by headings and chips. Unknown
/// strings and English return the source unchanged, so callers keep working
/// while a catalog is being extended.
pub fn tr(text: &str) -> String {
    let language = current();
    let Some(catalog) = language.catalog() else {
        return text.to_string();
    };
    if let Some(value) = catalog.get(text) {
        return value.clone();
    }
    let trimmed = text.trim();
    if trimmed != text {
        if let Some(value) = catalog.get(trimmed) {
            return value.clone();
        }
    }
    if let Some(lower) = language.lowercase_catalog() {
        if let Some(value) = lower.get(&trimmed.to_lowercase()) {
            return value.clone();
        }
    }
    text.to_string()
}

/// `RichText` with the source string translated. Used everywhere the UI builds
/// styled text, so a page keeps its type and colour while changing language.
pub fn text(text: impl AsRef<str>) -> RichText {
    RichText::new(tr(text.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static LANGUAGE_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn every_language_has_a_catalog_and_a_native_name() {
        for language in Language::ALL {
            assert!(!language.native_label().is_empty());
            assert!(!language.code().is_empty());
            if language != Language::English {
                assert!(!source(language).trim().is_empty(), "{} has no catalog", language.code());
            }
        }
    }

    #[test]
    fn every_shipped_catalog_covers_the_shared_ui() {
        for language in Language::ALL.iter().skip(1) {
            let catalog = language.catalog().expect("shipped catalog");
            assert!(catalog.len() > 1000, "{} has only {} entries", language.code(), catalog.len());
            for key in [
                "Overview",
                "Settings",
                "Play",
                "Language",
                "Save profile",
                "Next start",
                "Game ready",
            ] {
                assert!(catalog.contains_key(key), "{} is missing {key}", language.code());
            }
        }
    }

    #[test]
    fn english_returns_the_source_and_unknown_strings_fall_back() {
        let _guard = LANGUAGE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        set_language(Language::English);
        assert_eq!(tr("At a glance"), "At a glance");
        assert_eq!(tr("A string no catalog has"), "A string no catalog has");
    }

    #[test]
    fn uppercase_labels_use_curated_entries_or_the_sentence_case_fallback() {
        let _guard = LANGUAGE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        set_language(Language::French);
        assert_eq!(tr("PLAY"), "JOUER");
        assert_eq!(tr("BUILD"), "Compilation");
        assert_eq!(tr("SETTINGS"), "Paramètres");
        set_language(Language::English);
    }

    #[test]
    fn a_translated_string_is_used_and_switching_back_is_clean() {
        let _guard = LANGUAGE_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let german = Language::German.catalog().expect("German catalog");
        assert_eq!(german.get("Settings").map(String::as_str), Some("Einstellungen"));
        assert_eq!(german.get("A string no catalog has"), None);
        set_language(Language::German);
        assert_eq!(tr("Settings"), "Einstellungen");
        assert_eq!(tr("SETTINGS"), "Einstellungen");
        assert_eq!(tr("PLAY"), "SPIELEN");
        assert_eq!(tr("XiØN"), "XiØN");
        set_language(Language::English);
        assert_eq!(tr("Settings"), "Settings");
    }
}
