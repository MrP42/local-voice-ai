//! Text aufbereiten (lva#658): Struktur und Format als gemeinsamer Baustein.

/// Einmal kompilierte Muster; ein grosses Buch ruft sie zehntausendfach auf.
macro_rules! re {
    ($name:ident, $pat:expr) => {
        pub(crate) static $name: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new($pat).expect("static regex"));
    };
}
pub(crate) use re;

#[allow(dead_code)] // Anschluss folgt mit dem Befehl (A1 3b)
pub mod aenderungen;
#[allow(dead_code)] // wird mit der Aenderungsliste (A2) angeschlossen
pub mod format;
pub mod struktur;
