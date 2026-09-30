//! Freigabe-Anfragen („fragen“): ein Aufrufer will etwas tun, der Nutzer
//! entscheidet in der App (A1).
//!
//! Eigenschaften, auf die sich das Sicherheitsreview stuetzt:
//! - **Einmalig und gebunden**: eine Genehmigung gilt fuer genau eine
//!   Ausfuehrung (`consume` setzt `used`) und nur fuer denselben Aufrufer, dieselbe
//!   Integration, dieselbe Faehigkeit und dieselben Argumente (`args_hash`). Ein
//!   Agent kann mit einer genehmigten Mail nicht eine andere senden.
//! - **Befristet**: eine offene Anfrage verfaellt nach `TTL_MS`, eine
//!   genehmigte ebenfalls (ab Entscheidung).
//! - **Entscheidung ohne Rennen**: `decide` und `consume` sind je ein bedingtes
//!   UPDATE (`WHERE state = ...`): bei zwei gleichzeitigen Entscheidern gewinnt
//!   genau einer.
//! - **Begrenzt**: hoechstens `MAX_PENDING` offene Anfragen insgesamt und
//!   `MAX_PENDING_PER_SOURCE` je Aufrufer und Integration; ein Agent in der
//!   Schleife kann die Oberflaeche nicht fluten und den anderen nicht den Platz nehmen.
//! - **Keine Dubletten**: Fragt derselbe Aufrufer dieselbe Aktion (Integration,
//!   Faehigkeit, `args_hash`) erneut, solange die Freigabe offen ist, bekommt er
//!   dieselbe Freigabe zurueck (`open` meldet `reused`), keine neue.
//! - Die Vorschau (`args_preview`) ist geschwaerzt und wird NIE gekuerzt: passt sie
//!   nicht in `MAX_PREVIEW_CHARS`, wird die Anfrage abgelehnt (`PreviewTooLong`),
//!   statt dem Nutzer eine unvollstaendige Vorschau zur Freigabe vorzulegen. Die
//!   sinnvolle, gegliederte Kuerzung besorgt `preview::build` vor dem Anlegen.
//! - Pruefen, Zusammenfassen und Einfuegen sind EINE `IMMEDIATE`-Transaktion: zwei
//!   gleichzeitige gleiche Anfragen ergeben eine Zeile.

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};
use ulid::Ulid;

use super::audit::redact_text;
use super::model::{Approval, ApprovalState, IntegrationError};

/// Gueltigkeit einer offenen bzw. genehmigten Anfrage: eine Stunde.
pub const TTL_MS: i64 = 60 * 60 * 1000;
/// Hoechstzahl gleichzeitig offener Anfragen.
pub const MAX_PENDING: i64 = 50;
/// Hoechstzahl offener Anfragen je Aufrufer und Integration.
pub const MAX_PENDING_PER_SOURCE: i64 = 10;
/// Laengste Vorschau in Zeichen. `preview::build` bleibt im schlechtesten Fall
/// darunter; was darueber liegt, wird abgelehnt und nie abgeschnitten.
pub const MAX_PREVIEW_CHARS: usize = 4000;

/// Warum `decide`/`consume` nichts getan hat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalError {
    NotFound,
    /// Steht nicht (mehr) auf dem noetigen Zustand.
    WrongState(ApprovalState),
    Expired,
    /// Aufrufer, Integration, Faehigkeit oder Argumente stimmen nicht.
    Mismatch,
    Store(String),
}

impl std::fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApprovalError::NotFound => write!(f, "Freigabe nicht gefunden."),
            ApprovalError::WrongState(s) => {
                write!(f, "Freigabe nicht mehr offen (Zustand: {}).", s.as_str())
            }
            ApprovalError::Expired => write!(f, "Die Freigabe ist abgelaufen."),
            ApprovalError::Mismatch => {
                write!(f, "Die Freigabe gehört zu einer anderen Aktion.")
            }
            ApprovalError::Store(m) => write!(f, "Speicherfehler: {m}"),
        }
    }
}

impl std::error::Error for ApprovalError {}

impl From<rusqlite::Error> for ApprovalError {
    fn from(e: rusqlite::Error) -> Self {
        ApprovalError::Store(e.to_string())
    }
}

/// Was zum Anlegen noetig ist.
#[derive(Clone, Debug)]
pub struct NewApproval<'a> {
    pub caller: &'a str,
    pub integration_id: Option<&'a str>,
    pub capability: &'a str,
    pub args_preview: Option<&'a str>,
    pub args_hash: Option<&'a str>,
}

/// Bindung einer Genehmigung an eine Ausfuehrung.
#[derive(Clone, Debug)]
pub struct Expect<'a> {
    pub caller: &'a str,
    pub integration_id: Option<&'a str>,
    pub capability: &'a str,
    pub args_hash: Option<&'a str>,
}

/// SHA-256 (hex) ueber Integration, Faehigkeit, Ziel und Argumente. Was der
/// Nutzer genehmigt, ist genau das, was spaeter ausgefuehrt wird.
pub fn args_hash(integration_id: &str, capability: &str, target: &str, args_json: &str) -> String {
    let mut h = Sha256::new();
    for part in [integration_id, capability, target, args_json] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn map_approval(r: &rusqlite::Row<'_>) -> rusqlite::Result<Approval> {
    let state: String = r.get("state")?;
    Ok(Approval {
        id: r.get("id")?,
        created_at: r.get("created_at")?,
        caller: r.get("caller")?,
        integration_id: r.get("integration_id")?,
        tool_or_capability: r.get("tool_or_capability")?,
        args_preview: r.get("args_preview")?,
        state: ApprovalState::parse(&state).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                format!("unbekannter Zustand {state}").into(),
            )
        })?,
        decided_at: r.get("decided_at")?,
    })
}

const SELECT: &str = "SELECT id, created_at, caller, integration_id, tool_or_capability,
                             args_preview, state, decided_at FROM approvals";

/// Ergebnis von `open`: die Freigabe und ob sie schon offen war.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    pub approval: Approval,
    /// `true`: dieselbe Anfrage war noch offen und wurde wiederverwendet.
    pub reused: bool,
}

/// Warum `open` keine Freigabe angelegt hat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Insgesamt `MAX_PENDING` offene Anfragen.
    TooManyPending,
    /// `MAX_PENDING_PER_SOURCE` offene Anfragen dieses Aufrufers an dieser Integration.
    TooManyForSource,
    /// Die Vorschau ist laenger als `MAX_PREVIEW_CHARS`.
    PreviewTooLong,
    Store(IntegrationError),
}

impl OpenError {
    /// Maschinenlesbarer Grund (Audit, Antwort an den Aufrufer).
    pub fn code(&self) -> &'static str {
        match self {
            OpenError::TooManyPending => "too_many_pending",
            OpenError::TooManyForSource => "too_many_pending_caller",
            OpenError::PreviewTooLong => "preview_unsafe",
            OpenError::Store(_) => "store_error",
        }
    }
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::TooManyPending => {
                write!(f, "Zu viele offene Freigaben: bitte erst entscheiden.")
            }
            OpenError::TooManyForSource => write!(
                f,
                "Zu viele offene Freigaben dieses Zugangs für diese Integration: bitte erst entscheiden."
            ),
            OpenError::PreviewTooLong => write!(
                f,
                "Die Anfrage lässt sich nicht vollständig zur Freigabe anzeigen und wurde abgelehnt."
            ),
            OpenError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<rusqlite::Error> for OpenError {
    fn from(e: rusqlite::Error) -> Self {
        OpenError::Store(e.into())
    }
}

impl From<OpenError> for IntegrationError {
    fn from(e: OpenError) -> Self {
        match e {
            OpenError::Store(inner) => inner,
            other => IntegrationError::Invalid(other.to_string()),
        }
    }
}

/// Legt eine Freigabe-Anfrage an oder gibt die noch offene gleiche zurueck
/// (siehe Moduldoku: Grenzen, Dubletten, Vorschau).
pub fn open(conn: &Connection, a: &NewApproval<'_>, now_ms: i64) -> Result<Opened, OpenError> {
    // Die Vorschau wird geschwaerzt und NICHT gekuerzt; ist sie zu lang, gar nicht anlegen.
    let preview = a.args_preview.map(redact_text);
    if preview
        .as_deref()
        .is_some_and(|p| p.chars().count() > MAX_PREVIEW_CHARS)
    {
        return Err(OpenError::PreviewTooLong);
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    // Zuerst Verfallenes wegraeumen, damit alte Anfragen den Platz nicht blockieren.
    expire_stale(&tx, now_ms).map_err(OpenError::Store)?;
    if let Some(hash) = a.args_hash {
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM approvals
                 WHERE state = 'pending' AND created_at > ?1
                   AND caller = ?2 AND COALESCE(integration_id, '') = COALESCE(?3, '')
                   AND tool_or_capability = ?4 AND args_hash = ?5
                 ORDER BY created_at, id LIMIT 1",
                params![
                    now_ms - TTL_MS,
                    a.caller,
                    a.integration_id,
                    a.capability,
                    hash
                ],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            let approval = get(&tx, &id)
                .map_err(OpenError::Store)?
                .ok_or_else(|| OpenError::Store(IntegrationError::NotFound(id)))?;
            tx.commit()?;
            return Ok(Opened {
                approval,
                reused: true,
            });
        }
    }
    let pending: i64 = tx.query_row(
        "SELECT COUNT(*) FROM approvals WHERE state = 'pending'",
        [],
        |r| r.get(0),
    )?;
    if pending >= MAX_PENDING {
        return Err(OpenError::TooManyPending);
    }
    let from_source: i64 = tx.query_row(
        "SELECT COUNT(*) FROM approvals
         WHERE state = 'pending' AND caller = ?1
           AND COALESCE(integration_id, '') = COALESCE(?2, '')",
        params![a.caller, a.integration_id],
        |r| r.get(0),
    )?;
    if from_source >= MAX_PENDING_PER_SOURCE {
        return Err(OpenError::TooManyForSource);
    }
    let id = Ulid::new().to_string();
    tx.execute(
        "INSERT INTO approvals (id, created_at, caller, integration_id, tool_or_capability,
                                args_preview, args_hash, state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending')",
        params![
            id,
            now_ms,
            a.caller,
            a.integration_id,
            a.capability,
            preview,
            a.args_hash,
        ],
    )?;
    let approval = get(&tx, &id).map_err(OpenError::Store)?.ok_or_else(|| {
        OpenError::Store(IntegrationError::Store("Freigabe nicht lesbar".to_string()))
    })?;
    tx.commit()?;
    Ok(Opened {
        approval,
        reused: false,
    })
}

/// Wie `open`, gibt aber nur die Freigabe zurueck (neu oder wiederverwendet).
pub fn create(
    conn: &Connection,
    a: &NewApproval<'_>,
    now_ms: i64,
) -> Result<Approval, IntegrationError> {
    Ok(open(conn, a, now_ms)?.approval)
}

/// Nimmt eine eben angelegte, noch offene Anfrage zurueck (z. B. wenn ihr
/// Audit-Eintrag nicht geschrieben werden konnte): sie gilt als verfallen.
pub fn withdraw(conn: &Connection, id: &str, now_ms: i64) -> Result<(), IntegrationError> {
    conn.execute(
        "UPDATE approvals SET state = 'expired', decided_at = ?2
         WHERE id = ?1 AND state = 'pending'",
        params![id, now_ms],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Approval>, IntegrationError> {
    Ok(conn
        .query_row(
            &format!("{SELECT} WHERE id = ?1"),
            params![id],
            map_approval,
        )
        .optional()?)
}

/// Offene Anfragen (nicht verfallen), aelteste zuerst.
pub fn list_pending(conn: &Connection, now_ms: i64) -> Result<Vec<Approval>, IntegrationError> {
    let mut stmt = conn.prepare(&format!(
        "{SELECT} WHERE state = 'pending' AND created_at > ?1 ORDER BY created_at, id"
    ))?;
    let rows = stmt
        .query_map(params![now_ms - TTL_MS], map_approval)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Setzt verfallene offene und genehmigte Anfragen auf `expired`. Gibt die Zahl zurueck.
pub fn expire_stale(conn: &Connection, now_ms: i64) -> Result<usize, IntegrationError> {
    let cutoff = now_ms - TTL_MS;
    let n = conn.execute(
        "UPDATE approvals SET state = 'expired', decided_at = COALESCE(decided_at, ?1)
         WHERE (state = 'pending' AND created_at <= ?2)
            OR (state = 'approved' AND COALESCE(decided_at, created_at) <= ?2)",
        params![now_ms, cutoff],
    )?;
    Ok(n)
}

fn diagnose(conn: &Connection, id: &str, now_ms: i64) -> ApprovalError {
    match get(conn, id) {
        Ok(None) => ApprovalError::NotFound,
        Ok(Some(a)) => match a.state {
            ApprovalState::Expired => ApprovalError::Expired,
            ApprovalState::Pending if a.created_at <= now_ms - TTL_MS => ApprovalError::Expired,
            state => ApprovalError::WrongState(state),
        },
        Err(e) => ApprovalError::Store(e.to_string()),
    }
}

/// Der Nutzer entscheidet eine offene Anfrage. Bei zwei gleichzeitigen
/// Entscheidern gewinnt genau einer; der andere bekommt `WrongState`.
pub fn decide(
    conn: &Connection,
    id: &str,
    approve: bool,
    now_ms: i64,
) -> Result<Approval, ApprovalError> {
    let target = if approve {
        ApprovalState::Approved
    } else {
        ApprovalState::Denied
    };
    let changed = conn.execute(
        "UPDATE approvals SET state = ?2, decided_at = ?3
         WHERE id = ?1 AND state = 'pending' AND created_at > ?4",
        params![id, target.as_str(), now_ms, now_ms - TTL_MS],
    )?;
    if changed == 0 {
        // Verfallene Anfrage nicht als „offen“ stehen lassen.
        let _ = expire_stale(conn, now_ms);
        return Err(diagnose(conn, id, now_ms));
    }
    get(conn, id)
        .map_err(|e| ApprovalError::Store(e.to_string()))?
        .ok_or(ApprovalError::NotFound)
}

/// Loest eine genehmigte Anfrage ein (einmalig). Stimmen Aufrufer,
/// Integration, Faehigkeit oder Argumente nicht, bleibt sie genehmigt, und der
/// Aufruf scheitert mit `Mismatch`.
pub fn consume(
    conn: &Connection,
    id: &str,
    expect: &Expect<'_>,
    now_ms: i64,
) -> Result<(), ApprovalError> {
    let changed = conn.execute(
        "UPDATE approvals SET state = 'used'
         WHERE id = ?1 AND state = 'approved'
           AND caller = ?2
           AND COALESCE(integration_id, '') = COALESCE(?3, '')
           AND tool_or_capability = ?4
           AND COALESCE(args_hash, '') = COALESCE(?5, '')
           AND COALESCE(decided_at, created_at) > ?6",
        params![
            id,
            expect.caller,
            expect.integration_id,
            expect.capability,
            expect.args_hash,
            now_ms - TTL_MS
        ],
    )?;
    if changed == 1 {
        return Ok(());
    }
    // Warum nicht? Ein genehmigter, aber nicht passender Eintrag ist `Mismatch`.
    let Some(a) = get(conn, id).map_err(|e| ApprovalError::Store(e.to_string()))? else {
        return Err(ApprovalError::NotFound);
    };
    if a.state == ApprovalState::Approved {
        let cutoff = now_ms - TTL_MS;
        if a.decided_at.unwrap_or(a.created_at) <= cutoff {
            let _ = expire_stale(conn, now_ms);
            return Err(ApprovalError::Expired);
        }
        return Err(ApprovalError::Mismatch);
    }
    Err(diagnose(conn, id, now_ms))
}

#[cfg(test)]
mod tests;
