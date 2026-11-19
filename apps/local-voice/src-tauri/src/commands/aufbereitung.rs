//! Dokumentnahe Formatentwürfe; Dokumenttexte werden ausschließlich zurückgegeben.
//! Weder Texte noch Änderungen oder fremde Fehlermeldungen werden protokolliert.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::aufbereitung::{
    self, AenderungStatus, Aufbereitung, AufbereitungsSpeicher, Dokument, DokumentArt,
    ErgebnisFehler, Schritt, SpeicherFehler, Status,
};
use crate::text_aufbereiten::aenderungen::{regel_aenderungen, utf16_bereiche};

use super::text_aufbereiten::optionen_aus_regeln;

const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;

// Die Sperre umfasst auch Lesen, Entscheiden und mehrteilige Statuswechsel.
static BEFEHLSSPERRE: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Deserialize, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AufbereitungEntscheidung {
    Angenommen,
    Abgelehnt,
    Offen,
}

#[derive(Serialize, specta::Type)]
pub struct Uebernahme {
    pub text: Option<String>,
    pub aufbereitung: Aufbereitung,
}

#[derive(Serialize, specta::Type)]
pub struct Rueckgaengig {
    pub text: String,
    pub original: bool,
}

fn dokument_art(dokument: &Dokument) -> Result<&'static str, String> {
    match dokument.art {
        DokumentArt::Buch => Ok("buch"),
        DokumentArt::Seite => Ok("seite"),
        DokumentArt::Gespraech => Err("unsupported_document".into()),
    }
}

fn gleiches_dokument(links: &Dokument, rechts: &Dokument) -> bool {
    links.art == rechts.art && links.id == rechts.id
}

fn speicher_fehler(_: SpeicherFehler) -> String {
    "storage".into()
}

fn ergebnis_fehler(fehler: ErgebnisFehler) -> String {
    match fehler {
        ErgebnisFehler::Status => "wrong_status".into(),
        _ => "invalid_changes".into(),
    }
}

fn laden(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
) -> Result<Aufbereitung, String> {
    let aufbereitung = speicher.laden(id).map_err(speicher_fehler)?;
    if !gleiches_dokument(&aufbereitung.dokument, dokument) {
        return Err("storage".into());
    }
    Ok(aufbereitung)
}

fn liste(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
) -> Result<Vec<Aufbereitung>, String> {
    let mut fassungen = speicher.liste_lesbar().map_err(speicher_fehler)?;
    fassungen.retain(|a| gleiches_dokument(&a.dokument, dokument));
    fassungen.sort_by(|a, b| b.erstellt.cmp(&a.erstellt).then_with(|| b.id.cmp(&a.id)));
    Ok(fassungen)
}

pub(crate) fn entwurf_anlegen_in(
    speicher: &AufbereitungsSpeicher,
    dokument: Dokument,
    text: &str,
    regeln: Option<&[String]>,
    jetzt_ms: u64,
) -> Result<Aufbereitung, String> {
    dokument_art(&dokument)?;
    if text.len() > MAX_TEXT_BYTES {
        return Err("text_too_large".into());
    }
    let optionen = optionen_aus_regeln(regeln).map_err(str::to_string)?;
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    match speicher.original_einfrieren(text) {
        Ok(()) | Err(SpeicherFehler::OriginalFest) => {}
        Err(fehler) => return Err(speicher_fehler(fehler)),
    }
    let mut aufbereitung = Aufbereitung::neu(dokument, text, jetzt_ms);
    aufbereitung.schritte = vec![Schritt::Format];
    aufbereitung.aenderungen = regel_aenderungen(text, &optionen);
    speicher
        .speichern_mit_basis(&aufbereitung, text)
        .map_err(speicher_fehler)?;
    Ok(aufbereitung)
}

pub(crate) fn entscheiden_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
    gruppe: &str,
    entscheidung: AufbereitungEntscheidung,
    jetzt_ms: u64,
) -> Result<Aufbereitung, String> {
    dokument_art(dokument)?;
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    let mut aufbereitung = laden(speicher, dokument, id)?;
    if aufbereitung.status != Status::Entwurf {
        return Err("wrong_status".into());
    }
    let status = match entscheidung {
        AufbereitungEntscheidung::Angenommen => AenderungStatus::Angenommen,
        AufbereitungEntscheidung::Abgelehnt => AenderungStatus::Abgelehnt,
        AufbereitungEntscheidung::Offen => AenderungStatus::Offen,
    };
    let mut gefunden = false;
    for aenderung in &mut aufbereitung.aenderungen {
        if aenderung.gruppe.as_deref() == Some(gruppe) {
            aenderung.status = status;
            gefunden = true;
        }
    }
    if !gefunden {
        return Err("unknown_group".into());
    }
    aufbereitung.geaendert = jetzt_ms.max(aufbereitung.geaendert);
    speicher.speichern(&aufbereitung).map_err(speicher_fehler)?;
    Ok(aufbereitung)
}

pub(crate) fn uebernehmen_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
    aktueller_text: &str,
    jetzt_ms: u64,
) -> Result<Uebernahme, String> {
    dokument_art(dokument)?;
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    let mut aufbereitung = laden(speicher, dokument, id)?;
    let jetzt_ms = jetzt_ms.max(aufbereitung.geaendert);
    let text = match aufbereitung::uebernehmen(&mut aufbereitung, aktueller_text, jetzt_ms) {
        Ok(text) => text,
        Err(ErgebnisFehler::Konflikt) => {
            speicher.speichern(&aufbereitung).map_err(speicher_fehler)?;
            return Ok(Uebernahme {
                text: None,
                aufbereitung,
            });
        }
        Err(fehler) => return Err(ergebnis_fehler(fehler)),
    };
    let aktive: Vec<_> = liste(speicher, dokument)?
        .into_iter()
        .filter(|a| a.status == Status::Aktiv)
        .collect();
    if aktive.len() > 1 {
        return Err("storage".into());
    }
    aufbereitung.vorgaenger_aktiv = aktive.first().map(|a| a.id.clone());
    speicher
        .aktivierung_speichern(&aufbereitung)
        .map_err(speicher_fehler)?;
    Ok(Uebernahme {
        text: Some(text),
        aufbereitung,
    })
}

pub(crate) fn rueckgaengig_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
    jetzt_ms: u64,
) -> Result<Rueckgaengig, String> {
    dokument_art(dokument)?;
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    let mut aufbereitung = laden(speicher, dokument, id)?;
    if aufbereitung.status != Status::Aktiv {
        return Err("wrong_status".into());
    }
    let aktive = liste(speicher, dokument)?
        .into_iter()
        .filter(|a| a.status == Status::Aktiv)
        .count();
    if aktive != 1 {
        return Err("wrong_status".into());
    }
    let mut vorgaenger = match &aufbereitung.vorgaenger_aktiv {
        Some(id) => Some(laden(speicher, dokument, id)?),
        None => None,
    };
    let text = if let Some(vorgaenger) = &vorgaenger {
        if vorgaenger.status != Status::Abgeloest {
            return Err("wrong_status".into());
        }
        let basis = speicher
            .basis_text(&vorgaenger.id)
            .map_err(speicher_fehler)?;
        aufbereitung::ergebnis_text(&basis, &vorgaenger.aenderungen).unwrap_or(basis)
    } else {
        speicher
            .original_lesen()
            .map_err(speicher_fehler)?
            .ok_or_else(|| "storage".to_string())?
    };
    let vorher = aufbereitung.clone();
    aufbereitung.status = Status::Verworfen;
    aufbereitung.geaendert = jetzt_ms.max(aufbereitung.geaendert);
    speicher.speichern(&aufbereitung).map_err(speicher_fehler)?;
    if let Some(vorgaenger) = &mut vorgaenger {
        vorgaenger.status = Status::Aktiv;
        vorgaenger.geaendert = jetzt_ms.max(vorgaenger.geaendert);
        if let Err(fehler) = speicher.speichern(vorgaenger) {
            // Ohne erfolgreiche Reaktivierung bleibt die bisher aktive Fassung gültig.
            speicher.speichern(&vorher).map_err(speicher_fehler)?;
            return Err(speicher_fehler(fehler));
        }
    }
    Ok(Rueckgaengig {
        text,
        original: vorgaenger.is_none(),
    })
}

pub(crate) fn liste_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
) -> Result<Vec<Aufbereitung>, String> {
    dokument_art(dokument)?;
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    liste(speicher, dokument)
}

pub(crate) fn original_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
) -> Result<Option<String>, String> {
    dokument_art(dokument)?;
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    speicher.original_lesen().map_err(speicher_fehler)
}

pub(crate) fn utf16_bereiche_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
) -> Result<Vec<[usize; 2]>, String> {
    dokument_art(dokument)?;
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    let aufbereitung = laden(speicher, dokument, id)?;
    let basis = speicher.basis_text(id).map_err(speicher_fehler)?;
    Ok(utf16_bereiche(&basis, &aufbereitung.aenderungen))
}

async fn im_speicher<T, F>(
    app: tauri::AppHandle,
    dokument: Dokument,
    funktion: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&AufbereitungsSpeicher, Dokument, u64) -> Result<T, String> + Send + 'static,
{
    let art = dokument_art(&dokument)?;
    tauri::async_runtime::spawn_blocking(move || {
        let hash = format!("{:x}", Sha256::digest(dokument.id.as_bytes()));
        let ordner = crate::portable::app_data_dir(&app)
            .map_err(|_| "storage".to_string())?
            .join("aufbereitung_dokumente")
            .join(art)
            .join(&hash[..32]);
        let jetzt_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "storage".to_string())?
            .as_millis()
            .try_into()
            .map_err(|_| "storage".to_string())?;
        funktion(&AufbereitungsSpeicher::neu(ordner), dokument, jetzt_ms)
    })
    .await
    .map_err(|_| "worker_failed".to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn aufbereitung_entwurf_anlegen(
    app: tauri::AppHandle,
    dokument: Dokument,
    text: String,
    regeln: Option<Vec<String>>,
) -> Result<Aufbereitung, String> {
    im_speicher(app, dokument, move |speicher, dokument, jetzt| {
        entwurf_anlegen_in(speicher, dokument, &text, regeln.as_deref(), jetzt)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn aufbereitung_entscheiden(
    app: tauri::AppHandle,
    dokument: Dokument,
    id: String,
    gruppe: String,
    entscheidung: AufbereitungEntscheidung,
) -> Result<Aufbereitung, String> {
    im_speicher(app, dokument, move |speicher, dokument, jetzt| {
        entscheiden_in(speicher, &dokument, &id, &gruppe, entscheidung, jetzt)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn aufbereitung_uebernehmen(
    app: tauri::AppHandle,
    dokument: Dokument,
    id: String,
    aktueller_text: String,
) -> Result<Uebernahme, String> {
    im_speicher(app, dokument, move |speicher, dokument, jetzt| {
        uebernehmen_in(speicher, &dokument, &id, &aktueller_text, jetzt)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn aufbereitung_rueckgaengig(
    app: tauri::AppHandle,
    dokument: Dokument,
    id: String,
) -> Result<Rueckgaengig, String> {
    im_speicher(app, dokument, move |speicher, dokument, jetzt| {
        rueckgaengig_in(speicher, &dokument, &id, jetzt)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn aufbereitung_liste(
    app: tauri::AppHandle,
    dokument: Dokument,
) -> Result<Vec<Aufbereitung>, String> {
    im_speicher(app, dokument, |speicher, dokument, _| {
        liste_in(speicher, &dokument)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn aufbereitung_original(
    app: tauri::AppHandle,
    dokument: Dokument,
) -> Result<Option<String>, String> {
    im_speicher(app, dokument, |speicher, dokument, _| {
        original_in(speicher, &dokument)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn aufbereitung_utf16_bereiche(
    app: tauri::AppHandle,
    id: String,
    dokument: Dokument,
) -> Result<Vec<[usize; 2]>, String> {
    im_speicher(app, dokument, move |speicher, dokument, _| {
        utf16_bereiche_in(speicher, &dokument, &id)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aufbereitung::{ergebnis_text, Basis};

    fn dokument() -> Dokument {
        Dokument {
            art: DokumentArt::Seite,
            id: "seite".into(),
            fassung_id: None,
        }
    }

    fn umgebung() -> Result<(tempfile::TempDir, AufbereitungsSpeicher), String> {
        let temp = tempfile::tempdir().map_err(|_| "tempdir".to_string())?;
        let speicher = AufbereitungsSpeicher::neu(temp.path());
        Ok((temp, speicher))
    }

    fn entwurf(
        speicher: &AufbereitungsSpeicher,
        text: &str,
        jetzt: u64,
    ) -> Result<Aufbereitung, String> {
        entwurf_anlegen_in(speicher, dokument(), text, None, jetzt)
    }

    fn annehmen(
        speicher: &AufbereitungsSpeicher,
        aufbereitung: &Aufbereitung,
        jetzt: u64,
    ) -> Result<(), String> {
        let mut gruppen: Vec<_> = aufbereitung
            .aenderungen
            .iter()
            .filter_map(|a| a.gruppe.as_deref())
            .collect();
        gruppen.sort_unstable();
        gruppen.dedup();
        for gruppe in gruppen {
            entscheiden_in(
                speicher,
                &dokument(),
                &aufbereitung.id,
                gruppe,
                AufbereitungEntscheidung::Angenommen,
                jetzt,
            )?;
        }
        Ok(())
    }

    #[test]
    fn entwurf_friert_original_ein_und_speichert_basis_und_gruppen() -> Result<(), String> {
        // Gegenprobe: Ohne Einfrieren, Snapshot oder Regelauswertung fehlen diese Daten.
        let (_temp, speicher) = umgebung()?;
        assert_eq!(original_in(&speicher, &dokument())?, None);
        let text = "ﬃ\nﬀ\nEnde...";
        let a = entwurf(&speicher, text, 100)?;
        assert_eq!(a.status, Status::Entwurf);
        assert_eq!(a.basis, Basis::aus_text(text));
        assert_eq!(a.schritte, vec![Schritt::Format]);
        assert_eq!(a.aenderungen.len(), 3);
        assert!(a.aenderungen.iter().all(|a| a.gruppe.is_some()));
        assert_eq!(speicher.basis_text(&a.id).map_err(speicher_fehler)?, text);
        assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, a);
        entwurf(&speicher, "Neue Basis...", 200)?;
        assert_eq!(original_in(&speicher, &dokument())?, Some(text.into()));
        let leer = entwurf_anlegen_in(&speicher, dokument(), text, Some(&[]), 300)?;
        assert!(leer.aenderungen.is_empty());
        assert_eq!(speicher.laden(&leer.id).map_err(speicher_fehler)?, leer);
        let sauber = entwurf(&speicher, "Ein sauberer Satz.", 400)?;
        assert!(sauber.aenderungen.is_empty());
        Ok(())
    }

    #[test]
    fn regel_ids_entsprechen_dem_bestehenden_befehl() -> Result<(), String> {
        // Gegenprobe: Jede ausgelassene oder vertauschte Regel verändert den Vorschlagssatz.
        let (_temp, speicher) = umgebung()?;
        let text = "A\u{301}\nﬃ\nSil\u{ad}be\n\"Ja\"\nA - B\nEnde...\nA  B\n• Punkt";
        for regel in [
            "nfc",
            "ligaturen",
            "weiche_trennstriche",
            "anfuehrungszeichen",
            "gedankenstrich",
            "auslassung",
            "leerzeichen",
            "listen",
        ] {
            let regeln = vec![regel.to_string()];
            let a = entwurf_anlegen_in(&speicher, dokument(), text, Some(&regeln), 100)?;
            let opt = optionen_aus_regeln(Some(&regeln)).map_err(str::to_string)?;
            assert!(!a.aenderungen.is_empty());
            assert_eq!(a.aenderungen, regel_aenderungen(text, &opt));
        }
        Ok(())
    }

    #[test]
    fn entscheidung_gilt_fuer_alle_aenderungen_der_gruppe() -> Result<(), String> {
        // Gegenprobe: Nur der erste Treffer ließe die zweite Ligatur im alten Status.
        let (_temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "ﬃ\nﬀ\nEnde...", 100)?;
        for (entscheidung, status) in [
            (
                AufbereitungEntscheidung::Angenommen,
                AenderungStatus::Angenommen,
            ),
            (
                AufbereitungEntscheidung::Abgelehnt,
                AenderungStatus::Abgelehnt,
            ),
            (AufbereitungEntscheidung::Offen, AenderungStatus::Offen),
        ] {
            let a = entscheiden_in(
                &speicher,
                &dokument(),
                &a.id,
                "ligaturen",
                entscheidung,
                200,
            )?;
            let gruppe: Vec<_> = a
                .aenderungen
                .iter()
                .filter(|a| a.gruppe.as_deref() == Some("ligaturen"))
                .collect();
            assert_eq!(gruppe.len(), 2);
            assert!(gruppe.iter().all(|a| a.status == status));
            assert!(a
                .aenderungen
                .iter()
                .filter(|a| a.gruppe.as_deref() != Some("ligaturen"))
                .all(|a| a.status == AenderungStatus::Offen));
            assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, a);
        }
        let vorher = speicher.laden(&a.id).map_err(speicher_fehler)?;
        assert_eq!(
            entscheiden_in(
                &speicher,
                &dokument(),
                &a.id,
                "unbekannt",
                AufbereitungEntscheidung::Angenommen,
                300,
            )
            .err()
            .as_deref(),
            Some("unknown_group")
        );
        assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, vorher);
        Ok(())
    }

    #[test]
    fn teilannahme_liefert_genau_den_ergebnistext() -> Result<(), String> {
        // Gegenprobe: Vollannahme würde zusätzlich die Auslassung ersetzen.
        let (_temp, speicher) = umgebung()?;
        let text = "ﬃ\nﬀ\nEnde...";
        let a = entwurf(&speicher, text, 100)?;
        let a = entscheiden_in(
            &speicher,
            &dokument(),
            &a.id,
            "ligaturen",
            AufbereitungEntscheidung::Angenommen,
            200,
        )?;
        let erwartet = ergebnis_text(text, &a.aenderungen).map_err(ergebnis_fehler)?;
        let uebernahme = uebernehmen_in(&speicher, &dokument(), &a.id, text, 300)?;
        assert_eq!(uebernahme.text.as_deref(), Some(erwartet.as_str()));
        assert_eq!(uebernahme.text.as_deref(), Some("ffi\nff\nEnde..."));
        assert_eq!(uebernahme.aufbereitung.status, Status::Aktiv);
        assert_eq!(uebernahme.aufbereitung.vorgaenger_aktiv, None);
        assert_eq!(
            speicher.laden(&a.id).map_err(speicher_fehler)?,
            uebernahme.aufbereitung
        );
        Ok(())
    }

    #[test]
    fn konflikt_speichert_veraltet_und_liefert_keinen_text() -> Result<(), String> {
        // Gegenprobe: Gleich lange Fremdtexte müssen ebenfalls die SHA-Prüfung auslösen.
        let (_temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "ab", 100)?;
        let ergebnis = uebernehmen_in(&speicher, &dokument(), &a.id, "ba", 200)?;
        assert_eq!(ergebnis.text, None);
        assert_eq!(ergebnis.aufbereitung.status, Status::Veraltet);
        assert_eq!(ergebnis.aufbereitung.geaendert, 200);
        assert_eq!(
            speicher.laden(&a.id).map_err(speicher_fehler)?,
            ergebnis.aufbereitung
        );
        Ok(())
    }

    #[test]
    fn zweite_aktivierung_und_rueckgaengig_bis_zum_original() -> Result<(), String> {
        // Gegenprobe: Ohne nachträgliche Verknüpfung scheitert schon die zweite Aktivierung
        // an der Speicheridentität; ohne Reaktivierung scheitert das zweite Rückgängigmachen.
        let (_temp, speicher) = umgebung()?;
        let original = "ﬃ\nEnde...";
        let erste = entwurf_anlegen_in(
            &speicher,
            dokument(),
            original,
            Some(&["ligaturen".into()]),
            100,
        )?;
        annehmen(&speicher, &erste, 200)?;
        let erster_text = uebernehmen_in(&speicher, &dokument(), &erste.id, original, 300)?
            .text
            .ok_or_else(|| "missing_text".to_string())?;
        assert_eq!(erster_text, "ffi\nEnde...");
        let zweite = entwurf(&speicher, &erster_text, 400)?;
        annehmen(&speicher, &zweite, 500)?;
        let ergebnis = uebernehmen_in(&speicher, &dokument(), &zweite.id, &erster_text, 600)?;
        assert_eq!(ergebnis.text.as_deref(), Some("ffi\nEnde…"));
        assert_eq!(
            ergebnis.aufbereitung.vorgaenger_aktiv.as_deref(),
            Some(erste.id.as_str())
        );
        assert_eq!(
            speicher.laden(&erste.id).map_err(speicher_fehler)?.status,
            Status::Abgeloest
        );
        assert_eq!(
            rueckgaengig_in(&speicher, &dokument(), &erste.id, 650)
                .err()
                .as_deref(),
            Some("wrong_status")
        );
        let zurueck = rueckgaengig_in(&speicher, &dokument(), &zweite.id, 700)?;
        assert_eq!(zurueck.text, erster_text);
        assert!(!zurueck.original);
        assert_eq!(
            speicher.laden(&zweite.id).map_err(speicher_fehler)?.status,
            Status::Verworfen
        );
        assert_eq!(
            speicher.laden(&erste.id).map_err(speicher_fehler)?.status,
            Status::Aktiv
        );
        let zurueck = rueckgaengig_in(&speicher, &dokument(), &erste.id, 800)?;
        assert_eq!(zurueck.text, original);
        assert!(zurueck.original);
        assert!(liste_in(&speicher, &dokument())?
            .iter()
            .all(|a| a.status == Status::Verworfen));
        assert_eq!(original_in(&speicher, &dokument())?, Some(original.into()));
        Ok(())
    }

    #[test]
    fn status_sperrt_entscheiden_uebernehmen_und_rueckgaengig() -> Result<(), String> {
        // Gegenprobe: Jede fehlende Statusschranke erlaubt mindestens einen dieser Aufrufe.
        let (_temp, speicher) = umgebung()?;
        for status in [
            Status::Laeuft,
            Status::Entwurf,
            Status::Aktiv,
            Status::Abgeloest,
            Status::Verworfen,
            Status::Veraltet,
            Status::Abgebrochen,
        ] {
            let mut a = entwurf(&speicher, "ﬃ", 100)?;
            a.status = status;
            speicher.speichern(&a).map_err(speicher_fehler)?;
            if status != Status::Entwurf {
                assert_eq!(
                    entscheiden_in(
                        &speicher,
                        &dokument(),
                        &a.id,
                        "ligaturen",
                        AufbereitungEntscheidung::Angenommen,
                        200,
                    )
                    .err()
                    .as_deref(),
                    Some("wrong_status")
                );
                assert_eq!(
                    uebernehmen_in(&speicher, &dokument(), &a.id, "ﬃ", 200)
                        .err()
                        .as_deref(),
                    Some("wrong_status")
                );
            }
            if status != Status::Aktiv {
                assert_eq!(
                    rueckgaengig_in(&speicher, &dokument(), &a.id, 200)
                        .err()
                        .as_deref(),
                    Some("wrong_status")
                );
            }
            assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, a);
        }
        Ok(())
    }

    #[test]
    fn gespraech_wird_von_allen_kernen_abgelehnt() -> Result<(), String> {
        // Gegenprobe: Auch reine Leseaufrufe dürfen keinen Gesprächsspeicher öffnen.
        let (_temp, speicher) = umgebung()?;
        let mut d = dokument();
        d.art = DokumentArt::Gespraech;
        let fehler = [
            entwurf_anlegen_in(&speicher, d.clone(), "GEHEIMTEXT", None, 100).err(),
            entscheiden_in(
                &speicher,
                &d,
                "id",
                "gruppe",
                AufbereitungEntscheidung::Offen,
                100,
            )
            .err(),
            uebernehmen_in(&speicher, &d, "id", "GEHEIMTEXT", 100).err(),
            rueckgaengig_in(&speicher, &d, "id", 100).err(),
            liste_in(&speicher, &d).err(),
            original_in(&speicher, &d).err(),
            utf16_bereiche_in(&speicher, &d, "id").err(),
        ];
        assert!(fehler
            .iter()
            .all(|f| f.as_deref() == Some("unsupported_document")));
        assert_eq!(speicher.original_lesen().map_err(speicher_fehler)?, None);
        Ok(())
    }

    #[test]
    fn grenzen_und_unbekannte_regeln_geben_nur_codes_zurueck() -> Result<(), String> {
        // Gegenprobe: Fehler dürfen weder Inhalt übernehmen noch das Original anlegen.
        let (_temp, speicher) = umgebung()?;
        let geheim = "GEHEIMTEXT";
        let riesig = geheim.repeat(MAX_TEXT_BYTES / geheim.len() + 1);
        let zu_gross = entwurf(&speicher, &riesig, 100).err();
        assert_eq!(zu_gross.as_deref(), Some("text_too_large"));
        let regeln = vec![geheim.to_string()];
        let unbekannt = entwurf_anlegen_in(&speicher, dokument(), geheim, Some(&regeln), 100).err();
        assert_eq!(unbekannt.as_deref(), Some("unknown_rule"));
        let laden = uebernehmen_in(&speicher, &dokument(), geheim, geheim, 100).err();
        assert_eq!(laden.as_deref(), Some("storage"));
        for fehler in [zu_gross, unbekannt, laden].into_iter().flatten() {
            assert!(!fehler.contains(geheim));
            assert!(!fehler.contains(riesig.as_str()));
        }
        assert_eq!(original_in(&speicher, &dokument())?, None);
        let grenze = "a".repeat(MAX_TEXT_BYTES);
        let a = entwurf_anlegen_in(&speicher, dokument(), &grenze, Some(&[]), 200)?;
        assert_eq!(a.basis.laenge, MAX_TEXT_BYTES as u64);
        Ok(())
    }

    #[test]
    fn liste_sortiert_neueste_zuerst_und_ueberspringt_defekte_dateien() -> Result<(), String> {
        // Gegenprobe: Die bisherige strikte Speicherliste bricht am kaputten JSON ab.
        let (temp, speicher) = umgebung()?;
        let erste = entwurf(&speicher, "ab", 100)?;
        let zweite = entwurf(&speicher, "ab", 300)?;
        let dritte = entwurf(&speicher, "ab", 200)?;
        let ordner = temp.path().join("aufbereitung");
        for name in ["00000000000000000000000001.json", "ungueltige-id.json"] {
            std::fs::write(ordner.join(name), b"GEHEIMTEXT{").map_err(|_| "write".to_string())?;
        }
        let liste = liste_in(&speicher, &dokument())?;
        let ids: Vec<_> = liste.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![zweite.id.as_str(), dritte.id.as_str(), erste.id.as_str()]
        );
        assert!(speicher.liste().is_err());
        Ok(())
    }

    #[test]
    fn utf16_bereiche_verwenden_den_snapshot_statt_des_originals() -> Result<(), String> {
        // Gegenprobe: Byte-Indizes und das frühere Original ergeben andere Bereiche.
        let (_temp, speicher) = umgebung()?;
        entwurf(&speicher, "Anderes Original", 100)?;
        let a = entwurf(&speicher, "🦀 ﬃ\nä ﬀ\nÖ...", 200)?;
        assert_eq!(a.aenderungen.len(), 3);
        assert_eq!(
            utf16_bereiche_in(&speicher, &dokument(), &a.id)?,
            vec![[3, 4], [7, 8], [10, 13]]
        );
        Ok(())
    }

    #[test]
    fn dokumentzuordnung_und_buch_werden_beachtet() -> Result<(), String> {
        // Gegenprobe: Ein bloßer ID-Zugriff würde die Fassung eines anderen Dokuments liefern.
        let (_temp, speicher) = umgebung()?;
        let mut buch = dokument();
        buch.art = DokumentArt::Buch;
        let a = entwurf_anlegen_in(&speicher, buch.clone(), "ﬃ", None, 100)?;
        assert_eq!(liste_in(&speicher, &buch)?.len(), 1);
        assert!(liste_in(&speicher, &dokument())?.is_empty());
        assert_eq!(
            utf16_bereiche_in(&speicher, &dokument(), &a.id)
                .err()
                .as_deref(),
            Some("storage")
        );
        assert_eq!(utf16_bereiche_in(&speicher, &buch, &a.id)?, vec![[0, 1]]);
        assert_eq!(dokument_art(&buch)?, "buch");
        assert_eq!(dokument_art(&dokument())?, "seite");
        Ok(())
    }

    #[test]
    fn auswertungsfehler_werden_auf_feste_codes_begrenzt() {
        // Gegenprobe: Durchgereichte Fehlertexte wären kein stabiler Befehlscode.
        assert_eq!(ergebnis_fehler(ErgebnisFehler::Status), "wrong_status");
        for fehler in [
            ErgebnisFehler::Bereich,
            ErgebnisFehler::Ueberlappung,
            ErgebnisFehler::Geschuetzt,
            ErgebnisFehler::Alt,
            ErgebnisFehler::Gruppe,
        ] {
            assert_eq!(ergebnis_fehler(fehler), "invalid_changes");
        }
    }
}
