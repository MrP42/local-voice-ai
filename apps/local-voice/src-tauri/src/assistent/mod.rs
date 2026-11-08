//! Assistent-Kern in Local Voice AI (ADR-0028 Stufe A2, Epic lva#384, Issue lva#387).
//!
//! * [`uhr`]: `SystemUhr` (lokale Zeit) fuer Uhrzeit und Datum der S0-Antworten.
//! * Einstufung: `assistent_einstufen` stuft eine Aeusserung lokal nach Schwierigkeit S0-S3 ein
//!   (Regeltabelle `wai_assistent::Regeltabelle::standard()`, einmal geladen). S0 liefert eine
//!   Vorlagenantwort (`antwort`), ab S1 eine Ansage (`ansage`); kein Modell erzeugt diesen Text.
//! * Messung: `assistent_messung_melden` nimmt die Messpunkte E9 entgegen (nur Zeiten und Merkmale).
//!
//! Einstellung `assistent_einstufung_aktiv` (Standard an): aus = `assistent_einstufen` liefert
//! `None`, der Sprachanruf laeuft wie vor A2.
//!
//! # Eingehaengt
//! `src/components/voice-call/engine.ts` stuft vor dem Modellaufruf ein (Sprachanruf und Textmodus
//! des Gespraechs): S0 wird aus der Vorlage beantwortet ohne Modellaufruf, ab S1 kommt die Ansage
//! sofort, danach laeuft der bisherige Ablauf.
//!
//! # Belegregel (E4.2): nur verfuegbar, NICHT im Antwortpfad
//! [`belege_pruefen`] und [`ansagen`] machen `wai_assistent::belegregel` nutzbar und sind getestet
//! (`tests.rs`). Sie sind bewusst noch nicht in den bestehenden Antwortpfad geschaltet: das
//! Quellenmodell (welche Quelle belegt welchen Satz) kommt mit A3/A4. Bis dahin gibt das Sprach-
//! gespraech Modellsaetze unveraendert aus; die Belegregel greift dort noch nicht.
//!
//! # Datenschutz
//! Weder die Aeusserung noch Antworten stehen im Log. Die Messung traegt Kennung, Zeiten und
//! Merkmale. Sie wird nur als Debug-Zeile ohne Text ausgegeben; ein Feld im Beitrag (`ext`) gibt
//! es dafuer noch nicht (siehe `assistent_messung_melden`).

pub mod uhr;

#[cfg(test)]
mod tests;

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;
use wai_assistent::{
    einstufen, Ansagen, Einstufung, Messung, Pruefergebnis, Quellen, Regeltabelle, Satz, Sprache,
    Stufe, Uhr, Weg,
};

use uhr::SystemUhr;

/// Laengste Aeusserung, die eingestuft wird (Schutz vor Missbrauch des Befehls).
const MAX_TEXT_ZEICHEN: usize = 4_000;
/// Laengste Beitragskennung in der Messung.
const MAX_ID_ZEICHEN: usize = 128;

/// Die gemeinsame Regeltabelle, einmal geladen.
pub fn tabelle() -> &'static Regeltabelle {
    static TABELLE: OnceLock<Regeltabelle> = OnceLock::new();
    TABELLE.get_or_init(Regeltabelle::standard)
}

/// Bekannte Ansagetexte fuer die Belegregel (Ansagen der Tabelle plus feste Texte), einmal gebaut.
pub fn ansagen() -> &'static Ansagen {
    static ANSAGEN: OnceLock<Ansagen> = OnceLock::new();
    ANSAGEN.get_or_init(|| Ansagen::aus_tabelle(tabelle()))
}

/// Belegregel E4.2 ueber die gemeinsame Ansagenmenge. Noch nicht im Antwortpfad (siehe Modulkopf).
#[allow(dead_code)]
pub fn belege_pruefen(saetze: &[Satz], quellen: &dyn Quellen) -> Pruefergebnis {
    wai_assistent::pruefen(saetze, quellen, ansagen())
}

/// Stufe der Einstufung, wie sie die Oberflaeche sieht.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum StufeDto {
    S0,
    S1,
    S2,
    S3,
}

impl From<Stufe> for StufeDto {
    fn from(s: Stufe) -> Self {
        match s {
            Stufe::S0 => Self::S0,
            Stufe::S1 => Self::S1,
            Stufe::S2 => Self::S2,
            Stufe::S3 => Self::S3,
        }
    }
}

impl From<StufeDto> for Stufe {
    fn from(s: StufeDto) -> Self {
        match s {
            StufeDto::S0 => Self::S0,
            StufeDto::S1 => Self::S1,
            StufeDto::S2 => Self::S2,
            StufeDto::S3 => Self::S3,
        }
    }
}

/// Ergebnis der Einstufung (Vertragstyp `Einstufung` des Kerns).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EinstufungDto {
    pub stufe: StufeDto,
    /// Kennung der Absicht (Slug der Regeltabelle), z. B. `uhrzeit` oder `mail-lesen`.
    pub absicht: String,
    /// Ansage ab S1 (Vorlage der Tabelle).
    pub ansage: Option<String>,
    /// Vorlagenantwort bei S0.
    pub antwort: Option<String>,
}

impl From<Einstufung> for EinstufungDto {
    fn from(e: Einstufung) -> Self {
        Self {
            stufe: e.stufe.into(),
            absicht: e.absicht,
            ansage: e.ansage,
            antwort: e.antwort,
        }
    }
}

/// Sprache der Tabelle zu einem Sprachcode der Oberflaeche (`en`, `en-US` -> Englisch, sonst Deutsch).
pub fn sprache_aus(code: &str) -> Sprache {
    if code.trim().to_ascii_lowercase().starts_with("en") {
        Sprache::En
    } else {
        Sprache::De
    }
}

/// Einstufung mit Schalter und Uhr von aussen (testbar). `None`: Schalter aus, kein Text oder zu lang.
pub fn einstufen_mit(
    aktiv: bool,
    text: &str,
    sprache: Sprache,
    tabelle: &Regeltabelle,
    uhr: &dyn Uhr,
) -> Option<EinstufungDto> {
    if !aktiv || text.trim().is_empty() || text.chars().count() > MAX_TEXT_ZEICHEN {
        return None;
    }
    Some(einstufen(text, sprache, tabelle, uhr).into())
}

/// Stuft eine Aeusserung lokal ein (ADR-0028 E4.1). `None`, wenn die Einstufung abgeschaltet ist
/// (`assistent_einstufung_aktiv`). Der Text wird nicht protokolliert.
#[tauri::command]
#[specta::specta]
pub fn assistent_einstufen(
    app: AppHandle,
    text: String,
    sprache: String,
) -> Result<Option<EinstufungDto>, String> {
    let aktiv = crate::settings::get_settings(&app).assistent_einstufung_aktiv;
    Ok(einstufen_mit(
        aktiv,
        &text,
        sprache_aus(&sprache),
        tabelle(),
        &SystemUhr,
    ))
}

/// Weg der Antwort (Messpunkt E9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum WegDto {
    Lokal,
    Relay,
}

impl From<WegDto> for Weg {
    fn from(w: WegDto) -> Self {
        match w {
            WegDto::Lokal => Self::Lokal,
            WegDto::Relay => Self::Relay,
        }
    }
}

/// Messpunkte eines Beitrags (ADR-0028 E9), alle Zeiten in ms relativ zum Start (Sprachende bzw.
/// Senden). Keine Inhalte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MessungDto {
    pub beitrag_id: String,
    pub erste_ansage_ms: Option<u32>,
    pub erster_inhalt_ms: Option<u32>,
    pub fertig_ms: Option<u32>,
    pub warm: bool,
    pub weg: WegDto,
    pub stufe: StufeDto,
}

impl MessungDto {
    /// Pruefung und Umwandlung in die Messung des Kerns (Start = 0, die Werte sind schon relativ).
    pub fn pruefen(&self) -> Result<Messung, String> {
        let id = &self.beitrag_id;
        let schlicht = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.');
        if id.is_empty() || id.chars().count() > MAX_ID_ZEICHEN || !id.chars().all(schlicht) {
            return Err("Ungueltige Beitragskennung.".into());
        }
        let mut m = Messung::neu(id.clone(), 0, self.warm, self.weg.into(), self.stufe.into());
        m.erste_ansage_ms = self.erste_ansage_ms.map(u64::from);
        m.erster_inhalt_ms = self.erster_inhalt_ms.map(u64::from);
        m.fertig_ms = self.fertig_ms.map(u64::from);
        Ok(m)
    }
}

/// Textzeile der Messung ohne jeden Inhalt (Kennung, Zeiten, Merkmale).
pub fn messung_zeile(m: &Messung) -> String {
    let ms = |w: Option<u64>| w.map_or_else(|| "-".to_string(), |v| v.to_string());
    format!(
        "assistent-messung beitrag={} stufe={:?} weg={:?} warm={} ansage_ms={} inhalt_ms={} fertig_ms={}",
        m.beitrag_id,
        m.stufe,
        m.weg,
        m.warm,
        ms(m.erste_ansage_ms),
        ms(m.erster_inhalt_ms),
        ms(m.fertig_ms),
    )
}

/// Nimmt die Messpunkte eines Beitrags entgegen (E9). Nur Debug-Zeile ohne Text: das Beitragsmodell
/// (`conversations`, `ext["local-voice-ai"]`) hat noch keine Felder dafuer, und die Beitraege werden
/// von der Oberflaeche gespeichert; die Ablage am Beitrag folgt mit der Chat-Startseite.
#[tauri::command]
#[specta::specta]
pub fn assistent_messung_melden(messung: MessungDto) -> Result<(), String> {
    let m = messung.pruefen()?;
    log::debug!("{}", messung_zeile(&m));
    Ok(())
}

/// „Einstufung vor dem Modellaufruf“ ein- oder ausschalten (Standard an).
#[tauri::command]
#[specta::specta]
pub fn change_assistent_einstufung_aktiv_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.assistent_einstufung_aktiv = enabled;
    crate::settings::write_settings(&app, settings);
    Ok(())
}
