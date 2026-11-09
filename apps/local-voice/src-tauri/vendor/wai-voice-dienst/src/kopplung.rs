//! Kopplung mit Schlüsselaustausch und Vergleichscode (Sicherheitsreview S2, Befund „Kopplung über
//! vorbelegten Endpunkt“).
//!
//! Früher schickte die App Fingerabdruck und Code im Klartext. Ein fremder Prozess, der Pipe oder
//! Socket vorbelegt, konnte beides mitlesen, an den echten Dienst weiterreichen und sich danach als
//! App anmelden. Jetzt:
//!
//! 1. App → `koppeln {…, zusage}`, `zusage = hex(SHA-256(app_pub))` (X25519, je Versuch frisch).
//! 2. Dienst → `kopplung_angebot {dienst_schluessel}` (X25519, je Versuch frisch).
//! 3. App → `kopplung_schluessel {app_schluessel}`; der Dienst prüft ihn gegen die Zusage.
//! 4. Beide leiten aus dem gemeinsamen Geheimnis und dem Transkript (Kontext, Protokoll, `client`,
//!    `gruss.zufall`, beide öffentlichen Schlüssel) per HKDF-SHA256 den Zugangsschlüssel und einen
//!    sechsstelligen Vergleichscode ab.
//! 5. App und Local Voice AI zeigen den Code; der Nutzer bestätigt in Local Voice AI nur, wenn beide
//!    gleich sind. Dann folgt `gekoppelt`.
//!
//! Wer sich dazwischenschaltet, hält zwei getrennte Austausche und damit zwei verschiedene Codes.
//! Die Zusage verhindert, dass er seinen Schlüssel so lange variiert, bis die Codes passen: Die App
//! legt sich fest, bevor sie den Dienstschlüssel sieht, der Dienst antwortet, bevor er den
//! App-Schlüssel kennt. Ein Angriff gelingt so nur mit Wahrscheinlichkeit 10⁻⁶ je Versuch, und der
//! Dienst begrenzt die Versuche ([`crate::grenzen::KOPPELN_JE_MINUTE`]). Weder Schlüssel noch
//! Fingerabdruck noch Code gehen über die Leitung.

use ring::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey, X25519};
use ring::hkdf;
use ring::rand::SystemRandom;
use sha2::{Digest, Sha256};

use crate::PROTOKOLL;

const KONTEXT: &[u8] = b"wai-sprache/1/kopplung";

/// Ergebnis einer Kopplung auf beiden Seiten.
#[derive(Clone, PartialEq, Eq)]
pub struct Ergebnis {
    /// Zugangsschlüssel als Hex (64 Zeichen); die App legt ihn im Tresor ab, der Dienst speichert
    /// nur [`crate::beweis::fingerabdruck`] davon.
    pub schluessel: String,
    /// Sechsstelliger Vergleichscode, den beide Seiten anzeigen.
    pub vergleich: String,
}

/// Ohne Schlüssel, damit er nie über `{:?}` in Logs oder Panikmeldungen landet.
impl std::fmt::Debug for Ergebnis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ergebnis")
            .field("schluessel", &"…")
            .field("vergleich", &self.vergleich)
            .finish()
    }
}

/// Fehlgeschlagener Austausch (Zufall nicht verfügbar, Schlüssel ungültig, Zusage passt nicht).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("kopplung")]
pub struct KopplungsFehler;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hex_lesen(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut aus = [0u8; 32];
    for (i, z) in s.as_bytes().chunks(2).enumerate() {
        aus[i] = u8::from_str_radix(std::str::from_utf8(z).ok()?, 16).ok()?;
    }
    Some(aus)
}

/// `zusage` zu einem öffentlichen Schlüssel.
fn zusage_von(oeffentlich: &[u8; 32]) -> String {
    hex(&Sha256::digest(oeffentlich))
}

struct Seite {
    privat: EphemeralPrivateKey,
    oeffentlich: [u8; 32],
}

impl Seite {
    fn neu() -> Result<Self, KopplungsFehler> {
        let rng = SystemRandom::new();
        let privat = EphemeralPrivateKey::generate(&X25519, &rng).map_err(|_| KopplungsFehler)?;
        let p = privat.compute_public_key().map_err(|_| KopplungsFehler)?;
        let oeffentlich: [u8; 32] = p.as_ref().try_into().map_err(|_| KopplungsFehler)?;
        Ok(Self {
            privat,
            oeffentlich,
        })
    }
}

struct Laenge(usize);

impl hkdf::KeyType for Laenge {
    fn len(&self) -> usize {
        self.0
    }
}

fn ableiten(
    geteilt: &[u8],
    client: &str,
    zufall_hex: &str,
    app_pub: &[u8; 32],
    dienst_pub: &[u8; 32],
) -> Result<Ergebnis, KopplungsFehler> {
    let mut transkript = KONTEXT.to_vec();
    transkript.push(0);
    transkript.extend_from_slice(&PROTOKOLL.to_le_bytes());
    transkript.extend_from_slice(client.as_bytes());
    transkript.push(0);
    transkript.extend_from_slice(zufall_hex.as_bytes());
    transkript.push(0);
    transkript.extend_from_slice(app_pub);
    transkript.extend_from_slice(dienst_pub);
    let salz = Sha256::digest(&transkript);
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salz).extract(geteilt);
    let mut schluessel = [0u8; 32];
    prk.expand(&[b"schluessel"], Laenge(32))
        .and_then(|o| o.fill(&mut schluessel))
        .map_err(|_| KopplungsFehler)?;
    let mut v = [0u8; 8];
    prk.expand(&[b"vergleich"], Laenge(8))
        .and_then(|o| o.fill(&mut v))
        .map_err(|_| KopplungsFehler)?;
    Ok(Ergebnis {
        schluessel: hex(&schluessel),
        // 2⁶⁴ mod 10⁶ ist gegenüber 2⁶⁴ verschwindend klein; die Verteilung ist praktisch gleich.
        vergleich: format!("{:06}", u64::from_le_bytes(v) % 1_000_000),
    })
}

/// App-Seite: erst [`AppSeite::zusage`] in `koppeln` senden, nach `kopplung_angebot`
/// [`AppSeite::oeffentlich`] in `kopplung_schluessel` senden und [`AppSeite::abschliessen`].
pub struct AppSeite(Seite);

impl AppSeite {
    pub fn neu() -> Result<Self, KopplungsFehler> {
        Seite::neu().map(Self)
    }

    pub fn zusage(&self) -> String {
        zusage_von(&self.0.oeffentlich)
    }

    pub fn oeffentlich(&self) -> String {
        hex(&self.0.oeffentlich)
    }

    pub fn abschliessen(
        self,
        dienst_schluessel_hex: &str,
        client: &str,
        zufall_hex: &str,
    ) -> Result<Ergebnis, KopplungsFehler> {
        let dienst_pub = hex_lesen(dienst_schluessel_hex).ok_or(KopplungsFehler)?;
        let app_pub = self.0.oeffentlich;
        agreement::agree_ephemeral(
            self.0.privat,
            &UnparsedPublicKey::new(&X25519, dienst_pub),
            |g| ableiten(g, client, zufall_hex, &app_pub, &dienst_pub),
        )
        .map_err(|_| KopplungsFehler)?
    }
}

/// Dienstseite: nach `koppeln` [`DienstSeite::oeffentlich`] in `kopplung_angebot` senden, nach
/// `kopplung_schluessel` [`DienstSeite::abschliessen`] mit der Zusage aus `koppeln`; dann den
/// Vergleichscode anzeigen und erst nach Bestätigung durch den Nutzer den Fingerabdruck speichern.
pub struct DienstSeite(Seite);

impl DienstSeite {
    pub fn neu() -> Result<Self, KopplungsFehler> {
        Seite::neu().map(Self)
    }

    pub fn oeffentlich(&self) -> String {
        hex(&self.0.oeffentlich)
    }

    pub fn abschliessen(
        self,
        app_schluessel_hex: &str,
        zusage_hex: &str,
        client: &str,
        zufall_hex: &str,
    ) -> Result<Ergebnis, KopplungsFehler> {
        let app_pub = hex_lesen(app_schluessel_hex).ok_or(KopplungsFehler)?;
        let soll = zusage_von(&app_pub);
        let passt = soll.len() == zusage_hex.len()
            && soll
                .bytes()
                .zip(zusage_hex.bytes())
                .fold(0u8, |a, (x, y)| a | (x ^ y))
                == 0;
        if !passt {
            return Err(KopplungsFehler);
        }
        let dienst_pub = self.0.oeffentlich;
        agreement::agree_ephemeral(
            self.0.privat,
            &UnparsedPublicKey::new(&X25519, app_pub),
            |g| ableiten(g, client, zufall_hex, &app_pub, &dienst_pub),
        )
        .map_err(|_| KopplungsFehler)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZUFALL: &str = "abababababababababababababababababababababababababababababababab";

    #[test]
    fn beide_seiten_kommen_zum_selben_ergebnis() {
        let app = AppSeite::neu().unwrap();
        let dienst = DienstSeite::neu().unwrap();
        let zusage = app.zusage();
        let app_pub = app.oeffentlich();
        let d = dienst
            .abschliessen(&app_pub, &zusage, "corporate-llm", ZUFALL)
            .unwrap();
        assert_eq!(d.schluessel.len(), 64);
        assert_eq!(d.vergleich.len(), 6);
        assert!(d.vergleich.bytes().all(|b| b.is_ascii_digit()));
    }

    #[test]
    fn app_und_dienst_stimmen_ueberein_und_haengen_vom_transkript_ab() {
        let app = AppSeite::neu().unwrap();
        let dienst = DienstSeite::neu().unwrap();
        let (zusage, app_pub, dienst_pub) = (app.zusage(), app.oeffentlich(), dienst.oeffentlich());
        let a = app
            .abschliessen(&dienst_pub, "corporate-llm", ZUFALL)
            .unwrap();
        let d = dienst
            .abschliessen(&app_pub, &zusage, "corporate-llm", ZUFALL)
            .unwrap();
        assert_eq!(a, d);

        // Anderer Client oder anderer Zufall: anderes Ergebnis.
        let app = AppSeite::neu().unwrap();
        let dienst = DienstSeite::neu().unwrap();
        let (zusage, app_pub, dienst_pub) = (app.zusage(), app.oeffentlich(), dienst.oeffentlich());
        let a = app
            .abschliessen(&dienst_pub, "corporate-llm", ZUFALL)
            .unwrap();
        let d = dienst
            .abschliessen(&app_pub, &zusage, "andere-app", ZUFALL)
            .unwrap();
        assert_ne!(a.schluessel, d.schluessel);
    }

    #[test]
    fn mittelsmann_bekommt_zwei_verschiedene_codes() {
        // App ↔ Angreifer ↔ Dienst: zwei getrennte Austausche.
        let app = AppSeite::neu().unwrap();
        let angreifer_als_dienst = DienstSeite::neu().unwrap();
        let angreifer_als_app = AppSeite::neu().unwrap();
        let dienst = DienstSeite::neu().unwrap();
        let q1 = angreifer_als_dienst.oeffentlich();
        let (z2, p2) = (angreifer_als_app.zusage(), angreifer_als_app.oeffentlich());
        let in_app = app.abschliessen(&q1, "corporate-llm", ZUFALL).unwrap();
        let im_dienst = dienst
            .abschliessen(&p2, &z2, "corporate-llm", ZUFALL)
            .unwrap();
        assert_ne!(in_app.schluessel, im_dienst.schluessel);
    }

    #[test]
    fn falsche_zusage_und_kaputte_schluessel_werden_abgewiesen() {
        let app = AppSeite::neu().unwrap();
        let andere = AppSeite::neu().unwrap();
        let dienst = DienstSeite::neu().unwrap();
        assert_eq!(
            dienst.abschliessen(&app.oeffentlich(), &andere.zusage(), "c", ZUFALL),
            Err(KopplungsFehler)
        );
        let dienst = DienstSeite::neu().unwrap();
        assert!(
            dienst
                .abschliessen(&"0".repeat(64), &zusage_von(&[0; 32]), "c", ZUFALL)
                .is_err(),
            "Nullpunkt liefert kein gemeinsames Geheimnis"
        );
        assert!(
            AppSeite::neu()
                .unwrap()
                .abschliessen("ZZ", "c", ZUFALL)
                .is_err()
        );
        assert!(
            AppSeite::neu()
                .unwrap()
                .abschliessen(&"A".repeat(64), "c", ZUFALL)
                .is_err()
        );
    }

    #[test]
    fn ableitung_ist_stabil() {
        let a = ableiten(&[7; 32], "corporate-llm", ZUFALL, &[1; 32], &[2; 32]).unwrap();
        let b = ableiten(&[7; 32], "corporate-llm", ZUFALL, &[1; 32], &[2; 32]).unwrap();
        assert_eq!(a, b);
        let c = ableiten(&[7; 32], "corporate-llm", ZUFALL, &[2; 32], &[1; 32]).unwrap();
        assert_ne!(a, c, "Rollen sind nicht vertauschbar");
    }
}
