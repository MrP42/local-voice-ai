//! Protokoll ohne Inhalte (ADR-0026 Abschnitt 6).
//!
//! Ein Ereignis traegt Aktion, Objektkennung (Lauf, Projekt), Status, einen Zaehlercode und die Dauer.
//! Nie Frage, Suchanfragen, Antworten, Quellen, Titel oder Adressen.

use crate::managers::integrations::audit as register;
use crate::managers::integrations::model::{AuditOutcome, NewAudit};
use crate::managers::meetings::store::MeetingStore;
use std::sync::Arc;

/// Ein Ereignis. `code` ist ein Zaehlercode wie `q2.s2.f2.k1.personenbezogen.w1`, kein Text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEreignis {
    pub aktion: String,
    pub objekt: String,
    pub status: String,
    pub code: String,
    pub dauer_ms: u64,
}

pub trait AuditSenke: Send + Sync {
    fn aufzeichnen(&self, e: AuditEreignis);
}

/// Faehigkeit im Audit-Log des Besprechungsspeichers.
pub const FAEHIGKEIT: &str = "recherche";
/// Aufrufer im Audit-Log.
pub const AUFRUFER: &str = "projekte";

/// Produktion: das Audit-Log der App (Tabelle `audit_log` im Besprechungsspeicher).
pub struct RegisterAudit(pub Arc<MeetingStore>);

fn ergebnis(status: &str) -> AuditOutcome {
    match status {
        "abgelehnt" | "abgelaufen" => AuditOutcome::Denied,
        "fehler" => AuditOutcome::Error,
        "freigabe_offen" => AuditOutcome::Pending,
        _ => AuditOutcome::Ok,
    }
}

impl AuditSenke for RegisterAudit {
    fn aufzeichnen(&self, e: AuditEreignis) {
        let Ok(conn) = self.0.get_connection() else {
            return;
        };
        let eintrag = NewAudit {
            caller: AUFRUFER.into(),
            integration_id: None,
            capability: Some(FAEHIGKEIT.into()),
            target: Some(e.objekt),
            outcome: ergebnis(&e.status),
            detail: Some(serde_json::json!({
                "aktion": e.aktion,
                "status": e.status,
                "code": e.code,
                "dauer_ms": e.dauer_ms,
            })),
        };
        let _ = register::record(&conn, &eintrag);
    }
}
