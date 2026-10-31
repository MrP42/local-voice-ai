//! Gekoppelte Apps des Sprachdienstes (ADR-0021): je App ein Zugang mit Fingerabdruck und
//! Rechten, widerrufbar, in einer eigenen Datei neben den Einstellungen.
//!
//! - **Fingerabdruck statt Schlüssel**: Die App erzeugt ihren Schlüssel selbst und schickt bei der
//!   Kopplung nur `SHA-256(schlüssel)`. Der Dienst prüft später den HMAC-Beweis des Handschlags
//!   gegen diesen Wert (`wai_voice_dienst::beweis`). Wer die Datei liest, kann sich anmelden; sie
//!   ist deshalb wie ein Geheimnis zu behandeln (Unix: Modus 0600, Windows: Profilordner des
//!   Benutzers). Der Fingerabdruck verlässt diese Datei nie in Richtung Oberfläche oder Log.
//! - **Rechte** (`hoeren`, `sprechen`) stehen nach der Kopplung auf „aus“, bis der Nutzer sie
//!   freigibt. `lernen` vergibt dieser Stand nicht (Paket S2 nimmt kein Korrektursignal an).
//! - **Widerruf** setzt `widerrufen_ms`; die Zeile bleibt (ein alter Schlüssel wird so als
//!   „widerrufen“ erkannt und der Fingerabdruck bleibt belegt). Erst **Entfernen** gibt ihn frei
//!   und erlaubt eine neue Kopplung ("Entkoppeln in Local Voice AI" des Protokolls).
//! - **Der Dienst ist standardmäßig aus** (`aktiv == false`, Opt-in des Nutzers).
//!
//! Die Datei wird bei jeder Änderung atomar ersetzt (Temp-Datei und Umbenennen).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use wai_voice_dienst::msg::Recht;

/// Höchstzahl aktiver (nicht widerrufener) Zugänge.
pub const MAX_ZUGAENGE: usize = 20;
/// `zuletzt_ms` wird höchstens so oft geschrieben (eine Zeile je Minute, nicht je Verbindung).
pub const BERUEHREN_INTERVALL_MS: i64 = 60_000;
const DATEI_VERSION: u32 = 1;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Zugang {
    /// Öffentliche Kennung (`[a-z0-9_-]`), die die App in `hallo` nennt.
    pub zugang: String,
    /// Kennung der App aus `koppeln` (`corporate-llm`).
    pub client: String,
    /// Angezeigter Name (vom Vertrag auf unverfängliche Zeichen geprüft).
    pub anzeige: String,
    /// `SHA-256(schlüssel)` als Kleinbuchstaben-Hex. Geheimnis, nie anzeigen oder loggen.
    pub fingerabdruck: String,
    pub rechte: Vec<Recht>,
    pub erstellt_ms: i64,
    #[serde(default)]
    pub zuletzt_ms: Option<i64>,
    #[serde(default)]
    pub widerrufen_ms: Option<i64>,
}

impl std::fmt::Debug for Zugang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Zugang")
            .field("zugang", &self.zugang)
            .field("client", &self.client)
            .field("fingerabdruck", &"…")
            .finish()
    }
}

impl Zugang {
    pub fn aktiv(&self) -> bool {
        self.widerrufen_ms.is_none()
    }

    pub fn hat_recht(&self, recht: Recht) -> bool {
        self.aktiv() && self.rechte.contains(&recht)
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Datei {
    #[serde(default)]
    version: u32,
    /// Opt-in: ohne `true` lauscht der Dienst nicht.
    #[serde(default)]
    aktiv: bool,
    #[serde(default)]
    zugaenge: Vec<Zugang>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fehler {
    Unbekannt,
    BereitsGekoppelt,
    ZuViele,
    Ungueltig(&'static str),
    Speichern(String),
}

impl std::fmt::Display for Fehler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unbekannt => f.write_str("Diese App ist nicht gekoppelt."),
            Self::BereitsGekoppelt => f.write_str("Diese App ist schon gekoppelt."),
            Self::ZuViele => {
                f.write_str("Es sind schon 20 Apps gekoppelt. Bitte zuerst eine widerrufen.")
            }
            Self::Ungueltig(was) => write!(f, "Ungültige Angabe: {was}."),
            Self::Speichern(e) => write!(f, "Die Kopplungen ließen sich nicht speichern: {e}"),
        }
    }
}

pub struct ZugangStore {
    pfad: Option<PathBuf>,
    daten: Mutex<Datei>,
}

impl ZugangStore {
    /// Nur im Speicher (Tests).
    pub fn im_speicher() -> Self {
        Self {
            pfad: None,
            daten: Mutex::new(Datei::default()),
        }
    }

    /// Liest die Datei; fehlt sie, beginnt der Speicher leer und aus. Eine unlesbare Datei wird
    /// zur Seite gelegt (`.defekt`) statt überschrieben, der Dienst bleibt aus und ohne Kopplungen.
    pub fn oeffnen(pfad: PathBuf) -> Self {
        let daten = match std::fs::read(&pfad) {
            Ok(bytes) => serde_json::from_slice::<Datei>(&bytes).unwrap_or_else(|_| {
                log::warn!("sprachdienst: Kopplungsdatei unlesbar, wird zur Seite gelegt");
                let _ = std::fs::rename(&pfad, pfad.with_extension("defekt"));
                Datei::default()
            }),
            Err(_) => Datei::default(),
        };
        Self {
            pfad: Some(pfad),
            daten: Mutex::new(daten),
        }
    }

    fn sperre(&self) -> std::sync::MutexGuard<'_, Datei> {
        self.daten.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ändert eine Kopie, speichert sie und übernimmt sie erst bei Erfolg.
    fn aendern<R>(&self, f: impl FnOnce(&mut Datei) -> Result<R, Fehler>) -> Result<R, Fehler> {
        let mut g = self.sperre();
        let mut kopie = g.clone();
        let r = f(&mut kopie)?;
        kopie.version = DATEI_VERSION;
        if let Some(pfad) = &self.pfad {
            speichern(pfad, &kopie).map_err(|e| Fehler::Speichern(e.to_string()))?;
        }
        *g = kopie;
        Ok(r)
    }

    pub fn aktiv(&self) -> bool {
        self.sperre().aktiv
    }

    pub fn aktiv_setzen(&self, aktiv: bool) -> Result<(), Fehler> {
        self.aendern(|d| {
            d.aktiv = aktiv;
            Ok(())
        })
    }

    pub fn liste(&self) -> Vec<Zugang> {
        self.sperre().zugaenge.clone()
    }

    pub fn get(&self, zugang: &str) -> Option<Zugang> {
        self.sperre()
            .zugaenge
            .iter()
            .find(|z| z.zugang == zugang)
            .cloned()
    }

    /// Belegt der Fingerabdruck schon einen Zugang (auch einen widerrufenen)?
    pub fn bereits_gekoppelt(&self, fingerabdruck: &str) -> bool {
        self.sperre()
            .zugaenge
            .iter()
            .any(|z| z.fingerabdruck == fingerabdruck)
    }

    /// Legt nach der Bestätigung durch den Nutzer einen Zugang an. Rechte: keine.
    pub fn hinzufuegen(
        &self,
        client: &str,
        anzeige: &str,
        fingerabdruck: &str,
        jetzt_ms: i64,
    ) -> Result<Zugang, Fehler> {
        self.aendern(|d| {
            if d.zugaenge.iter().any(|z| z.fingerabdruck == fingerabdruck) {
                return Err(Fehler::BereitsGekoppelt);
            }
            if d.zugaenge.iter().filter(|z| z.aktiv()).count() >= MAX_ZUGAENGE {
                return Err(Fehler::ZuViele);
            }
            let z = Zugang {
                zugang: neue_kennung(d),
                client: client.to_string(),
                anzeige: anzeige.to_string(),
                fingerabdruck: fingerabdruck.to_string(),
                rechte: Vec::new(),
                erstellt_ms: jetzt_ms,
                zuletzt_ms: None,
                widerrufen_ms: None,
            };
            d.zugaenge.push(z.clone());
            Ok(z)
        })
    }

    /// Schaltet ein Recht ein oder aus. Nur `hoeren` und `sprechen`; ein widerrufener Zugang
    /// bekommt keine Rechte mehr.
    pub fn recht_setzen(&self, zugang: &str, recht: Recht, an: bool) -> Result<Zugang, Fehler> {
        if recht == Recht::Lernen {
            return Err(Fehler::Ungueltig("lernen"));
        }
        self.aendern(|d| {
            let z = d
                .zugaenge
                .iter_mut()
                .find(|z| z.zugang == zugang)
                .ok_or(Fehler::Unbekannt)?;
            if !z.aktiv() {
                return Err(Fehler::Ungueltig("widerrufen"));
            }
            z.rechte.retain(|r| *r != recht);
            if an {
                z.rechte.push(recht);
            }
            Ok(z.clone())
        })
    }

    /// Zieht den Zugang zurück: Er gilt sofort nicht mehr, die Zeile bleibt.
    pub fn widerrufen(&self, zugang: &str, jetzt_ms: i64) -> Result<Zugang, Fehler> {
        self.aendern(|d| {
            let z = d
                .zugaenge
                .iter_mut()
                .find(|z| z.zugang == zugang)
                .ok_or(Fehler::Unbekannt)?;
            if z.widerrufen_ms.is_none() {
                z.widerrufen_ms = Some(jetzt_ms);
                z.rechte.clear();
            }
            Ok(z.clone())
        })
    }

    /// Entfernt den Zugang ganz (der Fingerabdruck wird frei, die App kann neu koppeln).
    pub fn entfernen(&self, zugang: &str) -> Result<(), Fehler> {
        self.aendern(|d| {
            let vorher = d.zugaenge.len();
            d.zugaenge.retain(|z| z.zugang != zugang);
            if d.zugaenge.len() == vorher {
                return Err(Fehler::Unbekannt);
            }
            Ok(())
        })
    }

    /// Merkt die letzte Nutzung (höchstens einmal je Minute geschrieben). Fehler beim Speichern
    /// sind hier unerheblich.
    pub fn beruehren(&self, zugang: &str, jetzt_ms: i64) {
        let faellig = self.get(zugang).is_some_and(|z| {
            z.zuletzt_ms
                .is_none_or(|t| jetzt_ms - t >= BERUEHREN_INTERVALL_MS)
        });
        if !faellig {
            return;
        }
        let _ = self.aendern(|d| {
            if let Some(z) = d.zugaenge.iter_mut().find(|z| z.zugang == zugang) {
                z.zuletzt_ms = Some(jetzt_ms);
            }
            Ok(())
        });
    }
}

/// `z-` plus 12 Hex-Zeichen aus dem Zufall des Betriebssystems, nicht doppelt vergeben.
fn neue_kennung(d: &Datei) -> String {
    loop {
        let mut b = [0u8; 6];
        OsRng.fill_bytes(&mut b);
        let id = format!(
            "z-{}",
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        );
        if !d.zugaenge.iter().any(|z| z.zugang == id) {
            return id;
        }
    }
}

/// Fingerabdruck (64 Hex-Zeichen) als Bytes; `None` bei jeder anderen Form.
pub fn fingerabdruck_bytes(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

// TODO(L2, Issue "Sprachdienst: Kopplungsdatei unter Windows schuetzen"): Unter Windows erbt die
// Datei die Rechte des Datenordners (unter Unix gilt 0600). Die Fingerabdruecke sind wie
// Geheimnisse zu behandeln (wer sie liest, kann sich anmelden): DACL nur fuer den Benutzer
// setzen und/oder den Inhalt mit DPAPI (CryptProtectData, Benutzerbereich) verschluesseln.
// Bewusst noch nicht umgesetzt.
fn speichern(pfad: &Path, daten: &Datei) -> std::io::Result<()> {
    if let Some(ordner) = pfad.parent() {
        std::fs::create_dir_all(ordner)?;
    }
    let json = serde_json::to_vec_pretty(daten).map_err(std::io::Error::other)?;
    let tmp = pfad.with_extension("tmp");
    schreiben_privat(&tmp, &json)?;
    std::fs::rename(&tmp, pfad)
}

#[cfg(unix)]
fn schreiben_privat(pfad: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(pfad)?;
    f.write_all(bytes)?;
    f.sync_all()
}

#[cfg(not(unix))]
fn schreiben_privat(pfad: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(pfad, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn koppeln_widerrufen_entfernen() {
        let s = ZugangStore::im_speicher();
        assert!(!s.aktiv(), "Opt-in: standardmäßig aus");
        let z = s
            .hinzufuegen("corporate-llm", "Corporate LLM", FP, 100)
            .unwrap();
        assert!(z.rechte.is_empty(), "neue Zugänge haben keine Rechte");
        assert!(z.zugang.starts_with("z-") && z.zugang.len() == 14);
        assert_eq!(
            s.hinzufuegen("x", "X", FP, 200),
            Err(Fehler::BereitsGekoppelt)
        );

        let z = s.recht_setzen(&z.zugang, Recht::Hoeren, true).unwrap();
        assert!(z.hat_recht(Recht::Hoeren) && !z.hat_recht(Recht::Sprechen));
        assert_eq!(
            s.recht_setzen(&z.zugang, Recht::Lernen, true),
            Err(Fehler::Ungueltig("lernen"))
        );

        let w = s.widerrufen(&z.zugang, 300).unwrap();
        assert!(
            !w.aktiv() && !w.hat_recht(Recht::Hoeren),
            "Widerruf entzieht alles"
        );
        assert!(
            s.bereits_gekoppelt(FP),
            "der Fingerabdruck bleibt bis zum Entfernen belegt"
        );
        assert_eq!(
            s.hinzufuegen("x", "X", FP, 400),
            Err(Fehler::BereitsGekoppelt)
        );
        assert_eq!(
            s.recht_setzen(&z.zugang, Recht::Sprechen, true),
            Err(Fehler::Ungueltig("widerrufen"))
        );

        s.entfernen(&z.zugang).unwrap();
        assert!(!s.bereits_gekoppelt(FP));
        assert_eq!(s.entfernen(&z.zugang), Err(Fehler::Unbekannt));
        assert!(s
            .hinzufuegen("corporate-llm", "Corporate LLM", FP, 500)
            .is_ok());
    }

    #[test]
    fn hoechstens_zwanzig_aktive() {
        let s = ZugangStore::im_speicher();
        for i in 0..MAX_ZUGAENGE {
            s.hinzufuegen("a", "A", &format!("{i:064x}"), 1).unwrap();
        }
        assert_eq!(
            s.hinzufuegen("a", "A", &format!("{:064x}", 99), 1),
            Err(Fehler::ZuViele)
        );
        let erste = s.liste()[0].zugang.clone();
        s.widerrufen(&erste, 2).unwrap();
        assert!(s.hinzufuegen("a", "A", &format!("{:064x}", 99), 3).is_ok());
    }

    #[test]
    fn beruehren_ist_gedrosselt() {
        let s = ZugangStore::im_speicher();
        let z = s.hinzufuegen("a", "A", FP, 1).unwrap();
        s.beruehren(&z.zugang, 1_000);
        s.beruehren(&z.zugang, 1_000 + BERUEHREN_INTERVALL_MS - 1);
        assert_eq!(s.get(&z.zugang).unwrap().zuletzt_ms, Some(1_000));
        s.beruehren(&z.zugang, 1_000 + BERUEHREN_INTERVALL_MS);
        assert_eq!(
            s.get(&z.zugang).unwrap().zuletzt_ms,
            Some(1_000 + BERUEHREN_INTERVALL_MS)
        );
    }

    #[test]
    fn fingerabdruck_wird_gelesen() {
        assert!(fingerabdruck_bytes(FP).is_some());
        assert!(fingerabdruck_bytes(&FP[..62]).is_none());
        assert!(fingerabdruck_bytes(&format!("{}zz", &FP[..62])).is_none());
        assert_eq!(fingerabdruck_bytes(FP).unwrap()[0], 0x01);
    }

    #[test]
    fn datei_ueberlebt_neustart_und_defekte_wird_beiseite_gelegt() {
        let ordner = tempfile::tempdir().unwrap();
        let pfad = ordner.path().join("sprachdienst.json");
        let s = ZugangStore::oeffnen(pfad.clone());
        s.aktiv_setzen(true).unwrap();
        let z = s.hinzufuegen("a", "A", FP, 1).unwrap();
        s.recht_setzen(&z.zugang, Recht::Sprechen, true).unwrap();
        drop(s);

        let s = ZugangStore::oeffnen(pfad.clone());
        assert!(s.aktiv());
        assert!(s.get(&z.zugang).unwrap().hat_recht(Recht::Sprechen));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let modus = std::fs::metadata(&pfad).unwrap().permissions().mode() & 0o777;
            assert_eq!(modus, 0o600, "die Datei enthält Fingerabdrücke");
        }

        std::fs::write(&pfad, b"{kaputt").unwrap();
        let s = ZugangStore::oeffnen(pfad.clone());
        assert!(!s.aktiv() && s.liste().is_empty());
        assert!(pfad.with_extension("defekt").exists());
    }

    #[test]
    fn debug_zeigt_den_fingerabdruck_nicht() {
        let s = ZugangStore::im_speicher();
        let z = s.hinzufuegen("a", "A", FP, 1).unwrap();
        assert!(!format!("{z:?}").contains("0123456789abcdef"));
    }
}
