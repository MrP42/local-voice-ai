//! Schlüsselableitung und Objekt-Verschlüsselung des Geräte-Syncs.
//!
//! Der Schlüssel entsteht nur auf dem Gerät aus Passwort und E-Mail
//! (Argon2id → HKDF) und verlässt es nie. Jedes Objekt wird einzeln mit
//! XChaCha20-Poly1305 versiegelt; die Zusatzdaten (AAD) binden das Chiffrat an
//! Benutzer, Sammlung und Objekt-Kennung — ein serverseitig an einen anderen
//! Ort verschobenes Blob entschlüsselt nicht.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

pub const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

/// Argon2id-Parameter: 64 MiB, 3 Durchläufe, 1 Spur. Etwa 0,3 s auf einem
/// Desktop — einmal je Anmeldung, nie im Sync-Zyklus.
const ARGON_M_KIB: u32 = 64 * 1024;
const ARGON_T: u32 = 3;
const ARGON_P: u32 = 1;

/// Schlüssel aus Passwort und E-Mail. Das Salz ist aus der E-Mail abgeleitet,
/// damit zwei Geräte desselben Kontos ohne Austausch denselben Schlüssel
/// erhalten.
pub fn derive_key(password: &str, email: &str) -> Result<[u8; KEY_LEN], String> {
    let salt_src = format!("local-voice-ai/v1/{}", email.trim().to_lowercase());
    let salt_full = Sha256::digest(salt_src.as_bytes());
    let salt = &salt_full[..16];
    let params = Params::new(ARGON_M_KIB, ARGON_T, ARGON_P, Some(KEY_LEN))
        .map_err(|e| format!("Argon2-Parameter: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut master = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut master)
        .map_err(|e| format!("Schlüsselableitung: {e}"))?;
    let hk = Hkdf::<Sha256>::new(None, &master);
    let mut key = [0u8; KEY_LEN];
    hk.expand(b"lv-sync-enc-v1", &mut key)
        .map_err(|e| format!("HKDF: {e}"))?;
    master.zeroize();
    Ok(key)
}

/// Kurze, öffentliche Kennung des Schlüssels: ein Gerät mit anderem Passwort
/// erkennt so „falscher Schlüssel" statt „kaputtes Blob".
pub fn key_id(key: &[u8; KEY_LEN]) -> String {
    let digest = Sha256::digest(key);
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

pub fn aad(user_id: i64, collection: &str, object_id: &str) -> Vec<u8> {
    format!("{user_id}\u{1f}{collection}\u{1f}{object_id}").into_bytes()
}

/// Versiegeln: zufällige 24-Byte-Nonce, dann `base64(nonce ‖ chiffrat)`.
pub fn seal(key: &[u8; KEY_LEN], aad: &[u8], plaintext: &[u8]) -> Result<String, String> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad })
        .map_err(|_| "Verschlüsselung fehlgeschlagen".to_string())?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(B64.encode(out))
}

pub fn open(key: &[u8; KEY_LEN], aad: &[u8], sealed_b64: &str) -> Result<Vec<u8>, String> {
    let raw = B64
        .decode(sealed_b64.trim())
        .map_err(|_| "Chiffrat ist kein Base64".to_string())?;
    if raw.len() < NONCE_LEN + 16 {
        return Err("Chiffrat zu kurz".into());
    }
    let (nonce_bytes, ct) = raw.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(XNonce::from_slice(nonce_bytes), Payload { msg: ct, aad })
        .map_err(|_| "Entschlüsselung fehlgeschlagen (falscher Schlüssel oder manipuliertes Objekt)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; KEY_LEN] {
        // Bewusst ohne Argon2 (langsam): ein fester Schlüssel reicht für Siegel-Tests.
        let mut k = [0u8; KEY_LEN];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        k
    }

    #[test]
    fn siegel_roundtrip() {
        let key = test_key();
        let a = aad(7, "page", "page_1");
        let sealed = seal(&key, &a, b"Guten Tag").unwrap();
        assert_eq!(open(&key, &a, &sealed).unwrap(), b"Guten Tag");
        // Jede Versiegelung hat eine neue Nonce.
        assert_ne!(sealed, seal(&key, &a, b"Guten Tag").unwrap());
    }

    #[test]
    fn falscher_ort_oder_schluessel_oeffnet_nicht() {
        let key = test_key();
        let sealed = seal(&key, &aad(7, "page", "page_1"), b"x").unwrap();
        assert!(open(&key, &aad(7, "page", "page_2"), &sealed).is_err(), "anderes Objekt");
        assert!(open(&key, &aad(8, "page", "page_1"), &sealed).is_err(), "anderer Benutzer");
        let mut other = test_key();
        other[0] ^= 1;
        assert!(open(&other, &aad(7, "page", "page_1"), &sealed).is_err(), "anderer Schlüssel");
        assert!(open(&key, &aad(7, "page", "page_1"), "nicht base64!").is_err());
        assert!(open(&key, &aad(7, "page", "page_1"), "AAAA").is_err(), "zu kurz");
    }

    #[test]
    fn key_id_ist_stabil_und_kurz() {
        let id = key_id(&test_key());
        assert_eq!(id.len(), 8);
        assert_eq!(id, key_id(&test_key()));
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn ableitung_ist_deterministisch_und_email_normalisiert() {
        // Kleinere Kosten wären schöner, aber die Parameter sind Teil des
        // Vertrags: zwei Geräte müssen denselben Schlüssel bekommen.
        let a = derive_key("geheim", "Mail@Example.com").unwrap();
        let b = derive_key("geheim", "  mail@example.com ").unwrap();
        let c = derive_key("geheim2", "mail@example.com").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
