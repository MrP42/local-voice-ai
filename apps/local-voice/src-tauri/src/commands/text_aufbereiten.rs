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

/// Baut die Optionen aus den Regel-IDs. `None` = alle Regeln; unbekannte ID = Fehlercode.
fn optionen_aus_regeln(regeln: Option<&[String]>) -> Result<FormatOptionen, &'static str> {
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

    #[test]
    fn without_a_rule_list_all_rules_run() {
        assert_eq!(optionen_aus_regeln(None), Ok(FormatOptionen::default()));
        let ergebnis = vorschlagen("Er sagte \"ja\". Dann kam es so...", None).unwrap();
        let regeln: Vec<_> = ergebnis.iter().filter_map(|a| a.gruppe.clone()).collect();
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
            .all(|a| a.gruppe.as_deref() == Some("auslassung")));
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
