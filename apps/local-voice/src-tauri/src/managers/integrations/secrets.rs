//! Geheimnisse der Integrationen (A1): Namensraum ueber dem DPAPI-Speicher
//! `calendar::secret`.
//!
//! - Kalenderquellen (`ics`, `graph`) behalten ihren bisherigen Namen (Datei
//!   `<id>.bin`): kein Neu-Login nach der Uebernahme ins Register.
//! - Alle anderen Arten legen ihre Geheimnisse als `int-<id>-<fach>.bin` mit
//!   eigenem Zusatzgeheimnis ab (siehe `secret.rs`).
//! - In Register, Audit, Dump und Logs stehen nur Zustaende
//!   (`present`/`missing`/`broken`), nie Inhalte.
//!
//! Fehlerfall „Geheimnis fehlt oder ist kaputt“: `status*` meldet `Missing` bzw.
//! `Broken(Klartext)`; die Integration zeigt dann „neu einrichten“, statt mit
//! einem leeren Wert zu arbeiten. Es gibt nie ein stilles Weiterarbeiten ohne
//! Geheimnis.

use std::path::Path;

use crate::managers::calendar::secret::{self, SecretRef, SecretStatus};

use super::model::{Integration, Kind};

/// Faecher, die eine Art braucht (leer: kein Geheimnis noetig).
pub fn required_slots(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Ics | Kind::Graph => &["main"],
        Kind::M365 => &["token"],
        Kind::Smtp => &["password"],
        Kind::Wissen => &["token"],
        // Die Adresse eines Webhooks traegt oft den Schluessel im Pfad (n8n).
        Kind::Webhook => &["url"],
        // Webhook-Adresse (Slack, Teams, Discord) oder API-Schluessel.
        Kind::Service => &["token"],
        Kind::Youtube | Kind::Folder | Kind::Obsidian | Kind::Agent | Kind::Presenter => &[],
    }
}

/// Verweis auf das Geheimnis einer Integration. Kalenderarten ignorieren das
/// Fach (ein Geheimnis je Quelle, alter Name).
pub fn secret_ref(i: &Integration, slot: &str) -> SecretRef {
    if i.kind.is_calendar_managed() {
        SecretRef::calendar(&i.id)
    } else {
        SecretRef::integration(&i.id, slot)
    }
}

/// Zustand eines Faches im Speicher dieser App-Instanz.
pub fn status(i: &Integration, slot: &str) -> SecretStatus {
    secret::secret_status_ref(&secret_ref(i, slot))
}

/// Zustand eines Faches in einem bestimmten Ordner (Dump, Tests).
pub fn status_in(dir: &Path, i: &Integration, slot: &str) -> SecretStatus {
    secret::status_ref_in(dir, &secret_ref(i, slot))
}

/// Zustand aller noetigen Faecher.
pub fn statuses_in(dir: &Path, i: &Integration) -> Vec<(&'static str, SecretStatus)> {
    required_slots(i.kind)
        .iter()
        .map(|slot| (*slot, status_in(dir, i, slot)))
        .collect()
}

/// Legt ein Geheimnis ab (DPAPI, atomar).
pub fn put(i: &Integration, slot: &str, data: &[u8]) -> Result<(), String> {
    secret::secret_put_ref(&secret_ref(i, slot), data)
}

pub fn put_in(dir: &Path, i: &Integration, slot: &str, data: &[u8]) -> Result<(), String> {
    secret::put_ref_in(dir, &secret_ref(i, slot), data)
}

/// Liest ein Geheimnis als Text (UTF-8); `Ok(None)`, wenn es fehlt. Der Inhalt
/// verlaesst diese Datei nur als `Zeroizing`-Puffer in die Adapter (A6).
pub fn get_text(i: &Integration, slot: &str) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    to_text(secret::secret_get_ref(&secret_ref(i, slot))?)
}

pub fn get_text_in(
    dir: &Path,
    i: &Integration,
    slot: &str,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    to_text(secret::get_ref_in(dir, &secret_ref(i, slot))?)
}

fn to_text(
    raw: Option<zeroize::Zeroizing<Vec<u8>>>,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    match raw {
        None => Ok(None),
        Some(bytes) => String::from_utf8(bytes.to_vec())
            .map(|s| Some(zeroize::Zeroizing::new(s)))
            .map_err(|_| "Das Geheimnis ist kein gültiger Text.".to_string()),
    }
}

/// Loescht alle Geheimnisse einer Integration (beim Entfernen). Kalenderarten
/// raeumt der Kalender selbst auf; hier bleibt ihr Geheimnis unberuehrt.
pub fn delete_all(i: &Integration) -> usize {
    if i.kind.is_calendar_managed() {
        return 0;
    }
    secret::secret_delete_all_for_integration(&i.id)
}

pub fn delete_all_in(dir: &Path, i: &Integration) -> usize {
    if i.kind.is_calendar_managed() {
        return 0;
    }
    secret::delete_all_for_integration_in(dir, &i.id)
}

/// Kurztext fuer Dump und Oberflaeche.
pub fn status_label(s: &SecretStatus) -> &'static str {
    match s {
        SecretStatus::Present => "present",
        SecretStatus::Missing => "missing",
        SecretStatus::Broken(_) => "broken",
    }
}

/// Verweis auf ein Schnittstellen-Geheimnis; der Name wird nie zum Dateipfad.
pub fn schnittstelle_ref(schluessel: &str) -> Option<SecretRef> {
    use sha2::{Digest, Sha256};

    let bytes = schluessel.as_bytes();
    let klein_oder_ziffer = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    if bytes.is_empty()
        || bytes.len() > 64
        || !klein_oder_ziffer(bytes[0])
        || !bytes[1..]
            .iter()
            .all(|&c| klein_oder_ziffer(c) || matches!(c, b'.' | b'_' | b'-'))
    {
        return None;
    }
    let mut hash = Sha256::new();
    hash.update(b"lva.schnittstelle/1");
    hash.update([0u8]);
    hash.update(bytes);
    let hex = format!("{:x}", hash.finalize());
    let id = format!("lvs{}", &hex[..32]);
    Some(SecretRef::integration(&id, "token"))
}

/// Liest nur aus dem Geheimnisspeicher, nie aus dem Schnittstellen-Verzeichnis.
pub fn schnittstelle_get_text(
    schluessel: &str,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    let verweis = schnittstelle_ref(schluessel)
        .ok_or_else(|| "Ungültiger Name für ein Geheimnis.".to_string())?;
    to_text(secret::secret_get_ref(&verweis)?)
}

pub fn schnittstelle_put(schluessel: &str, data: &[u8]) -> Result<(), String> {
    let verweis = schnittstelle_ref(schluessel)
        .ok_or_else(|| "Ungültiger Name für ein Geheimnis.".to_string())?;
    secret::secret_put_ref(&verweis, data)
}

pub fn schnittstelle_get_text_in(
    dir: &Path,
    schluessel: &str,
) -> Result<Option<zeroize::Zeroizing<String>>, String> {
    let verweis = schnittstelle_ref(schluessel)
        .ok_or_else(|| "Ungültiger Name für ein Geheimnis.".to_string())?;
    to_text(secret::get_ref_in(dir, &verweis)?)
}

pub fn schnittstelle_put_in(dir: &Path, schluessel: &str, data: &[u8]) -> Result<(), String> {
    let verweis = schnittstelle_ref(schluessel)
        .ok_or_else(|| "Ungültiger Name für ein Geheimnis.".to_string())?;
    secret::put_ref_in(dir, &verweis, data)
}

#[cfg(test)]
mod tests;
