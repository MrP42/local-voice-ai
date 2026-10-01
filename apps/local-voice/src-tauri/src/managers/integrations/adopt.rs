//! Uebernahme der Kalenderquellen ins Register (A1, R6).
//!
//! `calendar_sources` bleibt die fuehrende Tabelle. Migration Index 5 fuellt das
//! Register einmal aus ihr und haengt Trigger an, die jede spaetere Aenderung in
//! derselben Transaktion spiegeln. `reconcile` ist das Sicherheitsnetz beim
//! Oeffnen: es ergaenzt Fehlendes, gleicht Abweichungen an und entfernt Waisen --
//! idempotent, sodass ein zweiter Start nichts mehr aendert und nie Dubletten
//! anlegt (die ID ist der Primaerschluessel und zugleich der Name des
//! Geheimnisses, `secret::Namespace::Calendar`).

use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::model::IntegrationError;

/// Was `reconcile` geaendert hat. Nach einem sauberen Start ist alles `0`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdoptReport {
    pub inserted: usize,
    pub updated: usize,
    pub removed: usize,
}

impl AdoptReport {
    pub fn changed(&self) -> usize {
        self.inserted + self.updated + self.removed
    }
}

/// Gleicht die Kalender-Integrationen an `calendar_sources` an (eine
/// `IMMEDIATE`-Transaktion; zwei Prozesse, die gleichzeitig oeffnen, laufen
/// nacheinander und der zweite findet nichts mehr zu tun).
pub fn reconcile(conn: &Connection) -> Result<AdoptReport, IntegrationError> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let inserted = tx.execute(
        "INSERT INTO integrations (id, kind, label, enabled, direction, config_json, account_hint,
                                   data_class, created_at, updated_at, last_ok_at, last_error)
         SELECT s.id, s.kind, s.label, s.enabled, 'read', '{}', s.account_hint, NULL,
                s.created_at, s.updated_at, s.last_ok_at, s.last_error
         FROM calendar_sources s
         WHERE s.deleted_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM integrations i WHERE i.id = s.id)",
        [],
    )?;
    let updated = tx.execute(
        "UPDATE integrations SET label = s.label, enabled = s.enabled,
                account_hint = s.account_hint, last_ok_at = s.last_ok_at,
                last_error = s.last_error, updated_at = s.updated_at
         FROM calendar_sources s
         WHERE integrations.id = s.id AND s.deleted_at IS NULL
           AND integrations.kind IN ('ics', 'graph')
           AND (integrations.label IS NOT s.label
             OR integrations.enabled IS NOT s.enabled
             OR integrations.account_hint IS NOT s.account_hint
             OR integrations.last_ok_at IS NOT s.last_ok_at
             OR integrations.last_error IS NOT s.last_error)",
        [],
    )?;
    // Waisen: Kalender-Integrationen ohne lebende Quelle (Quelle entfernt, als der
    // Trigger noch nicht da war, oder von Hand geloescht).
    let orphan = "integrations.kind IN ('ics', 'graph') AND NOT EXISTS
         (SELECT 1 FROM calendar_sources s WHERE s.id = integrations.id AND s.deleted_at IS NULL)";
    tx.execute(
        &format!(
            "DELETE FROM integration_grants WHERE integration_id IN
             (SELECT id FROM integrations WHERE {orphan})"
        ),
        [],
    )?;
    let removed = tx.execute(&format!("DELETE FROM integrations WHERE {orphan}"), [])?;
    tx.commit()?;
    Ok(AdoptReport {
        inserted,
        updated,
        removed,
    })
}

#[cfg(test)]
mod tests;
