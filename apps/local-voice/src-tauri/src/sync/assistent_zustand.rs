//! Schlüsselmaterial und Zustandsdatei des Assistent-Syncs (lva#388, ADR-0028 E3.1).
//!
//! Reine Logik ohne Tauri und ohne Netz.
//!
//! **Schlüssel.** Der Hub hat keinen Inhaltsschlüssel (E3.0). Kontosalz und Schlüssel der Epoche 1
//! leitet jedes Gerät mit HKDF-SHA256 aus dem Objekt-Schlüssel des Bibliotheks-Syncs ab (E3.1: „mit
//! dem Kontoschlüssel des Bibliothek-Syncs“); Zweck und Version stehen in den `info`-Texten, damit
//! der Assistent-Sync nie denselben Schlüssel wie der Bibliotheks-Sync benutzt. Alle Geräte eines
//! Kontos, die sich mit denselben Zugangsdaten anmelden oder gekoppelt werden, kommen so ohne
//! Zusatzübertragung auf denselben Bund der Epoche 1.
//!
//! **Epochen ab 2** (nach einem Geräte-Widerruf) sind zufällig und unabhängig; sie liegen in der
//! Zustandsdatei `assistent_sync.json` (privat, wie `sync.json`) und müssen an die übrigen Geräte über
//! die Kopplung gehen. Dieser Transport gehört nicht zu A1 (siehe Übergabe).
//!
//! **Zustandsdatei.** Sie gehört zu genau einem Konto und Gerät (`konto`: Hash aus Hub-URL, Konto-ID
//! und Geräte-Kennung). Passt sie nicht mehr zum angemeldeten Konto, gilt sie als leer.

use std::collections::BTreeMap;
use std::fmt;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wai_assistent::sync::{Hex128, OffenerWiderruf, Schluessel, Schluesselbund};

pub const DATEI: &str = "assistent_sync.json";
pub const KEY_LEN: usize = 32;

const INFO_SALZ: &[u8] = b"wai-assistent-sync/v1/id-salz";
const INFO_EPOCHE_1: &[u8] = b"wai-assistent-sync/v1/epoche-1";

fn ableiten(konto_schluessel: &[u8; KEY_LEN], info: &[u8]) -> Result<[u8; KEY_LEN], String> {
    let mut aus = [0u8; KEY_LEN];
    Hkdf::<Sha256>::new(None, konto_schluessel)
        .expand(info, &mut aus)
        .map_err(|_| "Schlüsselableitung fehlgeschlagen".to_string())?;
    Ok(aus)
}

/// Bund der Epoche 1 aus dem Objekt-Schlüssel des Bibliotheks-Syncs.
pub fn grundbund(konto_schluessel: &[u8; KEY_LEN]) -> Result<Schluesselbund, String> {
    Ok(Schluesselbund::neu(
        Schluessel::aus_bytes(ableiten(konto_schluessel, INFO_SALZ)?),
        1,
        Schluessel::aus_bytes(ableiten(konto_schluessel, INFO_EPOCHE_1)?),
    ))
}

/// Vorgemerkter Widerruf in der Zustandsdatei (verlorene Antwort: wiederholbar mit derselben Kennung).
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OffenerWiderrufDatei {
    pub geraet: String,
    pub erwartete_epoche: u64,
    pub vorgang: String,
    /// Kandidat für den Schlüssel der neuen Epoche, base64.
    pub schluessel: String,
}

/// Inhalt von `assistent_sync.json`.
#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Zustand {
    /// Kontokennung, zu der dieser Zustand gehört (siehe [`Zustand::konto_kennung`]).
    #[serde(default)]
    pub konto: String,
    /// Dieses Gerät ist am Hub zugelassen (`POST /geraet` war erfolgreich).
    #[serde(default)]
    pub zugelassen: bool,
    /// Epochenschlüssel ab Epoche 2: Epochennummer → base64.
    #[serde(default)]
    pub epochen: BTreeMap<String, String>,
    #[serde(default)]
    pub offener_widerruf: Option<OffenerWiderrufDatei>,
}

impl fmt::Debug for Zustand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Zustand")
            .field("zugelassen", &self.zugelassen)
            .field("epochen", &self.epochen.keys().collect::<Vec<_>>())
            .field("offener_widerruf", &self.offener_widerruf.is_some())
            .finish()
    }
}

fn schluessel_aus(b64: &str) -> Result<Schluessel, String> {
    let roh = B64
        .decode(b64)
        .map_err(|_| "assistent_sync.json: Schlüssel unlesbar".to_string())?;
    let bytes: [u8; KEY_LEN] = roh
        .try_into()
        .map_err(|_| "assistent_sync.json: Schlüssel hat die falsche Länge".to_string())?;
    Ok(Schluessel::aus_bytes(bytes))
}

impl Zustand {
    /// Leerer Zustand für dieses Konto und Gerät.
    pub fn neu(konto: &str) -> Self {
        Self {
            konto: konto.to_owned(),
            ..Self::default()
        }
    }

    /// Kontokennung: SHA-256 über Hub-URL, Konto-ID und Geräte-Kennung (hex). Ein anderes Konto
    /// oder eine neue Geräte-Kennung verwirft Zulassung und Epochenschlüssel.
    pub fn konto_kennung(url: &str, user_id: i64, geraet_id: &str) -> String {
        let mut h = Sha256::new();
        for teil in [
            url.trim().trim_end_matches('/'),
            &user_id.to_string(),
            geraet_id,
        ] {
            h.update((teil.len() as u32).to_be_bytes());
            h.update(teil.as_bytes());
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Liest die Zustandsdatei; fehlt sie, ist sie unlesbar oder gehört sie zu einem anderen Konto,
    /// kommt ein leerer Zustand zurück.
    pub fn aus_json(text: Option<&str>, konto: &str) -> Self {
        text.and_then(|t| serde_json::from_str::<Self>(t).ok())
            .filter(|z| z.konto == konto)
            .unwrap_or_else(|| Self::neu(konto))
    }

    pub fn zu_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// Bund dieses Geräts: Epoche 1 aus dem Kontoschlüssel, dazu die gespeicherten Epochen.
    pub fn bund(&self, konto_schluessel: &[u8; KEY_LEN]) -> Result<Schluesselbund, String> {
        let mut bund = grundbund(konto_schluessel)?;
        for (epoche, schluessel) in &self.epochen {
            let nummer: u64 = epoche
                .parse()
                .map_err(|_| "assistent_sync.json: Epoche unlesbar".to_string())?;
            bund.epoche_aufnehmen(nummer, schluessel_aus(schluessel)?)
                .map_err(|e| e.to_string())?;
        }
        Ok(bund)
    }

    /// Nimmt die Epochen ab 2 aus dem Bund auf (nach einem Widerruf).
    pub fn aus_bund(&mut self, bund: &Schluesselbund) {
        for epoche in 2..=bund.aktuelle_epoche() {
            if let Some(s) = bund.schluessel(epoche) {
                self.epochen
                    .insert(epoche.to_string(), B64.encode(s.bytes()));
            }
        }
    }

    pub fn offenen_widerruf_merken(&mut self, offen: Option<&OffenerWiderruf>) {
        self.offener_widerruf = offen.map(|o| OffenerWiderrufDatei {
            geraet: o.geraet().to_owned(),
            erwartete_epoche: o.erwartete_epoche(),
            vorgang: o.vorgang().as_str().to_owned(),
            schluessel: B64.encode(o.schluessel().bytes()),
        });
    }

    pub fn offenen_widerruf(&self) -> Result<Option<OffenerWiderruf>, String> {
        let Some(d) = &self.offener_widerruf else {
            return Ok(None);
        };
        let vorgang = Hex128::parse(&d.vorgang)
            .map_err(|_| "assistent_sync.json: Vorgangskennung unlesbar".to_string())?;
        Ok(Some(OffenerWiderruf::neu(
            &d.geraet,
            d.erwartete_epoche,
            vorgang,
            schluessel_aus(&d.schluessel)?,
        )))
    }
}

/// Kennung dieses Geräts für den Assistent-Hub: klein geschrieben, `[A-Za-z0-9._-]{1,64}`.
pub fn geraete_kennung(device_id: &str) -> Option<String> {
    let k = device_id.to_ascii_lowercase();
    gueltige_kennung(&k).then_some(k)
}

/// Form einer Geräte-Kennung nach der Hub-Doku.
pub fn gueltige_kennung(k: &str) -> bool {
    (1..=64).contains(&k.len())
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; KEY_LEN] = [7; KEY_LEN];

    #[test]
    fn grundbund_ist_auf_jedem_geraet_gleich_und_vom_kontoschluessel_getrennt() {
        let (a, b) = (grundbund(&KEY).unwrap(), grundbund(&KEY).unwrap());
        assert_eq!(a.id_salz(), b.id_salz());
        assert_eq!(
            a.schluessel(1).unwrap().bytes(),
            b.schluessel(1).unwrap().bytes()
        );
        assert_eq!(a.aktuelle_epoche(), 1);
        // Weder Salz noch Epochenschlüssel sind der Kontoschlüssel oder gleich.
        assert_ne!(a.schluessel(1).unwrap().bytes(), &KEY);
        assert_ne!(a.id_salz(), &KEY);
        assert_ne!(a.id_salz(), a.schluessel(1).unwrap().bytes());
        // Anderes Konto, anderer Bund.
        let c = grundbund(&[8; KEY_LEN]).unwrap();
        assert_ne!(
            a.schluessel(1).unwrap().bytes(),
            c.schluessel(1).unwrap().bytes()
        );
    }

    #[test]
    fn zwei_geraete_lesen_gegenseitig_ihre_objekte() {
        use wai_assistent::sync::{oeffnen, versiegeln, Hex128, Sammlung, Steuerfelder};
        let (pc, tablet) = (grundbund(&KEY).unwrap(), grundbund(&KEY).unwrap());
        let steuer = Steuerfelder::inhalt(
            Sammlung::Beitrag,
            Hex128::zufall().unwrap(),
            Hex128::zufall().unwrap(),
            "pc",
            1,
        );
        let u = versiegeln(&pc, &steuer, &serde_json::json!({ "x": 1 })).unwrap();
        assert_eq!(oeffnen(&tablet, &u).unwrap().inhalt["x"], 1);
    }

    #[test]
    fn zustand_gehoert_zu_einem_konto_und_geraet() {
        let k1 = Zustand::konto_kennung("https://hub.example/", 7, "geraet-a");
        assert_eq!(
            k1,
            Zustand::konto_kennung("https://hub.example", 7, "geraet-a"),
            "Schrägstrich am Ende egal"
        );
        assert_ne!(
            k1,
            Zustand::konto_kennung("https://hub.example", 8, "geraet-a")
        );
        assert_ne!(
            k1,
            Zustand::konto_kennung("https://hub.example", 7, "geraet-b")
        );
        assert_ne!(
            Zustand::konto_kennung("ab", 1, "c"),
            Zustand::konto_kennung("a", 1, "bc"),
            "Längenpräfix verhindert Verschiebungen"
        );
        let mut z = Zustand::neu(&k1);
        z.zugelassen = true;
        let json = z.zu_json().unwrap();
        assert!(Zustand::aus_json(Some(&json), &k1).zugelassen);
        let fremd = Zustand::aus_json(Some(&json), "anderes-konto");
        assert!(!fremd.zugelassen && fremd.konto == "anderes-konto");
        assert_eq!(Zustand::aus_json(None, "k").konto, "k");
        assert!(!Zustand::aus_json(Some("{kaputt"), "k").zugelassen);
    }

    #[test]
    fn epochen_ab_zwei_ueberleben_die_zustandsdatei() {
        let mut bund = grundbund(&KEY).unwrap();
        assert_eq!(bund.neue_epoche().unwrap(), 2);
        let mut z = Zustand::neu("k");
        z.aus_bund(&bund);
        assert_eq!(z.epochen.keys().collect::<Vec<_>>(), ["2"]);
        let z2 = Zustand::aus_json(Some(&z.zu_json().unwrap()), "k");
        let wieder = z2.bund(&KEY).unwrap();
        assert_eq!(wieder.aktuelle_epoche(), 2);
        assert_eq!(
            wieder.schluessel(2).unwrap().bytes(),
            bund.schluessel(2).unwrap().bytes()
        );
        assert_eq!(
            wieder.schluessel(1).unwrap().bytes(),
            bund.schluessel(1).unwrap().bytes()
        );
        // Debug zeigt keinen Schlüssel.
        let text = format!("{z:?}");
        assert!(!text.contains(&B64.encode(bund.schluessel(2).unwrap().bytes())));
    }

    #[test]
    fn offener_widerruf_ueberlebt_den_neustart() {
        let o = OffenerWiderruf::neu(
            "tablet",
            3,
            Hex128::zufall().unwrap(),
            Schluessel::aus_bytes([5; KEY_LEN]),
        );
        let mut z = Zustand::neu("k");
        z.offenen_widerruf_merken(Some(&o));
        let z2 = Zustand::aus_json(Some(&z.zu_json().unwrap()), "k");
        let o2 = z2.offenen_widerruf().unwrap().unwrap();
        assert_eq!((o2.geraet(), o2.erwartete_epoche()), ("tablet", 3));
        assert_eq!(o2.vorgang(), o.vorgang());
        assert_eq!(o2.schluessel().bytes(), o.schluessel().bytes());
        z.offenen_widerruf_merken(None);
        assert!(z.offenen_widerruf().unwrap().is_none());
        let kaputt = Zustand {
            offener_widerruf: Some(OffenerWiderrufDatei {
                geraet: "x".into(),
                erwartete_epoche: 1,
                vorgang: "kein-hex".into(),
                schluessel: "AAAA".into(),
            }),
            ..Zustand::neu("k")
        };
        assert!(kaputt.offenen_widerruf().is_err());
    }

    #[test]
    fn beschaedigte_epochen_sind_ein_fehler_und_kein_stiller_verlust() {
        let mut z = Zustand::neu("k");
        z.epochen.insert("2".into(), "AAAA".into());
        assert!(z.bund(&KEY).is_err());
        z.epochen.clear();
        z.epochen.insert("zwei".into(), B64.encode([1u8; KEY_LEN]));
        assert!(z.bund(&KEY).is_err());
    }

    #[test]
    fn geraetekennung_nach_der_hub_doku() {
        // Die Kennung der App (32 Hexzeichen) passt.
        assert_eq!(
            geraete_kennung("0123456789ABCDEF0123456789abcdef").as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert!(geraete_kennung("pc-1_a.b").is_some());
        assert!(geraete_kennung("").is_none());
        assert!(geraete_kennung(&"a".repeat(65)).is_none());
        assert!(geraete_kennung("hat leerzeichen").is_none());
        assert!(geraete_kennung("ümlaut").is_none());
        assert!(gueltige_kennung("ok") && !gueltige_kennung("a/b"));
    }
}
