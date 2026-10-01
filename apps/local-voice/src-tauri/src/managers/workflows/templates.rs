//! Mitgelieferte Vorlagen (B1: eine Beispielvorlage fuer Tests und Trockenlauf).
//!
//! B4 bis B6 haengen hier ihre Vorlagen an (`Eingangsordner -> Word`, `Termin -> Mail`,
//! `Kanal -> Wissen`); jede Vorlage muss `validate::parse_definition_str` bestehen
//! (Test `every_shipped_template_is_valid`).

/// (Kennung, JSON-Text).
pub fn all() -> Vec<(&'static str, &'static str)> {
    vec![
        ("vorlage-besprechung", MEETING_SAMPLE),
        ("eingangsordner-word", FOLDER_TO_WORD),
        ("termin-protokoll-mail", TERMIN_MAIL),
        ("besprechung-ergebnis", BESPRECHUNG_ERGEBNIS),
    ]
}

/// Termin beginnt -> Aufnahme -> Protokoll -> Word -> Mail (Beispielvorlage fuer den
/// Trockenlauf AK2; die endgueltige Vorlage "Termin -> Mail" liefert B5).
pub const MEETING_SAMPLE: &str = include_str!("templates/vorlage-besprechung.json");

/// Datei im Eingangsordner -> importieren und transkribieren -> Protokoll -> Word im Zielordner
/// (B4, AK6). Die Kennungen `folder-eingang` und `folder-protokolle` sind Platzhalter fuer die
/// Ordner-Integrationen des Nutzers; der Editor (B7) laesst sie waehlen.
pub const FOLDER_TO_WORD: &str = include_str!("templates/eingangsordner-word.json");

/// Termin beginnt -> Aufnahme -> Protokoll -> Word -> Mail (B5, AK7). Die Kennungen `cal-1`,
/// `m365-1`, `folder-protokolle` sind Platzhalter fuer die Integrationen des Nutzers; die Variable
/// `empfaenger` (`ich` oder `alle`) legt je Kalender fest, wer die Mail bekommt (zwei Ablaeufe aus
/// derselben Vorlage: Kalender A mit `ich`, Kalender B mit `alle`).
pub const TERMIN_MAIL: &str = include_str!("templates/termin-protokoll-mail.json");

/// Besprechung fertig -> extrahieren -> Vault-Notiz, (Kalender), Frist-Erinnerungen (C4, AK6, AK7). Die
/// Kennungen `vault-1` und `m365-1` sind Platzhalter fuer die Integrationen des Nutzers; der Editor
/// laesst sie waehlen. Die Erinnerung steht zuletzt, weil der Lauf auf sie wartet.
pub const BESPRECHUNG_ERGEBNIS: &str = include_str!("templates/besprechung-ergebnis.json");
