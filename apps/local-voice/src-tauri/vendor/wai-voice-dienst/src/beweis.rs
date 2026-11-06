//! Handschlag ohne Schlüsselübertragung (Codex-Auflage 2, gegenseitige Authentisierung).
//!
//! Der Dienst speichert je Zugang nur den Fingerabdruck `SHA-256(schluessel)`. Die App kennt den
//! Schlüssel und leitet denselben Fingerabdruck ab. Beweis =
//! `HMAC-SHA256(fingerabdruck, "wai-sprache/1" || zufall || client || protokoll)`.
//! Damit geht der Schlüssel nie über die Leitung, auch nicht an einen falschen Dienst, der die
//! Pipe vorbelegt hat; ein mitgeschnittener Beweis gilt nur für diesen Zufallswert.
//! Grenze: Wer die Fingerabdruck-Ablage des Dienstes liest, kann sich anmelden; die Ablage ist
//! deshalb wie ein Geheimnis zu schützen (nur Benutzer, Tresor).
//!
//! Umgekehrt beweist der Dienst in `willkommen` mit [`dienst_erzeugen`], dass er den Fingerabdruck
//! kennt (eigener Kontext). Der Gegenbeweis bindet zusätzlich den Zufall der App aus `hallo`;
//! sonst könnte ein Prozess, der Pipe oder Socket belegt, ein mitgeschnittenes
//! `gruss`/`willkommen`-Paar später wieder abspielen. Die App sendet weder Audio noch Text, bevor sie diesen Gegenbeweis
//! geprüft hat; ein Prozess, der die Pipe vorbelegt, kennt den Fingerabdruck nicht.

use sha2::{Digest, Sha256};

use crate::PROTOKOLL;

const KONTEXT: &[u8] = b"wai-sprache/1";
const KONTEXT_DIENST: &[u8] = b"wai-sprache/1/dienst";
const BLOCK: usize = 64;

/// Fingerabdruck eines Zugangsschlüssels (das, was der Dienst speichert).
pub fn fingerabdruck(schluessel: &str) -> [u8; 32] {
    Sha256::digest(schluessel.as_bytes()).into()
}

fn hmac(key: &[u8; 32], nachricht: &[u8]) -> [u8; 32] {
    let mut k = [0u8; BLOCK];
    k[..32].copy_from_slice(key);
    let mut innen = Sha256::new();
    innen.update(k.map(|b| b ^ 0x36));
    innen.update(nachricht);
    let mut aussen = Sha256::new();
    aussen.update(k.map(|b| b ^ 0x5c));
    aussen.update(innen.finalize());
    aussen.finalize().into()
}

fn nachricht(zufall_hex: &str, client: &str) -> Vec<u8> {
    nachricht_mit(KONTEXT, zufall_hex, client)
}

fn nachricht_mit(kontext: &[u8], zufall_hex: &str, client: &str) -> Vec<u8> {
    let mut n = kontext.to_vec();
    n.extend_from_slice(zufall_hex.as_bytes());
    n.push(0);
    n.extend_from_slice(client.as_bytes());
    n.push(0);
    n.extend_from_slice(&PROTOKOLL.to_le_bytes());
    n
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Beweis, den die App in `hallo` schickt.
pub fn erzeugen(fingerabdruck: &[u8; 32], zufall_hex: &str, client: &str) -> String {
    hex(&hmac(fingerabdruck, &nachricht(zufall_hex, client)))
}

/// Prüft den Beweis in konstanter Zeit.
pub fn pruefen(fingerabdruck: &[u8; 32], zufall_hex: &str, client: &str, beweis_hex: &str) -> bool {
    gleich(&erzeugen(fingerabdruck, zufall_hex, client), beweis_hex)
}

/// Gegenbeweis des Dienstes in `willkommen`: zeigt der App, dass der Dienst den Fingerabdruck ihres
/// Zugangs kennt, also kein fremder Prozess die Pipe vorbelegt hat. Eigener Kontext, damit ein
/// App-Beweis nie als Dienstbeweis zurückgespielt werden kann.
pub fn dienst_erzeugen(
    fingerabdruck: &[u8; 32],
    zufall_hex: &str,
    app_zufall_hex: &str,
    client: &str,
) -> String {
    let mut n = nachricht_mit(KONTEXT_DIENST, zufall_hex, client);
    n.push(0);
    n.extend_from_slice(app_zufall_hex.as_bytes());
    hex(&hmac(fingerabdruck, &n))
}

/// Prüft den Gegenbeweis des Dienstes in konstanter Zeit (App-Seite).
pub fn dienst_pruefen(
    fingerabdruck: &[u8; 32],
    zufall_hex: &str,
    app_zufall_hex: &str,
    client: &str,
    beweis_hex: &str,
) -> bool {
    gleich(
        &dienst_erzeugen(fingerabdruck, zufall_hex, app_zufall_hex, client),
        beweis_hex,
    )
}

fn gleich(soll: &str, beweis_hex: &str) -> bool {
    soll.len() == beweis_hex.len()
        && soll
            .bytes()
            .zip(beweis_hex.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_stimmt_mit_bekanntem_wert_ueberein() {
        // HMAC-SHA256 mit leerem Schlüssel und leerer Nachricht (bekannter Referenzwert); ein
        // Schlüssel aus Nullen ist nach dem Auffüllen auf die Blockgröße gleichwertig.
        let r = hmac(&[0u8; 32], b"");
        assert_eq!(
            hex(&r),
            "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad"
        );
    }

    #[test]
    fn beweis_bindet_zufall_und_client() {
        let fp = fingerabdruck("lvat_geheim");
        let z = "ab".repeat(32);
        let b = erzeugen(&fp, &z, "corporate-llm");
        assert_eq!(b.len(), 64);
        assert!(pruefen(&fp, &z, "corporate-llm", &b));
        assert!(
            !pruefen(&fp, &"cd".repeat(32), "corporate-llm", &b),
            "anderer Zufall"
        );
        assert!(!pruefen(&fp, &z, "poststelle", &b), "anderer Client");
        assert!(
            !pruefen(&fingerabdruck("lvat_anders"), &z, "corporate-llm", &b),
            "anderer Schlüssel"
        );
    }

    #[test]
    fn gegenbeweis_ist_nicht_der_app_beweis() {
        let fp = fingerabdruck("k");
        let z = "ab".repeat(32);
        let app = erzeugen(&fp, &z, "corporate-llm");
        let a = "cd".repeat(32);
        let dienst = dienst_erzeugen(&fp, &z, &a, "corporate-llm");
        assert_ne!(app, dienst);
        assert!(dienst_pruefen(&fp, &z, &a, "corporate-llm", &dienst));
        assert!(!dienst_pruefen(&fp, &z, &a, "corporate-llm", &app));
        assert!(
            !dienst_pruefen(&fp, &z, &"ef".repeat(32), "corporate-llm", &dienst),
            "anderer App-Zufall: alter Gegenbeweis gilt nicht"
        );
    }
}
