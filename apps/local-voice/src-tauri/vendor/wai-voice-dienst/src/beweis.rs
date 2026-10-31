//! Handschlag ohne Schlüsselübertragung (Codex-Auflage 2, gegenseitige Authentisierung).
//!
//! Der Dienst speichert je Zugang nur den Fingerabdruck `SHA-256(schluessel)`. Die App kennt den
//! Schlüssel und leitet denselben Fingerabdruck ab. Beweis =
//! `HMAC-SHA256(fingerabdruck, "wai-sprache/1" || zufall || client || protokoll)`.
//! Damit geht der Schlüssel nie über die Leitung, auch nicht an einen falschen Dienst, der die
//! Pipe vorbelegt hat; ein mitgeschnittener Beweis gilt nur für diesen Zufallswert.
//! Grenze: Wer die Fingerabdruck-Ablage des Dienstes liest, kann sich anmelden; die Ablage ist
//! deshalb wie ein Geheimnis zu schützen (nur Benutzer, Tresor).

use sha2::{Digest, Sha256};

use crate::PROTOKOLL;

const KONTEXT: &[u8] = b"wai-sprache/1";
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
    let mut n = KONTEXT.to_vec();
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
    let soll = erzeugen(fingerabdruck, zufall_hex, client);
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
}
