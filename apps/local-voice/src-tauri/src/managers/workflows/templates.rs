//! Mitgelieferte Vorlagen (B1: eine Beispielvorlage fuer Tests und Trockenlauf).
//!
//! B4 bis B6 haengen hier ihre Vorlagen an (`Eingangsordner -> Word`, `Termin -> Mail`,
//! `Kanal -> Wissen`); jede Vorlage muss `validate::parse_definition_str` bestehen
//! (Test `every_shipped_template_is_valid`).

/// (Kennung, JSON-Text).
pub fn all() -> Vec<(&'static str, &'static str)> {
    vec![("vorlage-besprechung", MEETING_SAMPLE)]
}

/// Termin beginnt -> Aufnahme -> Protokoll -> Word -> Mail (Beispielvorlage fuer den
/// Trockenlauf AK2; die endgueltige Vorlage "Termin -> Mail" liefert B5).
pub const MEETING_SAMPLE: &str = include_str!("templates/vorlage-besprechung.json");
