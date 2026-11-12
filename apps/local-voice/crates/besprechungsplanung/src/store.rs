//! Speicher der Besprechungsplanung (SQLite, Konzept 3.2).
//!
//! Nur `CREATE TABLE`/`CREATE INDEX`, wie im Repo ueblich. Die App haengt
//! [`MIGRATIONS`] an die Migrationskette ihrer Besprechungs-Datenbank an (jede
//! Zeile ein eigener Schritt; nie umsortieren, nur anhaengen). Die Funktionen
//! arbeiten auf einer geliehenen Verbindung; Transaktionen steuert die App.
//!
//! Fehlercodes (Text des Fehlers): `plan_id_ungueltig`, `plan_nicht_gefunden`,
//! `plan_abgeschlossen`, `daten_ungueltig`.

use rusqlite::{params, Connection, OptionalExtension, Row};

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
        sync_state TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_meeting_plan_links_remote ON meeting_plan_links(remote_id);
    CREATE INDEX IF NOT EXISTS idx_meeting_plan_links_uid ON meeting_plan_links(ical_uid);
    CREATE TABLE IF NOT EXISTS meeting_plan_consents (
        plan_id TEXT NOT NULL REFERENCES meeting_plans(id) ON DELETE CASCADE,
        person TEXT NOT NULL,
        state TEXT NOT NULL,
        channel TEXT,
        recorded_at INTEGER,
        PRIMARY KEY (plan_id, person)
    );"];

/// Wendet [`MIGRATIONS`] direkt an (Tests, eigenstaendige Nutzung).
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
    format!("sql: {e}")
}

fn json<T: serde::Serialize>(v: &T) -> Res<String> {
    serde_json::to_string(v).map_err(|_| "daten_ungueltig".to_string())
}

/// Legt eine Planung im Status `entwurf` an.
pub fn plan_anlegen(conn: &Connection, id: &str, input: &PlanInput, jetzt: i64) -> Res<Plan> {
    if !gueltige_id(id) {
        return Err("plan_id_ungueltig".into());
    }
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

/// Aendert die Inhalte einer Planung. Nach dem Versand des Protokolls nicht mehr.
pub fn plan_aendern(conn: &Connection, id: &str, input: &PlanInput, jetzt: i64) -> Res<Plan> {
    let alt = plan_laden(conn, id)?.ok_or("plan_nicht_gefunden")?;
    if alt.status == PlanStatus::Versendet {
        return Err("plan_abgeschlossen".into());
    }
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

/// Setzt den Status.
pub fn plan_status_setzen(conn: &Connection, id: &str, status: PlanStatus, jetzt: i64) -> Res<()> {
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
    fn j<T: serde::de::DeserializeOwned + Default>(s: String) -> T {
        serde_json::from_str(&s).unwrap_or_default()
    }
    let goal: String = r.get(3)?;
    let status: String = r.get(14)?;
    Ok(Plan {
        id: r.get(0)?,
        meeting_id: r.get(1)?,
        title: r.get(2)?,
        goal_type: GoalType::parse(&goal).unwrap_or(GoalType::Klaeren),
        goal_text: r.get(4)?,
        expected_result: r.get(5)?,
        duration_min: r.get(6)?,
        classification: r.get(7)?,
        agenda: j(r.get(8)?),
        roles: j(r.get(9)?),
        materials: j(r.get(10)?),
        hints: j(r.get(11)?),
        transcription_planned: r.get(12)?,
        minutes_template_id: r.get(13)?,
        status: PlanStatus::parse(&status).unwrap_or(PlanStatus::Entwurf),
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

/// Speichert eine Verknuepfung (ersetzt die bisherige der Planung).
pub fn link_speichern(conn: &Connection, l: &PlanLink) -> Res<()> {
    conn.execute(
        "INSERT INTO meeting_plan_links (plan_id, adapter, source_id, remote_id, ical_uid,
            change_key, starts_at, ends_at, location, join_url, body_hash, last_seen_at,
            last_change_at, sync_state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(plan_id) DO UPDATE SET adapter = excluded.adapter,
            source_id = excluded.source_id, remote_id = excluded.remote_id,
            ical_uid = excluded.ical_uid, change_key = excluded.change_key,
            starts_at = excluded.starts_at, ends_at = excluded.ends_at,
            location = excluded.location, join_url = excluded.join_url,
            body_hash = excluded.body_hash, last_seen_at = excluded.last_seen_at,
            last_change_at = excluded.last_change_at, sync_state = excluded.sync_state",
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
        ],
    )
    .map_err(sql)?;
    Ok(())
}

fn link_aus_zeile(r: &Row) -> rusqlite::Result<PlanLink> {
    let adapter: String = r.get(1)?;
    let state: String = r.get(13)?;
    Ok(PlanLink {
        plan_id: r.get(0)?,
        adapter: AdapterKind::parse(&adapter).unwrap_or(AdapterKind::IcsMarker),
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
        sync_state: SyncState::parse(&state).unwrap_or(SyncState::Verwaist),
    })
}

const LINK_SPALTEN: &str = "plan_id, adapter, source_id, remote_id, ical_uid, change_key,
    starts_at, ends_at, location, join_url, body_hash, last_seen_at, last_change_at, sync_state";

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

/// Setzt eine Einwilligung (legt sie an oder ersetzt sie).
pub fn einwilligung_setzen(conn: &Connection, c: &Consent) -> Res<()> {
    conn.execute(
        "INSERT INTO meeting_plan_consents (plan_id, person, state, channel, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(plan_id, person) DO UPDATE SET state = excluded.state,
            channel = excluded.channel, recorded_at = excluded.recorded_at",
        params![
            c.plan_id,
            c.person,
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
            let state: String = r.get(2)?;
            let channel: Option<String> = r.get(3)?;
            Ok(Consent {
                plan_id: r.get(0)?,
                person: r.get(1)?,
                state: ConsentState::parse(&state).unwrap_or(ConsentState::Offen),
                channel: channel.as_deref().and_then(ConsentChannel::parse),
                recorded_at: r.get(4)?,
            })
        })
        .map_err(sql)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(sql)
}
