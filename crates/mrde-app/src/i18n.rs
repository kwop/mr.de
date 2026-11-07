//! GPUI renders on one UI thread. Tests on other threads have independent locales.
use mrde_core::language::{Language, translate};
thread_local! { static CURRENT: std::cell::Cell<Language> = const { std::cell::Cell::new(Language::English) }; }
pub fn set(language: Language) {
    CURRENT.set(language);
}
pub fn language() -> Language {
    CURRENT.get()
}
pub fn t(source: &'static str) -> &'static str {
    translate(language(), source)
}
pub fn text(source: &str) -> String {
    translate(language(), source).to_owned()
}
pub fn message(template: &str, values: &[(&str, String)]) -> String {
    mrde_core::language::message(language(), template, values)
}

#[macro_export]
macro_rules! tr {
    ($template:literal $(, $key:ident = $value:expr)* $(,)?) => {
        $crate::i18n::message($template, &[$((stringify!($key), ($value).to_string())),*])
    };
}
