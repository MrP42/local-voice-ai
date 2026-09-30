//! Lesen und Schreiben des Registers und seiner Rechte (A1).
//!
//! Jede Funktion arbeitet auf einer `&Connection` (Tests und Aufrufer holen sie
//! aus `MeetingStore::get_connection`). Mehrschrittige Schreibwege sind EINE
//! `IMMEDIATE`-Transaktion: Abbruch oder volle Platte lassen den alten Stand.

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value;
use ulid::Ulid;

use super::audit::{is_secret_key, sanitize_audit_text, sanitize_text};
use super::grants::{effective_mode, GrantSet};
use super::model::{
    Caller, Capability, Direction, GrantMode, GrantRow, Integration, IntegrationError,
    IntegrationPatch, Kind, NewIntegration,
};

pub const MAX_LABEL_CHARS: usize = 120;
pub const MAX_CONFIG_BYTES: usize = 64 * 1024;
pub const MAX_ERROR_CHARS: usize = 500;
pub const MAX_ID_LEN: usize = 40;

fn invalid<T>(msg: &str) -> Result<T, IntegrationError> {
    Err(IntegrationError::Invalid(msg.to_string()))
}

/// Erlaubte Kennung: Buchstaben, Ziffern, `-` und `_` (auch Dateiname des Geheimnisses).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn looks_like_secret_value(s: &str) -> bool {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("bearer ") || lower.starts_with("basic ") || t.starts_with("eyJ") {
        return true;
    }
    // Adresse mit Zugangsdaten: `scheme://user:pw@host`.
    if let Some(pos) = t.find("://") {
        let rest = &t[pos + 3..];
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority.contains('@') {
            return true;
        }
    }
    false
}

fn check_config_value(v: &Value, depth: usize) -> Result<(), IntegrationError> {
    if depth > 8 {
        return invalid("Die Konfiguration ist zu tief verschachtelt.");
    }
    match v {
        Value::Object(map) => {
            for (key, val) in map {
                if is_secret_key(key) {
                    return Err(IntegrationError::Invalid(format!(
                        "Die Konfiguration enthält das Feld „{key}“: Geheimnisse gehören in den Geheimnisspeicher, nicht in die Konfiguration."
                    )));
                }
                check_config_value(val, depth + 1)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(|x| check_config_value(x, depth + 1)),
        Value::String(s) if looks_like_secret_value(s) => invalid(
            "Die Konfiguration enthält einen Wert, der wie ein Zugangsschlüssel aussieht: Geheimnisse gehören in den Geheimnisspeicher.",
        ),
        _ => Ok(()),
    }
}

/// Konfiguration pruefen: ein JSON-Objekt, hoechstens `MAX_CONFIG_BYTES`, ohne
/// Geheimnis-Felder (`password`, `token`, ...) und ohne Werte, die wie Zugangsdaten
/// aussehen (Bearer-Token, JWT, Adresse mit `user:pw@`).
pub fn validate_config(v: &Value) -> Result<(), IntegrationError> {
    if !v.is_object() {
        return invalid("Die Konfiguration muss ein JSON-Objekt sein.");
    }
    if v.to_string().len() > MAX_CONFIG_BYTES {
        return invalid("Die Konfiguration ist zu groß.");
    }
    check_config_value(v, 0)
}

fn validate_label(label: &str) -> Result<String, IntegrationError> {
    let t = label.trim();
    if t.is_empty() {
        return invalid("Der Name darf nicht leer sein.");
    }
    if t.chars().count() > MAX_LABEL_CHARS {
        return invalid("Der Name ist zu lang (höchstens 120 Zeichen).");
    }
    Ok(t.to_string())
}

fn map_integration(r: &rusqlite::Row<'_>) -> rusqlite::Result<Integration> {
    let kind: String = r.get("kind")?;
    let direction: String = r.get("direction")?;
    let bad = |col: &str, val: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unbekannter Wert in {col}: {val}").into(),
        )
    };
    Ok(Integration {
        id: r.get("id")?,
        kind: Kind::parse(&kind).ok_or_else(|| bad("kind", &kind))?,
        label: r.get("label")?,
        enabled: r.get::<_, i64>("enabled")? != 0,
        direction: Direction::parse(&direction).ok_or_else(|| bad("direction", &direction))?,
        config_json: r.get("config_json")?,
        account_hint: r.get("account_hint")?,
        data_class: r.get("data_class")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        last_ok_at: r.get("last_ok_at")?,
        last_error: r.get("last_error")?,
    })
}

const SELECT: &str = "SELECT id, kind, label, enabled, direction, config_json, account_hint,
                             data_class, created_at, updated_at, last_ok_at, last_error
                      FROM integrations";

pub fn get(conn: &Connection, id: &str) -> Result<Option<Integration>, IntegrationError> {
    Ok(conn
        .query_row(
            &format!("{SELECT} WHERE id = ?1"),
            params![id],
            map_integration,
        )
        .optional()?)
}

/// Alle Integrationen, aelteste zuerst. Eine Zeile mit unbekannter Art oder
/// Richtung (neuere Version, kaputte Zeile) wird uebersprungen und protokolliert;
/// sie laesst nicht die ganze Liste scheitern. Andere Fehler (Datenbank) brechen ab.
pub fn list(conn: &Connection) -> Result<Vec<Integration>, IntegrationError> {
    let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY created_at, id"))?;
    let mut out = Vec::new();
    for row in stmt.query_map([], map_integration)? {
        match row {
            Ok(i) => out.push(i),
            Err(rusqlite::Error::FromSqlConversionFailure(_, _, cause)) => {
                log::warn!("integrations: Zeile des Registers uebersprungen: {cause}");
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

/// Legt eine Integration an. Kalenderarten (`ics`, `graph`) legt der Kalender
/// an und das Register spiegelt sie; hier sind sie gesperrt.
pub fn create(
    conn: &Connection,
    n: &NewIntegration,
    now_ms: i64,
) -> Result<Integration, IntegrationError> {
    if n.kind.is_calendar_managed() {
        return Err(IntegrationError::Managed(
            "Kalenderquellen werden im Kalender angelegt; das Register übernimmt sie automatisch."
                .to_string(),
        ));
    }
    let label = validate_label(&n.label)?;
    let direction = n.direction.unwrap_or_else(|| n.kind.default_direction());
    if !n.kind.allowed_directions().contains(&direction) {
        return invalid("Diese Art von Integration kennt die gewählte Richtung nicht.");
    }
    validate_config(&n.config)?;
    let id = match &n.id {
        Some(id) if valid_id(id) => id.clone(),
        Some(_) => return invalid("Ungültige Kennung."),
        None => Ulid::new().to_string(),
    };
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    // Die Kennungen des Kalenders gehoeren dem Kalender, auch die entfernter Quellen:
    // deren Zeile steht in `calendar_sources` weiter, und die Spiegel-Trigger wuerden
    // sonst an einem fremden Eintrag arbeiten.
    let taken_by_calendar: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM calendar_sources WHERE id = ?1)",
        params![id],
        |r| r.get(0),
    )?;
    if taken_by_calendar {
        return invalid("Diese Kennung ist schon vergeben.");
    }
    tx.execute(
        "INSERT INTO integrations (id, kind, label, enabled, direction, config_json, account_hint,
                                   data_class, created_at, updated_at)
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            id,
            n.kind.as_str(),
            label,
            direction.as_str(),
            n.config.to_string(),
            n.account_hint
                .as_deref()
                .map(|h| sanitize_text(h, MAX_LABEL_CHARS)),
            n.data_class,
            now_ms
        ],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            IntegrationError::Invalid("Diese Kennung ist schon vergeben.".to_string())
        }
        other => other.into(),
    })?;
    tx.commit()?;
    get(conn, &id)?.ok_or_else(|| IntegrationError::NotFound(id))
}

/// Aendert eine Integration. Bei Kalenderarten sind Name, Schalter und Entfernen
/// Sache des Kalenders (`Managed`); Richtung, Konfiguration und Datenklasse
/// gehoeren dem Register.
pub fn update(
    conn: &Connection,
    id: &str,
    patch: &IntegrationPatch,
    now_ms: i64,
) -> Result<Integration, IntegrationError> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let Some(current) = get(&tx, id)? else {
        return Err(IntegrationError::NotFound(id.to_string()));
    };
    if current.kind.is_calendar_managed() && (patch.label.is_some() || patch.enabled.is_some()) {
        return Err(IntegrationError::Managed(
            "Name und Schalter einer Kalenderquelle ändert man im Kalender.".to_string(),
        ));
    }
    let label = match &patch.label {
        Some(l) => validate_label(l)?,
        None => current.label.clone(),
    };
    let direction = patch.direction.unwrap_or(current.direction);
    if !current.kind.allowed_directions().contains(&direction) {
        return invalid("Diese Art von Integration kennt die gewählte Richtung nicht.");
    }
    let config_json = match &patch.config {
        Some(c) => {
            validate_config(c)?;
            c.to_string()
        }
        None => current.config_json.clone(),
    };
    let data_class = match &patch.data_class {
        Some(d) => d.clone(),
        None => current.data_class.clone(),
    };
    tx.execute(
        "UPDATE integrations SET label = ?2, enabled = ?3, direction = ?4, config_json = ?5,
                data_class = ?6, updated_at = ?7 WHERE id = ?1",
        params![
            id,
            label,
            i64::from(patch.enabled.unwrap_or(current.enabled)),
            direction.as_str(),
            config_json,
            data_class,
            now_ms
        ],
    )?;
    tx.commit()?;
    get(conn, id)?.ok_or_else(|| IntegrationError::NotFound(id.to_string()))
}

/// Entfernt eine Integration samt ihrer Rechte (eine Transaktion). Das Audit-Log
/// behaelt seine Eintraege. Geheimnisse raeumt der Aufrufer ueber
/// `secrets::delete_all` weg. Kalenderquellen nur im Kalender.
pub fn delete(conn: &Connection, id: &str) -> Result<(), IntegrationError> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let Some(current) = get(&tx, id)? else {
        return Err(IntegrationError::NotFound(id.to_string()));
    };
    if current.kind.is_calendar_managed() {
        return Err(IntegrationError::Managed(
            "Eine Kalenderquelle entfernt man im Kalender.".to_string(),
        ));
    }
    tx.execute(
        "DELETE FROM integration_grants WHERE integration_id = ?1",
        params![id],
    )?;
    tx.execute("DELETE FROM integrations WHERE id = ?1", params![id])?;
    tx.commit()?;
    Ok(())
}

/// Letzter Erfolg (loescht den Fehler).
pub fn mark_ok(conn: &Connection, id: &str, now_ms: i64) -> Result<(), IntegrationError> {
    conn.execute(
        "UPDATE integrations SET last_ok_at = ?2, last_error = NULL, updated_at = ?2 WHERE id = ?1",
        params![id, now_ms],
    )?;
    Ok(())
}

/// Letzter Fehler als Klartext (geschwaerzt, gekuerzt).
pub fn mark_error(
    conn: &Connection,
    id: &str,
    error: &str,
    now_ms: i64,
) -> Result<(), IntegrationError> {
    conn.execute(
        "UPDATE integrations SET last_error = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, sanitize_audit_text(error, MAX_ERROR_CHARS), now_ms],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Rechte
// ---------------------------------------------------------------------------

/// Gespeicherte Rechte einer Integration (ohne Vorgaben).
pub fn grants_for(conn: &Connection, integration_id: &str) -> Result<GrantSet, IntegrationError> {
    let mut set = GrantSet::new();
    for row in list_grants(conn, integration_id)? {
        set.insert((row.capability, row.caller), row.mode);
    }
    Ok(set)
}

pub fn list_grants(
    conn: &Connection,
    integration_id: &str,
) -> Result<Vec<GrantRow>, IntegrationError> {
    let mut stmt = conn.prepare(
        "SELECT capability, caller, mode FROM integration_grants
         WHERE integration_id = ?1 ORDER BY capability, caller",
    )?;
    let rows = stmt
        .query_map(params![integration_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Unbekannte Werte (neuere Version, kaputte Zeile) werden uebersprungen statt
    // geraten: fehlt die Zeile, gilt die strenge Vorgabe.
    Ok(rows
        .into_iter()
        .filter_map(|(cap, caller, mode)| {
            Some(GrantRow {
                integration_id: integration_id.to_string(),
                capability: Capability::parse(&cap)?,
                caller: Caller::parse(&caller)?,
                mode: GrantMode::parse(&mode)?,
            })
        })
        .collect())
}

/// Setzt ein Recht. Nur fuer Aufrufer ohne Nutzer, nur fuer Faehigkeiten, die
/// die Art anbietet; `recording.start` nie auf `allow`.
pub fn set_grant(
    conn: &Connection,
    integration_id: &str,
    cap: Capability,
    caller: Caller,
    mode: GrantMode,
) -> Result<(), IntegrationError> {
    let Some(i) = get(conn, integration_id)? else {
        return Err(IntegrationError::NotFound(integration_id.to_string()));
    };
    if caller == Caller::User {
        return invalid(
            "Für den Nutzer in der Oberfläche gibt es keine Rechte: er braucht keine Freigabe.",
        );
    }
    if !i.kind.capabilities().contains(&cap) {
        return invalid("Diese Integration bietet die Fähigkeit nicht an.");
    }
    if cap.never_allow() && mode == GrantMode::Allow {
        return invalid(
            "Aufnahme starten kann nie dauerhaft erlaubt werden: die App zeigt vor jeder Aufnahme den Einwilligungsdialog.",
        );
    }
    conn.execute(
        "INSERT INTO integration_grants (integration_id, capability, caller, mode)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(integration_id, capability, caller) DO UPDATE SET mode = excluded.mode",
        params![integration_id, cap.as_str(), caller.as_str(), mode.as_str()],
    )?;
    Ok(())
}

/// Entfernt die gespeicherte Zeile (zurueck auf die Vorgabe). `true`, wenn es eine gab.
pub fn clear_grant(
    conn: &Connection,
    integration_id: &str,
    cap: Capability,
    caller: Caller,
) -> Result<bool, IntegrationError> {
    let n = conn.execute(
        "DELETE FROM integration_grants WHERE integration_id = ?1 AND capability = ?2 AND caller = ?3",
        params![integration_id, cap.as_str(), caller.as_str()],
    )?;
    Ok(n > 0)
}

/// Wirksamer Modus aus der Datenbank (Integration + Rechte geladen).
pub fn effective(
    conn: &Connection,
    integration_id: &str,
    cap: Capability,
    caller: Caller,
    tool_mode: Option<GrantMode>,
) -> Result<GrantMode, IntegrationError> {
    let Some(i) = get(conn, integration_id)? else {
        return Ok(GrantMode::Off);
    };
    let grants = grants_for(conn, integration_id)?;
    Ok(effective_mode(&i, cap, caller, &grants, tool_mode))
}

#[cfg(test)]
mod tests;
