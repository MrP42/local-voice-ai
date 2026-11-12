//! Adapter C (Konzept 3.3): Abgleich der Planungen mit dem Kalender-Cache der ICS-Quellen.
//!
//! Der Mensch legt den Termin in Outlook an und fuegt den Einladungstext mit der Marke
//! `lva-plan:<id>` ein; der ICS-Feed liefert ihn spaeter in `calendar_events`. Nach jedem Abruf
//! (`CalendarService::run_sync`) gleicht [`abgleich_lauf`] jede offene Verknuepfung
//! (`besprechungsplanung::store::links_offen`) mit dem Cache ab:
//!
//! - **Erstverknuepfung nur ueber die Marke** im Klartext von `description` (`finde_marke`). Gibt
//!   es mehrere Termine mit der Marke (Instanzen einer Serie, Kopien), wird nichts verknuepft,
//!   sondern gemeldet (`mehrdeutig`); die Auswahl trifft die Nutzerin bzw. der Nutzer
//!   ([`termin_waehlen`]).
//! - **Danach ueber UID und Instanz** (`calendar_events.uid`, `instance_start` = RECURRENCE-ID):
//!   ein anderer Termin oder eine andere Serieninstanz ersetzt den verknuepften nie still
//!   (`sync::abgleichen` verwirft die Beobachtung und meldet sie einmal). Ein Wechsel geht nur
//!   ueber [`super::identitaet_loesen`] oder [`termin_waehlen`].
//! - **Ein Termin gilt nur dann als geloescht** (`Beobachtung = None`, Status "verwaist"), wenn der
//!   Cache ihn haette zeigen muessen: Beginn im Abruffenster (mit einem Tag Rand) und Quelle aktiv.
//!   Sonst (Termin weit in der Zukunft, Quelle abgeschaltet oder entfernt) wird die Planung nicht
//!   angefasst: ein Cache ohne den Termin ist dann keine Aussage.
//! - Adapter C schreibt nie in den Termin; die Pruefsumme des Blocks bleibt ungenutzt.
//!
//! Jede Planung wird in EINER Transaktion (`BEGIN IMMEDIATE`) abgeglichen: Verknuepfung, Planstatus
//! und neue Einwilligungen werden zusammen geschrieben, der Stand wird erst NACH der Sperre
//! gelesen, und der Planstatus wird nur geschrieben, wenn er noch der gelesene ist. Ein Fehler
//! rollt diese Planung ganz zurueck; die anderen laufen weiter.

use std::sync::LazyLock;

use besprechungsplanung::consent;
use besprechungsplanung::invitation::{finde_marke, marke};
use besprechungsplanung::model::{AdapterKind, Plan, PlanLink, SyncState};
use besprechungsplanung::store as plan_store;
use besprechungsplanung::sync::{self, Beobachtung, Ereignis, Feld};
use regex::Regex;
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use specta::Type;

use super::{hat_termin, lose, schreib_tx, status_wechseln, verbindung, Res};
use crate::managers::calendar::service::window_for;
use crate::managers::meetings::store::MeetingStore;

const DAY_MS: i64 = 86_400_000;

/// Ein Ereignis des Abgleichs, einer Planung zugeordnet (fuer Meldungen der Oberflaeche).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PlanEreignis {
    pub plan_id: String,
    pub ereignis: Ereignis,
}

/// Eine Planung, deren Abgleich scheiterte (Code des Crates oder `sql: ...`); nichts davon ist
/// geschrieben.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PlanFehler {
    pub plan_id: String,
    pub code: String,
}

/// Ergebnis eines Abgleich-Laufs. Enthaelt nur Ids, Codes und Zahlen, keine Inhalte.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AbgleichBericht {
    /// Offene Verknuepfungen dieses Adapters.
    pub geprueft: u32,
    /// Planungen, bei denen sich etwas geaendert hat (gespeichert).
    pub geaendert: u32,
    pub ereignisse: Vec<PlanEreignis>,
    /// Die Marke steht auf mehreren Terminen: nichts verknuepft, Auswahl noetig.
    pub mehrdeutig: Vec<String>,
    /// Termin nicht im Cache, aber ohne Aussage dazu (ausserhalb des Fensters, Quelle weg).
    pub uebersprungen: u32,
    pub fehler: Vec<PlanFehler>,
}

/// Ein Termin aus dem Cache, wie der Abgleich ihn braucht.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Termin {
    pub key: String,
    pub source_id: String,
    pub uid: String,
    pub starts_at: i64,
    pub ends_at: i64,
    pub cancelled: bool,
    pub location: Option<String>,
    pub join_url: Option<String>,
    /// Beschreibung als Klartext ([`klartext`]); leer, wenn der Termin keine hat.
    pub text: String,
    pub instance_start: Option<i64>,
    /// Adressen der Teilnehmenden (nur solche mit E-Mail-Adresse).
    pub teilnehmende: Vec<String>,
}

// ---------------------------------------------------------------------------
// Klartext
// ---------------------------------------------------------------------------

static HTML_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<\s*/?\s*(html|body|head|div|p|br|span|table|tr|td|ul|ol|li|a|b|i|u|strong|em|font|h[1-6])\b[^>]*>")
        .expect("statischer Ausdruck")
});

/// Die Marke wird nur im Klartext gesucht (Konzept 3.4). ICS fuehrt in `DESCRIPTION` Text; manche
/// Anbieter legen dort aber HTML ab. Sieht die Beschreibung nach HTML aus, wird sie vorher in
/// Text gewandelt (`media::html_to_text`: Blockenden werden Zeilenumbrueche, Tags fallen weg).
/// Gewoehnlicher Text bleibt unveraendert, damit `<` und `>` darin keine Tags werden.
pub(crate) fn klartext(beschreibung: &str) -> String {
    if HTML_TAG.is_match(beschreibung) {
        crate::media::html_to_text(beschreibung)
    } else {
        beschreibung.to_string()
    }
}

// ---------------------------------------------------------------------------
// Lesen aus dem Cache
// ---------------------------------------------------------------------------

/// Termine aus aktiven ICS-Quellen (Adapter C beobachtet nur Terminfeeds; die Beschreibung der
/// Graph-Termine ist nur eine Vorschau und kann die Marke nicht tragen).
const TERMIN_SELECT: &str = "SELECT e.key, e.source_id, e.uid, e.starts_at, e.ends_at, e.cancelled,
        e.location, e.join_url, e.description, e.instance_start
     FROM calendar_events e JOIN calendar_sources s ON s.id = e.source_id
     WHERE s.kind = 'ics' AND s.enabled = 1 AND s.deleted_at IS NULL";

fn termine(conn: &Connection, bedingung: &str, args: &[&dyn rusqlite::ToSql]) -> Res<Vec<Termin>> {
    let sql = format!("{TERMIN_SELECT} AND {bedingung} ORDER BY e.starts_at, e.key");
    let mut st = conn.prepare(&sql).map_err(sql_fehler)?;
    let rows = st
        .query_map(args, |r| {
            let beschreibung: Option<String> = r.get(8)?;
            Ok(Termin {
                key: r.get(0)?,
                source_id: r.get(1)?,
                uid: r.get(2)?,
                starts_at: r.get(3)?,
                ends_at: r.get(4)?,
                cancelled: r.get::<_, i64>(5)? != 0,
                location: r.get(6)?,
                join_url: r.get(7)?,
                text: klartext(beschreibung.as_deref().unwrap_or("")),
                instance_start: r.get(9)?,
                teilnehmende: Vec::new(),
            })
        })
        .map_err(sql_fehler)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_fehler)?;
    let mut att = conn
        .prepare(
            "SELECT email FROM calendar_event_attendees
             WHERE event_key = ?1 AND email <> '' ORDER BY rowid",
        )
        .map_err(sql_fehler)?;
    rows.into_iter()
        .map(|mut t| {
            t.teilnehmende = att
                .query_map(params![t.key], |r| r.get::<_, String>(0))
                .map_err(sql_fehler)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_fehler)?;
            Ok(t)
        })
        .collect()
}

fn sql_fehler(e: rusqlite::Error) -> String {
    format!("sql: {e}")
}

/// Termine, deren Klartext die Marke dieser Planung als Blockende traegt.
fn termine_mit_marke(conn: &Connection, plan_id: &str) -> Res<Vec<Termin>> {
    // Vorauswahl in SQL (`instr`, kein LIKE: `_` waere ein Platzhalter), genau pruefen in Rust.
    let m = marke(plan_id);
    let mut gefunden: Vec<Termin> = Vec::new();
    for t in termine(conn, "instr(e.description, ?1) > 0", &[&m])? {
        let schon = gefunden
            .iter()
            .any(|g| g.uid == t.uid && g.instance_start == t.instance_start);
        if !schon && finde_marke(&t.text).as_deref() == Some(plan_id) {
            gefunden.push(t);
        }
    }
    Ok(gefunden)
}

/// Gehoert der Termin (UID, Instanz) schon einer anderen Planung?
fn belegt(conn: &Connection, plan_id: &str, t: &Termin) -> Res<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM meeting_plan_links
         WHERE adapter = ?1 AND remote_id = ?2 AND instance_start IS ?3 AND plan_id <> ?4",
        params![
            AdapterKind::IcsMarker.as_str(),
            t.uid,
            t.instance_start,
            plan_id
        ],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(sql_fehler)
}

fn quelle_aktiv(conn: &Connection, source_id: Option<&str>) -> Res<bool> {
    let Some(id) = source_id else {
        return Ok(false);
    };
    conn.query_row(
        "SELECT COUNT(*) FROM calendar_sources
         WHERE id = ?1 AND kind = 'ics' AND enabled = 1 AND deleted_at IS NULL",
        params![id],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(sql_fehler)
}

// ---------------------------------------------------------------------------
// Beobachtung bauen
// ---------------------------------------------------------------------------

/// ICS liefert Ort und Beitritts-Link immer; ein fehlender Wert heisst "leer" (loescht), nicht
/// "unbekannt".
fn feld(v: &Option<String>) -> Feld {
    match v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Feld::Wert(s.to_string()),
        None => Feld::Leer,
    }
}

/// Die Beobachtung zu einem Termin des Caches. Der Text ist immer da (ohne Beschreibung leer): ein
/// Termin, dessen Beschreibung die Marke nicht mehr traegt, ist nicht mehr die Planung.
pub(crate) fn beobachtung(t: &Termin) -> Beobachtung {
    Beobachtung {
        remote_id: Some(t.uid.clone()),
        ical_uid: Some(t.uid.clone()),
        change_key: None,
        instanz: t.instance_start,
        starts_at: t.starts_at,
        ends_at: t.ends_at,
        location: feld(&t.location),
        join_url: feld(&t.join_url),
        cancelled: t.cancelled,
        description: Some(t.text.clone()),
        attendees: t.teilnehmende.clone(),
    }
}

// ---------------------------------------------------------------------------
// Anwenden: Verknuepfung, Planstatus und Einwilligungen in einer Transaktion
// ---------------------------------------------------------------------------

/// Was `anwenden` geschrieben hat.
pub(crate) struct Angewendet {
    pub ereignisse: Vec<Ereignis>,
    pub geschrieben: bool,
}

/// Gleicht `link` mit der Beobachtung ab und speichert das Ergebnis: Verknuepfung (nur bei
/// Aenderung), Planstatus (nur wenn er noch `plan.status` ist) und neue Einwilligungen fuer neue
/// Teilnehmende (Zustand `offen`; nicht, wenn der Termin verwaist ist). Aufrufer halten die
/// Schreib-Transaktion.
pub(crate) fn anwenden(
    tx: &Transaction<'_>,
    plan: &Plan,
    link: &PlanLink,
    termin: Option<&Termin>,
    jetzt: i64,
) -> Res<Angewendet> {
    let obs = termin.map(beobachtung);
    let mut a = sync::abgleichen(plan.status, link, obs.as_ref(), jetzt);
    // Erstverknuepfung: die Kalenderquelle des Termins merken (spaeter pruefen wir an ihr, ob ein
    // fehlender Termin etwas aussagt).
    if let Some(t) = termin {
        if !hat_termin(link) && a.link.starts_at.is_some() {
            a.link.source_id = Some(t.source_id.clone());
        }
    }
    let mut geschrieben = false;
    // `last_seen_at` rueckt bei jedem Abgleich vor; allein deshalb wird nichts geschrieben (sonst
    // schriebe jeder Abruf jede offene Verknuepfung neu).
    let mut vergleich = a.link.clone();
    vergleich.last_seen_at = link.last_seen_at;
    if vergleich != *link {
        plan_store::link_speichern(tx, &a.link)?;
        geschrieben = true;
    }
    if let Some(ziel) = a.plan_status {
        status_wechseln(tx, &plan.id, plan.status, ziel, jetzt)?;
        geschrieben = true;
    }
    if a.link.sync_state != SyncState::Verwaist && !a.teilnehmende.is_empty() {
        let bestehend = plan_store::einwilligungen(tx, &plan.id)?;
        for c in consent::neue_einwilligungen(&plan.id, &bestehend, &a.teilnehmende) {
            plan_store::einwilligung_setzen(tx, &c, jetzt)?;
            geschrieben = true;
        }
    }
    Ok(Angewendet {
        ereignisse: a.ereignisse,
        geschrieben,
    })
}

// ---------------------------------------------------------------------------
// Suche je Verknuepfung
// ---------------------------------------------------------------------------

enum Suche {
    /// Der Termin (bei bekannter Verknuepfung ggf. ein anderer derselben UID: der Abgleich
    /// verwirft ihn und meldet es).
    Treffer(Box<Termin>),
    /// Der Cache haette den Termin zeigen muessen und zeigt ihn nicht.
    Verschwunden,
    /// Mehrere Termine tragen die Marke.
    Mehrdeutig,
    /// Nichts zu tun: noch kein Termin mit der Marke, oder der Cache sagt nichts.
    Nichts,
}

fn suche(tx: &Transaction<'_>, plan: &Plan, link: &PlanLink, jetzt: i64) -> Res<Suche> {
    if hat_termin(link) {
        suche_bekannt(tx, link, jetzt)
    } else {
        suche_marke(tx, &plan.id)
    }
}

fn suche_marke(tx: &Transaction<'_>, plan_id: &str) -> Res<Suche> {
    let mut frei = Vec::new();
    for t in termine_mit_marke(tx, plan_id)? {
        if !belegt(tx, plan_id, &t)? {
            frei.push(t);
        }
    }
    Ok(match frei.len() {
        0 => Suche::Nichts,
        1 => Suche::Treffer(Box::new(frei.remove(0))),
        _ => Suche::Mehrdeutig,
    })
}

fn suche_bekannt(tx: &Transaction<'_>, link: &PlanLink, jetzt: i64) -> Res<Suche> {
    let Some(uid) = link.remote_id.as_deref().or(link.ical_uid.as_deref()) else {
        return Ok(Suche::Nichts);
    };
    let mut zeilen = termine(tx, "e.uid = ?1", &[&uid])?;
    // Dieselbe Instanz, bevorzugt aus der Quelle der Verknuepfung.
    let passend = |t: &Termin| t.instance_start == link.instanz;
    let bevorzugt = zeilen
        .iter()
        .position(|t| passend(t) && Some(t.source_id.as_str()) == link.source_id.as_deref())
        .or_else(|| zeilen.iter().position(passend));
    if let Some(i) = bevorzugt {
        return Ok(Suche::Treffer(Box::new(zeilen.swap_remove(i))));
    }
    if !zeilen.is_empty() {
        // Die UID ist da, die Instanz nicht: die Beobachtung gehoert zu einer anderen Instanz.
        // Der Abgleich verwirft sie und meldet es einmal; verwaist ist der Termin damit nicht.
        return Ok(Suche::Treffer(Box::new(zeilen.remove(0))));
    }
    let (von, bis) = window_for(jetzt);
    let im_fenster = link
        .starts_at
        .is_some_and(|s| s >= von + DAY_MS && s <= bis - DAY_MS);
    if im_fenster && quelle_aktiv(tx, link.source_id.as_deref())? {
        Ok(Suche::Verschwunden)
    } else {
        Ok(Suche::Nichts)
    }
}

// ---------------------------------------------------------------------------
// Lauf
// ---------------------------------------------------------------------------

enum Ergebnis {
    Angewendet(Angewendet),
    Mehrdeutig,
    Uebersprungen,
}

/// Gleicht eine Planung ab (eine Transaktion). Stand und Verknuepfung werden erst nach der
/// Schreibsperre gelesen.
fn pruefe_plan(conn: &mut Connection, plan_id: &str, jetzt: i64) -> Res<Ergebnis> {
    let tx = schreib_tx(conn)?;
    let Some(plan) = plan_store::plan_laden(&tx, plan_id)? else {
        return Ok(Ergebnis::Uebersprungen);
    };
    if plan.status.ist_abgeschlossen() {
        return Ok(Ergebnis::Uebersprungen);
    }
    let Some(link) = plan_store::link_laden(&tx, plan_id)? else {
        return Ok(Ergebnis::Uebersprungen);
    };
    if link.adapter != AdapterKind::IcsMarker {
        return Ok(Ergebnis::Uebersprungen);
    }
    let angewendet = match suche(&tx, &plan, &link, jetzt)? {
        Suche::Treffer(t) => anwenden(&tx, &plan, &link, Some(&*t), jetzt)?,
        Suche::Verschwunden => anwenden(&tx, &plan, &link, None, jetzt)?,
        Suche::Mehrdeutig => return Ok(Ergebnis::Mehrdeutig),
        Suche::Nichts => return Ok(Ergebnis::Uebersprungen),
    };
    tx.commit().map_err(sql_fehler)?;
    Ok(Ergebnis::Angewendet(angewendet))
}

/// Gleicht alle offenen Verknuepfungen von Adapter C mit dem Kalender-Cache ab. Laeuft nach jedem
/// Kalenderabruf; `jetzt` in ms. Eine Planung, deren Abgleich scheitert, steht im Bericht unter
/// `fehler` (nichts von ihr ist geschrieben); die anderen werden trotzdem abgeglichen.
pub fn abgleich_lauf(store: &MeetingStore, jetzt: i64) -> Res<AbgleichBericht> {
    let mut conn = verbindung(store)?;
    let ids: Vec<String> = plan_store::links_offen(&conn)?
        .into_iter()
        .filter(|l| l.adapter == AdapterKind::IcsMarker)
        .map(|l| l.plan_id)
        .collect();
    let mut bericht = AbgleichBericht {
        geprueft: ids.len() as u32,
        ..AbgleichBericht::default()
    };
    for id in ids {
        match pruefe_plan(&mut conn, &id, jetzt) {
            Ok(Ergebnis::Angewendet(a)) => {
                if a.geschrieben {
                    bericht.geaendert += 1;
                }
                bericht
                    .ereignisse
                    .extend(a.ereignisse.into_iter().map(|ereignis| PlanEreignis {
                        plan_id: id.clone(),
                        ereignis,
                    }));
            }
            Ok(Ergebnis::Mehrdeutig) => bericht.mehrdeutig.push(id),
            Ok(Ergebnis::Uebersprungen) => bericht.uebersprungen += 1,
            Err(code) => bericht.fehler.push(PlanFehler { plan_id: id, code }),
        }
    }
    Ok(bericht)
}

/// Bestaetigte Auswahl: bindet die Planung an genau diesen Termin des Caches (`event_key`). Eine
/// bestehende Bindung wird vorher geloest ([`lose`]). Der Termin muss die Marke DIESER Planung
/// tragen (`marke_fehlt`), darf keiner anderen Planung gehoeren (`termin_belegt`) und muss in
/// einer aktiven ICS-Quelle stehen (`termin_nicht_gefunden`). Gedacht fuer den Wechsel des
/// Termins und fuer mehrdeutige Marken (Serien).
pub fn termin_waehlen(
    store: &MeetingStore,
    plan_id: &str,
    event_key: &str,
    jetzt: i64,
) -> Res<Vec<Ereignis>> {
    let mut conn = verbindung(store)?;
    let tx = schreib_tx(&mut conn)?;
    let plan = plan_store::plan_laden(&tx, plan_id)?.ok_or("plan_nicht_gefunden")?;
    if plan.status.ist_abgeschlossen() {
        return Err("plan_abgeschlossen".to_string());
    }
    let termin = termine(&tx, "e.key = ?1", &[&event_key])?
        .into_iter()
        .next()
        .ok_or("termin_nicht_gefunden")?;
    if finde_marke(&termin.text).as_deref() != Some(plan_id) {
        return Err("marke_fehlt".to_string());
    }
    if belegt(&tx, plan_id, &termin)? {
        return Err("termin_belegt".to_string());
    }
    let link = match plan_store::link_laden(&tx, plan_id)? {
        Some(l) => lose(&l),
        None => super::leere_verknuepfung(plan_id),
    };
    let angewendet = anwenden(&tx, &plan, &link, Some(&termin), jetzt)?;
    tx.commit().map_err(sql_fehler)?;
    Ok(angewendet.ereignisse)
}
