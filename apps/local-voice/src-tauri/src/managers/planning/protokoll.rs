//! Protokoll und Entwurf an die Teilnehmenden (Besprechungsplanung P5, lva#736, Konzept 3.6).
//!
//! Das Modul baut nichts neben dem Bestand auf, sondern haengt sich an die vorhandenen Wege:
//!
//! - **Vorlage**: aus der Agenda entsteht eine Vorlage des Vorlagensystems der KI-Notizen
//!   (`meeting_templates`, [`vorlage_anlegen`]) und wird der Besprechung zugeordnet. Das Protokoll
//!   selbst erzeugt weiter `meetings::minutes` (mit dieser Vorlage als Nutzerwahl); dort kommt der
//!   Kopf aus [`kopf_fuer_besprechung`] hinzu und das Einwilligungs-Tor gilt davor und danach.
//!   Besprechungen ohne Planung sind unveraendert.
//! - **Ziel-Abgleich**: "Ziel erreicht" ja/teilweise/nein setzt ein Mensch ([`erreicht_setzen`]);
//!   das Feld steht in `meeting_plan_protocols` und im Kopf des Protokolls. Ein Modell setzt es nie.
//! - **Freigabe**: der Mensch gibt das Protokoll frei ([`freigeben`]): Dokument und Pruefsumme des
//!   Textes werden festgehalten, die Planung wird `protokolliert`. Wird der Text danach geaendert
//!   oder neu erzeugt, gilt er nicht mehr als freigegeben.
//! - **Entwurf**: aus dem freigegebenen Text, ohne Transkript und ohne Modell ([`entwurf`]); die
//!   Ausgabe (Kopieren, `mailto:`, `.eml`) ist der vorhandene Follow-up-Weg. Versendet wird nie
//!   automatisch.
//!
//! Datenschutz: weder Protokoll- noch Transkripttext gehen ins Log.

use besprechungsplanung::model::{Plan, PlanStatus};
use besprechungsplanung::protokoll::{self as pk, Erreicht};
use besprechungsplanung::store as plan_store;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{
    bezug_der_besprechung, bindungen, in_tx, plan_holen, sql, status_wechseln, verbindung, Res,
};
use crate::managers::calendar::model::CalEvent;
use crate::managers::meetings::export::markdown_to_text;
use crate::managers::meetings::mail::{self, MailDraft};
use crate::managers::meetings::minutes::DOC_KIND;
use crate::managers::meetings::notes::model::{SectionKind, TemplateSection, TemplateSpec};
use crate::managers::meetings::notes::templates::SPEC_VERSION;
use crate::managers::meetings::store::{MeetingDocument, MeetingStore};

/// Migration (Index 24, `meetings::store::P5_INDEX`): Stand des Protokolls je Planung. Nur
/// CREATE ... IF NOT EXISTS.
/// Die Planung wird endgueltig geloescht -> die Zeile geht mit (Fremdschluessel).
pub(crate) const PROTOKOLL_MIGRATION: &str = "CREATE TABLE IF NOT EXISTS meeting_plan_protocols (
        plan_id TEXT PRIMARY KEY REFERENCES meeting_plans(id) ON DELETE CASCADE,
        template_id TEXT,
        goal_reached TEXT,
        goal_reached_at INTEGER,
        approved_document_id TEXT,
        approved_hash TEXT,
        approved_at INTEGER,
        updated_at INTEGER NOT NULL
    );";

/// Der gespeicherte Stand (eine Zeile je Planung).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Zeile {
    template_id: Option<String>,
    erreicht: Option<Erreicht>,
    erreicht_am: Option<i64>,
    freigabe_dokument: Option<String>,
    freigabe_hash: Option<String>,
    freigabe_am: Option<i64>,
}

fn zeile_lesen(conn: &Connection, plan_id: &str) -> Res<Zeile> {
    let roh = conn
        .query_row(
            "SELECT template_id, goal_reached, goal_reached_at, approved_document_id,
                    approved_hash, approved_at
             FROM meeting_plan_protocols WHERE plan_id = ?1",
            [plan_id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .optional()
        .map_err(sql)?;
    let Some((template_id, erreicht, erreicht_am, dok, hash, am)) = roh else {
        return Ok(Zeile::default());
    };
    // Ein unbekannter Wert wird gemeldet, nie still durch "nicht bewertet" ersetzt.
    let erreicht = match erreicht {
        None => None,
        Some(s) => Some(Erreicht::parse(&s).ok_or_else(|| "daten_beschaedigt".to_string())?),
    };
    Ok(Zeile {
        template_id,
        erreicht,
        erreicht_am,
        freigabe_dokument: dok,
        freigabe_hash: hash,
        freigabe_am: am,
    })
}

/// Legt die Zeile an, falls sie fehlt (Planung muss vorhanden sein, sonst Fremdschluessel-Fehler).
fn zeile_sichern(conn: &Connection, plan_id: &str, jetzt: i64) -> Res<()> {
    conn.execute(
        "INSERT OR IGNORE INTO meeting_plan_protocols (plan_id, updated_at) VALUES (?1, ?2)",
        params![plan_id, jetzt],
    )
    .map_err(sql)?;
    Ok(())
}

/// Die Besprechung, zu der die Planung gehoert: ihr ausdruecklicher Eintrag (Notizen), sonst die
/// juengste Besprechung, an die sie das Einwilligungs-Tor gebunden hat (Start mit Transkription),
/// sonst die Besprechung, die mit ihrem Termin verknuepft ist. Nur lebende Besprechungen.
pub(crate) fn besprechung_der_planung(
    conn: &Connection,
    store: &MeetingStore,
    plan: &Plan,
) -> Res<Option<String>> {
    let lebt = |id: &str| -> Res<bool> {
        Ok(store
            .get_meeting(id)
            .map_err(|e| format!("sql: {e}"))?
            .is_some())
    };
    if let Some(id) = plan.meeting_id.as_deref() {
        if lebt(id)? {
            return Ok(Some(id.to_string()));
        }
    }
    let mut st = conn
        .prepare(
            "SELECT meeting_id FROM meeting_plan_bindings WHERE plan_id = ?1
             ORDER BY created_at DESC, meeting_id",
        )
        .map_err(sql)?;
    let ids = st
        .query_map([&plan.id], |r| r.get::<_, String>(0))
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    for id in ids {
        if lebt(&id)? {
            return Ok(Some(id));
        }
    }
    let Some(link) = plan_store::link_laden(conn, &plan.id)? else {
        return Ok(None);
    };
    let Some(uid) = link.remote_id.as_deref().or(link.ical_uid.as_deref()) else {
        return Ok(None);
    };
    let mut st = conn
        .prepare(
            "SELECT meeting_id FROM meeting_calendar_links
             WHERE uid = ?1 AND (?2 IS NULL OR event_start = ?2)
             ORDER BY created_at DESC, meeting_id",
        )
        .map_err(sql)?;
    let ids = st
        .query_map(params![uid, link.starts_at], |r| r.get::<_, String>(0))
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    for id in ids {
        if lebt(&id)? {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Das juengste Protokoll (Markdown) der Besprechung.
fn juengstes_protokoll(store: &MeetingStore, meeting_id: &str) -> Res<Option<MeetingDocument>> {
    Ok(store
        .get_documents(meeting_id)
        .map_err(|e| format!("sql: {e}"))?
        .into_iter()
        .filter(|d| d.kind == DOC_KIND)
        .max_by_key(|d| d.version))
}

/// Stand des Protokolls einer Planung, wie die Oberflaeche ihn braucht.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProtokollStand {
    pub plan_id: String,
    /// Die Besprechung der Planung; `None`, solange es keine gibt.
    pub meeting_id: Option<String>,
    /// Die Vorlage aus der Agenda, falls schon angelegt.
    pub vorlage_id: Option<String>,
    /// Das juengste Protokoll der Besprechung.
    pub dokument_id: Option<String>,
    pub dokument_version: Option<u32>,
    /// "Ziel erreicht", von Hand gesetzt.
    pub erreicht: Option<Erreicht>,
    /// Das Protokoll traegt keine Zeile "Ziel erreicht" (vor P5 erzeugt oder von Hand entfernt).
    pub kopf_fehlt: bool,
    /// Das juengste Protokoll ist freigegeben und seither unveraendert.
    pub freigegeben: bool,
    pub freigegeben_am: Option<i64>,
    /// Es war freigegeben, der Text wurde seither geaendert oder neu erzeugt: erneut freigeben.
    pub geaendert_seit_freigabe: bool,
}

fn stand_aus(
    store: &MeetingStore,
    conn: &Connection,
    plan: &Plan,
) -> Res<(ProtokollStand, Option<MeetingDocument>)> {
    let zeile = zeile_lesen(conn, &plan.id)?;
    let meeting_id = besprechung_der_planung(conn, store, plan)?;
    let dokument = match meeting_id.as_deref() {
        Some(id) => juengstes_protokoll(store, id)?,
        None => None,
    };
    let freigegeben = match (&dokument, &zeile.freigabe_dokument, &zeile.freigabe_hash) {
        (Some(d), Some(id), Some(hash)) => d.id == *id && pk::text_hash(&d.body) == *hash,
        _ => false,
    };
    let stand = ProtokollStand {
        plan_id: plan.id.clone(),
        meeting_id,
        vorlage_id: zeile.template_id.clone(),
        dokument_id: dokument.as_ref().map(|d| d.id.clone()),
        dokument_version: dokument.as_ref().map(|d| d.version),
        erreicht: zeile.erreicht,
        kopf_fehlt: dokument
            .as_ref()
            .is_some_and(|d| pk::erreicht_lesen(&d.body).is_none()),
        freigegeben,
        freigegeben_am: freigegeben.then_some(zeile.freigabe_am).flatten(),
        geaendert_seit_freigabe: zeile.freigabe_hash.is_some() && !freigegeben,
    };
    Ok((stand, dokument))
}

pub fn stand(store: &MeetingStore, plan_id: &str) -> Res<ProtokollStand> {
    let conn = verbindung(store)?;
    let plan = plan_holen(&conn, plan_id)?;
    Ok(stand_aus(store, &conn, &plan)?.0)
}

// ---------------------------------------------------------------------------
// Vorlage aus der Agenda
// ---------------------------------------------------------------------------

/// Ergebnis von [`vorlage_anlegen`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct VorlageErgebnis {
    pub template_id: String,
    pub meeting_id: String,
    pub abschnitte: u32,
}

fn spec_aus(v: &pk::Vorlage) -> TemplateSpec {
    TemplateSpec {
        version: SPEC_VERSION,
        context: v.kontext.clone(),
        sections: v
            .abschnitte
            .iter()
            .map(|a| TemplateSection {
                id: a.id.clone(),
                title: a.titel.clone(),
                instruction: a.anweisung.clone(),
                kind: if a.aufgaben {
                    SectionKind::Tasks
                } else {
                    SectionKind::Text
                },
            })
            .collect(),
    }
}

/// Legt die Protokollvorlage aus der Agenda an (oder bringt die schon angelegte auf den Stand der
/// Planung) und ordnet sie der Besprechung zu. Das Protokoll erzeugt danach der vorhandene Weg
/// (`meetings_generate_minutes` mit dieser Vorlage). Fehler: `plan_nicht_gefunden`,
/// `besprechung_fehlt` (noch keine Besprechung zur Planung), `template_invalid:...`.
pub fn vorlage_anlegen(store: &MeetingStore, plan_id: &str, jetzt: i64) -> Res<VorlageErgebnis> {
    let conn = verbindung(store)?;
    let plan = plan_holen(&conn, plan_id)?;
    let meeting_id = besprechung_der_planung(&conn, store, &plan)?
        .ok_or_else(|| "besprechung_fehlt".to_string())?;
    let vorlage = pk::vorlage(&plan);
    let spec = spec_aus(&vorlage);
    let alt = zeile_lesen(&conn, plan_id)?
        .template_id
        .filter(|id| matches!(store.get_template_info(id), Ok(Some(_))));
    let info = store
        .save_template(alt.as_deref(), &vorlage.titel, &spec)
        .map_err(|e| e.to_string())?;
    in_tx(store, |tx| {
        zeile_sichern(tx, plan_id, jetzt)?;
        tx.execute(
            "UPDATE meeting_plan_protocols SET template_id = ?2, updated_at = ?3 WHERE plan_id = ?1",
            params![plan_id, info.id, jetzt],
        )
        .map_err(sql)?;
        Ok(())
    })?;
    store
        .set_meeting_template(&meeting_id, Some(&info.id))
        .map_err(|e| e.to_string())?;
    Ok(VorlageErgebnis {
        template_id: info.id,
        meeting_id,
        abschnitte: vorlage.abschnitte.len() as u32,
    })
}

/// Der Kopf fuer das Protokoll einer Besprechung mit Planung (Markdown, siehe
/// [`pk::kopf_markdown`]); `None` fuer eine Besprechung ohne Planung. Gehoeren mehrere Planungen zur
/// Besprechung, gilt die, deren Agenda-Vorlage das Protokoll nutzt (`template_id`), sonst die erste.
/// Nur lesend.
pub(crate) fn kopf_fuer_besprechung(
    store: &MeetingStore,
    meeting_id: &str,
    template_id: Option<&str>,
) -> Res<Option<String>> {
    let conn = verbindung(store)?;
    let mut ids: Vec<String> = bezug_der_besprechung(store, meeting_id)?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    ids.sort();
    ids.dedup();
    let mut gewaehlt: Option<(Plan, Zeile)> = None;
    for id in ids {
        let Some(plan) = plan_store::plan_laden(&conn, &id)? else {
            continue;
        };
        let zeile = zeile_lesen(&conn, &id)?;
        let passt = template_id.is_some() && zeile.template_id.as_deref() == template_id;
        if gewaehlt.is_none() || passt {
            gewaehlt = Some((plan, zeile));
        }
        if passt {
            break;
        }
    }
    Ok(gewaehlt.map(|(plan, zeile)| pk::kopf_markdown(&plan, zeile.erreicht)))
}

// ---------------------------------------------------------------------------
// Ziel-Abgleich und Freigabe
// ---------------------------------------------------------------------------

/// Setzt "Ziel erreicht" (von Hand) und schreibt die Zeile ins juengste Protokoll nach, falls es
/// sie traegt. `None` nimmt die Bewertung zurueck. Eine freigegebene Fassung gilt danach nicht mehr
/// als freigegeben (der Text hat sich geaendert). Fehler: `plan_nicht_gefunden`,
/// `protokoll_nicht_aktualisiert` (das Feld ist gespeichert, der Text liess sich nicht anpassen).
pub fn erreicht_setzen(
    store: &MeetingStore,
    plan_id: &str,
    erreicht: Option<Erreicht>,
    jetzt: i64,
) -> Res<ProtokollStand> {
    in_tx(store, |tx| {
        plan_holen(tx, plan_id)?;
        zeile_sichern(tx, plan_id, jetzt)?;
        tx.execute(
            "UPDATE meeting_plan_protocols
             SET goal_reached = ?2, goal_reached_at = ?3, updated_at = ?4 WHERE plan_id = ?1",
            params![
                plan_id,
                erreicht.map(|e| e.as_str()),
                erreicht.map(|_| jetzt),
                jetzt
            ],
        )
        .map_err(sql)?;
        Ok(())
    })?;
    // Optimistische Sperre des Dokuments: bei einer gleichzeitigen Aenderung einmal neu lesen.
    for versuch in 0..2 {
        let conn = verbindung(store)?;
        let plan = plan_holen(&conn, plan_id)?;
        let (_, dokument) = stand_aus(store, &conn, &plan)?;
        let Some(dokument) = dokument else { break };
        let Some(neu) = pk::erreicht_ersetzen(&dokument.body, erreicht) else {
            break;
        };
        if neu == dokument.body {
            break;
        }
        match store.update_markdown_document(&dokument.id, &neu, dokument.updated_at) {
            Ok(_) => break,
            Err(e) if versuch == 0 && e.to_string().contains("stale_document") => continue,
            Err(e) => {
                log::warn!("planning: Protokollkopf nicht aktualisiert ({e})");
                return Err("protokoll_nicht_aktualisiert".to_string());
            }
        }
    }
    stand(store, plan_id)
}

/// Gibt das juengste Protokoll frei: Dokument und Pruefsumme des Textes werden festgehalten, die
/// Planung wird `protokolliert`. Fehler: `plan_nicht_gefunden`, `besprechung_fehlt`,
/// `protokoll_fehlt` (noch nichts erzeugt).
pub fn freigeben(store: &MeetingStore, plan_id: &str, jetzt: i64) -> Res<ProtokollStand> {
    let conn = verbindung(store)?;
    let plan = plan_holen(&conn, plan_id)?;
    let (stand, dokument) = stand_aus(store, &conn, &plan)?;
    if stand.meeting_id.is_none() {
        return Err("besprechung_fehlt".to_string());
    }
    let dokument = dokument.ok_or_else(|| "protokoll_fehlt".to_string())?;
    drop(conn);
    in_tx(store, |tx| {
        let aktuell = plan_holen(tx, plan_id)?;
        zeile_sichern(tx, plan_id, jetzt)?;
        tx.execute(
            "UPDATE meeting_plan_protocols
             SET approved_document_id = ?2, approved_hash = ?3, approved_at = ?4, updated_at = ?4
             WHERE plan_id = ?1",
            params![plan_id, dokument.id, pk::text_hash(&dokument.body), jetzt],
        )
        .map_err(sql)?;
        match aktuell.status {
            PlanStatus::Protokolliert | PlanStatus::Versendet => Ok(()),
            PlanStatus::Durchgefuehrt => {
                plan_store::plan_status_setzen(tx, plan_id, PlanStatus::Protokolliert, jetzt)
            }
            s => status_wechseln(tx, plan_id, s, PlanStatus::Protokolliert, jetzt),
        }
    })?;
    self::stand(store, plan_id)
}

// ---------------------------------------------------------------------------
// Entwurf an die Teilnehmenden
// ---------------------------------------------------------------------------

/// Der Entwurf samt Hinweisen fuer die Oberflaeche.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProtokollEntwurf {
    pub entwurf: MailDraft,
    /// Teilnehmende ohne brauchbare E-Mail-Adresse (stehen nicht im An-Feld).
    pub ohne_adresse: u32,
    /// Der Termin war im Kalender-Cache; sonst stammen die Adressen aus dem Stand beim Start.
    pub termin_gefunden: bool,
}

/// Der Termin der Planung im Kalender-Cache (UID und Instanz der Verknuepfung, bevorzugt aus
/// derselben Quelle).
fn termin_der_planung(
    conn: &Connection,
    store: &MeetingStore,
    plan_id: &str,
) -> Res<Option<CalEvent>> {
    let Some(link) = plan_store::link_laden(conn, plan_id)? else {
        return Ok(None);
    };
    let Some(uid) = link.remote_id.as_deref().or(link.ical_uid.as_deref()) else {
        return Ok(None);
    };
    let mut st = conn
        .prepare("SELECT key, source_id, instance_start FROM calendar_events WHERE uid = ?1 ORDER BY key")
        .map_err(sql)?;
    let zeilen = st
        .query_map([uid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    let passend: Vec<_> = zeilen.iter().filter(|z| z.2 == link.instanz).collect();
    let gewaehlt = passend
        .iter()
        .find(|z| Some(z.1.as_str()) == link.source_id.as_deref())
        .or_else(|| passend.first());
    match gewaehlt {
        Some(z) => store.calendar_event(&z.0).map_err(|e| format!("sql: {e}")),
        None => Ok(None),
    }
}

/// Baut den Entwurf an die Teilnehmenden aus dem FREIGEGEBENEN Protokoll. Der Text kommt allein aus
/// dem Dokument (ohne den Abschnitt der Redeanteile), nie aus Transkript oder Notizen; die
/// Empfaenger aus den Teilnehmenden des verknuepften Termins (ohne die eigenen Adressen `eigene`);
/// Betreff `Protokoll: <Titel> (<Datum>)`. Fehler: `plan_nicht_gefunden`, `besprechung_fehlt`,
/// `protokoll_nicht_freigegeben`.
pub fn entwurf(store: &MeetingStore, plan_id: &str, eigene: &[String]) -> Res<ProtokollEntwurf> {
    let conn = verbindung(store)?;
    let plan = plan_holen(&conn, plan_id)?;
    let (stand, dokument) = stand_aus(store, &conn, &plan)?;
    let meeting_id = stand
        .meeting_id
        .clone()
        .ok_or_else(|| "besprechung_fehlt".to_string())?;
    let dokument = match dokument {
        Some(d) if stand.freigegeben => d,
        _ => return Err("protokoll_nicht_freigegeben".to_string()),
    };
    let meeting = store
        .get_meeting(&meeting_id)
        .map_err(|e| format!("sql: {e}"))?
        .ok_or_else(|| "besprechung_fehlt".to_string())?;
    let zeitpunkt = meeting.started_at.unwrap_or(meeting.created_at);
    let datum = chrono::DateTime::from_timestamp(zeitpunkt, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default();

    let termin = termin_der_planung(&conn, store, plan_id)?;
    let angaben: Vec<Option<String>> = match &termin {
        Some(t) => t
            .attendees
            .iter()
            .filter(|a| !a.is_self)
            .map(|a| a.email.clone())
            .collect(),
        None => bindungen(&conn, &meeting_id)?
            .into_iter()
            .filter(|(id, _)| *id == plan_id)
            .flat_map(|(_, anwesende)| anwesende)
            .map(Some)
            .collect(),
    };
    let referenzen: Vec<Option<&str>> = angaben.iter().map(|a| a.as_deref()).collect();
    let empfaenger = pk::empfaenger(&referenzen, eigene);

    let markdown = mail::strip_citation_marks(&pk::ohne_redeanteile(&dokument.body));
    let text = markdown_to_text(&markdown).replace("\r\n", "\n");
    let entwurf = mail::finalize(&MailDraft {
        to: empfaenger.adressen,
        subject: pk::betreff(&plan.title, &datum),
        body_text: pk::entwurfstext(&plan.classification, &text),
        body_html: String::new(),
    });
    Ok(ProtokollEntwurf {
        entwurf,
        ohne_adresse: empfaenger.ohne_adresse as u32,
        termin_gefunden: termin.is_some(),
    })
}
