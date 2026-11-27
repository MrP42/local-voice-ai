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
    pub warnung: Option<String>,
}

#[derive(Serialize, specta::Type)]
pub struct Rueckgaengig {
    pub text: String,
    pub original: bool,
    pub warnung: Option<String>,
}

fn dokument_art(dokument: &Dokument) -> Result<&'static str, String> {
    match dokument.art {
        DokumentArt::Buch => Ok("buch"),
        DokumentArt::Seite => Ok("seite"),
        DokumentArt::Gespraech => Err("unsupported_document".into()),
    }
}

fn gleiches_dokument(links: &Dokument, rechts: &Dokument) -> bool {
    links == rechts
}

fn dokument_hash(dokument: &Dokument) -> String {
    let mut hash = Sha256::new();
    hash.update(dokument.id.as_bytes());
    if let Some(fassung) = &dokument.fassung_id {
        hash.update([0]);
        hash.update(fassung.as_bytes());
    }
    format!("{:x}", hash.finalize())[..32].to_string()
}

fn speicher_fehler(_: SpeicherFehler) -> String {
    "storage".into()
}

fn rueckgaengig_fehler(fehler: SpeicherFehler) -> String {
    match fehler {
        SpeicherFehler::Aenderungen => "invalid_changes".into(),
        _ => speicher_fehler(fehler),
    }
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
    liste(speicher, dokument)?;
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
    liste_mit(speicher, dokument, |fassung| speicher.speichern(fassung))
}

fn liste_mit(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    mut speichern: impl FnMut(&Aufbereitung) -> Result<(), SpeicherFehler>,
) -> Result<Vec<Aufbereitung>, String> {
    let mut fassungen = speicher.liste_lesbar().map_err(speicher_fehler)?;
    fassungen.retain(|a| gleiches_dokument(&a.dokument, dokument));
    let aktive_id = fassungen
        .iter()
        .filter(|a| a.status == Status::Aktiv)
        .max_by(|a, b| {
            a.geaendert
                .cmp(&b.geaendert)
                .then_with(|| a.erstellt.cmp(&b.erstellt))
                .then_with(|| a.id.cmp(&b.id))
        })
        .map(|a| a.id.clone());
    for fassung in &mut fassungen {
        if fassung.status == Status::Aktiv && Some(&fassung.id) != aktive_id.as_ref() {
            // Nach einem Abbruch bleibt die zuletzt aktivierte Fassung maßgeblich.
            fassung.status = Status::Abgeloest;
            speichern(fassung).map_err(speicher_fehler)?;
        }
    }
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
    entwurf_anlegen_mit_ki_in(speicher, dokument, text, regeln, None, jetzt_ms)
}

fn entwurf_anlegen_mit_ki_in(
    speicher: &AufbereitungsSpeicher,
    dokument: Dokument,
    text: &str,
    regeln: Option<&[String]>,
    ki: Option<Vec<crate::aufbereitung::Aenderung>>,
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
    liste(speicher, &dokument)?;
    match speicher.original_einfrieren(text) {
        Ok(()) | Err(SpeicherFehler::OriginalFest) => {}
        Err(fehler) => return Err(speicher_fehler(fehler)),
    }
    let buch = dokument.art == DokumentArt::Buch && regeln.is_none();
    let mut aufbereitung = Aufbereitung::neu(dokument, text, jetzt_ms);
    aufbereitung.schritte = if buch { vec![Schritt::Struktur, Schritt::Format] } else { vec![Schritt::Format] };
    aufbereitung.aenderungen = if buch {
        crate::text_aufbereiten::buch::regeln_mit_format(text, &optionen)
    } else {
        regel_aenderungen(text, &optionen)
    };
    if let Some(ki) = ki {
        use crate::text_aufbereiten::sprache::{konflikt, fehler::Zaehler, vertrag::SprachGruppe};
        let mut gruppen = std::collections::BTreeMap::<String, Vec<crate::aufbereitung::Aenderung>>::new();
        for a in ki {
            gruppen.entry(a.gruppe.clone().unwrap_or_else(|| a.id.clone())).or_default().push(a);
        }
        let gruppen = gruppen.into_values().map(|aenderungen| SprachGruppe { zeile: 0, aenderungen }).collect();
        let gruppen = konflikt::verwerfe_regelkonflikte(gruppen, &aufbereitung.aenderungen, &mut Zaehler::default());
        konflikt::pruefe_gesamt(text, &aufbereitung.aenderungen, &gruppen).map_err(ergebnis_fehler)?;
        aufbereitung.aenderungen.extend(gruppen.into_iter().flat_map(|g| g.aenderungen));
        aufbereitung.aenderungen.sort_by_key(|a| a.bereich);
        aufbereitung.schritte.push(Schritt::Sprache);
    }
    // Änderungen und Schritte gehören zur unveränderlichen Identität der ersten Speicherung.
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

pub(crate) fn entscheiden_regel_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
    regel: Option<&str>,
    entscheidung: AufbereitungEntscheidung,
    jetzt_ms: u64,
) -> Result<Aufbereitung, String> {
    entscheiden_regel_in_mit(
        speicher,
        dokument,
        id,
        (regel, entscheidung),
        jetzt_ms,
        |fassung| speicher.speichern(fassung),
    )
}

fn entscheiden_regel_in_mit(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
    auswahl: (Option<&str>, AufbereitungEntscheidung),
    jetzt_ms: u64,
    speichern: impl FnOnce(&Aufbereitung) -> Result<(), SpeicherFehler>,
) -> Result<Aufbereitung, String> {
    let (regel, entscheidung) = auswahl;
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
        if regel.is_none()
            || matches!(
                &aenderung.art,
                crate::aufbereitung::AenderungArt::Regel(r) if Some(r.as_str()) == regel
            )
        {
            aenderung.status = status;
            gefunden = true;
        }
    }
    if !gefunden && regel.is_some() {
        return Err("unknown_group".into());
    }
    aufbereitung.geaendert = jetzt_ms.max(aufbereitung.geaendert);
    speichern(&aufbereitung).map_err(speicher_fehler)?;
    Ok(aufbereitung)
}

pub(crate) fn uebernehmen_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
    aktueller_text: &str,
    jetzt_ms: u64,
) -> Result<Uebernahme, String> {
    uebernehmen_in_mit(
        speicher,
        dokument,
        id,
        aktueller_text,
        jetzt_ms,
        |fassung| speicher.aktivierung_speichern(fassung),
    )
}

fn uebernehmen_in_mit(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
    aktueller_text: &str,
    jetzt_ms: u64,
    aktivieren: impl FnOnce(&Aufbereitung) -> Result<(), SpeicherFehler>,
) -> Result<Uebernahme, String> {
    dokument_art(dokument)?;
    if aktueller_text.len() > MAX_TEXT_BYTES {
        return Err("text_too_large".into());
    }
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    let mut aufbereitung = laden(speicher, dokument, id)?;
    let aktive: Vec<_> = liste(speicher, dokument)?
        .into_iter()
        .filter(|a| a.status == Status::Aktiv)
        .collect();
    let jetzt_ms = match aktive.first() {
        Some(aktiv) => jetzt_ms.max(
            aktiv
                .geaendert
                .checked_add(1)
                .ok_or_else(|| "storage".to_string())?,
        ),
        None => jetzt_ms,
    }
    .max(aufbereitung.geaendert);
    let text = match aufbereitung::uebernehmen(&mut aufbereitung, aktueller_text, jetzt_ms) {
        Ok(text) => text,
        Err(ErgebnisFehler::Konflikt) => {
            speicher.speichern(&aufbereitung).map_err(speicher_fehler)?;
            return Ok(Uebernahme {
                text: None,
                aufbereitung,
                warnung: None,
            });
        }
        Err(fehler) => return Err(ergebnis_fehler(fehler)),
    };
    aufbereitung.vorgaenger_aktiv = aktive.first().map(|a| a.id.clone());
    let warnung = if aktivieren(&aufbereitung).is_err() {
        // Nur die vollständig gespeicherte Aktivierung erlaubt die Textübernahme.
        if speicher.laden(id).map_err(speicher_fehler)? != aufbereitung {
            return Err("storage".into());
        }
        Some("storage_partial".into())
    } else {
        None
    };
    Ok(Uebernahme {
        text: Some(text),
        aufbereitung,
        warnung,
    })
}

pub(crate) fn rueckgaengig_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
    jetzt_ms: u64,
) -> Result<Rueckgaengig, String> {
    rueckgaengig_in_mit(speicher, dokument, id, jetzt_ms, |fassung| {
        speicher.speichern(fassung)
    })
}

fn rueckgaengig_in_mit(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    id: &str,
    jetzt_ms: u64,
    abloesen: impl FnOnce(&Aufbereitung) -> Result<(), SpeicherFehler>,
) -> Result<Rueckgaengig, String> {
    dokument_art(dokument)?;
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    liste(speicher, dokument)?;
    let mut aufbereitung = speicher.laden(id).map_err(rueckgaengig_fehler)?;
    if !gleiches_dokument(&aufbereitung.dokument, dokument) {
        return Err("storage".into());
    }
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
        Some(id) => {
            let vorgaenger = speicher.laden(id).map_err(rueckgaengig_fehler)?;
            if !gleiches_dokument(&vorgaenger.dokument, dokument) {
                return Err("storage".into());
            }
            Some(vorgaenger)
        }
        None => None,
    };
    if let Some(vorgaenger) = &vorgaenger {
        if vorgaenger.status != Status::Abgeloest {
            return Err("wrong_status".into());
        }
    }
    // Rückgängig stellt auch zwischenzeitliche Handänderungen wieder her.
    let text = speicher.basis_text(id).map_err(speicher_fehler)?;
    let original = speicher
        .original_lesen()
        .map_err(speicher_fehler)?
        .as_deref()
        == Some(text.as_str());
    if let Some(vorgaenger) = &mut vorgaenger {
        vorgaenger.status = Status::Aktiv;
        vorgaenger.geaendert = jetzt_ms.max(vorgaenger.geaendert).max(
            aufbereitung
                .geaendert
                .checked_add(1)
                .ok_or_else(|| "storage".to_string())?,
        );
        speicher.speichern(vorgaenger).map_err(speicher_fehler)?;
    }
    aufbereitung.status = Status::Abgeloest;
    aufbereitung.geaendert = jetzt_ms.max(aufbereitung.geaendert);
    let warnung = match abloesen(&aufbereitung) {
        Ok(()) => None,
        // Der reaktivierte Vorgänger gewinnt bei der nächsten Reparatur.
        Err(_) if vorgaenger.is_some() => Some("storage_partial".into()),
        Err(fehler) => return Err(speicher_fehler(fehler)),
    };
    Ok(Rueckgaengig {
        text,
        original,
        warnung,
    })
}

pub(crate) fn liste_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
) -> Result<Vec<Aufbereitung>, String> {
    liste_in_mit(speicher, dokument, |fassung| speicher.speichern(fassung))
}

fn liste_in_mit(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    mut speichern: impl FnMut(&Aufbereitung) -> Result<(), SpeicherFehler>,
) -> Result<Vec<Aufbereitung>, String> {
    dokument_art(dokument)?;
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    liste_mit(speicher, dokument, |fassung| {
        // Eine fehlgeschlagene Reparatur darf reines Lesen nicht verhindern.
        let _ = speichern(fassung);
        Ok(())
    })
}

pub(crate) fn original_in(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
) -> Result<Option<String>, String> {
    original_in_mit(speicher, dokument, |fassung| speicher.speichern(fassung))
}

fn original_in_mit(
    speicher: &AufbereitungsSpeicher,
    dokument: &Dokument,
    mut speichern: impl FnMut(&Aufbereitung) -> Result<(), SpeicherFehler>,
) -> Result<Option<String>, String> {
    dokument_art(dokument)?;
    let _sperre = BEFEHLSSPERRE
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    liste_mit(speicher, dokument, |fassung| {
        let _ = speichern(fassung);
        Ok(())
    })?;
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
        let hash = dokument_hash(&dokument);
        let ordner = crate::portable::app_data_dir(&app)
            .map_err(|_| "storage".to_string())?
            .join("aufbereitung_dokumente")
            .join(art)
            .join(hash);
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
pub async fn aufbereitung_buch_entwurf_aus_job(
    app: tauri::AppHandle,
    dokument: Dokument,
    text: String,
) -> Result<Aufbereitung, String> {
    let key = super::text_aufbereiten::buch_id(&dokument)?;
    let ki = crate::text_aufbereiten::buch_job::vorschlaege(&app, &key, &text)?;
    im_speicher(app, dokument, move |speicher, dokument, jetzt| {
        buch_entwurf_aus_job_in(speicher, dokument, &text, ki, jetzt)
    }).await
}

fn buch_entwurf_aus_job_in(
    speicher: &AufbereitungsSpeicher,
    dokument: Dokument,
    text: &str,
    ki: Vec<crate::aufbereitung::Aenderung>,
    jetzt: u64,
) -> Result<Aufbereitung, String> {
    entwurf_anlegen_mit_ki_in(speicher, dokument, text, None, Some(ki), jetzt)
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
pub async fn aufbereitung_entscheiden_regel(
    app: tauri::AppHandle,
    dokument: Dokument,
    id: String,
    regel: Option<String>,
    entscheidung: AufbereitungEntscheidung,
) -> Result<Aufbereitung, String> {
    im_speicher(app, dokument, move |speicher, dokument, jetzt| {
        entscheiden_regel_in(
            speicher,
            &dokument,
            &id,
            regel.as_deref(),
            entscheidung,
            jetzt,
        )
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
    #[test]
    fn buchentwurf_verwirft_bei_regelkonflikt_die_gesamte_ki_gruppe() -> Result<(), String> {
        // Gegenprobe: Ohne A3-Konfliktpruefung bliebe ein halber KI-Zeilenvorschlag uebrig.
        let (_temp, speicher) = umgebung()?;
        let mut buch = dokument();
        buch.art = DokumentArt::Buch;
        let ki = crate::text_aufbereiten::sprache::vertrag::SprachGruppe::neu(1, vec![
            ([0, 3], "ﬃ".into(), "ffi".into()), ([4, 9], "Feler".into(), "Fehler".into()),
        ]).aenderungen;
        let a = buch_entwurf_aus_job_in(&speicher, buch, "ﬃ Feler", ki, 100)?;
        assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, a);
        assert_eq!(speicher.liste().map_err(speicher_fehler)?.len(), 1);
        assert_eq!(a.aenderungen.len(), 1);
        assert!(matches!(a.aenderungen[0].art, crate::aufbereitung::AenderungArt::Regel(_)));
        Ok(())
    }

    #[test]
    fn ki_buchvorschlaege_werden_nur_als_neue_a0_fassung_angenommen() -> Result<(), String> {
        // Gegenprobe: Ohne A0-Anbindung kann kein KI-Treffer angenommen/zurueckgenommen werden.
        let (_temp, speicher) = umgebung()?;
        let mut buch = dokument();
        buch.art = DokumentArt::Buch;
        let text = "Ein Feler bleibt.";
        let ki = crate::text_aufbereiten::sprache::vertrag::SprachGruppe::neu(1,
            vec![([4, 9], "Feler".into(), "Fehler".into())]).aenderungen;
        let a = buch_entwurf_aus_job_in(&speicher, buch.clone(), text, ki, 100)?;
        assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, a);
        assert_eq!(speicher.liste().map_err(speicher_fehler)?.len(), 1);
        assert_eq!(a.schritte, vec![Schritt::Struktur, Schritt::Format, Schritt::Sprache]);
        assert_eq!(a.status, Status::Entwurf);
        assert_eq!(a.aenderungen.len(), 1);
        assert_eq!(a.aenderungen[0].status, AenderungStatus::Offen);
        let gruppe = a.aenderungen[0].gruppe.as_ref().unwrap();
        entscheiden_in(&speicher, &buch, &a.id, gruppe, AufbereitungEntscheidung::Angenommen, 200)?;
        let ergebnis = uebernehmen_in(&speicher, &buch, &a.id, text, 300)?;
        assert_eq!(ergebnis.text.as_deref(), Some("Ein Fehler bleibt."));
        assert_eq!(original_in(&speicher, &buch)?.as_deref(), Some(text));
        assert!(rueckgaengig_in(&speicher, &buch, &a.id, 400)?.original);
        Ok(())
    }

    #[test]
    fn buchtreffer_sind_im_a2_entwurf_einzeln_und_je_art_annehmbar() -> Result<(), String> {
        // Gegenprobe: Ohne Buchregeln im Entwurf existiert keine buch_kopfzeile-Gruppe.
        let (_temp, speicher) = umgebung()?;
        let mut buch = dokument();
        buch.art = DokumentArt::Buch;
        let text: String = (1..=5).map(|n| format!("{{{{S. {n}}}}}\nSynthetic Book\n\nKontrollabsatz.\n\n")).collect();
        let a = entwurf_anlegen_in(&speicher, buch.clone(), &text, None, 100)?;
        assert_eq!(a.aenderungen.len(), 5);
        let gruppe = a.aenderungen[0].gruppe.as_ref().unwrap();
        let einzeln = entscheiden_in(&speicher, &buch, &a.id, gruppe, AufbereitungEntscheidung::Angenommen, 200)?;
        assert_eq!(einzeln.aenderungen.iter().filter(|a| a.status == AenderungStatus::Angenommen).count(), 1);
        let alle = entscheiden_regel_in(&speicher, &buch, &a.id, Some("buch_kopfzeile"), AufbereitungEntscheidung::Angenommen, 300)?;
        assert!(alle.aenderungen.iter().all(|a| a.status == AenderungStatus::Angenommen));
        assert_eq!(original_in(&speicher, &buch)?, Some(text));
        Ok(())
    }
    use super::*;
    use crate::aufbereitung::{ergebnis_text, Basis};

    #[test]
    fn regelentscheidung_schreibt_alle_treffer_genau_einmal() -> Result<(), String> {
        // Gegenprobe: Einzelgruppenaufrufe schreiben mehrfach oder lassen Treffer offen.
        let (_temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "ﬃ\nﬀ\nEnde...", 100)?;
        assert_eq!(gruppen_der_regel(&a, "ligaturen").len(), 2);
        for regel in [Some("ligaturen"), None] {
            let mut schreibvorgaenge = 0;
            let entschieden = entscheiden_regel_in_mit(
                &speicher,
                &dokument(),
                &a.id,
                (regel, AufbereitungEntscheidung::Angenommen),
                200,
                |fassung| {
                    schreibvorgaenge += 1;
                    speicher.speichern(fassung)
                },
            )?;
            assert_eq!(schreibvorgaenge, 1);
            for aenderung in &entschieden.aenderungen {
                let erwartet = if regel.is_none()
                    || matches!(
                        &aenderung.art,
                        crate::aufbereitung::AenderungArt::Regel(r) if r == "ligaturen"
                    ) {
                    AenderungStatus::Angenommen
                } else {
                    AenderungStatus::Offen
                };
                assert_eq!(aenderung.status, erwartet);
            }
            assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, entschieden);
        }
        let vorher = speicher.laden(&a.id).map_err(speicher_fehler)?;
        assert_eq!(
            entscheiden_regel_in(
                &speicher,
                &dokument(),
                &a.id,
                Some("unbekannt"),
                AufbereitungEntscheidung::Abgelehnt,
                300,
            )
            .err()
            .as_deref(),
            Some("unknown_group")
        );
        assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, vorher);
        uebernehmen_in(&speicher, &dokument(), &a.id, "ﬃ\nﬀ\nEnde...", 400)?;
        for regel in [Some("ligaturen"), None] {
            assert_eq!(
                entscheiden_regel_in(
                    &speicher,
                    &dokument(),
                    &a.id,
                    regel,
                    AufbereitungEntscheidung::Offen,
                    500,
                )
                .err()
                .as_deref(),
                Some("wrong_status")
            );
        }
        Ok(())
    }

    #[test]
    fn verworfene_textuebernahme_rollt_nur_die_aktivierung_zurueck() -> Result<(), String> {
        // Gegenprobe: Ohne Rücknahme bleibt die im Editor verworfene Fassung aktiv.
        let (_temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "ﬃ\nEnde...", 100)?;
        uebernehmen_in(&speicher, &dokument(), &a.id, "ﬃ\nEnde...", 200)?;
        let b = entwurf(&speicher, "ﬃ\nEnde...", 300)?;
        annehmen(&speicher, &b, 350)?;
        let ergebnis = uebernehmen_in(&speicher, &dokument(), &b.id, "ﬃ\nEnde...", 400)?;
        assert_eq!(ergebnis.text.as_deref(), Some("ffi\nEnde…"));
        // Die UI verwirft beide Antworttexte und verwendet nur die Statusänderung.
        let _ = rueckgaengig_in(&speicher, &dokument(), &b.id, 500)?;
        assert_eq!(
            speicher.laden(&b.id).map_err(speicher_fehler)?.status,
            Status::Abgeloest
        );
        assert_eq!(
            speicher.laden(&a.id).map_err(speicher_fehler)?.status,
            Status::Aktiv
        );
        assert!(rueckgaengig_in(&speicher, &dokument(), &a.id, 600)?.original);
        Ok(())
    }

    #[test]
    fn regelentscheidung_lehnt_ab_oeffnet_und_meldet_speicherfehler() -> Result<(), String> {
        // Gegenprobe: Ein fest verdrahteter Status oder verschluckter Schreibfehler fällt auf.
        let (_temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "ﬃ\nﬀ", 100)?;
        for (entscheidung, status) in [
            (
                AufbereitungEntscheidung::Abgelehnt,
                AenderungStatus::Abgelehnt,
            ),
            (AufbereitungEntscheidung::Offen, AenderungStatus::Offen),
        ] {
            let entschieden = entscheiden_regel_in(
                &speicher,
                &dokument(),
                &a.id,
                Some("ligaturen"),
                entscheidung,
                200,
            )?;
            assert!(entschieden.aenderungen.iter().all(|a| a.status == status));
        }
        let vorher = speicher.laden(&a.id).map_err(speicher_fehler)?;
        assert_eq!(
            entscheiden_regel_in_mit(
                &speicher,
                &dokument(),
                &a.id,
                (None, AufbereitungEntscheidung::Angenommen),
                300,
                |_| Err(SpeicherFehler::Schreiben),
            )
            .err()
            .as_deref(),
            Some("storage")
        );
        assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, vorher);
        Ok(())
    }

    #[test]
    fn jeder_befehl_repariert_mehrfach_aktive_fassungen() -> Result<(), String> {
        // Gegenprobe: Ohne Reparatur bleiben zwei aktive Dateien oder die Übernahme blockiert.
        for befehl in 0..8 {
            let (_temp, speicher) = umgebung()?;
            let mut alt = entwurf(&speicher, "ﬃ", 100)?;
            alt.status = Status::Aktiv;
            alt.geaendert = 200;
            speicher.speichern(&alt).map_err(speicher_fehler)?;
            let mut neu = entwurf(&speicher, "ﬃ", 150)?;
            neu.status = Status::Aktiv;
            neu.geaendert = 300;
            speicher.speichern(&neu).map_err(speicher_fehler)?;
            // Direktes Speichern simuliert den Abbruch zwischen den beiden Statusdateien.
            match befehl {
                0 => {
                    entwurf(&speicher, "ﬃ", 400)?;
                }
                1 => {
                    let a = Aufbereitung::neu(dokument(), "ﬃ", 400);
                    speicher
                        .speichern_mit_basis(&a, "ﬃ")
                        .map_err(speicher_fehler)?;
                    let _ = entscheiden_in(
                        &speicher,
                        &dokument(),
                        &a.id,
                        "unbekannt",
                        AufbereitungEntscheidung::Offen,
                        500,
                    );
                }
                2 => {
                    let a = Aufbereitung::neu(dokument(), "ﬃ", 400);
                    speicher
                        .speichern_mit_basis(&a, "ﬃ")
                        .map_err(speicher_fehler)?;
                    uebernehmen_in(&speicher, &dokument(), &a.id, "ﬃ", 500)?;
                }
                3 => {
                    assert_eq!(
                        rueckgaengig_in(&speicher, &dokument(), &alt.id, 400)
                            .err()
                            .as_deref(),
                        Some("wrong_status")
                    );
                }
                4 => {
                    liste_in(&speicher, &dokument())?;
                }
                5 => {
                    original_in(&speicher, &dokument())?;
                }
                6 => {
                    let _ = entscheiden_regel_in(
                        &speicher,
                        &dokument(),
                        &alt.id,
                        None,
                        AufbereitungEntscheidung::Offen,
                        500,
                    );
                }
                _ => {
                    utf16_bereiche_in(&speicher, &dokument(), &neu.id)?;
                }
            }
            // Die Speicherliste selbst repariert nichts und prüft somit den Befehlsaufruf.
            let liste = speicher.liste_lesbar().map_err(speicher_fehler)?;
            assert_eq!(
                liste.iter().filter(|a| a.status == Status::Aktiv).count(),
                1
            );
            assert_eq!(
                speicher.laden(&alt.id).map_err(speicher_fehler)?.status,
                Status::Abgeloest
            );
            if befehl != 2 {
                assert_eq!(
                    speicher.laden(&neu.id).map_err(speicher_fehler)?.status,
                    Status::Aktiv
                );
            }
        }
        Ok(())
    }

    #[test]
    fn reparatur_entscheidet_nach_zeit_erstellung_und_id() -> Result<(), String> {
        // Gegenprobe: Nur Erstellzeit oder Dateireihenfolge wählt jeweils einen anderen Sieger.
        for gleichstand in 0..3 {
            let (_temp, speicher) = umgebung()?;
            let mut a = entwurf(&speicher, "ab", 100)?;
            a.status = Status::Aktiv;
            a.geaendert = 500;
            speicher.speichern(&a).map_err(speicher_fehler)?;
            let mut b = Aufbereitung::neu(dokument(), "ab", 100);
            b.id = "ZZZZZZZZZZZZZZZZZZZZZZZZZZ".into();
            b.status = Status::Aktiv;
            b.geaendert = 500;
            if gleichstand == 0 {
                b.geaendert = 400;
            } else if gleichstand == 1 {
                b.erstellt = 99;
            }
            speicher
                .speichern_mit_basis(&b, "ab")
                .map_err(speicher_fehler)?;
            let erwartet = if gleichstand == 2 { &b.id } else { &a.id };
            let liste = liste_in(&speicher, &dokument())?;
            let aktiv: Vec<_> = liste.iter().filter(|a| a.status == Status::Aktiv).collect();
            assert_eq!(aktiv.len(), 1);
            assert_eq!(&aktiv[0].id, erwartet);
        }
        Ok(())
    }

    #[test]
    fn defekte_aktive_datei_blockiert_nicht_und_wird_nach_reparatur_abgeloest() -> Result<(), String>
    {
        // Gegenprobe: Nach Rückkehr der alten Datei bleiben sonst zwei aktive Fassungen.
        let (temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "ab", 100)?;
        uebernehmen_in(&speicher, &dokument(), &a.id, "ab", 200)?;
        let pfad = temp
            .path()
            .join("aufbereitung")
            .join(format!("{}.json", a.id));
        let bytes = std::fs::read(&pfad).map_err(|_| "read".to_string())?;
        std::fs::write(&pfad, b"GEHEIMTEXT{").map_err(|_| "write".to_string())?;
        let b = entwurf(&speicher, "ab", 300)?;
        assert_eq!(liste_in(&speicher, &dokument())?.len(), 1);
        assert_eq!(original_in(&speicher, &dokument())?.as_deref(), Some("ab"));
        uebernehmen_in(&speicher, &dokument(), &b.id, "ab", 400)?;
        std::fs::write(&pfad, bytes).map_err(|_| "write".to_string())?;
        original_in(&speicher, &dokument())?;
        let liste = speicher.liste_lesbar().map_err(speicher_fehler)?;
        assert_eq!(
            liste.iter().filter(|a| a.status == Status::Aktiv).count(),
            1
        );
        assert_eq!(
            speicher.laden(&a.id).map_err(speicher_fehler)?.status,
            Status::Abgeloest
        );
        assert_eq!(
            speicher.laden(&b.id).map_err(speicher_fehler)?.status,
            Status::Aktiv
        );
        rueckgaengig_in(&speicher, &dokument(), &b.id, 500)?;
        Ok(())
    }

    #[test]
    fn abbruch_nach_reaktivierung_liefert_text_und_warnung() -> Result<(), String> {
        // Gegenprobe: Früher kam trotz Reaktivierung nur storage und damit kein Text zurück.
        let (_temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "ﬃ", 100)?;
        annehmen(&speicher, &a, 150)?;
        uebernehmen_in(&speicher, &dokument(), &a.id, "ﬃ", 200)?;
        let b = entwurf(&speicher, "ffi", 300)?;
        uebernehmen_in(&speicher, &dokument(), &b.id, "ffi", 400)?;
        let zurueck = rueckgaengig_in_mit(&speicher, &dokument(), &b.id, 500, |_| {
            Err(SpeicherFehler::Schreiben)
        })?;
        assert_eq!(zurueck.text, "ffi");
        assert!(!zurueck.original);
        assert_eq!(zurueck.warnung.as_deref(), Some("storage_partial"));
        assert_eq!(
            speicher.laden(&a.id).map_err(speicher_fehler)?.status,
            Status::Aktiv
        );
        assert_eq!(
            speicher.laden(&b.id).map_err(speicher_fehler)?.status,
            Status::Aktiv
        );
        liste_in(&speicher, &dokument())?;
        assert_eq!(
            speicher.laden(&a.id).map_err(speicher_fehler)?.status,
            Status::Aktiv
        );
        assert_eq!(
            speicher.laden(&b.id).map_err(speicher_fehler)?.status,
            Status::Abgeloest
        );
        assert!(rueckgaengig_in(&speicher, &dokument(), &a.id, 600)?.original);
        Ok(())
    }

    #[test]
    fn abbruch_nach_aktivierung_liefert_text_und_warnung() -> Result<(), String> {
        // Gegenprobe: Ohne Prüfung der gespeicherten Aktivierung geht der Ergebnistext verloren.
        let (temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "ab", 100)?;
        uebernehmen_in(&speicher, &dokument(), &a.id, "ab", 200)?;
        let b = entwurf(&speicher, "ﬃ", 300)?;
        annehmen(&speicher, &b, 350)?;
        let ergebnis = uebernehmen_in_mit(&speicher, &dokument(), &b.id, "ﬃ", 400, |aktiv| {
            // Der Speicher-Hook schreibt nur die neue Aktivierung; das Ablösen scheitert.
            let pfad = temp
                .path()
                .join("aufbereitung")
                .join(format!("{}.json", b.id));
            let bytes = std::fs::read(&pfad).map_err(|_| SpeicherFehler::Lesen)?;
            let mut wert: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| SpeicherFehler::Lesen)?;
            wert["aufbereitung"] =
                serde_json::to_value(aktiv).map_err(|_| SpeicherFehler::Schreiben)?;
            let bytes = serde_json::to_vec(&wert).map_err(|_| SpeicherFehler::Schreiben)?;
            std::fs::write(pfad, bytes).map_err(|_| SpeicherFehler::Schreiben)?;
            Err(SpeicherFehler::Schreiben)
        })?;
        assert_eq!(ergebnis.text.as_deref(), Some("ffi"));
        assert_eq!(ergebnis.warnung.as_deref(), Some("storage_partial"));
        assert_eq!(
            speicher.laden(&a.id).map_err(speicher_fehler)?.status,
            Status::Aktiv
        );
        assert_eq!(
            speicher.laden(&b.id).map_err(speicher_fehler)?,
            ergebnis.aufbereitung
        );
        let liste = liste_in(&speicher, &dokument())?;
        let aktive: Vec<_> = liste.iter().filter(|a| a.status == Status::Aktiv).collect();
        assert_eq!(aktive.len(), 1);
        assert_eq!(aktive[0].id, b.id);
        assert_eq!(
            speicher.laden(&a.id).map_err(speicher_fehler)?.status,
            Status::Abgeloest
        );
        assert_eq!(
            rueckgaengig_in(&speicher, &dokument(), &b.id, 500)?.text,
            "ﬃ"
        );
        Ok(())
    }

    #[test]
    fn fehler_vor_aktivierung_liefert_keinen_text_und_aendert_nichts() -> Result<(), String> {
        // Gegenprobe: Eine pauschale Warnung würde auch ohne gespeicherte Aktivierung Text liefern.
        let (_temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "ab", 100)?;
        uebernehmen_in(&speicher, &dokument(), &a.id, "ab", 200)?;
        let vorher = speicher.laden(&a.id).map_err(speicher_fehler)?;
        let b = entwurf(&speicher, "ﬃ", 300)?;
        assert_eq!(
            uebernehmen_in_mit(&speicher, &dokument(), &b.id, "ﬃ", 400, |_| {
                Err(SpeicherFehler::Schreiben)
            })
            .err()
            .as_deref(),
            Some("storage")
        );
        assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, vorher);
        assert_eq!(speicher.laden(&b.id).map_err(speicher_fehler)?, b);
        // Ohne Vorgänger wurde vor dem letzten Schreiben noch nichts reaktiviert.
        assert_eq!(
            rueckgaengig_in_mit(&speicher, &dokument(), &a.id, 500, |_| {
                Err(SpeicherFehler::Schreiben)
            })
            .err()
            .as_deref(),
            Some("storage")
        );
        assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, vorher);
        Ok(())
    }

    #[test]
    fn lesebefehle_liefern_trotz_reparatur_schreibfehler() -> Result<(), String> {
        // Gegenprobe: Durchgereichte Reparaturfehler lassen beide Leseaufrufe scheitern.
        let (_temp, speicher) = umgebung()?;
        let mut a = entwurf(&speicher, "ab", 100)?;
        let mut b = entwurf(&speicher, "ab", 200)?;
        a.status = Status::Aktiv;
        a.geaendert = 300;
        b.status = Status::Aktiv;
        b.geaendert = 400;
        speicher.speichern(&a).map_err(speicher_fehler)?;
        speicher.speichern(&b).map_err(speicher_fehler)?;
        let mut versuche = 0;
        let liste = liste_in_mit(&speicher, &dokument(), |_| {
            versuche += 1;
            Err(SpeicherFehler::Schreiben)
        })?;
        assert_eq!(versuche, 1);
        assert_eq!(liste.len(), 2);
        let aktive: Vec<_> = liste.iter().filter(|a| a.status == Status::Aktiv).collect();
        assert_eq!(aktive.len(), 1);
        assert_eq!(aktive[0].id, b.id);
        assert_eq!(
            original_in_mit(&speicher, &dokument(), |_| {
                versuche += 1;
                Err(SpeicherFehler::Schreiben)
            })?
            .as_deref(),
            Some("ab")
        );
        assert_eq!(versuche, 2);
        // Die Hooks haben nichts geschrieben; beide Dateien bleiben erneut reparierbar.
        let gespeichert = speicher.liste_lesbar().map_err(speicher_fehler)?;
        assert_eq!(
            gespeichert
                .iter()
                .filter(|a| a.status == Status::Aktiv)
                .count(),
            2
        );
        liste_in(&speicher, &dokument())?;
        assert_eq!(
            speicher.laden(&a.id).map_err(speicher_fehler)?.status,
            Status::Abgeloest
        );
        Ok(())
    }

    #[test]
    fn sprachfassungen_haben_getrennte_originale_und_ketten() -> Result<(), String> {
        // Gegenprobe: Der alte Hash teilt Original und Kette; der alte Vergleich mischt Listen.
        let temp = tempfile::tempdir().map_err(|_| "tempdir".to_string())?;
        let mut de = dokument();
        de.art = DokumentArt::Buch;
        de.fassung_id = Some("de".into());
        let mut en = de.clone();
        en.fassung_id = Some("en".into());
        assert!(!gleiches_dokument(&de, &en));
        assert_ne!(dokument_hash(&de), dokument_hash(&en));
        let ohne = dokument();
        let alter_hash = format!("{:x}", Sha256::digest(ohne.id.as_bytes()));
        assert_eq!(dokument_hash(&ohne), alter_hash[..32]);
        let deutsch = AufbereitungsSpeicher::neu(temp.path().join(dokument_hash(&de)));
        let englisch = AufbereitungsSpeicher::neu(temp.path().join(dokument_hash(&en)));
        let a = entwurf_anlegen_in(&deutsch, de.clone(), "Deutsch", None, 100)?;
        let b = entwurf_anlegen_in(&englisch, en.clone(), "English", None, 100)?;
        uebernehmen_in(&deutsch, &de, &a.id, "Deutsch", 200)?;
        let b = uebernehmen_in(&englisch, &en, &b.id, "English", 300)?;
        assert_eq!(b.aufbereitung.vorgaenger_aktiv, None);
        assert_eq!(
            deutsch.laden(&a.id).map_err(speicher_fehler)?.status,
            Status::Aktiv
        );
        assert_eq!(original_in(&deutsch, &de)?.as_deref(), Some("Deutsch"));
        assert_eq!(original_in(&englisch, &en)?.as_deref(), Some("English"));
        assert!(liste_in(&deutsch, &en)?.is_empty());
        assert_eq!(
            rueckgaengig_in(&englisch, &en, &b.aufbereitung.id, 400)?.text,
            "English"
        );
        Ok(())
    }

    #[test]
    fn rueckgaengig_erhaelt_handedits_mit_und_ohne_vorgaenger() -> Result<(), String> {
        // Gegenprobe: Original oder Vorgängerergebnis verlieren jeweils den Handedit.
        let (_temp, speicher) = umgebung()?;
        entwurf(&speicher, "Original", 100)?;
        let erste = entwurf(&speicher, "Handedit eins", 200)?;
        uebernehmen_in(&speicher, &dokument(), &erste.id, "Handedit eins", 300)?;
        let zweite = entwurf(&speicher, "Handedit zwei", 400)?;
        uebernehmen_in(&speicher, &dokument(), &zweite.id, "Handedit zwei", 500)?;
        let zurueck = rueckgaengig_in(&speicher, &dokument(), &zweite.id, 600)?;
        assert_eq!(zurueck.text, "Handedit zwei");
        assert!(!zurueck.original);
        assert_eq!(
            speicher.laden(&zweite.id).map_err(speicher_fehler)?.status,
            Status::Abgeloest
        );
        assert_eq!(
            speicher.laden(&erste.id).map_err(speicher_fehler)?.status,
            Status::Aktiv
        );
        let zurueck = rueckgaengig_in(&speicher, &dokument(), &erste.id, 700)?;
        assert_eq!(zurueck.text, "Handedit eins");
        assert!(!zurueck.original);
        Ok(())
    }

    #[test]
    fn rueckgaengig_mit_defekten_aenderungen_liefert_invalid_changes() -> Result<(), String> {
        // Gegenprobe: Defekte Änderungen im aktiven Stand oder Vorgänger dürfen nicht verschwinden.
        let (temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "ﬃ", 100)?;
        uebernehmen_in(&speicher, &dokument(), &a.id, "ﬃ", 200)?;
        let b = entwurf(&speicher, "ﬃ", 300)?;
        uebernehmen_in(&speicher, &dokument(), &b.id, "ﬃ", 400)?;
        for id in [&a.id, &b.id] {
            let pfad = temp.path().join("aufbereitung").join(format!("{id}.json"));
            let bytes = std::fs::read(&pfad).map_err(|_| "read".to_string())?;
            let mut wert: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| "json".to_string())?;
            wert["aufbereitung"]["aenderungen"][0]["bereich"] = serde_json::json!([0, 99]);
            let kaputt = serde_json::to_vec(&wert).map_err(|_| "json".to_string())?;
            std::fs::write(&pfad, &kaputt).map_err(|_| "write".to_string())?;
            assert_eq!(
                rueckgaengig_in(&speicher, &dokument(), &b.id, 500)
                    .err()
                    .as_deref(),
                Some("invalid_changes")
            );
            let danach = std::fs::read(&pfad).map_err(|_| "read".to_string())?;
            assert_eq!(danach, kaputt);
            std::fs::write(&pfad, bytes).map_err(|_| "write".to_string())?;
        }
        Ok(())
    }

    #[test]
    fn uebernehmen_begrenzt_aktuellen_text_vor_jeder_mutation() -> Result<(), String> {
        // Gegenprobe: Ohne Grenze würde der Entwurf als veraltet gespeichert.
        let (_temp, speicher) = umgebung()?;
        let a = entwurf(&speicher, "kurz", 100)?;
        let riesig = "a".repeat(MAX_TEXT_BYTES + 1);
        assert_eq!(
            uebernehmen_in(&speicher, &dokument(), &a.id, &riesig, 200)
                .err()
                .as_deref(),
            Some("text_too_large")
        );
        assert_eq!(speicher.laden(&a.id).map_err(speicher_fehler)?, a);
        Ok(())
    }

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

    /// Gruppen einer Regel (seit lva#700 je Vorkommen benannt, z. B. `ligaturen#1`).
    fn gruppen_der_regel(a: &Aufbereitung, regel: &str) -> Vec<String> {
        let mut gruppen: Vec<String> = a
            .aenderungen
            .iter()
            .filter(|c| matches!(&c.art, crate::aufbereitung::AenderungArt::Regel(r) if r == regel))
            .filter_map(|c| c.gruppe.clone())
            .collect();
        gruppen.sort();
        gruppen.dedup();
        gruppen
    }

    fn regel_entscheiden(
        speicher: &AufbereitungsSpeicher,
        a: &Aufbereitung,
        regel: &str,
        entscheidung: AufbereitungEntscheidung,
        jetzt: u64,
    ) -> Result<Aufbereitung, String> {
        let mut aktuell = a.clone();
        for gruppe in gruppen_der_regel(a, regel) {
            aktuell = entscheiden_in(
                speicher,
                &dokument(),
                &a.id,
                &gruppe,
                entscheidung.clone(),
                jetzt,
            )?;
        }
        Ok(aktuell)
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
            let a = regel_entscheiden(&speicher, &a, "ligaturen", entscheidung, 200)?;
            let regel = |c: &&crate::aufbereitung::Aenderung| matches!(&c.art, crate::aufbereitung::AenderungArt::Regel(r) if r == "ligaturen");
            let gruppe: Vec<_> = a.aenderungen.iter().filter(regel).collect();
            assert_eq!(gruppe.len(), 2);
            assert!(gruppe.iter().all(|a| a.status == status));
            assert!(a
                .aenderungen
                .iter()
                .filter(|c| !regel(c))
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
        let a = regel_entscheiden(
            &speicher,
            &a,
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
            Status::Abgeloest
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
            .all(|a| a.status == Status::Abgeloest));
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
                    entscheiden_regel_in(
                        &speicher,
                        &dokument(),
                        &a.id,
                        None,
                        AufbereitungEntscheidung::Angenommen,
                        200,
                    )
                    .err()
                    .as_deref(),
                    Some("wrong_status")
                );
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
            entscheiden_regel_in(
                &speicher,
                &d,
                "id",
                None,
                AufbereitungEntscheidung::Offen,
                100,
            )
            .err(),
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
