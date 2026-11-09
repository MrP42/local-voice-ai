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
//! **Zustandsdatei.** Sie hält je Kontokennung einen Eintrag (`konten`; Kennung: Hash aus Hub-URL,
//! Konto-ID und Fingerabdruck des Bibliotheks-Schlüssels, NICHT die Geräte-Kennung). Einträge anderer
//! Konten bleiben unberührt und werden nie gelöscht. Die Zulassung gehört zum Gerät: mit einer neuen
//! Geräte-Kennung gilt sie nicht mehr, die Epochenschlüssel des Kontos bleiben. Die Datei ist
//! fail-closed: nur „fehlt“ gilt als leer, ein Lese- oder Parsefehler ist ein Fehler (nichts wird
//! zugelassen oder überschrieben); vor jedem Überschreiben liegt eine `.bak` daneben.

use std::collections::BTreeMap;
use std::fmt;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wai_assistent::sync::{
    Abgleich, Hex128, HubFehler, HubSpeicher, OffenerWiderruf, Schluessel, Schluesselbund,
    SyncFehler,
};

pub const DATEI: &str = "assistent_sync.json";
pub const KEY_LEN: usize = 32;

const INFO_SALZ: &[u8] = b"wai-assistent-sync/v1/id-salz";
const INFO_EPOCHE_1: &[u8] = b"wai-assistent-sync/v1/epoche-1";
const INFO_KONTO: &[u8] = b"wai-assistent-sync/v1/konto-fingerabdruck";

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

/// Eintrag einer Kontokennung in `assistent_sync.json`.
#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Zustand {
    /// Gerät, für das `zugelassen` gilt.
    #[serde(default)]
    pub geraet: String,
    /// Dieses Gerät ist am Hub zugelassen (`POST /geraet` war erfolgreich).
    #[serde(default)]
    pub zugelassen: bool,
    /// Epochenschlüssel ab Epoche 2: Epochennummer → base64.
    #[serde(default)]
    pub epochen: BTreeMap<String, String>,
    #[serde(default)]
    pub offener_widerruf: Option<OffenerWiderrufDatei>,
}

/// Inhalt von `assistent_sync.json`.
#[derive(Clone, Serialize, Deserialize, Default)]
struct Datei {
    #[serde(default)]
    konten: BTreeMap<String, Zustand>,
}

impl fmt::Debug for Zustand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Zustand")
            .field("geraet", &self.geraet)
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
    /// Leerer Zustand für dieses Gerät.
    pub fn neu(geraet: &str) -> Self {
        Self {
            geraet: geraet.to_owned(),
            ..Self::default()
        }
    }

    /// Kontokennung: SHA-256 über Hub-URL, Konto-ID und Fingerabdruck des Bibliotheks-Schlüssels
    /// (hex). Ein anderes Konto oder ein anderer Schlüssel ist ein anderes Konto; die Geräte-Kennung
    /// gehört bewusst nicht dazu.
    pub fn konto_kennung(url: &str, user_id: i64, konto_schluessel: &[u8; KEY_LEN]) -> String {
        let mut fingerabdruck = [0u8; 16];
        // 16 Byte sind für HKDF-SHA256 immer ableitbar.
        let _ = Hkdf::<Sha256>::new(None, konto_schluessel).expand(INFO_KONTO, &mut fingerabdruck);
        let mut h = Sha256::new();
        for teil in [
            url.trim().trim_end_matches('/').as_bytes(),
            user_id.to_string().as_bytes(),
            &fingerabdruck[..],
        ] {
            h.update((teil.len() as u32).to_be_bytes());
            h.update(teil);
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Liest den Eintrag dieses Kontos. `text` ist der Dateiinhalt, `None` nur, wenn die Datei
    /// nicht existiert. Ein unlesbarer Inhalt ist ein Fehler, nie „leer“.
    pub fn aus_datei(text: Option<&str>, konto: &str, geraet: &str) -> Result<Self, String> {
        let datei = Self::datei_lesen(text)?;
        let mut z = datei.konten.get(konto).cloned().unwrap_or_default();
        if z.geraet != geraet {
            // Neue Geräte-Kennung: die Zulassung gilt nicht mehr, die Schlüssel des Kontos bleiben.
            z.geraet = geraet.to_owned();
            z.zugelassen = false;
            z.offener_widerruf = None;
        }
        Ok(z)
    }

    /// Neuer Dateiinhalt: dieser Eintrag ersetzt den des Kontos, alle anderen bleiben.
    pub fn in_datei(&self, alt: Option<&str>, konto: &str) -> Result<String, String> {
        let mut datei = Self::datei_lesen(alt)?;
        datei.konten.insert(konto.to_owned(), self.clone());
        serde_json::to_string_pretty(&datei).map_err(|e| e.to_string())
    }

    fn datei_lesen(text: Option<&str>) -> Result<Datei, String> {
        match text {
            None => Ok(Datei::default()),
            Some(t) => serde_json::from_str::<Datei>(t).map_err(|_| {
                "assistent_sync.json ist unlesbar; sie wird nicht überschrieben.".to_string()
            }),
        }
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

/// Fehler von [`widerruf_durchfuehren`]. Keine Variante trägt Schlüssel oder Inhalte.
#[derive(Debug)]
pub enum WiderrufFehler {
    Hub(HubFehler),
    Sync(SyncFehler),
    /// Schlüsselmaterial oder Zustandsdatei unbrauchbar, oder das Sichern scheiterte.
    Speicher(String),
}

fn sichern(
    a: &Abgleich,
    z: &mut Zustand,
    speichern: &mut dyn FnMut(&Zustand) -> Result<(), String>,
) -> Result<(), WiderrufFehler> {
    z.aus_bund(a.bund());
    z.offenen_widerruf_merken(a.offener_widerruf());
    speichern(z).map_err(WiderrufFehler::Speicher)
}

/// Widerruft `ziel` am Hub. Der Kandidat für den neuen Epochenschlüssel und die Vorgangskennung
/// werden **zuerst** dauerhaft gesichert (`speichern`), erst dann geht der Aufruf an den Hub. Stürzt
/// das Gerät dazwischen oder danach ab, wiederholt der nächste Aufruf mit derselben Vorgangskennung;
/// der Hub ist dafür idempotent und der gesicherte Schlüssel wird übernommen.
pub fn widerruf_durchfuehren<H: HubSpeicher + ?Sized>(
    hub: &H,
    geraet: &str,
    konto_schluessel: &[u8; KEY_LEN],
    z: &mut Zustand,
    ziel: &str,
    speichern: &mut dyn FnMut(&Zustand) -> Result<(), String>,
) -> Result<u64, WiderrufFehler> {
    let bund = z.bund(konto_schluessel).map_err(WiderrufFehler::Speicher)?;
    let mut a = Abgleich::new(geraet, bund);
    a.freigabe_einschalten();
    if let Some(offen) = z.offenen_widerruf().map_err(WiderrufFehler::Speicher)? {
        let anderes_ziel = offen.geraet() != ziel;
        a.offenen_widerruf_wiederherstellen(offen);
        if anderes_ziel {
            let r = a.widerruf_abschliessen(hub);
            sichern(&a, z, speichern)?;
            match r {
                Ok(_) | Err(SyncFehler::Hub(HubFehler::EpocheKonflikt { .. })) => {}
                Err(e) => return Err(WiderrufFehler::Sync(e)),
            }
        }
    }
    if a.offener_widerruf().is_none() {
        let kandidat = OffenerWiderruf::neu(
            ziel,
            a.bund().aktuelle_epoche(),
            Hex128::zufall().map_err(WiderrufFehler::Sync)?,
            Schluessel::zufall().map_err(WiderrufFehler::Sync)?,
        );
        a.offenen_widerruf_wiederherstellen(kandidat);
        // Dauerhaft sichern, bevor irgendetwas an den Hub geht.
        sichern(&a, z, speichern)?;
    }
    let r = a.widerruf_abschliessen(hub);
    sichern(&a, z, speichern)?;
    match r {
        Ok(Some(epoche)) => Ok(epoche),
        Ok(None) => Err(WiderrufFehler::Speicher("kein Widerruf offen".into())),
        Err(SyncFehler::Hub(h)) => Err(WiderrufFehler::Hub(h)),
        Err(e) => Err(WiderrufFehler::Sync(e)),
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
    fn kontokennung_haengt_an_konto_und_schluessel_nicht_am_geraet() {
        let k1 = Zustand::konto_kennung("https://hub.example/", 7, &KEY);
        assert_eq!(
            k1,
            Zustand::konto_kennung("https://hub.example", 7, &KEY),
            "Schrägstrich am Ende egal"
        );
        assert_ne!(k1, Zustand::konto_kennung("https://hub.example", 8, &KEY));
        assert_ne!(
            k1,
            Zustand::konto_kennung("https://hub.example", 7, &[8; KEY_LEN]),
            "ein anderer Bibliotheks-Schlüssel ist ein anderes Konto"
        );
        assert_ne!(
            Zustand::konto_kennung("ab", 1, &KEY),
            Zustand::konto_kennung("a", 1, &KEY)
        );
        assert!(
            !k1.contains(&B64.encode(KEY)),
            "kein Schlüssel in der Kennung"
        );
    }

    #[test]
    fn zustand_je_konto_neues_geraet_behaelt_schluessel_aber_nicht_die_zulassung() {
        let mut z = Zustand::neu("geraet-a");
        z.zugelassen = true;
        let mut bund = grundbund(&KEY).unwrap();
        bund.neue_epoche().unwrap();
        z.aus_bund(&bund);
        let json = z.in_datei(None, "konto-1").unwrap();
        // Zweites Konto in derselben Datei: Eintrag eins bleibt unberührt.
        let mut z2 = Zustand::neu("geraet-a");
        z2.zugelassen = true;
        let json = z2.in_datei(Some(&json), "konto-2").unwrap();
        assert!(
            Zustand::aus_datei(Some(&json), "konto-1", "geraet-a")
                .unwrap()
                .zugelassen
        );
        assert!(
            Zustand::aus_datei(Some(&json), "konto-2", "geraet-a")
                .unwrap()
                .zugelassen
        );
        let neu = Zustand::aus_datei(Some(&json), "konto-1", "geraet-b").unwrap();
        assert!(!neu.zugelassen, "neues Gerät muss neu zugelassen werden");
        assert_eq!(neu.epochen, z.epochen, "Schlüssel des Kontos bleiben");
        let unbekannt = Zustand::aus_datei(Some(&json), "konto-3", "geraet-a").unwrap();
        assert!(!unbekannt.zugelassen && unbekannt.epochen.is_empty());
        assert!(Zustand::aus_datei(None, "k", "g")
            .unwrap()
            .epochen
            .is_empty());
    }

    #[test]
    fn unlesbare_datei_ist_ein_fehler_und_wird_nie_ueberschrieben() {
        assert!(Zustand::aus_datei(Some("{kaputt"), "k", "g").is_err());
        assert!(Zustand::neu("g").in_datei(Some("{kaputt"), "k").is_err());
        assert!(Zustand::aus_datei(Some(""), "k", "g").is_err());
    }

    #[test]
    fn epochen_ab_zwei_ueberleben_die_zustandsdatei() {
        let mut bund = grundbund(&KEY).unwrap();
        assert_eq!(bund.neue_epoche().unwrap(), 2);
        let mut z = Zustand::neu("g");
        z.aus_bund(&bund);
        assert_eq!(z.epochen.keys().collect::<Vec<_>>(), ["2"]);
        let z2 = Zustand::aus_datei(Some(&z.in_datei(None, "k").unwrap()), "k", "g").unwrap();
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
        let mut z = Zustand::neu("g");
        z.offenen_widerruf_merken(Some(&o));
        let z2 = Zustand::aus_datei(Some(&z.in_datei(None, "k").unwrap()), "k", "g").unwrap();
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
            ..Zustand::neu("g")
        };
        assert!(kaputt.offenen_widerruf().is_err());
    }

    #[test]
    fn beschaedigte_epochen_sind_ein_fehler_und_kein_stiller_verlust() {
        let mut z = Zustand::neu("g");
        z.epochen.insert("2".into(), "AAAA".into());
        assert!(z.bund(&KEY).is_err());
        z.epochen.clear();
        z.epochen.insert("zwei".into(), B64.encode([1u8; KEY_LEN]));
        assert!(z.bund(&KEY).is_err());
    }

    // --- Widerruf: erst sichern, dann Hub ---------------------------------------------------

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use wai_assistent::sync::{AuthGeraet, LeseFilter, MemHub, Sammlung, Schreiben, Umschlag};

    /// Hub mit Absturz-Schalter: 0 normal, 1 Aufruf kommt nie an, 2 Hub wendet an, Antwort geht verloren.
    struct Absturzhub {
        inner: MemHub,
        modus: Cell<u8>,
        datei: Rc<RefCell<Option<String>>>,
        aufrufe: Cell<u32>,
    }

    impl HubSpeicher for Absturzhub {
        fn schreiben(&self, a: &AuthGeraet, u: &Umschlag) -> Result<Schreiben, HubFehler> {
            self.inner.schreiben(a, u)
        }
        fn cas(
            &self,
            a: &AuthGeraet,
            s: Sammlung,
            id: &Hex128,
            e: Option<u64>,
            u: &Umschlag,
        ) -> Result<u64, HubFehler> {
            self.inner.cas(a, s, id, e, u)
        }
        fn holen(
            &self,
            a: &AuthGeraet,
            s: Sammlung,
            id: &Hex128,
        ) -> Result<Option<Umschlag>, HubFehler> {
            self.inner.holen(a, s, id)
        }
        fn lesen(&self, a: &AuthGeraet, f: &LeseFilter) -> Result<Vec<Umschlag>, HubFehler> {
            self.inner.lesen(a, f)
        }
        fn widerrufen(
            &self,
            a: &AuthGeraet,
            ziel: &str,
            erwartet: u64,
            vorgang: &Hex128,
        ) -> Result<u64, HubFehler> {
            self.aufrufe.set(self.aufrufe.get() + 1);
            // Kandidat und Vorgangskennung müssen zu diesem Zeitpunkt schon auf der Platte liegen.
            let gesichert = self.datei.borrow().as_deref().map(|t| {
                Zustand::aus_datei(Some(t), "k", "pc")
                    .unwrap()
                    .offener_widerruf
            });
            assert!(
                matches!(gesichert, Some(Some(ref d)) if d.vorgang == vorgang.as_str()),
                "Hub-Aufruf vor dem dauerhaften Sichern"
            );
            match self.modus.get() {
                1 => Err(HubFehler::Nichtverfuegbar),
                2 => {
                    let _ = self.inner.widerrufen(a, ziel, erwartet, vorgang);
                    Err(HubFehler::Nichtverfuegbar)
                }
                _ => self.inner.widerrufen(a, ziel, erwartet, vorgang),
            }
        }
        fn epoche(&self, a: &AuthGeraet) -> Result<u64, HubFehler> {
            self.inner.epoche(a)
        }
        fn zugelassene(&self, a: &AuthGeraet) -> Result<Vec<String>, HubFehler> {
            self.inner.zugelassene(a)
        }
    }

    fn widerruf_aufbau() -> Absturzhub {
        let inner = MemHub::new();
        inner.zulassen("pc");
        inner.zulassen("tablet");
        Absturzhub {
            inner,
            modus: Cell::new(0),
            datei: Rc::new(RefCell::new(None)),
            aufrufe: Cell::new(0),
        }
    }

    /// Ein „Prozesslauf“: Zustand frisch aus der Datei lesen, widerrufen, Datei schreiben.
    fn widerruf_lauf(hub: &Absturzhub) -> Result<u64, WiderrufFehler> {
        let text = hub.datei.borrow().clone();
        let mut z = Zustand::aus_datei(text.as_deref(), "k", "pc").unwrap();
        let datei = hub.datei.clone();
        widerruf_durchfuehren(hub, "pc", &KEY, &mut z, "tablet", &mut |z| {
            let alt = datei.borrow().clone();
            let neu = z.in_datei(alt.as_deref(), "k")?;
            *datei.borrow_mut() = Some(neu);
            Ok(())
        })
    }

    fn gesicherter_kandidat(hub: &Absturzhub) -> OffenerWiderruf {
        let text = hub.datei.borrow().clone();
        Zustand::aus_datei(text.as_deref(), "k", "pc")
            .unwrap()
            .offenen_widerruf()
            .unwrap()
            .expect("Kandidat gesichert")
    }

    #[test]
    fn widerruf_absturz_zwischen_sichern_und_aufruf_wiederholt_mit_derselben_vorgangskennung() {
        let hub = widerruf_aufbau();
        hub.modus.set(1);
        assert!(matches!(
            widerruf_lauf(&hub),
            Err(WiderrufFehler::Hub(HubFehler::Nichtverfuegbar))
        ));
        let kandidat = gesicherter_kandidat(&hub);
        // Neustart: neuer Prozess, derselbe Vorgang, derselbe Schlüssel.
        hub.modus.set(0);
        assert_eq!(widerruf_lauf(&hub).unwrap(), 2);
        let text = hub.datei.borrow().clone();
        let z = Zustand::aus_datei(text.as_deref(), "k", "pc").unwrap();
        assert!(z.offener_widerruf.is_none(), "Vormerkung ist abgeschlossen");
        let bund = z.bund(&KEY).unwrap();
        assert_eq!(bund.aktuelle_epoche(), 2);
        assert_eq!(
            bund.schluessel(2).unwrap().bytes(),
            kandidat.schluessel().bytes(),
            "der vor dem Aufruf gesicherte Schlüssel gilt"
        );
        assert_eq!(hub.inner.epoche(&AuthGeraet::new("pc")).unwrap(), 2);
        assert!(!hub
            .inner
            .zugelassene(&AuthGeraet::new("pc"))
            .unwrap()
            .contains(&"tablet".to_string()));
    }

    #[test]
    fn widerruf_absturz_nach_dem_aufruf_verliert_den_schluessel_nicht() {
        let hub = widerruf_aufbau();
        hub.modus.set(2);
        assert!(widerruf_lauf(&hub).is_err(), "Antwort ging verloren");
        assert_eq!(
            hub.inner.epoche(&AuthGeraet::new("pc")).unwrap(),
            2,
            "Hub hat angewendet"
        );
        let kandidat = gesicherter_kandidat(&hub);
        hub.modus.set(0);
        assert_eq!(
            widerruf_lauf(&hub).unwrap(),
            2,
            "idempotent mit derselben Kennung"
        );
        let text = hub.datei.borrow().clone();
        let bund = Zustand::aus_datei(text.as_deref(), "k", "pc")
            .unwrap()
            .bund(&KEY)
            .unwrap();
        assert_eq!(
            bund.schluessel(2).unwrap().bytes(),
            kandidat.schluessel().bytes()
        );
        assert_eq!(hub.aufrufe.get(), 2);
        assert_eq!(
            hub.inner.epoche(&AuthGeraet::new("pc")).unwrap(),
            2,
            "keine dritte Epoche"
        );
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
