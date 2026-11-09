//! Umschlag: ein Sync-Objekt, AEAD-verschluesselt, mit kryptografisch gebundenen Steuerfeldern (E3.0) und
//! signiertem Klartext (E3.8, Protokoll v2).
//!
//! Verfahren: XChaCha20-Poly1305 (`chacha20poly1305` 0.10, zufaellige 24-Byte-Nonce je Versiegelung). Die AAD
//! ist die kanonische Folge `schema_version, sammlung, objekt_id, unterhaltung_ref, art, geraet_id,
//! key_epoch[, ziel.typ, ziel.wert]`, jedes Feld mit 4-Byte-Laengenpraefix (Big Endian) hinter der Kennung
//! `wai-assistent-sync/aad/v2`. `schema_version` muss exakt [`SCHEMA_VERSION`] sein.
//!
//! Klartext (Rahmen v2): `WAS2 || u32 laenge || kern || signatur(64)`. Der Kern (JSON) wiederholt die
//! Steuerfelder und traegt den Schreibzaehler des Autors, bei veraenderlichen Objekten das Kettenglied
//! (`version`, `vorgaenger`) und den Inhalt. Die Ed25519-Signatur des Autors (`geraet_id`, Schluessel aus dem
//! Geraeteverzeichnis) deckt AAD und Kern. Wer nur den Epochenschluessel kennt (etwa ein widerrufenes
//! Geraet), kann deshalb kein Objekt unter fremdem Namen erzeugen. Die Hub-`revision` ist nicht gebunden
//! und hat keine Sicherheitsbedeutung.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroizing;

use super::fehler::SyncFehler;
use super::identitaet::Geraeteschluessel;
use super::kennung::{Hex128, begrenzt_lesen, hash_teile, hexfeld, json_geheim, zufall};
use super::schluessel::Schluesselbund;
use super::verzeichnis::Verzeichnis;

/// Vertragsversion der Kernvertraege, an die dieser Kern gebunden ist (exakt geprueft, in der AAD).
pub const SCHEMA_VERSION: &str = "0.1.6";

const NONCE_LAENGE: usize = 24;
/// Base64-Laenge einer 24-Byte-Nonce (ohne Auffuellung).
const NONCE_B64_LAENGE: usize = 32;
const TAG_LAENGE: usize = 16;
const SIG_LAENGE: usize = 64;
const MAX_CIPHERTEXT_B64: usize = 8_388_608;
/// Groesste Transportform eines Umschlags in Bytes (Chiffrat plus Steuerfelder), vor dem Lesen geprueft.
pub const MAX_UMSCHLAG_JSON: usize = MAX_CIPHERTEXT_B64 + 4096;
/// Groesster Klartext, dessen Chiffrat noch in [`MAX_CIPHERTEXT_B64`] passt.
pub const MAX_KLARTEXT: usize = MAX_CIPHERTEXT_B64 / 4 * 3 - TAG_LAENGE;
const AAD_KENNUNG: &[u8] = b"wai-assistent-sync/aad/v2";
const SIG_KENNUNG: &[u8] = b"wai-assistent-sync/objekt-signatur/v2";
const GLIED_KENNUNG: &[u8] = b"wai-assistent-sync/glied/v2";
const RAHMEN: &[u8; 4] = b"WAS2";

/// Hub-Sammlung (`sammlung`), zugleich Wertebereich von `art`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sammlung {
    Unterhaltung,
    Beitrag,
    Auftragsereignis,
    Hinweis,
    Sitzung,
    Loeschmarke,
}

impl Sammlung {
    pub fn name(self) -> &'static str {
        match self {
            Self::Unterhaltung => "unterhaltung",
            Self::Beitrag => "beitrag",
            Self::Auftragsereignis => "auftragsereignis",
            Self::Hinweis => "hinweis",
            Self::Sitzung => "sitzung",
            Self::Loeschmarke => "loeschmarke",
        }
    }

    /// Nur `unterhaltung` (Kopf) und `sitzung` (Lease) sind veraenderlich, und nur per Vergleichsschreiben.
    pub fn veraenderlich(self) -> bool {
        matches!(self, Self::Unterhaltung | Self::Sitzung)
    }
}

/// Art des Grabsteinziels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZielTyp {
    Unterhaltung,
    Objekt,
}

impl ZielTyp {
    fn name(self) -> &'static str {
        match self {
            Self::Unterhaltung => "unterhaltung",
            Self::Objekt => "objekt",
        }
    }
}

/// Ziel einer Loeschmarke: `unterhaltung_ref` oder `objekt_id`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ziel {
    pub typ: ZielTyp,
    pub wert: Hex128,
}

/// Innere `schema_version` (Runde 8): beim Deserialisieren exakt mit [`SCHEMA_VERSION`] verglichen. Damit
/// prueft jede Stelle, die einen inneren Klartext liest, die Version an genau einer Stelle mit.
pub(crate) fn schema_exakt<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    use serde::de::Error;
    let s = String::deserialize(d)?;
    if s != SCHEMA_VERSION {
        return Err(D::Error::custom("innere schema_version nicht unterstuetzt"));
    }
    Ok(s)
}

/// Zaehler als Dezimalstring (Vertrag `Int64String`), intern `u64`.
pub(crate) mod zaehler {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        let s = String::deserialize(d)?;
        let kanonisch = s == "0"
            || (!s.starts_with('0') && !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
        if !kanonisch {
            return Err(D::Error::custom("Zaehler muss kanonische Dezimalzahl sein"));
        }
        s.parse().map_err(D::Error::custom)
    }
}

pub(crate) mod zaehler_opt {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => super::zaehler::serialize(v, s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        #[derive(Deserialize)]
        struct W(#[serde(with = "super::zaehler")] u64);
        Ok(Option::<W>::deserialize(d)?.map(|w| w.0))
    }
}

/// Die sichtbaren Steuerfelder (E3.0) ohne `revision`; genau die gebundenen Werte.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Steuerfelder {
    pub sammlung: Sammlung,
    pub objekt_id: Hex128,
    pub unterhaltung_ref: Hex128,
    pub art: Sammlung,
    pub geraet_id: String,
    #[serde(with = "zaehler")]
    pub key_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ziel: Option<Ziel>,
}

impl Steuerfelder {
    /// Steuerfelder eines Inhaltsobjekts (`art` = `sammlung`, kein Ziel).
    pub fn inhalt(
        sammlung: Sammlung,
        objekt_id: Hex128,
        unterhaltung_ref: Hex128,
        geraet_id: &str,
        key_epoch: u64,
    ) -> Self {
        Self {
            sammlung,
            objekt_id,
            unterhaltung_ref,
            art: sammlung,
            geraet_id: geraet_id.to_owned(),
            key_epoch,
            ziel: None,
        }
    }

    /// Steuerfelder einer Loeschmarke; `unterhaltung_ref` ist bei Objektzielen die der Unterhaltung des Ziels.
    pub fn loeschmarke(
        objekt_id: Hex128,
        unterhaltung_ref: Hex128,
        geraet_id: &str,
        key_epoch: u64,
        ziel: Ziel,
    ) -> Self {
        Self {
            sammlung: Sammlung::Loeschmarke,
            objekt_id,
            unterhaltung_ref,
            art: Sammlung::Loeschmarke,
            geraet_id: geraet_id.to_owned(),
            key_epoch,
            ziel: Some(ziel),
        }
    }

    /// Strukturpruefung der Steuerfelder (Vertragsregeln, die das Schema nicht ausdrueckt).
    pub fn pruefen(&self) -> Result<(), SyncFehler> {
        if self.art != self.sammlung {
            return Err(SyncFehler::Vertrag("art muss der sammlung entsprechen"));
        }
        if self.geraet_id.is_empty() || self.geraet_id.len() > 128 {
            return Err(SyncFehler::Vertrag(
                "geraet_id muss 1 bis 128 Zeichen haben",
            ));
        }
        match (&self.ziel, self.sammlung == Sammlung::Loeschmarke) {
            (None, true) => Err(SyncFehler::Vertrag("loeschmarke braucht ein ziel")),
            (Some(_), false) => Err(SyncFehler::Vertrag("nur loeschmarke hat ein ziel")),
            (Some(z), true)
                if z.typ == ZielTyp::Unterhaltung && z.wert != self.unterhaltung_ref =>
            {
                Err(SyncFehler::Vertrag("ziel.wert muss unterhaltung_ref sein"))
            }
            _ => Ok(()),
        }
    }

    /// Kanonische AAD-Kodierung (siehe Moduldoku).
    pub fn aad(&self) -> Result<Vec<u8>, SyncFehler> {
        self.pruefen()?;
        let epoche = self.key_epoch.to_string();
        let mut felder: Vec<&str> = vec![
            self.sammlung.name(),
            self.objekt_id.as_str(),
            self.unterhaltung_ref.as_str(),
            self.art.name(),
            &self.geraet_id,
            &epoche,
        ];
        if let Some(z) = &self.ziel {
            felder.push(z.typ.name());
            felder.push(z.wert.as_str());
        }
        let mut aad = AAD_KENNUNG.to_vec();
        for f in std::iter::once(SCHEMA_VERSION).chain(felder) {
            aad.extend_from_slice(&(f.len() as u32).to_be_bytes());
            aad.extend_from_slice(f.as_bytes());
        }
        Ok(aad)
    }
}

/// Hub-Objekt (Vertrag `assistent-sync-object`): sichtbare Steuerfelder und verschluesselter Inhalt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Umschlag {
    pub schema_version: String,
    pub sammlung: Sammlung,
    pub objekt_id: Hex128,
    pub unterhaltung_ref: Hex128,
    pub art: Sammlung,
    pub geraet_id: String,
    #[serde(with = "zaehler")]
    pub key_epoch: u64,
    /// Vom Hub nach dem Schreiben vergeben; nur fuer das Vergleichsschreiben, nie ein Fencing-Wert.
    #[serde(default, with = "zaehler_opt", skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    pub ciphertext: String,
    pub nonce: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ziel: Option<Ziel>,
}

impl Umschlag {
    /// Die sichtbaren Steuerfelder ohne `revision`.
    pub fn steuerfelder(&self) -> Steuerfelder {
        Steuerfelder {
            sammlung: self.sammlung,
            objekt_id: self.objekt_id.clone(),
            unterhaltung_ref: self.unterhaltung_ref.clone(),
            art: self.art,
            geraet_id: self.geraet_id.clone(),
            key_epoch: self.key_epoch,
            ziel: self.ziel.clone(),
        }
    }

    /// Vertragspruefung des Umschlags (Form, ohne Schluessel); auch der Hub ruft sie auf.
    pub fn pruefen(&self) -> Result<(), SyncFehler> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(SyncFehler::Vertrag("schema_version nicht unterstuetzt"));
        }
        self.steuerfelder().pruefen()?;
        if self.ciphertext.is_empty() || self.ciphertext.len() > MAX_CIPHERTEXT_B64 {
            return Err(SyncFehler::Vertrag("ciphertext hat unzulaessige Laenge"));
        }
        // Laenge vor dem Dekodieren begrenzen: 24 Byte sind genau 32 Base64-Zeichen.
        if self.nonce.len() != NONCE_B64_LAENGE {
            return Err(SyncFehler::Vertrag("nonce hat falsche Laenge"));
        }
        let nonce = B64
            .decode(&self.nonce)
            .map_err(|_| SyncFehler::Vertrag("nonce ist kein Base64"))?;
        if nonce.len() != NONCE_LAENGE {
            return Err(SyncFehler::Vertrag("nonce hat falsche Laenge"));
        }
        let ct = B64
            .decode(&self.ciphertext)
            .map_err(|_| SyncFehler::Vertrag("ciphertext ist kein Base64"))?;
        if ct.len() < TAG_LAENGE {
            return Err(SyncFehler::Vertrag("ciphertext kuerzer als das Auth-Tag"));
        }
        Ok(())
    }

    /// Liest einen Umschlag aus der Transportform (JSON). Die Rohlaenge wird **vor** der Deserialisierung
    /// gegen [`MAX_UMSCHLAG_JSON`] geprueft, danach die Vertragsform ([`Umschlag::pruefen`]). Der Transport
    /// liest jedes Objekt ueber diese Funktion, nie direkt per `serde_json`.
    pub fn aus_json(roh: &[u8]) -> Result<Self, SyncFehler> {
        let u: Self = begrenzt_lesen(roh, MAX_UMSCHLAG_JSON)?;
        u.pruefen()?;
        Ok(u)
    }

    /// Liest eine Leseantwort (JSON-Liste) mit hoechstens `max_anzahl` Objekten; die Rohlaenge ist vor dem
    /// Lesen auf `max_anzahl * MAX_UMSCHLAG_JSON` begrenzt, jedes Objekt wird danach geprueft.
    pub fn liste_aus_json(roh: &[u8], max_anzahl: usize) -> Result<Vec<Self>, SyncFehler> {
        let grenze = max_anzahl
            .checked_mul(MAX_UMSCHLAG_JSON)
            .ok_or(SyncFehler::Vertrag("transportgrenze ueberschritten"))?;
        let v: Vec<Self> = begrenzt_lesen(roh, grenze)?;
        if v.len() > max_anzahl {
            return Err(SyncFehler::Vertrag("transportgrenze ueberschritten"));
        }
        v.iter().try_for_each(Self::pruefen)?;
        Ok(v)
    }

    /// Kopie mit gesetzter Hub-Revision.
    pub fn mit_revision(&self, revision: u64) -> Self {
        Self {
            revision: Some(revision),
            ..self.clone()
        }
    }
}

/// Kettenglied eines veraenderlichen Objekts (Kopf, Lease): authentifizierte Zustandsversion (E3.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Glied {
    #[serde(with = "zaehler")]
    pub version: u64,
    /// Inhaltshash der Vorgaengerversion (Nullen bei Version 1).
    #[serde(with = "hexfeld")]
    pub vorgaenger: [u8; 32],
}

/// Signierter Kern im Chiffrat.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Kern {
    steuer: Steuerfelder,
    /// Schreibzaehler des Autors (je Geraet streng steigend).
    #[serde(with = "zaehler")]
    zaehler: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kette: Option<Glied>,
    inhalt: Value,
}

/// Erfolgreich geoeffnetes, signaturgeprueftes Objekt.
#[derive(Clone, PartialEq)]
pub struct Geoeffnet {
    pub steuer: Steuerfelder,
    pub revision: Option<u64>,
    /// Schreibzaehler des Autors (`steuer.geraet_id`).
    pub zaehler: u64,
    pub kette: Option<Glied>,
    /// Inhaltshash (AAD und Kern, ohne Nonce): Vorgaengerwert der naechsten Version.
    pub hash: [u8; 32],
    pub inhalt: Value,
}

impl std::fmt::Debug for Geoeffnet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Geoeffnet")
            .field("steuer", &self.steuer)
            .field("revision", &self.revision)
            .field("zaehler", &self.zaehler)
            .field("kette", &self.kette)
            .field("inhalt", &"<verborgen>")
            .finish()
    }
}

fn feld(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

/// Signierte Nachricht. Sie enthaelt den Klartextkern und liegt deshalb in einem vorab bemessenen,
/// beim Freigeben ueberschriebenen Puffer (keine ungeschuetzte Kopie, kein Umkopieren beim Wachsen).
fn signaturbytes(aad: &[u8], kern: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut m = Zeroizing::new(Vec::with_capacity(
        SIG_KENNUNG.len() + 8 + aad.len() + kern.len(),
    ));
    m.extend_from_slice(SIG_KENNUNG);
    feld(&mut m, aad);
    feld(&mut m, kern);
    m
}

/// Verschluesselt `inhalt` unter dem Schluessel der Epoche `steuer.key_epoch` und signiert als `autor`.
pub fn versiegeln(
    bund: &Schluesselbund,
    autor: &Geraeteschluessel,
    zaehler: u64,
    kette: Option<Glied>,
    steuer: &Steuerfelder,
    inhalt: &Value,
) -> Result<Umschlag, SyncFehler> {
    if steuer.geraet_id != autor.geraet_id() {
        return Err(SyncFehler::Vertrag("geraet_id muss der Autor sein"));
    }
    let schluessel = bund
        .schluessel(steuer.key_epoch)
        .ok_or(SyncFehler::EpocheUnbekannt(steuer.key_epoch))?;
    let aad = steuer.aad()?;
    // Klartextkern ohne wachsenden Puffer (Runde 3, Must-fix 7).
    let kern = json_geheim(&Kern {
        steuer: steuer.clone(),
        zaehler,
        kette,
        inhalt: inhalt.clone(),
    })
    .map_err(|_| SyncFehler::Inhalt)?;
    let gesamt = RAHMEN.len() + 4 + kern.len() + SIG_LAENGE;
    if gesamt > MAX_KLARTEXT || u32::try_from(kern.len()).is_err() {
        return Err(SyncFehler::Vertrag("inhalt zu gross"));
    }
    let signatur = autor.signieren(&signaturbytes(&aad, &kern));
    let mut klartext = Zeroizing::new(Vec::with_capacity(gesamt));
    klartext.extend_from_slice(RAHMEN);
    klartext.extend_from_slice(&(kern.len() as u32).to_be_bytes());
    klartext.extend_from_slice(&kern);
    klartext.extend_from_slice(&signatur);
    let nonce = zufall::<NONCE_LAENGE>()?;
    let ciphertext = XChaCha20Poly1305::new(schluessel.bytes().into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &klartext,
                aad: &aad,
            },
        )
        .map_err(|_| SyncFehler::Verschluesseln)?;
    Ok(Umschlag {
        schema_version: SCHEMA_VERSION.to_owned(),
        sammlung: steuer.sammlung,
        objekt_id: steuer.objekt_id.clone(),
        unterhaltung_ref: steuer.unterhaltung_ref.clone(),
        art: steuer.art,
        geraet_id: steuer.geraet_id.clone(),
        key_epoch: steuer.key_epoch,
        revision: None,
        ciphertext: B64.encode(ciphertext),
        nonce: B64.encode(nonce),
        ziel: steuer.ziel.clone(),
    })
}

/// Entschluesselt gegen die AAD der sichtbaren Felder, ohne Strukturpruefung des Umschlags.
fn entschluesseln(bund: &Schluesselbund, u: &Umschlag) -> Result<Zeroizing<Vec<u8>>, SyncFehler> {
    let schluessel = bund
        .schluessel(u.key_epoch)
        .ok_or(SyncFehler::EpocheUnbekannt(u.key_epoch))?;
    let aad = u.steuerfelder().aad().map_err(|_| SyncFehler::Oeffnen)?;
    if u.nonce.len() != NONCE_B64_LAENGE || u.ciphertext.len() > MAX_CIPHERTEXT_B64 {
        return Err(SyncFehler::Oeffnen);
    }
    let nonce = B64.decode(&u.nonce).map_err(|_| SyncFehler::Oeffnen)?;
    let ct = B64.decode(&u.ciphertext).map_err(|_| SyncFehler::Oeffnen)?;
    if nonce.len() != NONCE_LAENGE {
        return Err(SyncFehler::Oeffnen);
    }
    XChaCha20Poly1305::new(schluessel.bytes().into())
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ct,
                aad: &aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| SyncFehler::Oeffnen)
}

/// Prueft, entschluesselt, prueft die Signatur des Autors und vergleicht die sichtbaren Felder.
///
/// Scheitert bei vertauschter oder geaenderter ID, Unterhaltung, Art, Geraet, Epoche, Schemaversion oder
/// Grabsteinziel, bei fremdem Schluessel, manipuliertem Chiffrat und fremder Signatur. Ein im Verzeichnis
/// unbekannter Autor ergibt [`SyncFehler::AutorUnbekannt`] (Verzeichnis evtl. noch nicht aktuell). Ob ein
/// widerrufener Autor zaehlt, entscheidet der Aufrufer anhand von `zaehler` und Epoche.
pub fn oeffnen(
    bund: &Schluesselbund,
    verzeichnis: &Verzeichnis,
    u: &Umschlag,
) -> Result<Geoeffnet, SyncFehler> {
    u.pruefen()?;
    let autor = verzeichnis
        .stand()
        .mitglied(&u.geraet_id)
        .map(|m| m.ausweis.clone())
        .ok_or(SyncFehler::AutorUnbekannt)?;
    let klartext = entschluesseln(bund, u)?;
    let kern_laenge = klartext
        .get(4..8)
        .filter(|_| klartext.starts_with(RAHMEN))
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or(SyncFehler::Inhalt)?;
    if Some(klartext.len()) != kern_laenge.checked_add(8 + SIG_LAENGE) {
        return Err(SyncFehler::Inhalt);
    }
    let kern = &klartext[8..8 + kern_laenge];
    let mut signatur = [0u8; SIG_LAENGE];
    signatur.copy_from_slice(&klartext[8 + kern_laenge..]);
    let aad = u.steuerfelder().aad()?;
    if !autor.pruefe_signatur(&signaturbytes(&aad, kern), &signatur) {
        return Err(SyncFehler::Signatur);
    }
    let k: Kern = serde_json::from_slice(kern).map_err(|_| SyncFehler::Inhalt)?;
    if k.steuer != u.steuerfelder() {
        return Err(SyncFehler::SteuerfelderUngleich);
    }
    Ok(Geoeffnet {
        steuer: k.steuer,
        revision: u.revision,
        zaehler: k.zaehler,
        kette: k.kette,
        hash: hash_teile(GLIED_KENNUNG, &[&aad, kern]),
        inhalt: k.inhalt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::schluessel::Schluessel;
    use serde_json::json;

    fn hex(n: u8) -> Hex128 {
        Hex128::parse(&format!("{n:02x}").repeat(16)).unwrap()
    }

    struct Aufbau {
        bund: Schluesselbund,
        pc: Geraeteschluessel,
        ip: Geraeteschluessel,
        v: Verzeichnis,
    }

    fn aufbau() -> Aufbau {
        let mut bund = Schluesselbund::neu(
            Schluessel::aus_bytes([1; 32]),
            1,
            Schluessel::aus_bytes([2; 32]),
        );
        // Gleiches Schluesselmaterial in Epoche 2: nur die AAD kann die Epoche noch binden.
        bund.epoche_aufnehmen(2, Schluessel::aus_bytes([2; 32]))
            .unwrap();
        let pc = Geraeteschluessel::erzeugen("dev-pc").unwrap();
        let ip = Geraeteschluessel::erzeugen("dev-iphone").unwrap();
        let mut v = Verzeichnis::gruenden(&pc);
        v.anhaengen(v.aufnahme_vorschlagen(ip.ausweis(), &pc).unwrap())
            .unwrap();
        Aufbau { bund, pc, ip, v }
    }

    fn steuer() -> Steuerfelder {
        Steuerfelder::inhalt(Sammlung::Beitrag, hex(1), hex(2), "dev-pc", 1)
    }

    fn grab() -> Steuerfelder {
        Steuerfelder::loeschmarke(
            hex(3),
            hex(2),
            "dev-pc",
            1,
            Ziel {
                typ: ZielTyp::Objekt,
                wert: hex(4),
            },
        )
    }

    fn siegel(a: &Aufbau, s: &Steuerfelder, inhalt: &Value) -> Umschlag {
        versiegeln(&a.bund, &a.pc, 1, None, s, inhalt).unwrap()
    }

    #[test]
    fn rundlauf_mit_signatur_zaehler_und_glied() {
        let a = aufbau();
        let glied = Glied {
            version: 3,
            vorgaenger: [5; 32],
        };
        let u = versiegeln(
            &a.bund,
            &a.pc,
            9,
            Some(glied),
            &steuer(),
            &json!({"text": "Hallo"}),
        )
        .unwrap();
        u.pruefen().unwrap();
        let g = oeffnen(&a.bund, &a.v, &u).unwrap();
        assert_eq!(g.inhalt, json!({"text": "Hallo"}));
        assert_eq!(g.steuer, steuer());
        assert_eq!((g.zaehler, g.kette), (9, Some(glied)));
        let erneut = versiegeln(
            &a.bund,
            &a.pc,
            9,
            Some(glied),
            &steuer(),
            &json!({"text": "Hallo"}),
        )
        .unwrap();
        assert_eq!(
            oeffnen(&a.bund, &a.v, &erneut).unwrap().hash,
            g.hash,
            "Inhaltshash haengt nicht an der Nonce"
        );
    }

    #[test]
    fn autor_muss_geraet_sein_und_fremde_signatur_scheitert() {
        let a = aufbau();
        assert!(versiegeln(&a.bund, &a.ip, 1, None, &steuer(), &json!(1)).is_err());
        // iPhone kennt den Epochenschluessel und gibt sich als PC aus: es signiert mit eigenem Schluessel.
        let falsch = Geraeteschluessel::erzeugen("dev-pc").unwrap();
        let u = versiegeln(&a.bund, &falsch, 1, None, &steuer(), &json!(1)).unwrap();
        assert_eq!(oeffnen(&a.bund, &a.v, &u), Err(SyncFehler::Signatur));
        let mut fremd = steuer();
        fremd.geraet_id = "dev-unbekannt".into();
        let x = Geraeteschluessel::erzeugen("dev-unbekannt").unwrap();
        let u = versiegeln(&a.bund, &x, 1, None, &fremd, &json!(1)).unwrap();
        assert_eq!(oeffnen(&a.bund, &a.v, &u), Err(SyncFehler::AutorUnbekannt));
    }

    #[test]
    fn debug_von_geoeffnet_zeigt_keinen_inhalt() {
        let a = aufbau();
        let u = siegel(&a, &steuer(), &json!({"text": "Geheim-4711"}));
        let s = format!("{:?}", oeffnen(&a.bund, &a.v, &u).unwrap());
        assert!(!s.contains("Geheim-4711") && s.contains("verborgen"));
    }

    #[test]
    fn nonce_ist_je_versiegelung_neu() {
        let a = aufbau();
        let x = siegel(&a, &steuer(), &json!(1));
        let y = siegel(&a, &steuer(), &json!(1));
        assert_ne!(x.nonce, y.nonce);
        assert_ne!(x.ciphertext, y.ciphertext);
    }

    #[test]
    fn aad_ist_kanonisch_und_feldweise_verschieden() {
        let a = steuer().aad().unwrap();
        assert!(a.starts_with(AAD_KENNUNG));
        assert!(
            a.windows(SCHEMA_VERSION.len())
                .any(|w| w == SCHEMA_VERSION.as_bytes()),
            "schema_version steht in der AAD"
        );
        let mut s = steuer();
        s.geraet_id = "dev-pc2".into();
        assert_ne!(a, s.aad().unwrap());
        let mut x = steuer();
        x.geraet_id = "ab".into();
        x.key_epoch = 12;
        let mut y = steuer();
        y.geraet_id = "ab1".into();
        y.key_epoch = 2;
        assert_ne!(x.aad().unwrap(), y.aad().unwrap());
        assert_ne!(a, grab().aad().unwrap());
    }

    #[test]
    fn aad_allein_bindet_jedes_feld() {
        let a = aufbau();
        let u = siegel(&a, &steuer(), &json!({"x": 1}));
        type Mutation = Box<dyn Fn(&mut Umschlag)>;
        let mutationen: Vec<(&str, Mutation)> = vec![
            ("objekt_id", Box::new(|u| u.objekt_id = hex(9))),
            (
                "unterhaltung_ref",
                Box::new(|u| u.unterhaltung_ref = hex(9)),
            ),
            ("art", Box::new(|u| u.art = Sammlung::Hinweis)),
            ("sammlung", Box::new(|u| u.sammlung = Sammlung::Hinweis)),
            ("geraet_id", Box::new(|u| u.geraet_id = "dev-iphone".into())),
            ("key_epoch", Box::new(|u| u.key_epoch = 2)),
        ];
        for (feld, mutiere) in mutationen {
            let mut m = u.clone();
            mutiere(&mut m);
            assert_eq!(
                entschluesseln(&a.bund, &m).map(|_| ()),
                Err(SyncFehler::Oeffnen),
                "AAD muss {feld} binden"
            );
            assert!(oeffnen(&a.bund, &a.v, &m).is_err(), "{feld}");
        }
        let mut alt = u.clone();
        alt.schema_version = "0.1.5".into();
        assert!(
            oeffnen(&a.bund, &a.v, &alt).is_err(),
            "schema_version exakt"
        );
    }

    #[test]
    fn aad_bindet_grabsteinziel() {
        let a = aufbau();
        let u = siegel(&a, &grab(), &json!({}));
        for ziel in [
            Ziel {
                typ: ZielTyp::Objekt,
                wert: hex(9),
            },
            Ziel {
                typ: ZielTyp::Unterhaltung,
                wert: hex(2),
            },
        ] {
            let mut m = u.clone();
            m.ziel = Some(ziel);
            assert!(entschluesseln(&a.bund, &m).is_err());
            assert!(oeffnen(&a.bund, &a.v, &m).is_err());
        }
        let mut ohne = u.clone();
        ohne.ziel = None;
        assert!(oeffnen(&a.bund, &a.v, &ohne).is_err());
    }

    #[test]
    fn kern_muss_zu_sichtbaren_feldern_passen() {
        // Gueltig signiertes Chiffrat, dessen Kern andere Steuerfelder behauptet.
        let a = aufbau();
        let echt = steuer();
        let mut falsch = steuer();
        falsch.objekt_id = hex(9);
        let aad = echt.aad().unwrap();
        let kern = serde_json::to_vec(&Kern {
            steuer: falsch,
            zaehler: 1,
            kette: None,
            inhalt: json!(1),
        })
        .unwrap();
        let sig = a.pc.signieren(&signaturbytes(&aad, &kern));
        let mut klar = RAHMEN.to_vec();
        klar.extend_from_slice(&(kern.len() as u32).to_be_bytes());
        klar.extend_from_slice(&kern);
        klar.extend_from_slice(&sig);
        let nonce = zufall::<NONCE_LAENGE>().unwrap();
        let ct = XChaCha20Poly1305::new(a.bund.schluessel(1).unwrap().bytes().into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &klar,
                    aad: &aad,
                },
            )
            .unwrap();
        let mut u = siegel(&a, &echt, &json!(0));
        u.ciphertext = B64.encode(ct);
        u.nonce = B64.encode(nonce);
        assert_eq!(
            oeffnen(&a.bund, &a.v, &u),
            Err(SyncFehler::SteuerfelderUngleich)
        );
    }

    #[test]
    fn fremder_schluessel_und_unbekannte_epoche() {
        let a = aufbau();
        let u = siegel(&a, &steuer(), &json!(1));
        let fremd = Schluesselbund::neu(
            Schluessel::aus_bytes([1; 32]),
            1,
            Schluessel::aus_bytes([5; 32]),
        );
        assert_eq!(oeffnen(&fremd, &a.v, &u), Err(SyncFehler::Oeffnen));
        let nur_alt = Schluesselbund::neu(
            Schluessel::aus_bytes([1; 32]),
            7,
            Schluessel::aus_bytes([2; 32]),
        );
        assert_eq!(
            oeffnen(&nur_alt, &a.v, &u),
            Err(SyncFehler::EpocheUnbekannt(1))
        );
    }

    #[test]
    fn manipuliertes_chiffrat_wird_abgewiesen() {
        let a = aufbau();
        let mut u = siegel(&a, &steuer(), &json!({"a": "b"}));
        let mut ct = B64.decode(&u.ciphertext).unwrap();
        ct[3] ^= 1;
        u.ciphertext = B64.encode(ct);
        assert!(oeffnen(&a.bund, &a.v, &u).is_err());
    }

    #[test]
    fn grenzen_vor_dem_dekodieren_und_vor_dem_verschluesseln() {
        let a = aufbau();
        let u = siegel(&a, &steuer(), &json!(1));
        let mut lang = u.clone();
        lang.nonce = "A".repeat(10_000);
        assert_eq!(
            lang.pruefen(),
            Err(SyncFehler::Vertrag("nonce hat falsche Laenge"))
        );
        let riesig = Value::String("x".repeat(MAX_KLARTEXT));
        assert_eq!(
            versiegeln(&a.bund, &a.pc, 1, None, &steuer(), &riesig),
            Err(SyncFehler::Vertrag("inhalt zu gross"))
        );
    }

    #[test]
    fn vertragsregeln() {
        let a = aufbau();
        let mut s = steuer();
        s.art = Sammlung::Hinweis;
        assert!(versiegeln(&a.bund, &a.pc, 1, None, &s, &json!(1)).is_err());
        let mut s = steuer();
        s.ziel = Some(Ziel {
            typ: ZielTyp::Objekt,
            wert: hex(4),
        });
        assert!(
            versiegeln(&a.bund, &a.pc, 1, None, &s, &json!(1)).is_err(),
            "ziel nur an loeschmarke"
        );
        let mut g = grab();
        g.ziel = None;
        assert!(
            versiegeln(&a.bund, &a.pc, 1, None, &g, &json!(1)).is_err(),
            "loeschmarke braucht ziel"
        );
        let mut g = grab();
        g.ziel = Some(Ziel {
            typ: ZielTyp::Unterhaltung,
            wert: hex(9),
        });
        assert!(
            versiegeln(&a.bund, &a.pc, 1, None, &g, &json!(1)).is_err(),
            "unterhaltungsziel = unterhaltung_ref"
        );
        let u = siegel(&a, &steuer(), &json!(1));
        let mut k = u.clone();
        k.nonce = B64.encode([0u8; 3]);
        assert!(k.pruefen().is_err());
        let mut k = u.clone();
        k.geraet_id = String::new();
        assert!(k.pruefen().is_err());
        for v in ["1.0.0", "0.1.7", "0.1.60", "0.1.6 "] {
            let mut k = u.clone();
            k.schema_version = v.into();
            assert!(k.pruefen().is_err(), "{v}");
        }
    }

    #[test]
    fn wire_form_entspricht_dem_vertrag() {
        let a = aufbau();
        let u = siegel(&a, &steuer(), &json!(1)).mit_revision(17);
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(v["key_epoch"], "1");
        assert_eq!(v["revision"], "17");
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
        assert!(v.get("ziel").is_none());
        let zurueck: Umschlag = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(zurueck, u);
        let mut mit_zahl = v.clone();
        mit_zahl["key_epoch"] = json!(1);
        assert!(serde_json::from_value::<Umschlag>(mit_zahl).is_err());
        let mut fremdfeld = v;
        fremdfeld["extra"] = json!(1);
        assert!(serde_json::from_value::<Umschlag>(fremdfeld).is_err());
        let g = siegel(&a, &grab(), &json!({}));
        assert_eq!(serde_json::to_value(&g).unwrap()["ziel"]["typ"], "objekt");
    }

    #[test]
    fn vertragsfixtures_lassen_sich_lesen() {
        let wurzel = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/fixtures/assistent-sync-object");
        for (ordner, soll) in [("valid", true), ("invalid", false)] {
            for eintrag in std::fs::read_dir(wurzel.join(ordner)).unwrap() {
                let pfad = eintrag.unwrap().path();
                let doc: Value = serde_json::from_slice(&std::fs::read(&pfad).unwrap()).unwrap();
                // Die Vertragsfixtures tragen Platzhalter-Chiffrat; geprueft wird die Form der Steuerfelder,
                // die exakte Schemaversion und die Nonce-Laenge (wie der Hub vor jedem Dekodieren).
                let ergebnis = serde_json::from_value::<Umschlag>(doc["data"].clone())
                    .map_err(|_| ())
                    .and_then(|u| {
                        let form =
                            u.schema_version == SCHEMA_VERSION && u.nonce.len() == NONCE_B64_LAENGE;
                        u.steuerfelder().pruefen().map_err(|_| ())?;
                        if form { Ok(()) } else { Err(()) }
                    });
                if soll {
                    assert!(ergebnis.is_ok(), "{}", pfad.display());
                } else {
                    assert!(ergebnis.is_err(), "{}", pfad.display());
                }
            }
        }
    }
}
