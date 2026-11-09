//! Zustand des Assistent-Syncs auf diesem Gerät (lva#388, ADR-0028 E3.8): reine Logik ohne Tauri und
//! ohne Netz.
//!
//! **Zustandsblock des Kerns** ([`DateiSpeicher`], `Zustandsspeicher`). Der Kern legt seinen ganzen
//! Zustand (Geräteschlüssel, Schlüsselbund, geprüftes Geräteverzeichnis, Höchststände als Prüfpunkte,
//! Grabsteine, Ausgang, offener Widerruf, Unterhaltungen) als **einen** Block ab. Er enthält Geheimnisse
//! und wird deshalb verschlüsselt (XChaCha20-Poly1305, Schlüssel per HKDF aus dem Objekt-Schlüssel des
//! Bibliotheks-Syncs, eigener Zweck in `info`) und **atomar** gesichert: temporäre Datei schreiben,
//! auf die Platte zwingen, per Umbenennen ersetzen. Je Konto und Gerät gibt es eine Datei; die AAD
//! bindet den Block an beide, ein vertauschter oder veränderter Block lädt nicht. Jeder Lese-, Prüf- oder
//! Schreibfehler ist ein `SpeicherFehler`: der Kern sperrt sich dann (Must-fix 4), nichts gilt als leer.
//! Bewusst keine `.bak`: eine ältere Kopie wäre ein Rollback des Zustands.
//!
//! **Transportzulassung** (`assistent_sync.json`, [`Zustand`]): je Kontokennung, ob dieses Gerät am Hub
//! per `POST /geraet` zugelassen ist. Sie ist keine Sicherheitsaussage (die steht im Verzeichnis); die
//! Datei ist fail-closed (nur „fehlt“ gilt als leer) und bekommt vor jedem Überschreiben eine `.bak`.
//!
//! **Kopplungs-QR** ([`KopplungsQr`]): Angebot des Kerns (Prüfpunkt des Verzeichnisses und
//! QR-Geheimnis) samt Freigabecode für die Transportzulassung des neuen Geräts.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use wai_assistent::sync::{Kopplungsangebot, SpeicherFehler, Zustandsspeicher};
use zeroize::Zeroizing;

pub const DATEI: &str = "assistent_sync.json";
/// Verzeichnis der Zustandsblöcke (je Konto und Gerät eine Datei) im Datenverzeichnis der App.
pub const BLOCK_VERZEICHNIS: &str = "assistent_sync";
pub const KEY_LEN: usize = 32;

const INFO_KONTO: &[u8] = b"wai-assistent-sync/v1/konto-fingerabdruck";
const INFO_BLOCK: &[u8] = b"wai-assistent-sync/v2/zustandsblock";
const BLOCK_KENNUNG: &[u8; 8] = b"LVAZB\x00\x00\x01";
const NONCE_LEN: usize = 24;

// ---------------------------------------------------------------------------------------------
// Zustandsblock des Kerns
// ---------------------------------------------------------------------------------------------

/// `Zustandsspeicher` über eine verschlüsselte Datei, atomar ersetzt.
pub struct DateiSpeicher {
    pfad: PathBuf,
    schluessel: Zeroizing<[u8; KEY_LEN]>,
    aad: Vec<u8>,
}

impl fmt::Debug for DateiSpeicher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DateiSpeicher")
    }
}

impl DateiSpeicher {
    /// Speicher für `konto` (Kontokennung) und `geraet` unter `verzeichnis`. Der Blockschlüssel wird aus
    /// dem Objekt-Schlüssel des Bibliotheks-Syncs abgeleitet und nie mit ihm geteilt.
    pub fn neu(
        verzeichnis: &Path,
        konto: &str,
        geraet: &str,
        konto_schluessel: &[u8; KEY_LEN],
    ) -> Result<Self, String> {
        let mut schluessel = Zeroizing::new([0u8; KEY_LEN]);
        Hkdf::<Sha256>::new(None, konto_schluessel)
            .expand(INFO_BLOCK, schluessel.as_mut())
            .map_err(|_| "Schlüsselableitung fehlgeschlagen".to_string())?;
        let mut aad = BLOCK_KENNUNG.to_vec();
        for teil in [konto.as_bytes(), geraet.as_bytes()] {
            aad.extend_from_slice(&(teil.len() as u32).to_be_bytes());
            aad.extend_from_slice(teil);
        }
        Ok(Self {
            pfad: verzeichnis.join(block_dateiname(konto, geraet)),
            schluessel,
            aad,
        })
    }

    #[cfg(test)]
    pub fn pfad(&self) -> &Path {
        &self.pfad
    }

    /// Liegt schon ein Block (ohne ihn zu lesen)?
    pub fn vorhanden(&self) -> bool {
        self.pfad.exists()
    }

    fn chiffre(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(self.schluessel.as_ref().into())
    }

    fn schreiben_atomar(&self, inhalt: &[u8]) -> std::io::Result<()> {
        let dir = self
            .pfad
            .parent()
            .ok_or_else(|| std::io::Error::other("kein Verzeichnis"))?;
        std::fs::create_dir_all(dir)?;
        let mut tmp_name = self
            .pfad
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        tmp_name.push(".tmp");
        let tmp = self.pfad.with_file_name(tmp_name);
        {
            let mut opt = std::fs::OpenOptions::new();
            opt.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opt.mode(0o600);
            }
            let mut f = opt.open(&tmp)?;
            f.write_all(inhalt)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.pfad)?;
        // Das Umbenennen selbst dauerhaft machen (unter Windows nicht nötig und nicht möglich).
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    }
}

/// Dateiname des Blocks: Hash aus Konto und Gerät (keine Kennung im Klartext auf der Platte).
fn block_dateiname(konto: &str, geraet: &str) -> String {
    let mut h = Sha256::new();
    for teil in [konto.as_bytes(), geraet.as_bytes()] {
        h.update((teil.len() as u32).to_be_bytes());
        h.update(teil);
    }
    let hex: String = h.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{hex}.bin")
}

impl Zustandsspeicher for DateiSpeicher {
    fn sichern(&self, daten: &[u8]) -> Result<(), SpeicherFehler> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| SpeicherFehler)?;
        let ct = self
            .chiffre()
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: daten,
                    aad: &self.aad,
                },
            )
            .map_err(|_| SpeicherFehler)?;
        let mut inhalt = Vec::with_capacity(BLOCK_KENNUNG.len() + NONCE_LEN + ct.len());
        inhalt.extend_from_slice(BLOCK_KENNUNG);
        inhalt.extend_from_slice(&nonce);
        inhalt.extend_from_slice(&ct);
        self.schreiben_atomar(&inhalt).map_err(|_| {
            log::warn!("Assistent-Sync: Zustandsblock konnte nicht gesichert werden");
            SpeicherFehler
        })
    }

    fn laden(&self) -> Result<Option<Zeroizing<Vec<u8>>>, SpeicherFehler> {
        let roh = match std::fs::read(&self.pfad) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(SpeicherFehler),
        };
        let kopf = BLOCK_KENNUNG.len() + NONCE_LEN;
        if roh.len() < kopf || &roh[..BLOCK_KENNUNG.len()] != BLOCK_KENNUNG {
            return Err(SpeicherFehler);
        }
        self.chiffre()
            .decrypt(
                XNonce::from_slice(&roh[BLOCK_KENNUNG.len()..kopf]),
                Payload {
                    msg: &roh[kopf..],
                    aad: &self.aad,
                },
            )
            .map(|klar| Some(Zeroizing::new(klar)))
            .map_err(|_| SpeicherFehler)
    }
}

// ---------------------------------------------------------------------------------------------
// Transportzulassung je Konto (`assistent_sync.json`)
// ---------------------------------------------------------------------------------------------

/// Eintrag einer Kontokennung in `assistent_sync.json`.
#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Eq, Debug)]
pub struct Zustand {
    /// Gerät, für das `zugelassen` gilt.
    #[serde(default)]
    pub geraet: String,
    /// Dieses Gerät ist am Hub zugelassen (`POST /geraet` war erfolgreich).
    #[serde(default)]
    pub zugelassen: bool,
}

/// Inhalt von `assistent_sync.json`. Felder früherer Fassungen (Epochenschlüssel, offener Widerruf
/// der alten Hub-API) werden beim nächsten Schreiben verworfen; A1 war nie freigeschaltet.
#[derive(Clone, Serialize, Deserialize, Default)]
struct Datei {
    #[serde(default)]
    konten: BTreeMap<String, Zustand>,
}

impl Zustand {
    /// Leerer Zustand für dieses Gerät.
    #[cfg(test)]
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
            // Neue Geräte-Kennung: die Zulassung gilt nicht mehr.
            z.geraet = geraet.to_owned();
            z.zugelassen = false;
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
}

// ---------------------------------------------------------------------------------------------
// Kopplungs-QR
// ---------------------------------------------------------------------------------------------

/// Größte Transportform des QR-Inhalts (Angebot ≤ 2048 Byte plus Freigabecode und Kennung).
pub const MAX_QR: usize = 3072;
const QR_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QrForm {
    v: u32,
    geraet_id: String,
    freigabe_code: String,
    angebot: Value,
}

/// Inhalt des Kopplungs-QR-Codes: Angebot des aufnehmenden Geräts (Prüfpunkt des Verzeichnisses und
/// QR-Geheimnis) und der Freigabecode, mit dem sich das neue Gerät `geraet_id` am Hub zulässt.
/// `Debug` zeigt weder Code noch Geheimnis.
pub struct KopplungsQr {
    pub geraet_id: String,
    pub freigabe_code: Zeroizing<String>,
    pub angebot: Kopplungsangebot,
}

impl fmt::Debug for KopplungsQr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KopplungsQr")
            .field("freigabe_code", &"***")
            .field("angebot", &self.angebot)
            .finish()
    }
}

impl KopplungsQr {
    /// Transportform für den QR-Code (enthält Geheimnisse; der Aufrufer zeigt sie nur einmal an).
    pub fn als_text(&self) -> Result<Zeroizing<String>, String> {
        let angebot =
            serde_json::to_value(&self.angebot).map_err(|_| "Kopplungsangebot".to_string())?;
        serde_json::to_string(&QrForm {
            v: QR_VERSION,
            geraet_id: self.geraet_id.clone(),
            freigabe_code: self.freigabe_code.to_string(),
            angebot,
        })
        .map(Zeroizing::new)
        .map_err(|_| "Kopplungsangebot".to_string())
    }

    /// Liest den QR-Inhalt; Länge vor dem Lesen begrenzt, das Angebot über die Grenze des Kerns.
    pub fn aus_text(text: &str) -> Result<Self, String> {
        let fehler = || "Der Kopplungscode ist ungültig.".to_string();
        if text.len() > MAX_QR {
            return Err(fehler());
        }
        let form: QrForm = serde_json::from_str(text.trim()).map_err(|_| fehler())?;
        if form.v != QR_VERSION || !gueltige_kennung(&form.geraet_id) {
            return Err(fehler());
        }
        let code_ok = (16..=128).contains(&form.freigabe_code.len())
            && form
                .freigabe_code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !code_ok {
            return Err(fehler());
        }
        let roh = Zeroizing::new(serde_json::to_vec(&form.angebot).map_err(|_| fehler())?);
        let angebot = Kopplungsangebot::aus_json(&roh).map_err(|_| fehler())?;
        Ok(Self {
            geraet_id: form.geraet_id,
            freigabe_code: Zeroizing::new(form.freigabe_code),
            angebot,
        })
    }
}

/// Kennung dieses Geräts für den Assistent-Hub: klein geschrieben, `[A-Za-z0-9._-]{1,64}`. Der Hub
/// nimmt jede Kennung von 1 bis 128 Byte unverändert; die App bleibt bei ihrer strengeren, stabilen Form.
pub fn geraete_kennung(device_id: &str) -> Option<String> {
    let k = device_id.to_ascii_lowercase();
    gueltige_kennung(&k).then_some(k)
}

/// Form einer Geräte-Kennung der App.
pub fn gueltige_kennung(k: &str) -> bool {
    (1..=64).contains(&k.len())
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wai_assistent::sync::{Abgleich, MemHub, Testkonto};

    const KEY: [u8; KEY_LEN] = [7; KEY_LEN];

    fn speicher(dir: &Path, konto: &str, geraet: &str) -> DateiSpeicher {
        DateiSpeicher::neu(dir, konto, geraet, &KEY).unwrap()
    }

    #[test]
    fn block_rundlauf_verschluesselt_und_atomar() {
        let dir = tempfile::tempdir().unwrap();
        let s = speicher(dir.path(), "konto-1", "dev-a");
        assert_eq!(s.laden().unwrap(), None, "fehlt: noch nie gesichert");
        s.sichern(b"{\"geheim\":\"Schluessel-4711\"}").unwrap();
        let roh = std::fs::read(s.pfad()).unwrap();
        assert!(
            !String::from_utf8_lossy(&roh).contains("Schluessel-4711"),
            "kein Klartext auf der Platte"
        );
        assert_eq!(
            s.laden().unwrap().unwrap().as_slice(),
            b"{\"geheim\":\"Schluessel-4711\"}"
        );
        s.sichern(b"zwei").unwrap();
        assert_eq!(s.laden().unwrap().unwrap().as_slice(), b"zwei");
        // Keine Reste: nur der Block selbst liegt im Verzeichnis.
        let dateien: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(dateien.len(), 1);
        // Kennungen stehen nicht im Dateinamen.
        let name = s.pfad().file_name().unwrap().to_string_lossy().to_string();
        assert!(!name.contains("dev-a") && !name.contains("konto-1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(s.pfad()).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "nur für den Eigentümer");
        }
    }

    #[test]
    fn veraenderter_vertauschter_oder_fremder_block_ist_ein_fehler() {
        let dir = tempfile::tempdir().unwrap();
        let a = speicher(dir.path(), "konto-1", "dev-a");
        a.sichern(b"stand").unwrap();
        // Ein Bit gekippt: Fehler, nie „leer“.
        let mut roh = std::fs::read(a.pfad()).unwrap();
        let n = roh.len() - 1;
        roh[n] ^= 1;
        std::fs::write(a.pfad(), &roh).unwrap();
        assert_eq!(a.laden(), Err(SpeicherFehler));
        // Abgeschnitten.
        std::fs::write(a.pfad(), b"LVAZ").unwrap();
        assert_eq!(a.laden(), Err(SpeicherFehler));
        // Block eines anderen Geräts unter diesem Namen: AAD passt nicht.
        let b = speicher(dir.path(), "konto-1", "dev-b");
        b.sichern(b"anderes").unwrap();
        std::fs::copy(b.pfad(), a.pfad()).unwrap();
        assert_eq!(a.laden(), Err(SpeicherFehler));
        // Anderer Bibliotheks-Schlüssel: nicht lesbar.
        let fremd = DateiSpeicher::neu(dir.path(), "konto-1", "dev-b", &[8; KEY_LEN]).unwrap();
        assert_eq!(fremd.laden(), Err(SpeicherFehler));
    }

    #[test]
    fn nicht_schreibbares_ziel_ist_ein_speicherfehler_und_sperrt_den_kern() {
        let dir = tempfile::tempdir().unwrap();
        // Ziel ist ein Verzeichnis: das Umbenennen scheitert.
        let s = speicher(dir.path(), "konto-1", "dev-a");
        std::fs::create_dir_all(s.pfad()).unwrap();
        assert_eq!(s.sichern(b"x"), Err(SpeicherFehler));
        // Der Kern übernimmt keinen Zustand, den er nicht sichern konnte.
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["dev-a"], &hub);
        let kaputt = speicher(dir.path(), "konto-1", "dev-a");
        let gekoppelt = Abgleich::gekoppelt(
            k.schluessel("dev-a").clone(),
            k.bund().clone(),
            k.verzeichnis().clone(),
            Box::new(kaputt),
        );
        assert!(gekoppelt.is_err(), "erste Sicherung scheitert");
    }

    #[test]
    fn kern_ueberlebt_den_neustart_ueber_den_dateispeicher() {
        let dir = tempfile::tempdir().unwrap();
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["dev-a"], &hub);
        let mut a = Abgleich::gekoppelt(
            k.schluessel("dev-a").clone(),
            k.bund().clone(),
            k.verzeichnis().clone(),
            Box::new(speicher(dir.path(), "konto-1", "dev-a")),
        )
        .unwrap();
        a.freigabe_einschalten().unwrap();
        a.herunterladen(&hub).unwrap();
        let konto = a.konto().clone();
        drop(a);
        let wieder = Abgleich::wiederherstellen(Box::new(speicher(dir.path(), "konto-1", "dev-a")))
            .unwrap()
            .expect("gesichert");
        assert_eq!(wieder.konto(), &konto);
        assert!(wieder.freigabe().ist_aktiv());
        assert!(
            Abgleich::wiederherstellen(Box::new(speicher(dir.path(), "konto-2", "dev-a")))
                .unwrap()
                .is_none(),
            "anderes Konto: eigener Block"
        );
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
    }

    #[test]
    fn zulassung_je_konto_und_geraet() {
        let mut z = Zustand::neu("geraet-a");
        z.zugelassen = true;
        let json = z.in_datei(None, "konto-1").unwrap();
        let z2 = Zustand::neu("geraet-a");
        let json = z2.in_datei(Some(&json), "konto-2").unwrap();
        assert!(
            Zustand::aus_datei(Some(&json), "konto-1", "geraet-a")
                .unwrap()
                .zugelassen
        );
        assert!(
            !Zustand::aus_datei(Some(&json), "konto-2", "geraet-a")
                .unwrap()
                .zugelassen
        );
        assert!(
            !Zustand::aus_datei(Some(&json), "konto-1", "geraet-b")
                .unwrap()
                .zugelassen,
            "neues Gerät muss neu zugelassen werden"
        );
        // Felder der alten Fassung stören nicht und verschwinden beim nächsten Schreiben.
        let alt = r#"{"konten":{"k":{"geraet":"g","zugelassen":true,"epochen":{"2":"AAAA"},"offener_widerruf":null}}}"#;
        let z = Zustand::aus_datei(Some(alt), "k", "g").unwrap();
        assert!(z.zugelassen);
        assert!(!z.in_datei(Some(alt), "k").unwrap().contains("epochen"));
    }

    #[test]
    fn unlesbare_datei_ist_ein_fehler_und_wird_nie_ueberschrieben() {
        assert!(Zustand::aus_datei(Some("{kaputt"), "k", "g").is_err());
        assert!(Zustand::neu("g").in_datei(Some("{kaputt"), "k").is_err());
        assert!(Zustand::aus_datei(Some(""), "k", "g").is_err());
    }

    #[test]
    fn kopplungs_qr_rundlauf_ohne_geheimnis_im_debug() {
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["dev-a"], &hub);
        let a = k.geraet("dev-a");
        let qr = KopplungsQr {
            geraet_id: "dev-neu".into(),
            freigabe_code: Zeroizing::new("Abcdefghijklmnopqrstuvwxyz0".into()),
            angebot: a.kopplung_anbieten().unwrap(),
        };
        let text = qr.als_text().unwrap();
        let zurueck = KopplungsQr::aus_text(&text).unwrap();
        assert_eq!(zurueck.geraet_id, "dev-neu");
        assert_eq!(
            zurueck.freigabe_code.as_str(),
            "Abcdefghijklmnopqrstuvwxyz0"
        );
        assert_eq!(&zurueck.angebot.konto, a.konto());
        assert_eq!(zurueck.angebot.laenge, a.verzeichnis().laenge());
        let dbg = format!("{zurueck:?}");
        assert!(!dbg.contains("Abcdefghijklmnopqrstuvwxyz0"));
        let geheimnis = serde_json::from_str::<Value>(&text).unwrap()["angebot"]["geheimnis"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(!dbg.contains(&geheimnis));
        // Kaputte oder fremde Formen.
        for falsch in [
            String::from("{}"),
            text.replace("\"v\":1", "\"v\":2"),
            text.replace("dev-neu", "hat leerzeichen"),
            text.replace("Abcdefghijklmnopqrstuvwxyz0", "kurz"),
            format!("{}{}", &text[..text.len() - 1], ",\"x\":1}"),
            "x".repeat(MAX_QR + 1),
        ] {
            assert!(KopplungsQr::aus_text(&falsch).is_err(), "{falsch:.40}");
        }
    }

    #[test]
    fn geraetekennung_der_app() {
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
