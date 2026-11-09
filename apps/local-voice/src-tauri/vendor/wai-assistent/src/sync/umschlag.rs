//! Umschlag: ein Sync-Objekt, AEAD-verschluesselt, mit kryptografisch gebundenen Steuerfeldern (E3.0).
//!
//! Verfahren: XChaCha20-Poly1305 (`chacha20poly1305` 0.10, Workspace-Abhaengigkeit wie in `wai-security`,
//! zufaellige 24-Byte-Nonce je Versiegelung). Die zusaetzlichen Daten (AAD) sind die kanonische Folge
//! `sammlung, objekt_id, unterhaltung_ref, art, geraet_id, key_epoch[, ziel.typ, ziel.wert]`, jedes Feld
//! mit 4-Byte-Laengenpraefix (Big Endian) hinter einer festen Versionskennung. Der Klartext im Chiffrat
//! wiederholt dieselben Werte; [`oeffnen`] verwirft jedes Objekt, dessen sichtbare Felder nicht zu AAD
//! und Klartext passen. Die Hub-`revision` ist bewusst nicht gebunden und hat keine Sicherheitsbedeutung.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::fehler::SyncFehler;
use super::kennung::{Hex128, zufall};
use super::schluessel::Schluesselbund;

/// Vertragsversion der Kernvertraege, an die dieser Kern gebunden ist.
pub const SCHEMA_VERSION: &str = "0.1.6";

const NONCE_LAENGE: usize = 24;
const TAG_LAENGE: usize = 16;
const MAX_CIPHERTEXT_B64: usize = 8_388_608;
const AAD_KENNUNG: &[u8] = b"wai-assistent-sync/aad/v1";

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
        for f in felder {
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
        if !self.schema_version.starts_with("0.1.") {
            return Err(SyncFehler::Vertrag("schema_version nicht unterstuetzt"));
        }
        self.steuerfelder().pruefen()?;
        if self.ciphertext.is_empty() || self.ciphertext.len() > MAX_CIPHERTEXT_B64 {
            return Err(SyncFehler::Vertrag("ciphertext hat unzulaessige Laenge"));
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

    /// Kopie mit gesetzter Hub-Revision.
    pub fn mit_revision(&self, revision: u64) -> Self {
        Self {
            revision: Some(revision),
            ..self.clone()
        }
    }
}

/// Klartext im Chiffrat: Wiederholung der Steuerfelder plus Inhalt.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Innen {
    steuer: Steuerfelder,
    inhalt: Value,
}

/// Erfolgreich geoeffnetes Objekt.
#[derive(Clone, PartialEq)]
pub struct Geoeffnet {
    pub steuer: Steuerfelder,
    pub revision: Option<u64>,
    pub inhalt: Value,
}

impl std::fmt::Debug for Geoeffnet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Geoeffnet")
            .field("steuer", &self.steuer)
            .field("revision", &self.revision)
            .field("inhalt", &"<verborgen>")
            .finish()
    }
}

/// Verschluesselt `inhalt` unter dem Schluessel der Epoche `steuer.key_epoch`.
pub fn versiegeln(
    bund: &Schluesselbund,
    steuer: &Steuerfelder,
    inhalt: &Value,
) -> Result<Umschlag, SyncFehler> {
    let schluessel = bund
        .schluessel(steuer.key_epoch)
        .ok_or(SyncFehler::EpocheUnbekannt(steuer.key_epoch))?;
    let aad = steuer.aad()?;
    let innen = Innen {
        steuer: steuer.clone(),
        inhalt: inhalt.clone(),
    };
    let klartext = serde_json::to_vec(&innen).map_err(|_| SyncFehler::Inhalt)?;
    let nonce = zufall::<NONCE_LAENGE>()?;
    let chiffre = XChaCha20Poly1305::new(schluessel.bytes().into());
    let ciphertext = chiffre
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
fn entschluesseln(bund: &Schluesselbund, u: &Umschlag) -> Result<Vec<u8>, SyncFehler> {
    let schluessel = bund
        .schluessel(u.key_epoch)
        .ok_or(SyncFehler::EpocheUnbekannt(u.key_epoch))?;
    let aad = u.steuerfelder().aad().map_err(|_| SyncFehler::Oeffnen)?;
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
        .map_err(|_| SyncFehler::Oeffnen)
}

/// Prueft, entschluesselt und vergleicht die sichtbaren Felder mit dem Klartext.
///
/// Scheitert bei vertauschter oder geaenderter ID, Unterhaltung, Art, Geraet, Epoche oder Grabsteinziel,
/// bei fremdem Schluessel und bei manipuliertem Chiffrat.
pub fn oeffnen(bund: &Schluesselbund, u: &Umschlag) -> Result<Geoeffnet, SyncFehler> {
    u.pruefen()?;
    let klartext = entschluesseln(bund, u)?;
    let innen: Innen = serde_json::from_slice(&klartext).map_err(|_| SyncFehler::Inhalt)?;
    if innen.steuer != u.steuerfelder() {
        return Err(SyncFehler::SteuerfelderUngleich);
    }
    Ok(Geoeffnet {
        steuer: innen.steuer,
        revision: u.revision,
        inhalt: innen.inhalt,
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

    fn bund() -> Schluesselbund {
        let mut b = Schluesselbund::neu(
            Schluessel::aus_bytes([1; 32]),
            1,
            Schluessel::aus_bytes([2; 32]),
        );
        // Gleiches Schluesselmaterial in Epoche 2: nur die AAD kann die Epoche noch binden.
        b.epoche_aufnehmen(2, Schluessel::aus_bytes([2; 32]))
            .unwrap();
        b
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

    #[test]
    fn rundlauf() {
        let b = bund();
        let u = versiegeln(&b, &steuer(), &json!({"text": "Hallo"})).unwrap();
        u.pruefen().unwrap();
        let g = oeffnen(&b, &u).unwrap();
        assert_eq!(g.inhalt, json!({"text": "Hallo"}));
        assert_eq!(g.steuer, steuer());
    }

    #[test]
    fn debug_von_geoeffnet_zeigt_keinen_inhalt() {
        let b = bund();
        let u = versiegeln(&b, &steuer(), &json!({"text": "Geheim-4711"})).unwrap();
        let s = format!("{:?}", oeffnen(&b, &u).unwrap());
        assert!(!s.contains("Geheim-4711") && s.contains("verborgen"));
    }

    #[test]
    fn nonce_ist_je_versiegelung_neu() {
        let b = bund();
        let a = versiegeln(&b, &steuer(), &json!(1)).unwrap();
        let c = versiegeln(&b, &steuer(), &json!(1)).unwrap();
        assert_ne!(a.nonce, c.nonce);
        assert_ne!(a.ciphertext, c.ciphertext);
    }

    #[test]
    fn aad_ist_kanonisch_und_feldweise_verschieden() {
        let a = steuer().aad().unwrap();
        let mut s = steuer();
        s.geraet_id = "dev-pc2".into();
        assert_ne!(a, s.aad().unwrap());
        // Grenzverschiebung zwischen Feldern aendert die Kodierung.
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
        let b = bund();
        let u = versiegeln(&b, &steuer(), &json!({"x": 1})).unwrap();
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
                entschluesseln(&b, &m),
                Err(SyncFehler::Oeffnen),
                "AAD muss {feld} binden"
            );
            assert!(oeffnen(&b, &m).is_err(), "{feld}");
        }
    }

    #[test]
    fn aad_bindet_grabsteinziel() {
        let b = bund();
        let u = versiegeln(&b, &grab(), &json!({})).unwrap();
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
            assert!(entschluesseln(&b, &m).is_err());
            assert!(oeffnen(&b, &m).is_err());
        }
        let mut ohne = u.clone();
        ohne.ziel = None;
        assert!(oeffnen(&b, &ohne).is_err());
    }

    #[test]
    fn klartext_muss_zu_sichtbaren_feldern_passen() {
        // Gueltiges Chiffrat, dessen Klartext andere Steuerfelder behauptet (Schluesselbesitzer-Fehler).
        let b = bund();
        let echt = steuer();
        let mut falsch = steuer();
        falsch.objekt_id = hex(9);
        let innen = Innen {
            steuer: falsch,
            inhalt: json!(1),
        };
        let klar = serde_json::to_vec(&innen).unwrap();
        let nonce = zufall::<NONCE_LAENGE>().unwrap();
        let ct = XChaCha20Poly1305::new(b.schluessel(1).unwrap().bytes().into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &klar,
                    aad: &echt.aad().unwrap(),
                },
            )
            .unwrap();
        let mut u = versiegeln(&b, &echt, &json!(0)).unwrap();
        u.ciphertext = B64.encode(ct);
        u.nonce = B64.encode(nonce);
        assert_eq!(oeffnen(&b, &u), Err(SyncFehler::SteuerfelderUngleich));
    }

    #[test]
    fn fremder_schluessel_und_unbekannte_epoche() {
        let b = bund();
        let u = versiegeln(&b, &steuer(), &json!(1)).unwrap();
        let fremd = Schluesselbund::neu(
            Schluessel::aus_bytes([1; 32]),
            1,
            Schluessel::aus_bytes([5; 32]),
        );
        assert_eq!(oeffnen(&fremd, &u), Err(SyncFehler::Oeffnen));
        let nur_alt = Schluesselbund::neu(
            Schluessel::aus_bytes([1; 32]),
            7,
            Schluessel::aus_bytes([2; 32]),
        );
        assert_eq!(oeffnen(&nur_alt, &u), Err(SyncFehler::EpocheUnbekannt(1)));
    }

    #[test]
    fn manipuliertes_chiffrat_wird_abgewiesen() {
        let b = bund();
        let mut u = versiegeln(&b, &steuer(), &json!({"a": "b"})).unwrap();
        let mut ct = B64.decode(&u.ciphertext).unwrap();
        ct[3] ^= 1;
        u.ciphertext = B64.encode(ct);
        assert!(oeffnen(&b, &u).is_err());
    }

    #[test]
    fn vertragsregeln() {
        let b = bund();
        let mut s = steuer();
        s.art = Sammlung::Hinweis;
        assert!(versiegeln(&b, &s, &json!(1)).is_err());
        let mut s = steuer();
        s.ziel = Some(Ziel {
            typ: ZielTyp::Objekt,
            wert: hex(4),
        });
        assert!(
            versiegeln(&b, &s, &json!(1)).is_err(),
            "ziel nur an loeschmarke"
        );
        let mut g = grab();
        g.ziel = None;
        assert!(
            versiegeln(&b, &g, &json!(1)).is_err(),
            "loeschmarke braucht ziel"
        );
        let mut g = grab();
        g.ziel = Some(Ziel {
            typ: ZielTyp::Unterhaltung,
            wert: hex(9),
        });
        assert!(
            versiegeln(&b, &g, &json!(1)).is_err(),
            "unterhaltungsziel = unterhaltung_ref"
        );
        let u = versiegeln(&b, &steuer(), &json!(1)).unwrap();
        let mut k = u.clone();
        k.nonce = B64.encode([0u8; 3]);
        assert!(k.pruefen().is_err());
        let mut k = u.clone();
        k.geraet_id = String::new();
        assert!(k.pruefen().is_err());
        let mut k = u;
        k.schema_version = "1.0.0".into();
        assert!(k.pruefen().is_err());
    }

    #[test]
    fn wire_form_entspricht_dem_vertrag() {
        let b = bund();
        let u = versiegeln(&b, &steuer(), &json!(1))
            .unwrap()
            .mit_revision(17);
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
        let g = versiegeln(&b, &grab(), &json!({})).unwrap();
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
                let ergebnis = serde_json::from_value::<Umschlag>(doc["data"].clone())
                    .map_err(|_| ())
                    .and_then(|u| u.steuerfelder().pruefen().map_err(|_| ()));
                // Die Fixtures tragen 12-Byte-Nonces; der Kern nutzt XChaCha20 (24 Byte). Deshalb nur
                // Form und Steuerfeldregeln pruefen, nicht die Nonce-Laenge.
                if soll {
                    assert!(ergebnis.is_ok(), "{}", pfad.display());
                } else {
                    assert!(ergebnis.is_err(), "{}", pfad.display());
                }
            }
        }
    }
}
