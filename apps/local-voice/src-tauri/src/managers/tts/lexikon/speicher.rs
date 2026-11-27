//! JSON-Ablage ohne Datenbank-Migration. Lese-Aendern-Schreiben unter den bestehenden Sperren.

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::{decode, validieren, LexikonDatei, LexikonEintrag, FEHLER};
use crate::{commands::pages, library};

static GLOBAL_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "art", rename_all = "snake_case", deny_unknown_fields)]
pub enum LexikonZiel {
    Global,
    Buch { id: String },
    Seite { id: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct LexikonKontext {
    pub buch_id: Option<String>,
    pub seite_id: Option<String>,
}

fn lesen(pfad: &Path) -> Result<Vec<LexikonEintrag>, String> {
    match std::fs::read_to_string(pfad) {
        Ok(raw) => decode(&raw),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err(FEHLER.into()),
    }
}

fn schreiben(pfad: &Path, eintraege: Vec<LexikonEintrag>) -> Result<(), String> {
    validieren(&eintraege)?;
    let raw = serde_json::to_vec_pretty(&LexikonDatei { fassung: 1, eintraege }).map_err(|_| FEHLER)?;
    pages::write_atomic(pfad, &raw).map_err(|_| FEHLER.into())
}

fn seitenstand(pfad: &Path) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    match std::fs::read_to_string(pfad) {
        Ok(raw) if raw.trim().is_empty() => Ok(serde_json::Map::new()),
        Ok(raw) => serde_json::from_str(&raw).map_err(|_| FEHLER.into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::Map::new()),
        Err(_) => Err(FEHLER.into()),
    }
}

fn seite_lesen(pfad: &Path) -> Result<Vec<LexikonEintrag>, String> {
    let stand = seitenstand(pfad)?;
    let Some(wert) = stand.get("aussprache_lexikon") else { return Ok(Vec::new()); };
    let liste: Vec<LexikonEintrag> = serde_json::from_value(wert.clone()).map_err(|_| FEHLER)?;
    validieren(&liste)?;
    Ok(liste)
}

pub(super) fn laden(app: &AppHandle, ziel: &LexikonZiel) -> Result<Vec<LexikonEintrag>, String> {
    match ziel {
        LexikonZiel::Global => {
            let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let basis = crate::portable::app_data_dir(app).map_err(|_| FEHLER)?;
            lesen(&basis.join("aussprache-lexikon.json"))
        }
        LexikonZiel::Buch { id } => {
            let _guard = library::lock();
            let root = library::sync_root(app).map_err(|_| FEHLER)?;
            let dir = library::sync_book_dir(&root, id).map_err(|_| FEHLER)?;
            let book = library::sync_read_book(&dir).ok_or(FEHLER)?;
            validieren(&book.aussprache_lexikon)?;
            Ok(book.aussprache_lexikon)
        }
        LexikonZiel::Seite { id } => {
            let _guard = pages::lock();
            let dir = pages::page_path(app, id).map_err(|_| FEHLER)?;
            seite_lesen(&dir.join("state.json"))
        }
    }
}

pub(in crate::managers::tts) fn fuer_kontext(app: &AppHandle, kontext: &LexikonKontext) -> Result<Vec<LexikonEintrag>, String> {
    // Kein Mutex bleibt ueber einen Aufruf der naechsten Fachsperre hinweg gehalten.
    let mut liste = Vec::new();
    if let Some(id) = &kontext.seite_id {
        liste.extend(laden(app, &LexikonZiel::Seite { id: id.clone() })?);
    }
    let buch = match &kontext.buch_id {
        Some(id) => Some(id.clone()),
        None => {
            // Index-Zuordnung statt aus einem Dateipfad auf ein Buch zu schliessen.
            if let Some(id) = &kontext.seite_id {
                let _guard = pages::lock();
                pages::pages_snapshot(app).map_err(|_| FEHLER)?.into_iter()
                    .find(|p| &p.id == id).and_then(|p| p.buch)
            } else { None }
        }
    };
    if let Some(id) = buch {
        liste.extend(laden(app, &LexikonZiel::Buch { id })?);
    }
    liste.extend(laden(app, &LexikonZiel::Global)?);
    Ok(liste)
}

#[tauri::command]
#[specta::specta]
pub fn lexikon_laden(app: AppHandle, ziel: LexikonZiel) -> Result<Vec<LexikonEintrag>, String> {
    laden(&app, &ziel)
}

/// Speichert nur ausdruecklich angenommene Eintraege. Vorschlaege schreiben niemals hierhin.
#[tauri::command]
#[specta::specta]
pub fn lexikon_speichern(app: AppHandle, ziel: LexikonZiel, eintraege: Vec<LexikonEintrag>) -> Result<(), String> {
    validieren(&eintraege)?;
    match &ziel {
        LexikonZiel::Global => {
            let _guard = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let basis = crate::portable::app_data_dir(&app).map_err(|_| FEHLER)?;
            std::fs::create_dir_all(&basis).map_err(|_| FEHLER)?;
            schreiben(&basis.join("aussprache-lexikon.json"), eintraege)?;
        }
        LexikonZiel::Buch { id } => {
            let _guard = library::lock();
            let root = library::sync_root(&app).map_err(|_| FEHLER)?;
            let dir = library::sync_book_dir(&root, id).map_err(|_| FEHLER)?;
            let mut book = library::sync_read_book(&dir).ok_or(FEHLER)?;
            book.aussprache_lexikon = eintraege;
            library::sync_write_book(&dir, &book).map_err(|_| FEHLER)?;
        }
        LexikonZiel::Seite { id } => {
            let _guard = pages::lock();
            let dir = pages::page_path(&app, id).map_err(|_| FEHLER)?;
            let pfad = dir.join("state.json");
            let mut stand = seitenstand(&pfad)?;
            stand.insert("aussprache_lexikon".into(), serde_json::to_value(eintraege).map_err(|_| FEHLER)?);
            let raw = serde_json::to_vec(&stand).map_err(|_| FEHLER)?;
            pages::write_atomic(&pfad, &raw).map_err(|_| FEHLER)?;
        }
    }
    app.state::<Arc<super::super::TtsManager>>().lexikon_auffrischen()
}

/// Vor einem Vorlese-/Vorpuffer-/Exportauftrag setzen; leer bedeutet ausschliesslich global.
/// Kein Erraten anhand des Textes, keine dauerhafte Einstellung in der App-Konfiguration.
#[tauri::command]
#[specta::specta]
pub fn lexikon_kontext(app: AppHandle, kontext: LexikonKontext) -> Result<(), String> {
    app.state::<Arc<super::super::TtsManager>>().lexikon_kontext_setzen(kontext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datei_roundtrip_und_fehler_lassen_alte_datei_stehen() {
        let dir = tempfile::tempdir().unwrap();
        let pfad = dir.path().join("aussprache-lexikon.json");
        assert!(lesen(&pfad).unwrap().is_empty());
        let liste = vec![LexikonEintrag { wort: "SAP".into(), aussprache: "Es-A-Pe".into(), gross_klein: true }];
        schreiben(&pfad, liste.clone()).unwrap();
        assert_eq!(lesen(&pfad).unwrap(), liste);
        let vorher = std::fs::read(&pfad).unwrap();
        assert!(schreiben(&pfad, vec![LexikonEintrag { wort: "".into(), ..liste[0].clone() }]).is_err());
        assert_eq!(std::fs::read(&pfad).unwrap(), vorher);
        std::fs::write(&pfad, br#"{"fassung":99,"eintraege":[]}"#).unwrap();
        assert!(lesen(&pfad).is_err());
    }

    #[test]
    fn seitenstand_erhaelt_fremde_metadaten_und_verweigert_kaputte_datei() {
        let dir = tempfile::tempdir().unwrap();
        let pfad = dir.path().join("state.json");
        std::fs::write(&pfad, r#"{"text":"synthetisch","fremd":{"x":1},"aussprache_lexikon":[{"wort":"SAP","aussprache":"Es-A-Pe","gross_klein":true}]}"#).unwrap();
        assert_eq!(seite_lesen(&pfad).unwrap()[0].wort, "SAP");
        assert_eq!(seitenstand(&pfad).unwrap()["fremd"]["x"], 1);
        std::fs::write(&pfad, "kaputt").unwrap();
        assert!(seite_lesen(&pfad).is_err());
    }
}
