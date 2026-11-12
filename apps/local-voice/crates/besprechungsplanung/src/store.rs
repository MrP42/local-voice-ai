//! Speicher der Besprechungsplanung (SQLite, Konzept 3.2).
//!
//! Nur `CREATE TABLE`/`CREATE INDEX`, wie im Repo ueblich. Die App haengt
//! [`MIGRATIONS`] an die Migrationskette ihrer Besprechungs-Datenbank an (jede
//! Zeile ein eigener Schritt; nie umsortieren, nur anhaengen). Die Funktionen
//! arbeiten auf einer geliehenen Verbindung; Transaktionen steuert die App.
//!
//! Fehlercodes (Text des Fehlers): `plan_id_ungueltig`, `plan_nicht_gefunden`,
//! `plan_abgeschlossen`, `daten_ungueltig`, `status_uebergang_ungueltig`, sowie die
//! Codes der Regelpruefung mit Stufe `Fehler` (z. B. `titel_leer`, `dauer_null`).

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::rules;

use crate::consent::identitaet;
use crate::model::{
    AdapterKind, Consent, ConsentChannel, ConsentState, GoalType, Plan, PlanInput, PlanLink,
    PlanStatus, SyncState,
};

/// Migrationsschritte, in dieser Reihenfolge anzuhaengen.
pub const MIGRATIONS: &[&str] = &["CREATE TABLE IF NOT EXISTS meeting_plans (
        id TEXT PRIMARY KEY,
        meeting_id TEXT,
        title TEXT NOT NULL,
        goal_type TEXT NOT NULL,
        goal_text TEXT NOT NULL,
        expected_result TEXT,
        duration_min INTEGER NOT NULL,
        classification TEXT NOT NULL DEFAULT 'intern',
        agenda_json TEXT NOT NULL DEFAULT '[]',
        roles_json TEXT NOT NULL DEFAULT '{}',
        materials_json TEXT NOT NULL DEFAULT '[]',
        hints_json TEXT NOT NULL DEFAULT '{}',
        transcription_planned INTEGER NOT NULL DEFAULT 0,
        minutes_template_id TEXT,
        status TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER
    );
    CREATE INDEX IF NOT EXISTS idx_meeting_plans_meeting ON meeting_plans(meeting_id);
    CREATE TABLE IF NOT EXISTS meeting_plan_links (
        plan_id TEXT PRIMARY KEY REFERENCES meeting_plans(id) ON DELETE CASCADE,
        adapter TEXT NOT NULL,
        source_id TEXT,
        remote_id TEXT,
        ical_uid TEXT,
        change_key TEXT,
        starts_at INTEGER,
        ends_at INTEGER,
        location TEXT,
        join_url TEXT,
        body_hash TEXT,
        last_seen_at INTEGER,
        last_change_at INTEGER,
        sync_state TEXT NOT NULL,
        move_pending INTEGER NOT NULL DEFAULT 0,
        block_conflict INTEGER NOT NULL DEFAULT 0,
        cancelled INTEGER NOT NULL DEFAULT 0,
        instance_start INTEGER
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_meeting_plan_links_remote
        ON meeting_plan_links(adapter, remote_id, COALESCE(instance_start, -1))
        WHERE remote_id IS NOT NULL;
    CREATE INDEX IF NOT EXISTS idx_meeting_plan_links_uid ON meeting_plan_links(ical_uid);
    CREATE TABLE IF NOT EXISTS meeting_plan_consents (
        plan_id TEXT NOT NULL REFERENCES meeting_plans(id) ON DELETE CASCADE,
        person TEXT NOT NULL,
        state TEXT NOT NULL,
        channel TEXT,
        recorded_at INTEGER,
        PRIMARY KEY (plan_id, person)
    );
    CREATE TABLE IF NOT EXISTS meeting_plan_consent_log (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        plan_id TEXT NOT NULL REFERENCES meeting_plans(id) ON DELETE CASCADE,
        person TEXT NOT NULL,
        state TEXT NOT NULL,
        channel TEXT,
        recorded_at INTEGER,
        logged_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_meeting_plan_consent_log_plan
        ON meeting_plan_consent_log(plan_id, person);"];

/// Wendet [`MIGRATIONS`] direkt an (Tests, eigenstaendige Nutzung).
///
/// Die Verbindung braucht `PRAGMA foreign_keys = ON`, sonst greift das
/// `ON DELETE CASCADE` nicht und Einwilligungen (Personendaten) blieben beim
/// endgueltigen Loeschen einer Planung liegen. Die Zugriffe hier pruefen die
/// Planung zusaetzlich selbst.
pub fn migrieren(conn: &Connection) -> rusqlite::Result<()> {
    for m in MIGRATIONS {
        conn.execute_batch(m)?;
    }
    Ok(())
}

/// Ids gehen als Marke in fremde Termine: nur `[A-Za-z0-9_-]{1,64}`.
pub fn gueltige_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

type Res<T> = Result<T, String>;

fn sql(e: rusqlite::Error) -> String {
    match e {
        rusqlite::Error::FromSqlConversionFailure(_, _, ref inner)
            if inner.downcast_ref::<Beschaedigt>().is_some() =>
        {
            "daten_beschaedigt".to_string()
        }
        e => format!("sql: {e}"),
    }
}

/// Gespeicherter Wert passt nicht zum Schema (ungueltiges JSON, unbekannter
/// Status oder Adapter). Wird als `daten_beschaedigt` gemeldet, nie still ersetzt:
/// sonst verschwaenden Inhalte beim naechsten Speichern oder abgeschlossene
/// Planungen erschienen wieder editierbar.
#[derive(Debug)]
struct Beschaedigt;

impl std::fmt::Display for Beschaedigt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("daten_beschaedigt")
    }
}

impl std::error::Error for Beschaedigt {}

fn kaputt(spalte: usize) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        spalte,
        rusqlite::types::Type::Text,
        Box::new(Beschaedigt),
    )
}

fn enum_spalte<T>(r: &Row, i: usize, parse: fn(&str) -> Option<T>) -> rusqlite::Result<T> {
    let s: String = r.get(i)?;
    parse(&s).ok_or_else(|| kaputt(i))
}

fn json<T: serde::Serialize>(v: &T) -> Res<String> {
    serde_json::to_string(v).map_err(|_| "daten_ungueltig".to_string())
}

/// Harte Regeln auch im Speicher: was die Regelpruefung als Fehler meldet,
/// wird nicht gespeichert (unabhaengig davon, ob die App vorher prueft).
fn eingabe_pruefen(input: &PlanInput, regeln: &rules::Regeln) -> Res<()> {
    let hinweise = rules::pruefen(input, None, regeln);
    match hinweise.iter().find(|h| h.stufe == rules::Stufe::Fehler) {
        Some(h) => Err(h.code.clone()),
        None => Ok(()),
    }
}

/// Legt eine Planung im Status `entwurf` an.
pub fn plan_anlegen(
    conn: &Connection,
    id: &str,
    input: &PlanInput,
    regeln: &rules::Regeln,
    jetzt: i64,
) -> Res<Plan> {
    if !gueltige_id(id) {
        return Err("plan_id_ungueltig".into());
    }
    eingabe_pruefen(input, regeln)?;
    conn.execute(
        "INSERT INTO meeting_plans (id, meeting_id, title, goal_type, goal_text, expected_result,
            duration_min, classification, agenda_json, roles_json, materials_json, hints_json,
            transcription_planned, minutes_template_id, status, created_at, updated_at)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
        params![
            id,
            input.title.trim(),
            input.goal_type.as_str(),
            input.goal_text.trim(),
            input.expected_result.as_deref().map(str::trim),
            input.duration_min,
            input.classification.trim(),
            json(&input.agenda)?,
            json(&input.roles)?,
            json(&input.materials)?,
            json(&input.hints)?,
            input.transcription_planned,
            input.minutes_template_id,
            PlanStatus::Entwurf.as_str(),
            jetzt,
        ],
    )
    .map_err(sql)?;
    plan_laden(conn, id)?.ok_or_else(|| "plan_nicht_gefunden".into())
}

/// Aendert die Inhalte einer Planung. Nach der Durchfuehrung nicht mehr (das
/// Protokoll bezieht sich auf die Fassung, die galt).
pub fn plan_aendern(
    conn: &Connection,
    id: &str,
    input: &PlanInput,
    regeln: &rules::Regeln,
    jetzt: i64,
) -> Res<Plan> {
    let alt = plan_laden(conn, id)?.ok_or("plan_nicht_gefunden")?;
    if alt.status.ist_abgeschlossen() {
        return Err("plan_abgeschlossen".into());
    }
    eingabe_pruefen(input, regeln)?;
    conn.execute(
        "UPDATE meeting_plans SET title = ?2, goal_type = ?3, goal_text = ?4, expected_result = ?5,
            duration_min = ?6, classification = ?7, agenda_json = ?8, roles_json = ?9,
            materials_json = ?10, hints_json = ?11, transcription_planned = ?12,
            minutes_template_id = ?13, updated_at = ?14
         WHERE id = ?1 AND deleted_at IS NULL",
        params![
            id,
            input.title.trim(),
            input.goal_type.as_str(),
            input.goal_text.trim(),
            input.expected_result.as_deref().map(str::trim),
            input.duration_min,
            input.classification.trim(),
            json(&input.agenda)?,
            json(&input.roles)?,
            json(&input.materials)?,
            json(&input.hints)?,
            input.transcription_planned,
            input.minutes_template_id,
            jetzt,
        ],
    )
    .map_err(sql)?;
    plan_laden(conn, id)?.ok_or_else(|| "plan_nicht_gefunden".into())
}

/// Ist der Uebergang erlaubt? Ein abgeschlossener Status (durchgefuehrt,
/// protokolliert, versendet) geht nur vorwaerts, nie zurueck in die Planung.
pub fn uebergang_erlaubt(von: PlanStatus, nach: PlanStatus) -> bool {
    use PlanStatus::*;
    if !von.ist_abgeschlossen() {
        return true;
    }
    let rang = |s: PlanStatus| match s {
        Durchgefuehrt => 1,
        Protokolliert => 2,
        Versendet => 3,
        _ => 0,
    };
    rang(nach) >= rang(von)
}

/// Setzt den Status (nur erlaubte Uebergaenge, siehe [`uebergang_erlaubt`]).
pub fn plan_status_setzen(conn: &Connection, id: &str, status: PlanStatus, jetzt: i64) -> Res<()> {
    let alt = plan_laden(conn, id)?.ok_or("plan_nicht_gefunden")?;
    if !uebergang_erlaubt(alt.status, status) {
        return Err("status_uebergang_ungueltig".into());
    }
    let n = conn
        .execute(
            "UPDATE meeting_plans SET status = ?2, updated_at = ?3 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, status.as_str(), jetzt],
        )
        .map_err(sql)?;
    if n == 0 {
        return Err("plan_nicht_gefunden".into());
    }
    Ok(())
}

/// Verbindet die Planung mit ihrer Besprechung (leerer Eintrag der App).
pub fn plan_besprechung_setzen(
    conn: &Connection,
    id: &str,
    meeting_id: Option<&str>,
    jetzt: i64,
) -> Res<()> {
    let n = conn
        .execute(
            "UPDATE meeting_plans SET meeting_id = ?2, updated_at = ?3 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, meeting_id, jetzt],
        )
        .map_err(sql)?;
    if n == 0 {
        return Err("plan_nicht_gefunden".into());
    }
    Ok(())
}

/// Weiches Loeschen (Papierkorb der App). Verknuepfung und Einwilligungen bleiben,
/// bis die Zeile endgueltig entfernt wird.
pub fn plan_loeschen(conn: &Connection, id: &str, jetzt: i64) -> Res<()> {
    let n = conn
        .execute(
            "UPDATE meeting_plans SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, jetzt],
        )
        .map_err(sql)?;
    if n == 0 {
        return Err("plan_nicht_gefunden".into());
    }
    Ok(())
}

const PLAN_SPALTEN: &str = "id, meeting_id, title, goal_type, goal_text, expected_result,
    duration_min, classification, agenda_json, roles_json, materials_json, hints_json,
    transcription_planned, minutes_template_id, status, created_at, updated_at";

fn plan_aus_zeile(r: &Row) -> rusqlite::Result<Plan> {
    fn j<T: serde::de::DeserializeOwned>(r: &Row, i: usize) -> rusqlite::Result<T> {
        let s: String = r.get(i)?;
        serde_json::from_str(&s).map_err(|_| kaputt(i))
    }
    Ok(Plan {
        id: r.get(0)?,
        meeting_id: r.get(1)?,
        title: r.get(2)?,
        goal_type: enum_spalte(r, 3, GoalType::parse)?,
        goal_text: r.get(4)?,
        expected_result: r.get(5)?,
        duration_min: r.get(6)?,
        classification: r.get(7)?,
        agenda: j(r, 8)?,
        roles: j(r, 9)?,
        materials: j(r, 10)?,
        hints: j(r, 11)?,
        transcription_planned: r.get(12)?,
        minutes_template_id: r.get(13)?,
        status: enum_spalte(r, 14, PlanStatus::parse)?,
        created_at: r.get(15)?,
        updated_at: r.get(16)?,
    })
}

/// Laedt eine (nicht geloeschte) Planung.
pub fn plan_laden(conn: &Connection, id: &str) -> Res<Option<Plan>> {
    conn.query_row(
        &format!("SELECT {PLAN_SPALTEN} FROM meeting_plans WHERE id = ?1 AND deleted_at IS NULL"),
        [id],
        plan_aus_zeile,
    )
    .optional()
    .map_err(sql)
}

/// Alle nicht geloeschten Planungen, zuletzt geaenderte zuerst.
pub fn plan_liste(conn: &Connection) -> Res<Vec<Plan>> {
    let mut st = conn
        .prepare(&format!(
            "SELECT {PLAN_SPALTEN} FROM meeting_plans WHERE deleted_at IS NULL
             ORDER BY updated_at DESC, id"
        ))
        .map_err(sql)?;
    let rows = st.query_map([], plan_aus_zeile).map_err(sql)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(sql)
}

/// Speichert eine Verknuepfung (ersetzt die bisherige der Planung). Die Planung
/// muss existieren; ein Termin (`adapter`, `remote_id`) gehoert hoechstens einer Planung.
pub fn link_speichern(conn: &Connection, l: &PlanLink) -> Res<()> {
    plan_laden(conn, &l.plan_id)?.ok_or("plan_nicht_gefunden")?;
    conn.execute(
        "INSERT INTO meeting_plan_links (plan_id, adapter, source_id, remote_id, ical_uid,
            change_key, starts_at, ends_at, location, join_url, body_hash, last_seen_at,
            last_change_at, sync_state, move_pending, block_conflict, cancelled, instance_start)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT(plan_id) DO UPDATE SET adapter = excluded.adapter,
            source_id = excluded.source_id, remote_id = excluded.remote_id,
            ical_uid = excluded.ical_uid, change_key = excluded.change_key,
            starts_at = excluded.starts_at, ends_at = excluded.ends_at,
            location = excluded.location, join_url = excluded.join_url,
            body_hash = excluded.body_hash, last_seen_at = excluded.last_seen_at,
            last_change_at = excluded.last_change_at, sync_state = excluded.sync_state,
            move_pending = excluded.move_pending, block_conflict = excluded.block_conflict,
            cancelled = excluded.cancelled, instance_start = excluded.instance_start",
        params![
            l.plan_id,
            l.adapter.as_str(),
            l.source_id,
            l.remote_id,
            l.ical_uid,
            l.change_key,
            l.starts_at,
            l.ends_at,
            l.location,
            l.join_url,
            l.body_hash,
            l.last_seen_at,
            l.last_change_at,
            l.sync_state.as_str(),
            l.verschiebung_offen,
            l.block_konflikt,
            l.abgesagt,
            l.instanz,
        ],
    )
    .map_err(sql)?;
    Ok(())
}

fn link_aus_zeile(r: &Row) -> rusqlite::Result<PlanLink> {
    Ok(PlanLink {
        plan_id: r.get(0)?,
        adapter: enum_spalte(r, 1, AdapterKind::parse)?,
        source_id: r.get(2)?,
        remote_id: r.get(3)?,
        ical_uid: r.get(4)?,
        change_key: r.get(5)?,
        starts_at: r.get(6)?,
        ends_at: r.get(7)?,
        location: r.get(8)?,
        join_url: r.get(9)?,
        body_hash: r.get(10)?,
        last_seen_at: r.get(11)?,
        last_change_at: r.get(12)?,
        sync_state: enum_spalte(r, 13, SyncState::parse)?,
        verschiebung_offen: r.get(14)?,
        block_konflikt: r.get(15)?,
        abgesagt: r.get(16)?,
        instanz: r.get(17)?,
    })
}

const LINK_SPALTEN: &str = "plan_id, adapter, source_id, remote_id, ical_uid, change_key,
    starts_at, ends_at, location, join_url, body_hash, last_seen_at, last_change_at, sync_state,
    move_pending, block_conflict, cancelled, instance_start";

/// Laedt die Verknuepfung einer Planung.
pub fn link_laden(conn: &Connection, plan_id: &str) -> Res<Option<PlanLink>> {
    conn.query_row(
        &format!("SELECT {LINK_SPALTEN} FROM meeting_plan_links WHERE plan_id = ?1"),
        [plan_id],
        link_aus_zeile,
    )
    .optional()
    .map_err(sql)
}

/// Alle Verknuepfungen nicht geloeschter Planungen, die der Abgleich pruefen muss
/// (ohne abgeschlossene Planungen).
pub fn links_offen(conn: &Connection) -> Res<Vec<PlanLink>> {
    let spalten = LINK_SPALTEN
        .split(',')
        .map(|s| format!("l.{}", s.trim()))
        .collect::<Vec<_>>()
        .join(", ");
    let mut st = conn
        .prepare(&format!(
            "SELECT {spalten} FROM meeting_plan_links l JOIN meeting_plans p ON p.id = l.plan_id
             WHERE p.deleted_at IS NULL
               AND p.status NOT IN ('durchgefuehrt', 'protokolliert', 'versendet')
             ORDER BY l.plan_id"
        ))
        .map_err(sql)?;
    let rows = st.query_map([], link_aus_zeile).map_err(sql)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(sql)
}

/// Setzt eine Einwilligung (legt sie an oder ersetzt den aktuellen Stand). Die
/// Person wird hier normalisiert ([`identitaet`]), damit Abgleich und
/// Start-Tor sie finden. Zustimmung und Ablehnung brauchen einen Nachweis
/// (Zeitpunkt und Kanal), sonst `einwilligung_nachweis_fehlt`. Jede Aenderung,
/// auch ein Widerruf, wird zusaetzlich in `meeting_plan_consent_log` angehaengt;
/// der bisherige Nachweis geht so nie verloren.
pub fn einwilligung_setzen(conn: &Connection, c: &Consent, jetzt: i64) -> Res<()> {
    plan_laden(conn, &c.plan_id)?.ok_or("plan_nicht_gefunden")?;
    let Some(person) = identitaet(&c.person) else {
        return Err("person_ohne_identitaet".into());
    };
    if c.state != ConsentState::Offen && !crate::consent::mit_nachweis(c) {
        return Err("einwilligung_nachweis_fehlt".into());
    }
    conn.execute(
        "INSERT INTO meeting_plan_consent_log (plan_id, person, state, channel, recorded_at, logged_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            c.plan_id,
            person,
            c.state.as_str(),
            c.channel.map(|k| k.as_str()),
            c.recorded_at,
            jetzt,
        ],
    )
    .map_err(sql)?;
    conn.execute(
        "INSERT INTO meeting_plan_consents (plan_id, person, state, channel, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(plan_id, person) DO UPDATE SET state = excluded.state,
            channel = excluded.channel, recorded_at = excluded.recorded_at",
        params![
            c.plan_id,
            person,
            c.state.as_str(),
            c.channel.map(|k| k.as_str()),
            c.recorded_at,
        ],
    )
    .map_err(sql)?;
    Ok(())
}

/// Einwilligungen einer Planung, nach Person sortiert.
pub fn einwilligungen(conn: &Connection, plan_id: &str) -> Res<Vec<Consent>> {
    let mut st = conn
        .prepare(
            "SELECT plan_id, person, state, channel, recorded_at FROM meeting_plan_consents
             WHERE plan_id = ?1 ORDER BY person",
        )
        .map_err(sql)?;
    let rows = st
        .query_map([plan_id], |r| {
            let channel: Option<String> = r.get(3)?;
            let channel = match channel {
                None => None,
                Some(k) => Some(ConsentChannel::parse(&k).ok_or_else(|| kaputt(3))?),
            };
            Ok(Consent {
                plan_id: r.get(0)?,
                person: r.get(1)?,
                state: enum_spalte(r, 2, ConsentState::parse)?,
                channel,
                recorded_at: r.get(4)?,
            })
        })
        .map_err(sql)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(sql)
}

/// Verlauf der Einwilligungen einer Planung (aelteste zuerst), als Nachweis.
pub fn einwilligungs_verlauf(conn: &Connection, plan_id: &str) -> Res<Vec<(Consent, i64)>> {
    let mut st = conn
        .prepare(
            "SELECT plan_id, person, state, channel, recorded_at, logged_at
             FROM meeting_plan_consent_log WHERE plan_id = ?1 ORDER BY seq",
        )
        .map_err(sql)?;
    let rows = st
        .query_map([plan_id], |r| {
            let channel: Option<String> = r.get(3)?;
            let channel = match channel {
                None => None,
                Some(k) => Some(ConsentChannel::parse(&k).ok_or_else(|| kaputt(3))?),
            };
            Ok((
                Consent {
                    plan_id: r.get(0)?,
                    person: r.get(1)?,
                    state: enum_spalte(r, 2, ConsentState::parse)?,
                    channel,
                    recorded_at: r.get(4)?,
                },
                r.get(5)?,
            ))
        })
        .map_err(sql)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(sql)
}
