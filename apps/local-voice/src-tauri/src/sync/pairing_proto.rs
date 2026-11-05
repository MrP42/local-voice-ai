//! Geräte-Kopplung per QR, Protokoll v2: reine Kryptografie ohne Netz und
//! ohne Tauri, damit sie gegen die Testvektoren geprüft werden kann.
//! Spec: `docs/superpowers/specs/2026-11-05-geraete-kopplung-qr.md` (Abschnitt 4),
//! Vektoren: `…-geraete-kopplung-qr-vektoren.json`.
//!
//! Kodierung: `id` geht als ASCII-Text in alle Hashes ein; alle übrigen Werte
//! sind 32 Rohbytes und laufen als kanonisches base64url ohne Padding.

use base64::engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD as B64U};
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

pub const LEN: usize = 32;
pub const NONCE_LEN: usize = 24;

type HmacSha256 = Hmac<Sha256>;

const COMMIT: &[u8] = b"LV-PAIR-COMMIT-v2";
const CLAIM: &[u8] = b"LV-PAIR-CLAIM-v2";
const SAS: &[u8] = b"LV-PAIR-SAS-v2";
const KEY: &[u8] = b"LV-PAIR-KEY-v2";
const SEAL: &[u8] = b"LV-PAIR-SEAL-v2";
const CONFIRM: &[u8] = b"LV-PAIR-CONFIRM-v2";

pub fn b64u(bytes: &[u8]) -> String {
    B64U.encode(bytes)
}

/// 32 Bytes aus kanonischem base64url ohne Padding (43 Zeichen). Alles andere
/// wird abgelehnt, auch nicht kanonische Endbits.
pub fn b64u32(s: &str) -> Result<[u8; LEN], String> {
    if s.len() != 43 {
        return Err("Wert hat nicht 32 Byte".into());
    }
    let raw = B64U
        .decode(s)
        .map_err(|_| "Wert ist kein base64url".to_string())?;
    if raw.len() != LEN || B64U.encode(&raw) != s {
        return Err("Wert ist nicht kanonisch kodiert".into());
    }
    let mut out = [0u8; LEN];
    out.copy_from_slice(&raw);
    Ok(out)
}

/// Kopplungskennung: 16 Byte, kanonisch base64url (22 Zeichen).
pub fn valid_pair_id(id: &str) -> bool {
    id.len() == 22
        && B64U
            .decode(id)
            .map(|raw| raw.len() == 16 && B64U.encode(&raw) == id)
            .unwrap_or(false)
}

/// Gerätename, wie ihn der Hub annimmt: 1–60 Zeichen, ohne Rand-Leerzeichen,
/// ohne Steuer-, Format-/Bidi-, Nullbreiten- und Privatzeichen. Der Hub prüft
/// strenger (NFC, `\p{C}`); hier genügt die Abwehr gegen Täuschung in der Anzeige.
pub fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=60).contains(&n)
        && name.trim() == name
        && !name.chars().any(|c| {
            c.is_control()
                || matches!(c as u32,
                    // Format (Cf), Zeilen-/Absatztrenner, Füller und Privatzeichen
                    0x00AD | 0x034F | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F
                    | 0x08E2 | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180E
                    | 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x206F
                    | 0x3164 | 0xFEFF | 0xFFA0 | 0xFFF9..=0xFFFB
                    | 0x110BD | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3
                    | 0x1D173..=0x1D17A | 0xE0000..=0xE0FFF
                    | 0xE000..=0xF8FF | 0xF0000..=0x10FFFF)
        })
}

fn sha(parts: &[&[u8]]) -> [u8; LEN] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> HmacSha256 {
    let mut m = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC nimmt jede Schlüssellänge");
    for p in parts {
        m.update(p);
    }
    m
}

/// `C = H("LV-PAIR-COMMIT-v2" ‖ id ‖ p_pk ‖ n_p ‖ len16be(name) ‖ name)`,
/// Länge in UTF-8-Bytes.
pub fn commitment(id: &str, p_pk: &[u8; LEN], n_p: &[u8; LEN], name: &str) -> [u8; LEN] {
    let len = (name.len() as u16).to_be_bytes();
    sha(&[COMMIT, id.as_bytes(), p_pk, n_p, &len, name.as_bytes()])
}

/// `mac = HMAC(q, "LV-PAIR-CLAIM-v2" ‖ id ‖ C)`.
pub fn claim_mac(q: &[u8; LEN], id: &str, commit: &[u8; LEN]) -> [u8; LEN] {
    hmac(q, &[CLAIM, id.as_bytes(), commit])
        .finalize()
        .into_bytes()
        .into()
}

/// Prüft `mac` in konstanter Zeit.
pub fn verify_claim_mac(q: &[u8; LEN], id: &str, commit: &[u8; LEN], mac: &[u8; LEN]) -> bool {
    hmac(q, &[CLAIM, id.as_bytes(), commit])
        .verify_slice(mac)
        .is_ok()
}

/// Sechsstelliger Vergleichscode, mit führenden Nullen.
pub fn sas(
    id: &str,
    d_pk: &[u8; LEN],
    p_pk: &[u8; LEN],
    n_p: &[u8; LEN],
    n_d: &[u8; LEN],
) -> String {
    let h = sha(&[SAS, id.as_bytes(), d_pk, p_pk, n_p, n_d]);
    let v = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000;
    format!("{v:06}")
}

pub fn public_key(d_sk: &StaticSecret) -> [u8; LEN] {
    PublicKey::from(d_sk).to_bytes()
}

/// `z = X25519(d_sk, p_pk)`; Abbruch bei nicht beitragendem Ergebnis
/// (RFC 7748 §6.1, kleine Untergruppe → 32 Nullbytes).
pub fn shared(d_sk: &StaticSecret, p_pk: &[u8; LEN]) -> Result<Zeroizing<[u8; LEN]>, String> {
    let z = d_sk.diffie_hellman(&PublicKey::from(*p_pk));
    if !z.was_contributory() {
        return Err("Schlüsseltausch ungültig".into());
    }
    Ok(Zeroizing::new(z.to_bytes()))
}

/// `s = HKDF-SHA256(ikm=z, salt=q ‖ d_pk ‖ p_pk, info="LV-PAIR-KEY-v2" ‖ id, 32 B)`.
pub fn session_key(
    z: &[u8; LEN],
    q: &[u8; LEN],
    d_pk: &[u8; LEN],
    p_pk: &[u8; LEN],
    id: &str,
) -> Zeroizing<[u8; LEN]> {
    let mut salt = Vec::with_capacity(3 * LEN);
    salt.extend_from_slice(q);
    salt.extend_from_slice(d_pk);
    salt.extend_from_slice(p_pk);
    let mut info = KEY.to_vec();
    info.extend_from_slice(id.as_bytes());
    let mut s = Zeroizing::new([0u8; LEN]);
    Hkdf::<Sha256>::new(Some(&salt), z)
        .expand(&info, s.as_mut())
        .expect("32 Byte sind fuer HKDF-SHA256 zulaessig");
    s
}

pub fn seal_aad(id: &str, d_pk: &[u8; LEN], p_pk: &[u8; LEN]) -> Vec<u8> {
    let mut aad = SEAL.to_vec();
    aad.extend_from_slice(id.as_bytes());
    aad.extend_from_slice(d_pk);
    aad.extend_from_slice(p_pk);
    aad
}

/// Inhalt von `sealed`. Feldreihenfolge ist Teil der Testvektoren.
#[derive(serde::Serialize)]
struct SealPayload<'a> {
    v: u8,
    k: &'a str,
    email: &'a str,
    user_id: i64,
    key_id: &'a str,
}

/// Klartext für `sealed`: `{"v":1,"k":…,"email":…,"user_id":…,"key_id":…}`.
pub fn payload_json(k: &[u8; LEN], email: &str, user_id: i64) -> Zeroizing<String> {
    let k_b64 = Zeroizing::new(b64u(k));
    let key_id = super::crypto::key_id(k);
    Zeroizing::new(
        serde_json::to_string(&SealPayload {
            v: 1,
            k: &k_b64,
            email,
            user_id,
            key_id: &key_id,
        })
        .expect("feste Struktur ist serialisierbar"),
    )
}

/// `sealed = base64(nonce24 ‖ XChaCha20-Poly1305(s, P, AAD))`.
pub fn seal(
    s: &[u8; LEN],
    id: &str,
    d_pk: &[u8; LEN],
    p_pk: &[u8; LEN],
    plaintext: &[u8],
    nonce: &[u8; NONCE_LEN],
) -> Result<String, String> {
    let cipher = XChaCha20Poly1305::new(s.into());
    let aad = seal_aad(id, d_pk, p_pk);
    let ct = cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| "Versiegeln fehlgeschlagen".to_string())?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    Ok(B64.encode(out))
}

/// Bestätigung des iPhones: `HMAC(s, "LV-PAIR-CONFIRM-v2" ‖ id)`, konstante Zeit.
pub fn verify_confirm(s: &[u8; LEN], id: &str, confirm: &[u8; LEN]) -> bool {
    hmac(s, &[CONFIRM, id.as_bytes()])
        .verify_slice(confirm)
        .is_ok()
}

#[cfg(test)]
pub fn confirm_mac(s: &[u8; LEN], id: &str) -> [u8; LEN] {
    hmac(s, &[CONFIRM, id.as_bytes()])
        .finalize()
        .into_bytes()
        .into()
}

/// Vergleich zweier 32-Byte-Werte ohne frühen Ausstieg.
pub fn ct_eq(a: &[u8; LEN], b: &[u8; LEN]) -> bool {
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const VEKTOREN: &str = include_str!(
        "../../../../../docs/superpowers/specs/2026-11-05-geraete-kopplung-qr-vektoren.json"
    );

    fn v() -> serde_json::Value {
        serde_json::from_str(VEKTOREN).unwrap()
    }

    fn k32(v: &serde_json::Value, a: &str, b: &str) -> [u8; LEN] {
        b64u32(v[a][b].as_str().unwrap()).unwrap()
    }

    #[test]
    fn vektoren_v2_stimmen() {
        let v = v();
        let id = v["eingaben"]["pair_id"].as_str().unwrap();
        let name = v["eingaben"]["device_name"].as_str().unwrap();
        let q = k32(&v, "eingaben", "q");
        let d_sk = StaticSecret::from(k32(&v, "eingaben", "d_sk"));
        let p_sk = StaticSecret::from(k32(&v, "eingaben", "p_sk"));
        let n_p = k32(&v, "eingaben", "n_p");
        let n_d = k32(&v, "eingaben", "n_d");
        let k = k32(&v, "eingaben", "K");
        let nonce: [u8; NONCE_LEN] = B64U
            .decode(v["eingaben"]["seal_nonce"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();

        let d_pk = public_key(&d_sk);
        let p_pk = public_key(&p_sk);
        assert_eq!(b64u(&d_pk), v["ergebnisse"]["d_pk"]);
        assert_eq!(b64u(&p_pk), v["ergebnisse"]["p_pk"]);

        let c = commitment(id, &p_pk, &n_p, name);
        assert_eq!(b64u(&c), v["ergebnisse"]["phone_commit"]);
        let mac = claim_mac(&q, id, &c);
        assert_eq!(b64u(&mac), v["ergebnisse"]["phone_mac"]);
        assert!(verify_claim_mac(&q, id, &c, &mac));
        let mut falsch = q;
        falsch[0] ^= 1;
        assert!(!verify_claim_mac(&falsch, id, &c, &mac));

        let z = shared(&d_sk, &p_pk).unwrap();
        assert_eq!(b64u(z.as_ref()), v["ergebnisse"]["z"]);
        // Beide Seiten kommen auf dasselbe z.
        assert_eq!(*shared(&p_sk, &d_pk).unwrap(), *z);

        let s = session_key(&z, &q, &d_pk, &p_pk, id);
        assert_eq!(b64u(s.as_ref()), v["ergebnisse"]["s"]);
        assert_eq!(sas(id, &d_pk, &p_pk, &n_p, &n_d), v["ergebnisse"]["sas"]);

        let p = payload_json(&k, "patrick@example.org", 42);
        assert_eq!(p.as_str(), v["ergebnisse"]["plaintext"]);
        assert_eq!(b64u(&seal_aad(id, &d_pk, &p_pk)), v["ergebnisse"]["aad"]);
        assert_eq!(
            seal(&s, id, &d_pk, &p_pk, p.as_bytes(), &nonce).unwrap(),
            v["ergebnisse"]["sealed"]
        );

        let conf = confirm_mac(&s, id);
        assert_eq!(b64u(&conf), v["ergebnisse"]["confirm"]);
        assert!(verify_confirm(&s, id, &conf));
        let mut anders = conf;
        anders[31] ^= 1;
        assert!(!verify_confirm(&s, id, &anders));
    }

    #[test]
    fn vektor_utf8_laenge_in_bytes() {
        let v = v();
        let id = v["eingaben"]["pair_id"].as_str().unwrap();
        let p_pk = public_key(&StaticSecret::from(k32(&v, "eingaben", "p_sk")));
        let n_p = k32(&v, "eingaben", "n_p");
        let name = v["zusatz_utf8"]["device_name"].as_str().unwrap();
        assert_eq!(
            name.len() as u64,
            v["zusatz_utf8"]["laenge_bytes"].as_u64().unwrap()
        );
        assert_eq!(
            b64u(&commitment(id, &p_pk, &n_p, name)),
            v["zusatz_utf8"]["phone_commit"]
        );
    }

    #[test]
    fn nullpunkt_wird_abgelehnt() {
        let d_sk = StaticSecret::from([7u8; LEN]);
        assert!(shared(&d_sk, &[0u8; LEN]).is_err());
        // Punkt kleiner Ordnung (u = 1)
        let mut eins = [0u8; LEN];
        eins[0] = 1;
        assert!(shared(&d_sk, &eins).is_err());
    }

    #[test]
    fn kodierung_nur_kanonisch() {
        let v = v();
        let q = v["eingaben"]["q"].as_str().unwrap();
        assert!(b64u32(q).is_ok());
        assert!(b64u32(&format!("{q}=")).is_err());
        assert!(b64u32(&q[..42]).is_err());
        // Letztes Zeichen mit gesetzten Füllbits ist nicht kanonisch.
        let mut nk = q[..42].to_string();
        nk.push('B');
        assert!(b64u32(&nk).is_err());
        // Standard-Alphabet statt base64url wird abgelehnt.
        let secret = v["eingaben"]["secret"].as_str().unwrap();
        assert!(secret.contains('-') && b64u32(secret).is_ok());
        assert!(b64u32(&secret.replace('-', "+")).is_err());
        assert!(valid_pair_id("AAECAwQFBgcICQoLDA0ODw"));
        assert!(!valid_pair_id("AAECAwQFBgcICQoLDA0ODx"));
        assert!(!valid_pair_id("AAECAwQFBgcICQoLDA0OD"));
    }

    #[test]
    fn geraetename_gegen_taeuschung() {
        assert!(valid_name("iPhone von Patrick"));
        assert!(valid_name("Patricks iPhone Ä"));
        assert!(!valid_name(""));
        assert!(!valid_name(" iPhone"));
        assert!(!valid_name("iPhone\u{202E}enohPi"));
        assert!(!valid_name("i\u{200B}Phone"));
        assert!(!valid_name("a\nb"));
        assert!(!valid_name("iPhone\u{E0041}"));
        assert!(!valid_name("i\u{3164}Phone"));
        assert!(!valid_name("a\u{2028}b"));
        // Variationsselektoren (Mn) nimmt der Hub an, also auch hier.
        assert!(valid_name("iPhone \u{2764}\u{FE0F}"));
        assert!(!valid_name(&"x".repeat(61)));
        assert!(valid_name(&"ä".repeat(60)));
    }

    #[test]
    fn ct_eq_vergleicht() {
        let a = [3u8; LEN];
        let mut b = a;
        assert!(ct_eq(&a, &b));
        b[31] = 4;
        assert!(!ct_eq(&a, &b));
    }
}
