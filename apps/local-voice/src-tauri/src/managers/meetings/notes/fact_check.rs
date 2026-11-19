//! Faktenpruefung fuer `--eval-minutes`: Wie viele erwartete Aussagen stehen im
//! Protokoll, und wie viele bekannte Falschzuordnungen?
//!
//! Eine Fixture kann `fakten` (Pflichtaussagen) und `falsch` (Aussagen, die
//! nicht vorkommen duerfen) mitbringen. Jede Aussage ist eine Liste von
//! Gruppen; jede Gruppe ist eine Liste gleichwertiger Schreibweisen. Eine
//! Aussage gilt als gefunden, wenn jede Gruppe mit mindestens einer
//! Schreibweise vorkommt, und zwar innerhalb eines Fensters aufeinander
//! folgender nichtleerer Zeilen: drei Zeilen fuer Pflichtaussagen (Ueberschrift
//! plus Stichpunkt), eine Zeile fuer Falschaussagen (sonst schlagen
//! Aufzaehlungen mehrerer Personen falsch an).
//!
//! Verglichen wird ohne Gross-/Kleinschreibung, Umlaute gefaltet (`ä` -> `ae`,
//! `ß` -> `ss`), Tausenderpunkte entfernt (`55.000` -> `55000`), Gedanken- und
//! Bindestriche vereinheitlicht. Das ist bewusst grob: Es misst Abdeckung und
//! grobe Fehlzuordnung, nicht Stil oder Lesbarkeit.

use serde::{Deserialize, Serialize};

/// Fenster (nichtleere Zeilen) fuer Pflichtaussagen.
pub const WINDOW_FACT: usize = 3;
/// Fenster fuer Falschaussagen.
pub const WINDOW_FALSE: usize = 1;

/// Eine erwartete (oder verbotene) Aussage.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FactCheck {
    pub id: String,
    /// Alle Gruppen muessen passen; innerhalb einer Gruppe reicht eine
    /// Schreibweise.
    pub alle: Vec<Vec<String>>,
}

/// Ergebnis fuer ein Protokoll.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct FactReport {
    pub fakten_gesamt: usize,
    pub fakten_gefunden: usize,
    pub fehlend: Vec<String>,
    pub falsch_gesamt: usize,
    pub falsch_treffer: Vec<String>,
    /// Anteil gefundener Pflichtaussagen, `None` ohne Pflichtaussagen.
    pub abdeckung: Option<f64>,
}

/// Vereinheitlicht Text fuer den Vergleich.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            '\u{2010}'..='\u{2015}' | '\u{2212}' => out.push('-'),
            '\u{00a0}' | '\u{202f}' | '\u{2009}' | '\t' => out.push(' '),
            _ => out.push(c),
        }
    }
    strip_thousands(&out)
}

/// `55.000` / `55 000` -> `55000`, aber `15.` oder `1,8` bleiben.
/// Grob: jede Ziffer + Punkt/Leerzeichen + genau drei Ziffern wird verbunden
/// (`Q3 100` -> `q3100`); beide Seiten werden gleich behandelt. In Fixtures
/// deshalb keine Schreibweisen mit Leerzeichen zwischen Zahlen verwenden.
fn strip_thousands(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        let is_sep = c == '.' || c == ' ';
        if is_sep
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars
                .get(i + 1..i + 4)
                .is_some_and(|d| d.iter().all(char::is_ascii_digit))
            && !chars.get(i + 4).is_some_and(char::is_ascii_digit)
        {
            continue;
        }
        out.push(c);
    }
    out
}

/// Normalisierte, nichtleere Zeilen eines Protokolls.
fn lines(document: &str) -> Vec<String> {
    document
        .lines()
        .map(normalize)
        .filter(|l| !l.trim().is_empty())
        .collect()
}

fn group_matches(text: &str, group: &[String]) -> bool {
    group.iter().any(|alt| {
        let alt = normalize(alt);
        !alt.trim().is_empty() && text.contains(alt.as_str())
    })
}

/// Passt eine Aussage in irgendein Fenster von `window` Zeilen?
pub fn matches(lines: &[String], fact: &FactCheck, window: usize) -> bool {
    if fact.alle.is_empty() || lines.is_empty() {
        return false;
    }
    let window = window.max(1).min(lines.len());
    (0..=lines.len() - window).any(|start| {
        let joined = lines[start..start + window].join("\n");
        fact.alle.iter().all(|group| group_matches(&joined, group))
    })
}

/// Prueft ein Protokoll gegen Pflicht- und Falschaussagen.
pub fn check(document: &str, fakten: &[FactCheck], falsch: &[FactCheck]) -> FactReport {
    let lines = lines(document);
    let fehlend: Vec<String> = fakten
        .iter()
        .filter(|f| !matches(&lines, f, WINDOW_FACT))
        .map(|f| f.id.clone())
        .collect();
    let falsch_treffer: Vec<String> = falsch
        .iter()
        .filter(|f| matches(&lines, f, WINDOW_FALSE))
        .map(|f| f.id.clone())
        .collect();
    let gefunden = fakten.len() - fehlend.len();
    FactReport {
        fakten_gesamt: fakten.len(),
        fakten_gefunden: gefunden,
        fehlend,
        falsch_gesamt: falsch.len(),
        falsch_treffer,
        abdeckung: (!fakten.is_empty()).then(|| gefunden as f64 / fakten.len() as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(id: &str, groups: &[&[&str]]) -> FactCheck {
        FactCheck {
            id: id.into(),
            alle: groups
                .iter()
                .map(|g| g.iter().map(|s| s.to_string()).collect())
                .collect(),
        }
    }

    #[test]
    fn normalize_folds_umlauts_case_dashes_and_thousands() {
        assert_eq!(normalize("Größe Übergabe"), "groesse uebergabe");
        assert_eq!(normalize("55.000 €"), "55000 €");
        assert_eq!(normalize("150 000 Euro"), "150000 euro");
        assert_eq!(normalize("1.800.000"), "1800000");
        assert_eq!(normalize("am 15. Mai"), "am 15. mai");
        assert_eq!(normalize("1,8 Mio."), "1,8 mio.");
        assert_eq!(normalize("6–8 Wochen"), "6-8 wochen");
        assert_eq!(normalize("10:00 Uhr"), "10:00 uhr");
    }

    #[test]
    fn a_fact_needs_every_group_within_the_window() {
        let doc = "## Aufgaben\n\n- Miriam: Review Export\n- bis Mittwoch\n- Jonas: Suche";
        let f = fact("review", &[&["miriam"], &["review"], &["mittwoch"]]);
        assert!(check(doc, std::slice::from_ref(&f), &[]).fehlend.is_empty());
        let far = "Miriam\na\nb\nc\nReview Mittwoch";
        assert_eq!(check(far, &[f], &[]).fehlend, vec!["review".to_string()]);
    }

    #[test]
    fn alternatives_and_number_formats_match() {
        let doc = "| Reserve | 150.000 € |";
        let f = fact(
            "reserve",
            &[&["reserve"], &["150000", "hundertfuenfzigtausend"]],
        );
        let r = check(doc, &[f], &[]);
        assert_eq!(r.fakten_gefunden, 1);
        assert_eq!(r.abdeckung, Some(1.0));
    }

    #[test]
    fn false_statements_only_hit_on_one_line() {
        let wrong = fact("okafor-budget", &[&["okafor"], &["budgetplanung"]]);
        let ok = "- Brandt: Budgetplanung\n- Okafor: Archivplan";
        assert!(check(ok, &[], std::slice::from_ref(&wrong))
            .falsch_treffer
            .is_empty());
        let bad = "- Okafor legt die Budgetplanung vor";
        assert_eq!(
            check(bad, &[], &[wrong]).falsch_treffer,
            vec!["okafor-budget".to_string()]
        );
    }

    /// Die Referenzprotokolle (`tests/fixtures/notes_referenz/*.md`) erfuellen
    /// alle Pflichtaussagen ihrer Fixture und treffen keine Falschaussage.
    /// Haelt die Faktenlisten erfuellbar und nicht zu eng.
    #[test]
    fn reference_minutes_cover_every_fact_and_no_false_one() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let fixtures = super::super::eval::load_fixtures(&root.join("notes")).unwrap();
        let mut checked = 0;
        for (name, fixture) in fixtures {
            if fixture.fakten.is_empty() {
                continue;
            }
            let path = root.join("notes_referenz").join(format!("{name}.md"));
            let doc = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} fehlt: {e}", path.display()));
            let r = check(&doc, &fixture.fakten, &fixture.falsch);
            assert!(r.fehlend.is_empty(), "{name}: fehlend {:?}", r.fehlend);
            assert!(
                r.falsch_treffer.is_empty(),
                "{name}: falsch {:?}",
                r.falsch_treffer
            );
            assert!(!fixture.falsch.is_empty(), "{name}: keine Falschaussagen");
            // Ein Protokoll ohne Inhalt darf fast nichts treffen.
            let empty = check(
                "# Protokoll\n\n- Besprechung fand statt.",
                &fixture.fakten,
                &[],
            );
            assert_eq!(empty.fakten_gefunden, 0, "{name}: Fakten zu weit");
            checked += 1;
        }
        assert!(checked >= 3, "nur {checked} Fixtures mit Fakten");
    }

    #[test]
    fn empty_inputs_are_safe() {
        let r = check("", &[fact("x", &[&["a"]])], &[fact("y", &[&["b"]])]);
        assert_eq!(r.fakten_gefunden, 0);
        assert!(r.falsch_treffer.is_empty());
        assert_eq!(check("text", &[], &[]).abdeckung, None);
        assert!(!matches(&["a".into()], &fact("leer", &[]), 3));
    }
}
