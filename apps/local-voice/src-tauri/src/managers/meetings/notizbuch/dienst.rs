//! Dienst „Notizbuch-Seiten einer Besprechung zuordnen“ (Spec §5.2, lva#396 N4a Schritt 2).
//!
//! Liest die Besprechung aus dem Store, holt Kandidaten **nur** aus den für den Assistenten
//! freigegebenen Abschnitten (dieselbe Regel wie die Notizbuch-Werkzeuge: Bereich „Notizbuch“
//! aus → nichts; Spec §3.2 M1), bewertet sie mit [`zuordnung::entscheide`] und speichert das
//! Ergebnis in `meeting_note_links`.
//!
//! Automatik erst nach dem Eval-Tor (§5.6): [`TOR_BESTANDEN`] ist `false`, also entstehen nur
//! Vorschläge. Ergebnis und Log nennen nur sichtbare Seiten; verborgene Abschnitte verändern
//! weder Zahl noch Rückgabe (M1). Log nur mit Zählern, nie mit Inhalten.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use wai_notizbuch_zugang::{Freigabe, Zugang};

use super::links::{AutoLink, LinkArt};
use super::zeit::Zeitachse;
use super::zuordnung::{self, Abschnitt, Besprechung, Einstufung, Grund, Kandidat};
use crate::managers::meetings::store::MeetingStore;
use crate::settings::AppSettings;

/// Eval-Tor für die automatische Zuordnung (§5.2 M5). Bis die gemessene Präzision auf dem
/// Prüfsatz ≥ 0,95 liegt, gibt es nur Vorschläge.
pub const TOR_BESTANDEN: bool = false;

/// Höchstzahl Seiten je Zeitraumabfrage des Zugangs.
const MAX_KANDIDATEN: usize = 200;
/// Obergrenze für „seit Fensterbeginn geändert“ (offenes Ende).
const OFFENES_ENDE: &str = "9999-12-31T23:59:59Z";

/// Ablage des Notizbuchs (`…/de.wolffappliedai.notizbuch/notizbuch.db`), beim App-Start gesetzt.
/// Ohne Eintrag (Tests, portable Fassung) gibt es keine Notizbuch-Quelle.
static ABLAGE: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Merkt sich den Pfad der Notizbuch-Ablage (einmal beim App-Start).
pub fn ablage_merken(pfad: Option<PathBuf>) {
    let _ = ABLAGE.set(pfad);
}

/// Zugang und Freigabe aus den Einstellungen; `None`, wenn der Bereich „Notizbuch“ aus ist, keine
/// Abschnitte freigegeben sind oder kein Notizbuch lesbar ist.
pub fn sicht_aus_einstellungen(settings: &AppSettings) -> Option<(Zugang, Freigabe)> {
    use crate::agent_bridge::voice::areas::{Area, Areas};
    let areas = Areas::from_settings(
        &settings.voice_call_knowledge,
        settings.voice_call_read_transcripts,
    );
    if !areas.has(Area::Notebook) {
        return None;
    }
    let freigabe = crate::agent_bridge::voice::knowledge::notizbuch::freigabe_aus_einstellung(
        &settings.voice_call_notebook_sections,
    );
    if freigabe.ist_leer() {
        return None;
    }
    let pfad = ABLAGE.get()?.as_ref()?;
    Zugang::oeffnen(pfad).ok().map(|z| (z, freigabe))
}

/// Eine sichtbare Zuordnung dieses Laufs (für Aufrufer und Tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zuordnung {
    pub seite_id: String,
    pub art: LinkArt,
    pub punkte: u32,
}

/// Die Besprechung als Eingabe der Bewertung (besitzende Daten).
struct BesprechungsDaten {
    id: String,
    titel: String,
    started_at_s: i64,
    duration_ms: i64,
    projekt: Option<String>,
    teilnehmende: Vec<String>,
    kalendertitel: Option<String>,
    abschnitte: Vec<Abschnitt>,
}

impl BesprechungsDaten {
    fn laden(store: &MeetingStore, meeting_id: &str) -> Result<Option<Self>> {
        let Some(m) = store.get_meeting(meeting_id)? else {
            return Err(anyhow!("meeting_not_found"));
        };
        // Ohne Beginn keine Zeitachse, also keine Kandidaten.
        let Some(started_at_s) = m.started_at else {
            return Ok(None);
        };
        let duration_ms = m.duration_ms.map(|d| d as i64).unwrap_or_else(|| {
            m.ended_at
                .map(|e| e.saturating_sub(started_at_s).max(0).saturating_mul(1000))
                .unwrap_or(0)
        });
        let abschnitte = store
            .get_segments(meeting_id)?
            .into_iter()
            .map(|s| Abschnitt {
                start_ms: s.start_ms as i64,
                ende_ms: s.end_ms as i64,
                text: s.text,
            })
            .collect();
        let teilnehmende = store
            .meeting_participant_rows(meeting_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(name, _, _)| name)
            .collect();
        let kalendertitel = store
            .meeting_calendar_link(meeting_id)
            .ok()
            .flatten()
            .map(|l| l.event_title);
        let projekt = store
            .projekte_der_besprechung(meeting_id)
            .ok()
            .and_then(|p| p.into_iter().next());
        Ok(Some(Self {
            id: m.id,
            titel: m.title,
            started_at_s,
            duration_ms,
            projekt,
            teilnehmende,
            kalendertitel,
            abschnitte,
        }))
    }

    fn als_besprechung(&self) -> Besprechung<'_> {
        Besprechung {
            id: &self.id,
            titel: &self.titel,
            started_at_s: self.started_at_s,
            duration_ms: self.duration_ms,
            projekt: self.projekt.as_deref(),
            teilnehmende: &self.teilnehmende,
            kalendertitel: self.kalendertitel.as_deref(),
            abschnitte: &self.abschnitte,
        }
    }
}

fn iso(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

/// Seiten, die im Fenster angelegt oder seit Fensterbeginn geändert wurden – nur sichtbare.
/// Eine Seite mit einer Blockzeit im Fenster wurde frühestens zu dieser Zeit geändert; daher
/// genügt „geändert seit Fensterbeginn“ als Obermenge.
fn kandidaten_ids(zugang: &Zugang, freigabe: &Freigabe, achse: &Zeitachse) -> Vec<String> {
    let (von, bis) = achse.fenster();
    let (Some(von), Some(bis)) = (iso(von), iso(bis)) else {
        return Vec::new();
    };
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for ende in [bis.as_str(), OFFENES_ENDE] {
        if let Ok(koepfe) = zugang.zeitraum(&von, ende, MAX_KANDIDATEN, freigabe) {
            ids.extend(koepfe.into_iter().map(|k| k.id));
        }
    }
    ids.into_iter().collect()
}

/// Zuordnungsgründe als JSON (für die Anzeige „Zeit, gemeinsame Begriffe, Link“).
pub fn gruende_json(gruende: &[Grund]) -> String {
    let v: Vec<Value> = gruende
        .iter()
        .map(|g| match g {
            Grund::Link => json!({"art": "link", "punkte": g.punkte()}),
            Grund::Projekt => json!({"art": "projekt", "punkte": g.punkte()}),
            Grund::ZeitAnteil {
                im_fenster,
                gesamt,
                punkte,
            } => {
                json!({"art": "zeit", "im_fenster": im_fenster, "gesamt": gesamt, "punkte": punkte})
            }
            Grund::Begriffe { begriffe, punkte } => {
                json!({"art": "begriffe", "begriffe": begriffe, "punkte": punkte})
            }
            Grund::TitelDatum => json!({"art": "titel_datum", "punkte": g.punkte()}),
            Grund::TitelTeilnehmende => {
                json!({"art": "titel_teilnehmende", "punkte": g.punkte()})
            }
            Grund::TitelKalender => json!({"art": "titel_kalender", "punkte": g.punkte()}),
        })
        .collect();
    Value::Array(v).to_string()
}

/// Kern des Dienstes mit gegebener Sicht (testbar ohne App). `sicht = None` heißt: nichts
/// freigegeben → keine Kandidaten; vorhandene automatische Zeilen werden entfernt, Zeilen des
/// Nutzers bleiben.
pub fn zuordnen_mit(
    store: &MeetingStore,
    meeting_id: &str,
    sicht: Option<(&Zugang, &Freigabe)>,
) -> Result<Vec<Zuordnung>> {
    let daten = BesprechungsDaten::laden(store, meeting_id)?;
    let vorhanden = store.note_links(meeting_id)?;
    // Nutzerentscheidungen: gelöst wird nie, bestätigt nicht erneut bewertet.
    let geloest: HashSet<String> = vorhanden
        .iter()
        .filter(|l| l.art == LinkArt::Geloest)
        .map(|l| l.seite_id.clone())
        .collect();
    let bestaetigt: HashSet<&str> = vorhanden
        .iter()
        .filter(|l| l.art == LinkArt::Bestaetigt)
        .map(|l| l.seite_id.as_str())
        .collect();

    let (Some(daten), Some((zugang, freigabe))) = (daten, sicht) else {
        store.note_links_automatisch_ersetzen(meeting_id, "", &[])?;
        return Ok(Vec::new());
    };
    if freigabe.ist_leer() {
        store.note_links_automatisch_ersetzen(meeting_id, "", &[])?;
        return Ok(Vec::new());
    }
    let besp = daten.als_besprechung();
    let achse = besp.zeitachse();
    let stand = zugang.stand(freigabe).unwrap_or_default();

    let mut neu: Vec<AutoLink> = Vec::new();
    for id in kandidaten_ids(zugang, freigabe, &achse) {
        if bestaetigt.contains(id.as_str()) || geloest.contains(&id) {
            continue;
        }
        // Erneut über die Freigabe lesen: was nicht sichtbar ist, kommt nicht vor.
        let Ok(Some(seite)) = zugang.seite(&id, freigabe) else {
            continue;
        };
        let k = Kandidat {
            seite: &seite,
            projekt: None,
            herkunft_local_voice: false,
        };
        let e = zuordnung::entscheide(&besp, &k, &geloest, TOR_BESTANDEN);
        let art = match e.einstufung {
            Einstufung::Automatisch => LinkArt::Auto,
            Einstufung::Vorschlag => LinkArt::Vorschlag,
            Einstufung::Nichts => continue,
        };
        neu.push(AutoLink {
            seite_id: id,
            art,
            punkte: e.bewertung.punkte,
            gruende_json: gruende_json(&e.bewertung.gruende),
        });
    }
    store.note_links_automatisch_ersetzen(meeting_id, &stand, &neu)?;
    log::info!(
        "Notizbuch-Zuordnung: {} Vorschläge, {} automatisch",
        neu.iter().filter(|l| l.art == LinkArt::Vorschlag).count(),
        neu.iter().filter(|l| l.art == LinkArt::Auto).count()
    );
    Ok(neu
        .into_iter()
        .map(|l| Zuordnung {
            seite_id: l.seite_id,
            art: l.art,
            punkte: l.punkte,
        })
        .collect())
}

/// Dienst für die App: Sicht aus den Einstellungen (Bereich „Notizbuch“, Abschnittsfreigabe).
pub fn zuordnen(
    store: &MeetingStore,
    settings: &AppSettings,
    meeting_id: &str,
) -> Result<Vec<Zuordnung>> {
    let sicht = sicht_aus_einstellungen(settings);
    zuordnen_mit(store, meeting_id, sicht.as_ref().map(|(z, f)| (z, f)))
}
