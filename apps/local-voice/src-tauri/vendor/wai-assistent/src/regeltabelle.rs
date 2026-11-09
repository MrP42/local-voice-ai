//! Regeltabelle nach Vertrag `assistent-einstufung` 0.1.6 (ADR-0028 E4.1).
//! Feldnamen exakt wie im Schema, unbekannte Felder werden abgewiesen.

use serde::{Deserialize, Serialize};

use crate::einstufung::{Sprache, Stufe, normalisieren};

/// Vertragsversion, gegen die diese Typen gebaut sind.
pub const SCHEMA_VERSION: &str = "0.1.6";

/// Mitgelieferte Standardtabelle (Vertragsfixture plus eigene Muster).
const STANDARD_JSON: &str = include_str!("../regeln/standard.json");
/// Vertragsfixture, unveraendert.
const FIXTURE_JSON: &str =
    include_str!("../regeln/vertragsfixture.json");

#[derive(Debug, thiserror::Error)]
pub enum Fehler {
    #[error("Regeltabelle ist kein gueltiges JSON oder passt nicht zum Vertrag: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Regeltabelle verletzt den Vertrag: {0}")]
    Ungueltig(String),
}

/// Geschlossene Liste der S0-Absichten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum S0Absicht {
    Gruss,
    #[serde(rename = "bestaetigung")]
    Bestaetigung,
    Stopp,
    Lauter,
    Leiser,
    Wiederholen,
    Langsamer,
    Schneller,
    #[serde(rename = "rueckfrage")]
    Rueckfrage,
    Uhrzeit,
    Datum,
}

impl S0Absicht {
    pub fn als_str(self) -> &'static str {
        match self {
            Self::Gruss => "gruss",
            Self::Bestaetigung => "bestaetigung",
            Self::Stopp => "stopp",
            Self::Lauter => "lauter",
            Self::Leiser => "leiser",
            Self::Wiederholen => "wiederholen",
            Self::Langsamer => "langsamer",
            Self::Schneller => "schneller",
            Self::Rueckfrage => "rueckfrage",
            Self::Uhrzeit => "uhrzeit",
            Self::Datum => "datum",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MusterJeSprache {
    pub de: Vec<String>,
    pub en: Vec<String>,
}

impl MusterJeSprache {
    pub fn fuer(&self, sprache: Sprache) -> &[String] {
        match sprache {
            Sprache::De => &self.de,
            Sprache::En => &self.en,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VorlageJeSprache {
    pub de: String,
    pub en: String,
}

impl VorlageJeSprache {
    pub fn fuer(&self, sprache: Sprache) -> &str {
        match sprache {
            Sprache::De => &self.de,
            Sprache::En => &self.en,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S0Regel {
    pub absicht: S0Absicht,
    pub muster: MusterJeSprache,
    pub antwort_vorlage: VorlageJeSprache,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Absicht {
    pub absicht: String,
    pub stufe: Stufe,
    pub muster: MusterJeSprache,
    pub ansage_vorlage: VorlageJeSprache,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Regeltabelle {
    pub schema_version: String,
    pub version: String,
    pub s0: Vec<S0Regel>,
    pub absichten: Vec<Absicht>,
}

impl Regeltabelle {
    /// Parst und prueft eine Regeltabelle (reines Vertragsobjekt, ohne Fixture-Huelle).
    pub fn aus_json(json: &str) -> Result<Self, Fehler> {
        let tabelle: Self = serde_json::from_str(json)?;
        tabelle.pruefen()?;
        Ok(tabelle)
    }

    /// Mitgelieferte Standardtabelle (`regeln/standard.json`).
    pub fn standard() -> Self {
        Self::aus_json(STANDARD_JSON).expect("regeln/standard.json ist gueltig (Test sichert das)")
    }

    /// Unveraenderte Vertragsfixture `regeltabelle-de-en.json` (Huelle `data`).
    pub fn vertragsfixture() -> Self {
        let huelle: serde_json::Value =
            serde_json::from_str(FIXTURE_JSON).expect("Fixture ist JSON");
        Self::aus_json(&huelle["data"].to_string()).expect("Fixture ist gueltig")
    }

    /// Pruefungen, die serde nicht leistet (Laengen, Muster, Slug, Stufe in `absichten`).
    pub fn pruefen(&self) -> Result<(), Fehler> {
        let fehler = |t: String| Err(Fehler::Ungueltig(t));
        if self.schema_version != SCHEMA_VERSION {
            return fehler(format!("schema_version muss {SCHEMA_VERSION} sein"));
        }
        if !ist_semver(&self.version) {
            return fehler("version ist keine Semver".into());
        }
        if self.s0.is_empty() || self.s0.len() > 50 {
            return fehler("s0 braucht 1 bis 50 Regeln".into());
        }
        if self.absichten.len() > 200 {
            return fehler("absichten hat hoechstens 200 Eintraege".into());
        }
        for regel in &self.s0 {
            pruefe_muster(&regel.muster)?;
            pruefe_vorlage(&regel.antwort_vorlage)?;
        }
        for a in &self.absichten {
            if a.stufe == Stufe::S0 {
                return fehler(format!("absichten traegt nur S1-S3 ({})", a.absicht));
            }
            if !ist_slug(&a.absicht) {
                return fehler(format!("absicht ist kein Slug: {}", a.absicht));
            }
            pruefe_muster(&a.muster)?;
            pruefe_vorlage(&a.ansage_vorlage)?;
        }
        Ok(())
    }
}

fn pruefe_muster(m: &MusterJeSprache) -> Result<(), Fehler> {
    for liste in [&m.de, &m.en] {
        if liste.is_empty() || liste.len() > 200 {
            return Err(Fehler::Ungueltig(
                "Musterliste braucht 1 bis 200 Eintraege".into(),
            ));
        }
        for muster in liste {
            if muster.is_empty() || muster.chars().count() > 200 {
                return Err(Fehler::Ungueltig("Muster braucht 1 bis 200 Zeichen".into()));
            }
            // Ein Muster ohne Wortinhalt wuerde alles treffen.
            if normalisieren(muster).is_empty() {
                return Err(Fehler::Ungueltig("Muster ohne Wortinhalt".into()));
            }
        }
    }
    Ok(())
}

fn pruefe_vorlage(v: &VorlageJeSprache) -> Result<(), Fehler> {
    for text in [&v.de, &v.en] {
        if text.is_empty() || text.chars().count() > 500 {
            return Err(Fehler::Ungueltig(
                "Vorlage braucht 1 bis 500 Zeichen".into(),
            ));
        }
    }
    Ok(())
}

fn ist_semver(s: &str) -> bool {
    let teile: Vec<&str> = s.split('.').collect();
    teile.len() == 3
        && teile.iter().all(|t| {
            !t.is_empty()
                && t.bytes().all(|b| b.is_ascii_digit())
                && (*t == "0" || !t.starts_with('0'))
        })
}

/// Entspricht `^[a-z][a-z0-9]*([._-][a-z0-9]+)*$`, hoechstens 64 Zeichen.
fn ist_slug(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 || !s.as_bytes()[0].is_ascii_lowercase() {
        return false;
    }
    let mut vorher_trenner = false;
    for b in s.bytes() {
        let trenner = matches!(b, b'.' | b'_' | b'-');
        if trenner && vorher_trenner {
            return false;
        }
        if !(trenner || b.is_ascii_lowercase() || b.is_ascii_digit()) {
            return false;
        }
        vorher_trenner = trenner;
    }
    !vorher_trenner
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn fixturedir(art: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/fixtures/assistent-einstufung")
            .join(art)
    }

    fn data_of(pfad: &std::path::Path) -> String {
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(pfad).unwrap()).unwrap();
        v["data"].to_string()
    }

    #[test]
    fn alle_gueltigen_fixtures_parsen() {
        let mut n = 0;
        for e in fs::read_dir(fixturedir("valid")).unwrap() {
            let p = e.unwrap().path();
            Regeltabelle::aus_json(&data_of(&p)).unwrap_or_else(|f| panic!("{p:?}: {f}"));
            n += 1;
        }
        assert!(n >= 2);
    }

    #[test]
    fn alle_ungueltigen_fixtures_werden_abgewiesen() {
        let mut n = 0;
        for e in fs::read_dir(fixturedir("invalid")).unwrap() {
            let p = e.unwrap().path();
            assert!(
                Regeltabelle::aus_json(&data_of(&p)).is_err(),
                "{p:?} haette scheitern muessen"
            );
            n += 1;
        }
        assert!(n >= 4);
    }

    #[test]
    fn unbekannte_absicht_und_fehlende_sprache_scheitern_in_serde() {
        for name in ["s0-unknown-absicht.json", "missing-language.json"] {
            let j = data_of(&fixturedir("invalid").join(name));
            assert!(
                matches!(Regeltabelle::aus_json(&j), Err(Fehler::Json(_))),
                "{name}"
            );
        }
    }

    #[test]
    fn leere_vorlage_und_s0_in_absichten_scheitern_in_pruefen() {
        for name in ["empty-template.json", "absichten-stufe-s0.json"] {
            let j = data_of(&fixturedir("invalid").join(name));
            assert!(
                matches!(Regeltabelle::aus_json(&j), Err(Fehler::Ungueltig(_))),
                "{name}"
            );
        }
    }

    #[test]
    fn unbekanntes_feld_wird_abgewiesen() {
        let j = data_of(&fixturedir("valid").join("minimal.json"))
            .replace("\"absichten\":[]", "\"absichten\":[],\"extra\":1");
        assert!(Regeltabelle::aus_json(&j).is_err());
    }

    #[test]
    fn standard_enthaelt_alle_fixture_absichten() {
        let std = Regeltabelle::standard();
        let fix = Regeltabelle::vertragsfixture();
        for a in &fix.absichten {
            assert!(
                std.absichten.iter().any(|b| b.absicht == a.absicht),
                "{}",
                a.absicht
            );
        }
        for r in &fix.s0 {
            let s = std
                .s0
                .iter()
                .find(|x| x.absicht == r.absicht)
                .expect("S0 fehlt");
            assert_eq!(s.antwort_vorlage, r.antwort_vorlage);
            for m in &r.muster.de {
                assert!(s.muster.de.contains(m));
            }
        }
    }

    #[test]
    fn muster_ohne_wortinhalt_wird_abgewiesen() {
        let mut t = Regeltabelle::vertragsfixture();
        t.s0[0].muster.de = vec!["...".into()];
        assert!(t.pruefen().is_err());
    }

    #[test]
    fn slug_pruefung() {
        assert!(ist_slug("mail-abfragen"));
        assert!(!ist_slug("Mail"));
        assert!(!ist_slug("a--b"));
        assert!(!ist_slug("a-"));
    }
}
