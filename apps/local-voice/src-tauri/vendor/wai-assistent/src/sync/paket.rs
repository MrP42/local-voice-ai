//! Gewrappte Epochenschluessel (ADR-0028 E3.8, Must-fix 2): ein Paket je Epoche und Empfaengergeraet.
//!
//! Verfahren: X25519 (fluechtiger Absenderschluessel, gepinnter Empfaenger-Austauschschluessel aus dem
//! Verzeichnis), HKDF-SHA256, XChaCha20-Poly1305. Die AAD bindet Konto, Epoche, Empfaenger samt
//! Austauschschluessel, Absender und den Verzeichnisstand (`verzeichnis_nummer`, `verzeichnis_hash`), also
//! die Mitgliedschaft zum Zeitpunkt der Verteilung. Der Absender signiert AAD und Chiffrat mit Ed25519:
//! Der Hub kann keinen Schluessel einschleusen (er koennte sonst einfach an den oeffentlichen
//! Austauschschluessel verschluesseln).
//!
//! Klartext: Kontosalz (32 Byte) und Epochenschluessel (32 Byte).

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use super::fehler::SyncFehler;
use super::identitaet::{Ausweis, Geraeteschluessel};
use super::kennung::{Hex128, begrenzt_lesen, hexfeld, zufall};
use super::schluessel::Schluessel;
use super::umschlag::zaehler;
use super::verzeichnis::Verzeichnis;

const AAD_KENNUNG: &[u8] = b"wai-assistent-sync/paket/v1";
const SIG_KENNUNG: &[u8] = b"wai-assistent-sync/paket-signatur/v1";
const HKDF_INFO: &[u8] = b"wai-assistent-sync/paket-schluessel/v1";
const KLARTEXT_LAENGE: usize = 64;
const CHIFFRAT_B64_MAX: usize = 128;

/// Ein gewrappter Epochenschluessel fuer genau ein Empfaengergeraet (Hub-Transportform).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schluesselpaket {
    pub konto: Hex128,
    #[serde(with = "zaehler")]
    pub epoche: u64,
    pub empfaenger: String,
    pub absender: String,
    /// Verzeichniseintrag, dessen Stand die Mitgliedschaft festlegt.
    #[serde(with = "zaehler")]
    pub verzeichnis_nummer: u64,
    #[serde(with = "hexfeld")]
    pub verzeichnis_hash: [u8; 32],
    /// Fluechtiger X25519-Schluessel des Absenders.
    #[serde(with = "hexfeld")]
    pub efemer: [u8; 32],
    #[serde(with = "hexfeld")]
    pub nonce: [u8; 24],
    pub chiffrat: String,
    #[serde(with = "hexfeld")]
    pub signatur: [u8; 64],
}

fn feld(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

/// Groesste Transportform eines Schluesselpakets in Bytes (vor dem Lesen geprueft).
pub const MAX_PAKET_JSON: usize = 2048;

impl Schluesselpaket {
    /// Liest ein Paket aus der Transportform; Rohlaenge vor der Deserialisierung begrenzt (E3.8).
    pub fn aus_json(roh: &[u8]) -> Result<Self, SyncFehler> {
        begrenzt_lesen(roh, MAX_PAKET_JSON)
    }

    fn aad(&self, empfaenger_austausch: &[u8; 32]) -> Vec<u8> {
        let mut m = AAD_KENNUNG.to_vec();
        feld(&mut m, self.konto.as_str().as_bytes());
        feld(&mut m, &self.epoche.to_be_bytes());
        feld(&mut m, self.empfaenger.as_bytes());
        feld(&mut m, empfaenger_austausch);
        feld(&mut m, self.absender.as_bytes());
        feld(&mut m, &self.verzeichnis_nummer.to_be_bytes());
        feld(&mut m, &self.verzeichnis_hash);
        feld(&mut m, &self.efemer);
        m
    }

    fn signaturbytes(&self, aad: &[u8]) -> Vec<u8> {
        let mut m = SIG_KENNUNG.to_vec();
        feld(&mut m, aad);
        feld(&mut m, &self.nonce);
        feld(&mut m, self.chiffrat.as_bytes());
        m
    }
}

/// Weist ein nicht beitragendes X25519-Ergebnis (alles Null, z. B. Punkt kleiner Ordnung als
/// Gegenschluessel) vor der HKDF ab, in beide Richtungen (E3.8, Must-fix 7). Der Test laeuft in konstanter Zeit.
fn beitragend(s: x25519_dalek::SharedSecret) -> Result<Zeroizing<[u8; 32]>, SyncFehler> {
    if !s.was_contributory() {
        return Err(SyncFehler::Paket("schluesseltausch ohne beitrag"));
    }
    Ok(Zeroizing::new(s.to_bytes()))
}

fn kdf(geteilt: &[u8; 32], efemer: &[u8; 32], empfaenger: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let mut info = HKDF_INFO.to_vec();
    info.extend_from_slice(efemer);
    info.extend_from_slice(empfaenger);
    let mut okm = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(None, geteilt)
        .expand(&info, okm.as_mut())
        .expect("32 Byte sind eine gueltige HKDF-Laenge");
    okm
}

/// Wrappt `schluessel` (Epoche `epoche`) und `salz` fuer `empfaenger`, gebunden an den aktuellen
/// Verzeichnisstand. Der Empfaenger muss dort zugelassen sein; Grundlage ist nur das geprufte Verzeichnis.
pub(crate) fn wrappen(
    verzeichnis: &Verzeichnis,
    absender: &Geraeteschluessel,
    empfaenger: &Ausweis,
    epoche: u64,
    schluessel: &Schluessel,
    salz: &[u8; 32],
) -> Result<Schluesselpaket, SyncFehler> {
    let stand = verzeichnis.stand();
    let gepinnt = stand
        .mitglied(&empfaenger.geraet_id)
        .filter(|m| m.zugelassen() && &m.ausweis == empfaenger)
        .ok_or(SyncFehler::Verzeichnis("empfaenger nicht zugelassen"))?;
    if !stand.zugelassen(absender.geraet_id()) {
        return Err(SyncFehler::Verzeichnis("absender nicht zugelassen"));
    }
    if epoche == 0 || epoche > stand.epoche {
        return Err(SyncFehler::EpocheUnbekannt(epoche));
    }
    let eph_geheim = StaticSecret::from(*Zeroizing::new(zufall::<32>()?));
    let efemer = PublicKey::from(&eph_geheim).to_bytes();
    let geteilt =
        beitragend(eph_geheim.diffie_hellman(&PublicKey::from(gepinnt.ausweis.austausch)))?;
    let key = kdf(&geteilt, &efemer, &gepinnt.ausweis.austausch);
    let mut p = Schluesselpaket {
        konto: verzeichnis.konto().clone(),
        epoche,
        empfaenger: empfaenger.geraet_id.clone(),
        absender: absender.geraet_id().to_owned(),
        verzeichnis_nummer: stand.laenge - 1,
        verzeichnis_hash: stand.kopf,
        efemer,
        nonce: zufall::<24>()?,
        chiffrat: String::new(),
        signatur: [0; 64],
    };
    let aad = p.aad(&gepinnt.ausweis.austausch);
    let mut klar = Zeroizing::new(Vec::with_capacity(KLARTEXT_LAENGE));
    klar.extend_from_slice(salz);
    klar.extend_from_slice(schluessel.bytes());
    let ct = XChaCha20Poly1305::new(key.as_ref().into())
        .encrypt(
            XNonce::from_slice(&p.nonce),
            Payload {
                msg: &klar,
                aad: &aad,
            },
        )
        .map_err(|_| SyncFehler::Verschluesseln)?;
    p.chiffrat = B64.encode(ct);
    p.signatur = absender.signieren(&p.signaturbytes(&aad));
    Ok(p)
}

/// Geprueftes, entpacktes Paket.
pub(crate) struct Entpackt {
    pub epoche: u64,
    pub salz: Zeroizing<[u8; 32]>,
    pub schluessel: Schluessel,
}

/// Prueft und entpackt ein Paket fuer `ich`.
///
/// Bedingungen: eigenes Konto und eigener Empfaengername; der Verzeichnisstand `verzeichnis_nummer` ist
/// lokal bekannt und hat den angegebenen Hash; dort sind Absender und Empfaenger (mit genau dem eigenen
/// Austauschschluessel) zugelassen und die Epoche existiert; die Signatur des Absenders stimmt.
/// `Ok(None)`: Verzeichnis lokal noch zu kurz (spaeter erneut versuchen).
pub(crate) fn entpacken(
    verzeichnis: &Verzeichnis,
    ich: &Geraeteschluessel,
    p: &Schluesselpaket,
) -> Result<Option<Entpackt>, SyncFehler> {
    if &p.konto != verzeichnis.konto() || p.empfaenger != ich.geraet_id() {
        return Err(SyncFehler::Paket("konto oder empfaenger passt nicht"));
    }
    if p.chiffrat.len() > CHIFFRAT_B64_MAX {
        return Err(SyncFehler::Paket("chiffrat zu lang"));
    }
    if p.verzeichnis_nummer >= verzeichnis.laenge() {
        return Ok(None);
    }
    let stand = verzeichnis
        .stand_bei(p.verzeichnis_nummer)
        .ok_or(SyncFehler::Paket("verzeichnisstand"))?;
    if stand.kopf != p.verzeichnis_hash {
        return Err(SyncFehler::Paket("verzeichnishash passt nicht"));
    }
    let eigen = ich.ausweis();
    let empfaenger_ok = stand
        .mitglied(&eigen.geraet_id)
        .is_some_and(|m| m.zugelassen() && m.ausweis == eigen);
    let absender = stand
        .mitglied(&p.absender)
        .filter(|m| m.zugelassen())
        .ok_or(SyncFehler::Paket("absender nicht zugelassen"))?;
    if !empfaenger_ok {
        return Err(SyncFehler::Paket("empfaenger nicht zugelassen"));
    }
    if p.epoche == 0 || p.epoche > stand.epoche {
        return Err(SyncFehler::Paket("epoche existiert in diesem stand nicht"));
    }
    let aad = p.aad(&eigen.austausch);
    if !absender
        .ausweis
        .pruefe_signatur(&p.signaturbytes(&aad), &p.signatur)
    {
        return Err(SyncFehler::Paket("signatur ungueltig"));
    }
    let geteilt = beitragend(ich.austausch().diffie_hellman(&PublicKey::from(p.efemer)))?;
    let key = kdf(&geteilt, &p.efemer, &eigen.austausch);
    let ct = B64
        .decode(&p.chiffrat)
        .map_err(|_| SyncFehler::Paket("chiffrat"))?;
    let klar = Zeroizing::new(
        XChaCha20Poly1305::new(key.as_ref().into())
            .decrypt(
                XNonce::from_slice(&p.nonce),
                Payload {
                    msg: &ct,
                    aad: &aad,
                },
            )
            .map_err(|_| SyncFehler::Paket("entschluesseln"))?,
    );
    if klar.len() != KLARTEXT_LAENGE {
        return Err(SyncFehler::Paket("klartextlaenge"));
    }
    let mut salz = Zeroizing::new([0u8; 32]);
    salz.copy_from_slice(&klar[..32]);
    let mut k = Zeroizing::new([0u8; 32]);
    k.copy_from_slice(&klar[32..]);
    Ok(Some(Entpackt {
        epoche: p.epoche,
        salz,
        schluessel: Schluessel::aus_bytes(*k),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aufbau() -> (Verzeichnis, Geraeteschluessel, Geraeteschluessel) {
        let pc = Geraeteschluessel::erzeugen("pc").unwrap();
        let ip = Geraeteschluessel::erzeugen("iphone").unwrap();
        let mut v = Verzeichnis::gruenden(&pc);
        v.anhaengen(v.aufnahme_vorschlagen(ip.ausweis(), &pc).unwrap())
            .unwrap();
        (v, pc, ip)
    }

    #[test]
    fn schluesseltausch_mit_nullpunkt_wird_in_beide_richtungen_abgewiesen() {
        let (mut v, pc, ip) = aufbau();
        let k = Schluessel::aus_bytes([7; 32]);
        // Empfangsrichtung: der Hub setzt einen Punkt kleiner Ordnung als fluechtigen Schluessel ein.
        let mut p = wrappen(&v, &pc, &ip.ausweis(), 1, &k, &[3; 32]).unwrap();
        p.efemer = [0; 32];
        assert_eq!(
            entpacken(&v, &ip, &p).err(),
            Some(SyncFehler::Paket("signatur ungueltig")),
            "eine Faelschung scheitert schon an der Signatur"
        );
        p.signatur = pc.signieren(&p.signaturbytes(&p.aad(&ip.ausweis().austausch)));
        assert_eq!(
            entpacken(&v, &ip, &p).err(),
            Some(SyncFehler::Paket("schluesseltausch ohne beitrag")),
            "auch ein gueltig signierter Nullpunkt erreicht die HKDF nicht"
        );
        // Senderichtung: ein Empfaengerausweis mit Nullpunkt als Austauschschluessel.
        let mut null = Geraeteschluessel::erzeugen("null").unwrap().ausweis();
        null.austausch = [0; 32];
        v.anhaengen(v.aufnahme_vorschlagen(null.clone(), &pc).unwrap())
            .unwrap();
        assert_eq!(
            wrappen(&v, &pc, &null, 1, &k, &[3; 32]).err(),
            Some(SyncFehler::Paket("schluesseltausch ohne beitrag"))
        );
    }

    #[test]
    fn rundlauf() {
        let (v, pc, ip) = aufbau();
        let k = Schluessel::aus_bytes([7; 32]);
        let p = wrappen(&v, &pc, &ip.ausweis(), 1, &k, &[3; 32]).unwrap();
        let e = entpacken(&v, &ip, &p).unwrap().unwrap();
        assert_eq!(
            (e.epoche, *e.salz, *e.schluessel.bytes()),
            (1, [3; 32], [7; 32])
        );
        assert!(entpacken(&v, &pc, &p).is_err(), "falscher Empfaenger");
    }

    #[test]
    fn hub_kann_weder_faelschen_noch_umbinden() {
        let (v, pc, ip) = aufbau();
        let p = wrappen(
            &v,
            &pc,
            &ip.ausweis(),
            1,
            &Schluessel::aus_bytes([7; 32]),
            &[3; 32],
        )
        .unwrap();
        // Hub verschluesselt selbst an den oeffentlichen Austauschschluessel und gibt sich als pc aus.
        let hub = Geraeteschluessel::erzeugen("pc").unwrap();
        let mut v_hub = Verzeichnis::gruenden(&hub);
        v_hub
            .anhaengen(v_hub.aufnahme_vorschlagen(ip.ausweis(), &hub).unwrap())
            .unwrap();
        let mut f = wrappen(
            &v_hub,
            &hub,
            &ip.ausweis(),
            1,
            &Schluessel::aus_bytes([9; 32]),
            &[3; 32],
        )
        .unwrap();
        f.konto = v.konto().clone();
        f.verzeichnis_hash = p.verzeichnis_hash;
        assert!(entpacken(&v, &ip, &f).is_err());
        for m in [
            |p: &mut Schluesselpaket| p.epoche = 2,
            |p: &mut Schluesselpaket| p.verzeichnis_nummer = 0,
            |p: &mut Schluesselpaket| p.verzeichnis_hash = [0; 32],
            |p: &mut Schluesselpaket| p.absender = "iphone".into(),
        ] {
            let mut x = p.clone();
            m(&mut x);
            assert!(entpacken(&v, &ip, &x).is_err());
        }
        let mut spaeter = p.clone();
        spaeter.verzeichnis_nummer = 5;
        assert!(entpacken(&v, &ip, &spaeter).unwrap().is_none());
    }
}
