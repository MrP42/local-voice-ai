//! Befehl der Textaufbereitung (lva#658 A1 Schritt 3b): Formatregeln als Aenderungsvorschlaege.
//!
//! Reine Auswertung ohne Zustand und ohne Seiteneffekte: der Text geht hinein, die
//! Vorschlaege (Status `offen`, Byte-Bereiche im uebergebenen Text) kommen zurueck. Annehmen,
//! Ablehnen und Speichern gehoeren zu den Aufbereitungsfassungen (`aufbereitung`).
//! Weder Text noch Aenderungen werden protokolliert.

use crate::aufbereitung::Aenderung;
use crate::text_aufbereiten::aenderungen::regel_aenderungen;
use crate::text_aufbereiten::format::FormatOptionen;

/// Obergrenze des Textes (Schutz vor versehentlich riesigen Eingaben).
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn buch_id(dokument: &crate::aufbereitung::Dokument) -> Result<String, String> {
    if dokument.art != crate::aufbereitung::DokumentArt::Buch || dokument.id.is_empty() {
        return Err("unsupported_document".into());
    }
    // Auch Sprachfassungen desselben Buchs haben getrennte Jobs und Caches.
    serde_json::to_string(dokument).map_err(|_| "invalid_document".into())
}

#[tauri::command]
#[specta::specta]
pub async fn aufbereitung_buch_regeln(text: String) -> Result<Vec<Aenderung>, String> {
    if text.len() > MAX_TEXT_BYTES { return Err("text_too_large".into()); }
    tauri::async_runtime::spawn_blocking(move || crate::text_aufbereiten::buch::regeln(&text))
        .await.map_err(|_| "worker_failed".into())
}

#[tauri::command]
#[specta::specta]
pub fn aufbereitung_quelle_erkennen(text: String, art: Option<crate::aufbereitung::DokumentArt>)
    -> Result<crate::text_aufbereiten::buch::Quelle, String> {
    if text.len() > MAX_TEXT_BYTES { return Err("text_too_large".into()); }
    Ok(crate::text_aufbereiten::buch::quelle(&text, art))
}

#[tauri::command]
#[specta::specta]
pub fn aufbereitung_buch_umfang(app: tauri::AppHandle, text: String)
    -> Result<crate::text_aufbereiten::buch_job::BuchUmfang, String> {
    let versand = crate::text_aufbereiten::sprache::modell::Sprachversand::aus_app(&app)
        .map_err(|e| e.code().to_string())?;
    let wahl = versand.vorschau().map_err(|e| e.code().to_string())?;
    crate::text_aufbereiten::buch_job::umfang(&text, &wahl)
}

#[tauri::command]
#[specta::specta]
pub fn aufbereitung_buch_start(app: tauri::AppHandle, dokument: crate::aufbereitung::Dokument, text: String,
    ziel: String, ki_freigegeben: bool) -> Result<crate::text_aufbereiten::buch_job::BuchJobStatus, String> {
    crate::text_aufbereiten::buch_job::starten(app, buch_id(&dokument)?, text, ziel, ki_freigegeben, false)
}

#[tauri::command]
#[specta::specta]
pub fn aufbereitung_buch_fortsetzen(app: tauri::AppHandle, dokument: crate::aufbereitung::Dokument, text: String,
    ziel: String, ki_freigegeben: bool) -> Result<crate::text_aufbereiten::buch_job::BuchJobStatus, String> {
    crate::text_aufbereiten::buch_job::starten(app, buch_id(&dokument)?, text, ziel, ki_freigegeben, true)
}

#[tauri::command]
#[specta::specta]
pub fn aufbereitung_buch_pausieren(app: tauri::AppHandle, dokument: crate::aufbereitung::Dokument)
    -> Result<crate::text_aufbereiten::buch_job::BuchJobStatus, String> {
    crate::text_aufbereiten::buch_job::steuern(&app, &buch_id(&dokument)?, false)
}

#[tauri::command]
#[specta::specta]
pub fn aufbereitung_buch_stoppen(app: tauri::AppHandle, dokument: crate::aufbereitung::Dokument)
    -> Result<crate::text_aufbereiten::buch_job::BuchJobStatus, String> {
    crate::text_aufbereiten::buch_job::steuern(&app, &buch_id(&dokument)?, true)
}

#[tauri::command]
#[specta::specta]
pub fn aufbereitung_buch_status(app: tauri::AppHandle, dokument: crate::aufbereitung::Dokument)
    -> Result<crate::text_aufbereiten::buch_job::BuchJobStatus, String> {
    crate::text_aufbereiten::buch_job::status(&app, &buch_id(&dokument)?)
}

/// Baut die Optionen aus den Regel-IDs. `None` = alle Regeln; unbekannte ID = Fehlercode.
pub(crate) fn optionen_aus_regeln(
    regeln: Option<&[String]>,
) -> Result<FormatOptionen, &'static str> {
    let Some(regeln) = regeln else {
        return Ok(FormatOptionen::default());
    };
    let mut opt = FormatOptionen {
        nfc: false,
        ligaturen: false,
        weiche_trennstriche: false,
        anfuehrungszeichen: false,
        gedankenstrich: false,
        auslassung: false,
        leerzeichen: false,
        listen: false,
    };
    for regel in regeln {
        match regel.as_str() {
            "nfc" => opt.nfc = true,
            "ligaturen" => opt.ligaturen = true,
            "weiche_trennstriche" => opt.weiche_trennstriche = true,
            "anfuehrungszeichen" => opt.anfuehrungszeichen = true,
            "gedankenstrich" => opt.gedankenstrich = true,
            "auslassung" => opt.auslassung = true,
            "leerzeichen" => opt.leerzeichen = true,
            "listen" => opt.listen = true,
            _ => return Err("unknown_rule"),
        }
    }
    Ok(opt)
}

fn vorschlagen(text: &str, regeln: Option<&[String]>) -> Result<Vec<Aenderung>, String> {
    if text.len() > MAX_TEXT_BYTES {
        return Err("text_too_large".into());
    }
    let opt = optionen_aus_regeln(regeln).map_err(str::to_string)?;
    Ok(regel_aenderungen(text, &opt))
}

/// Formatregeln (ohne KI) als Aenderungsvorschlaege fuer `text`. `regeln` = Regel-IDs
/// (`nfc`, `ligaturen`, `weiche_trennstriche`, `anfuehrungszeichen`, `gedankenstrich`,
/// `auslassung`, `leerzeichen`, `listen`); ohne Angabe laufen alle. Fehlercodes: `text_too_large`,
/// `unknown_rule`.
#[tauri::command]
#[specta::specta]
pub async fn text_regeln_vorschlagen(
    text: String,
    regeln: Option<Vec<String>>,
) -> Result<Vec<Aenderung>, String> {
    tauri::async_runtime::spawn_blocking(move || vorschlagen(&text, regeln.as_deref()))
        .await
        .map_err(|_| "worker_failed".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aufbereitung::AenderungArt;

    #[test]
    fn buchjobs_trennen_sprachfassungen_und_lehnen_andere_quellen_ab() {
        // Gegenprobe: Nur die Buch-ID wuerde den Fortschritt verschiedener Fassungen vermischen.
        let mut dokument = crate::aufbereitung::Dokument {
            art: crate::aufbereitung::DokumentArt::Buch, id: "synthetisch".into(), fassung_id: Some("de".into()),
        };
        let deutsch = buch_id(&dokument).unwrap();
        dokument.fassung_id = Some("en".into());
        assert_ne!(deutsch, buch_id(&dokument).unwrap());
        dokument.art = crate::aufbereitung::DokumentArt::Seite;
        assert_eq!(buch_id(&dokument).err().as_deref(), Some("unsupported_document"));
    }

    /// Regel-ID eines Vorschlags (Gruppen sind seit lva#700 je Vorkommen benannt).
    fn regel_id(a: &Aenderung) -> Option<String> {
        match &a.art {
            AenderungArt::Regel(id) => Some(id.clone()),
            _ => None,
        }
    }

    #[test]
    fn without_a_rule_list_all_rules_run() {
        assert_eq!(optionen_aus_regeln(None), Ok(FormatOptionen::default()));
        let ergebnis = vorschlagen("Er sagte \"ja\". Dann kam es so...", None).unwrap();
        let regeln: Vec<_> = ergebnis.iter().filter_map(regel_id).collect();
        assert!(regeln.contains(&"anfuehrungszeichen".to_string()));
        assert!(regeln.contains(&"auslassung".to_string()));
    }

    #[test]
    fn a_rule_list_runs_only_those_rules() {
        let regeln = vec!["auslassung".to_string()];
        let ergebnis = vorschlagen("Er sagte \"ja\"...", Some(&regeln)).unwrap();
        assert!(!ergebnis.is_empty());
        assert!(ergebnis
            .iter()
            .all(|a| regel_id(a).as_deref() == Some("auslassung")));
    }

    #[test]
    fn an_empty_rule_list_proposes_nothing() {
        assert!(vorschlagen("Er sagte \"ja\"...", Some(&[]))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn an_unknown_rule_is_refused_with_a_code() {
        let regeln = vec!["nope".to_string()];
        assert_eq!(vorschlagen("x", Some(&regeln)).unwrap_err(), "unknown_rule");
    }

    #[test]
    fn a_huge_text_is_refused_before_any_work() {
        let riesig = "a".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(vorschlagen(&riesig, None).unwrap_err(), "text_too_large");
    }

    #[test]
    fn a_clean_text_proposes_nothing() {
        assert!(vorschlagen("Ein sauberer Satz.", None).unwrap().is_empty());
    }
}
