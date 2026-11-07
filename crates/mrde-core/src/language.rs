//! Presentation preferences shared by every frontend. Repository content is never translated.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "fr")]
    French,
}
impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::French];
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::French => "fr",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::French => "Français",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "en" => Some(Self::English),
            "fr" => Some(Self::French),
            _ => None,
        }
    }
    /// Keep schema keys and source identifiers stable while localizing natural language.
    pub fn instruction(self) -> &'static str {
        match self {
            Self::English => {
                "Write all explanations, summaries and conversation replies in English. Keep JSON keys, enum values, file paths, commands and source identifiers unchanged."
            }
            Self::French => {
                "Write all explanations, summaries and conversation replies in French. Keep JSON keys, enum values, file paths, commands and source identifiers unchanged."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Languages {
    pub interface: Language,
    pub information: Language,
}

/// Immutable catalog shared by the native UI and the terminal. Missing translations
/// fall back to the source text; unknown user-provided content stays untouched.
pub fn translate(language: Language, source: &str) -> &str {
    static FRENCH: std::sync::OnceLock<std::collections::HashMap<String, String>> =
        std::sync::OnceLock::new();
    if language == Language::English {
        return source;
    }
    FRENCH
        .get_or_init(|| {
            serde_json::from_str(include_str!("../locales/fr.json"))
                .expect("validated translation catalog")
        })
        .get(source)
        .map(String::as_str)
        .unwrap_or(source)
}

/// Named substitutions, performed once so values resembling placeholders remain literal.
pub fn message(language: Language, template: &str, values: &[(&str, String)]) -> String {
    let text = translate(language, template);
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}') {
            let key = &after[..end];
            if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
                result.push_str(value);
            } else {
                result.push_str(&rest[start..start + end + 2]);
            }
            rest = &after[end + 1..];
        } else {
            rest = &rest[start..];
            break;
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_are_independent_and_default_to_english() {
        let p: Languages = serde_json::from_str(r#"{"interface":"fr"}"#).unwrap();
        assert_eq!(p.interface, Language::French);
        assert_eq!(p.information, Language::English);
        assert!(serde_json::from_str::<Languages>(r#"{"interface":"de"}"#).is_err());
    }
    #[test]
    fn catalog_preserves_named_placeholders_and_does_not_translate_substitutions() {
        let catalog: std::collections::HashMap<String, String> =
            serde_json::from_str(include_str!("../locales/fr.json")).unwrap();
        fn placeholders(text: &str) -> std::collections::BTreeSet<&str> {
            text.split('{')
                .skip(1)
                .filter_map(|part| part.split_once('}').map(|(name, _)| name))
                .collect()
        }
        for (key, value) in &catalog {
            assert!(!value.is_empty(), "{key}");
            assert_eq!(placeholders(key), placeholders(value), "{key}");
        }
        assert_eq!(
            message(
                Language::French,
                "Project: {name}",
                &[("name", "{name} Release".into())]
            ),
            "Projet : {name} Release"
        );
        assert_eq!(
            translate(Language::French, "user-provided title"),
            "user-provided title"
        );
    }
}
