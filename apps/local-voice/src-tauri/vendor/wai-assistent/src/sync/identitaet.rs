//! Geraeteschluessel (ADR-0028 E3.8): je Geraet ein Ed25519-Schluessel zum Signieren und ein
//! X25519-Schluessel zum Empfang gewrappter Epochenschluessel.
//!
//! Der geheime Teil verlaesst das Geraet nie; der oeffentliche [`Ausweis`] wird bei der Kopplung
//! (QR, authentischer Kanal) gezeigt und im signierten Geraeteverzeichnis gepinnt.

use std::fmt;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use super::fehler::SyncFehler;
use super::kennung::{GeheimHex, hash_teile, hexfeld, zufall};

/// Oeffentlicher Teil einer Geraeteidentitaet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ausweis {
    pub geraet_id: String,
    /// Ed25519, prueft Signaturen dieses Geraets.
    #[serde(with = "hexfeld")]
    pub signatur: [u8; 32],
    /// X25519, Empfaenger gewrappter Epochenschluessel.
    #[serde(with = "hexfeld")]
    pub austausch: [u8; 32],
}

impl Ausweis {
    pub(crate) fn pruefen_form(&self) -> Result<(), SyncFehler> {
        if self.geraet_id.is_empty() || self.geraet_id.len() > 128 {
            return Err(SyncFehler::Vertrag(
                "geraet_id muss 1 bis 128 Zeichen haben",
            ));
        }
        VerifyingKey::from_bytes(&self.signatur)
            .map_err(|_| SyncFehler::Vertrag("signaturschluessel ungueltig"))?;
        Ok(())
    }

    /// Stabiler Hash des Ausweises (authentisierter Autor in kanonischen Ordnungen, E3.3/E3.8).
    pub fn hash(&self) -> [u8; 32] {
        hash_teile(
            b"wai-assistent-sync/ausweis/v1",
            &[self.geraet_id.as_bytes(), &self.signatur, &self.austausch],
        )
    }

    /// Prueft eine Ed25519-Signatur dieses Geraets (streng, ohne schwache Schluessel).
    pub(crate) fn pruefe_signatur(&self, nachricht: &[u8], signatur: &[u8; 64]) -> bool {
        let Ok(vk) = VerifyingKey::from_bytes(&self.signatur) else {
            return false;
        };
        vk.verify_strict(nachricht, &Signature::from_bytes(signatur))
            .is_ok()
    }
}

/// Geheimer Teil: Signatur- und Austauschschluessel. `Debug` zeigt nichts.
#[derive(Clone)]
pub struct Geraeteschluessel {
    geraet_id: String,
    signatur: SigningKey,
    austausch: StaticSecret,
}

/// Sicherungsform. Die geheimen Felder sind [`GeheimHex`]: keine ungeschuetzte `String`-Kopie, und bricht das
/// Lesen ab, werden schon gelesene Felder beim Freigeben ueberschrieben (E3.8, Must-fix 7).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Gesichert {
    geraet_id: String,
    signatur: GeheimHex,
    austausch: GeheimHex,
}

impl Geraeteschluessel {
    /// Neue Identitaet aus Betriebssystem-Zufall.
    pub fn erzeugen(geraet_id: &str) -> Result<Self, SyncFehler> {
        let s = Zeroizing::new(zufall::<32>()?);
        let a = Zeroizing::new(zufall::<32>()?);
        let k = Self {
            geraet_id: geraet_id.to_owned(),
            signatur: SigningKey::from_bytes(&s),
            austausch: StaticSecret::from(*a),
        };
        k.ausweis().pruefen_form()?;
        Ok(k)
    }

    pub fn geraet_id(&self) -> &str {
        &self.geraet_id
    }

    pub fn ausweis(&self) -> Ausweis {
        Ausweis {
            geraet_id: self.geraet_id.clone(),
            signatur: self.signatur.verifying_key().to_bytes(),
            austausch: PublicKey::from(&self.austausch).to_bytes(),
        }
    }

    pub(crate) fn signieren(&self, nachricht: &[u8]) -> [u8; 64] {
        self.signatur.sign(nachricht).to_bytes()
    }

    pub(crate) fn austausch(&self) -> &StaticSecret {
        &self.austausch
    }
}

impl fmt::Debug for Geraeteschluessel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Geraeteschluessel({}, ***)", self.geraet_id)
    }
}

impl Serialize for Geraeteschluessel {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Gesichert {
            geraet_id: self.geraet_id.clone(),
            signatur: GeheimHex(Zeroizing::new(self.signatur.to_bytes())),
            austausch: GeheimHex(Zeroizing::new(self.austausch.to_bytes())),
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Geraeteschluessel {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let g = Gesichert::deserialize(d)?;
        Ok(Self {
            geraet_id: g.geraet_id,
            signatur: SigningKey::from_bytes(&g.signatur.0),
            austausch: StaticSecret::from(*g.austausch.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatur_rundlauf_und_fremde_nachricht() {
        let k = Geraeteschluessel::erzeugen("dev-pc").unwrap();
        let a = k.ausweis();
        let s = k.signieren(b"abc");
        assert!(a.pruefe_signatur(b"abc", &s));
        assert!(!a.pruefe_signatur(b"abd", &s));
        let anderer = Geraeteschluessel::erzeugen("dev-pc").unwrap().ausweis();
        assert!(!anderer.pruefe_signatur(b"abc", &s));
    }

    #[test]
    fn abgebrochenes_lesen_und_falsche_form_scheitern() {
        let k = Geraeteschluessel::erzeugen("dev-pc").unwrap();
        let mut j: serde_json::Value = serde_json::to_value(&k).unwrap();
        j["austausch"] = serde_json::json!("zz");
        assert!(serde_json::from_value::<Geraeteschluessel>(j.clone()).is_err());
        j["austausch"] = serde_json::json!(7);
        assert!(serde_json::from_value::<Geraeteschluessel>(j).is_err());
        let gross = serde_json::json!({"geraet_id": "x", "signatur": "AB".repeat(32), "austausch": "00".repeat(32)});
        assert!(
            serde_json::from_value::<Geraeteschluessel>(gross).is_err(),
            "nur kleine Hexzeichen"
        );
    }

    #[test]
    fn sicherung_rundlauf_und_debug_ohne_geheimnis() {
        let k = Geraeteschluessel::erzeugen("dev-pc").unwrap();
        let j = serde_json::to_string(&k).unwrap();
        let z: Geraeteschluessel = serde_json::from_str(&j).unwrap();
        assert_eq!(z.ausweis(), k.ausweis());
        let d = format!("{k:?}");
        assert!(
            d.contains("***")
                && !d.contains(&*crate::sync::kennung::hex_geheim(k.signatur.as_bytes()))
        );
    }
}
