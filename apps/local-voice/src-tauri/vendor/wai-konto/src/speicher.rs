use crate::beobachter::Beobachter;
use crate::modell::{Profil, ProfilAenderung, ProfilDatei, ProfilEingabe};
use crate::{BILD_MAX_BYTES, DATEI_VERSION, FARBEN, KontoFehler, pruefen};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PNG_SIGNATUR: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const STANDARDNAME: &str = "Lokales Profil";
const POLL_INTERVALL: Duration = Duration::from_secs(1);

/// Standardwurzel des Kontospeichers je Betriebssystem (ADR-0024 E2).
pub fn standard_wurzel() -> Result<PathBuf, KontoFehler> {
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let basis = if cfg!(windows) {
        env("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        env("XDG_DATA_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| env("HOME").map(|h| h.join(".local").join("share")))
    };
    basis
        .map(|b| b.join("WolffAppliedAI").join("konto"))
        .ok_or(KontoFehler::KeinDatenordner)
}

/// Zugriff auf den gemeinsamen Kontospeicher. Billig klonbar; jede Instanz liest frisch.
#[derive(Debug, Clone)]
pub struct KontoSpeicher {
    wurzel: PathBuf,
}

impl KontoSpeicher {
    /// Öffnet den Speicher in `wurzel`; legt beim ersten Mal ein „Lokales Profil“ an.
    pub fn oeffnen(wurzel: PathBuf) -> Result<Self, KontoFehler> {
        fs::create_dir_all(&wurzel)?;
        let s = Self { wurzel };
        let _sperre = s.sperren()?;
        if !s.datei_pfad().try_exists()? {
            let profil = neues_profil(
                ProfilEingabe {
                    anzeigename: STANDARDNAME.into(),
                    ..ProfilEingabe::default()
                },
                0,
            )?;
            let datei = ProfilDatei {
                version: DATEI_VERSION,
                generation: 1,
                sitzungsrevision: 1,
                aktiv: profil.id.clone(),
                profile: vec![profil],
            };
            s.schreiben(&datei)?;
        }
        Ok(s)
    }

    /// Öffnet den Speicher an der Standardwurzel.
    pub fn oeffnen_standard() -> Result<Self, KontoFehler> {
        Self::oeffnen(standard_wurzel()?)
    }

    pub fn wurzel(&self) -> &Path {
        &self.wurzel
    }

    /// Liest die aktuelle Profildatei.
    pub fn lesen(&self) -> Result<ProfilDatei, KontoFehler> {
        let mut versuch = 0;
        loop {
            match fs::read(self.datei_pfad()) {
                Ok(bytes) => return pruefen_datei(&bytes),
                // Unter Windows kann der Austausch der Datei kurz den Zugriff verweigern.
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied && versuch < 20 => {
                    versuch += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Aktuelle Sitzungsrevision (frisch von der Platte).
    pub fn sitzungsrevision(&self) -> Result<u64, KontoFehler> {
        Ok(self.lesen()?.sitzungsrevision)
    }

    /// Prüft, ob die Sitzungsrevision noch `erwartet` entspricht. Apps rufen das beim
    /// Aktivieren des Fensters und vor jeder geschützten Aktion auf; `false` heißt:
    /// Profil oder Anmeldung haben sich geändert, Zustand neu laden.
    pub fn pruefe_revision(&self, erwartet: u64) -> Result<bool, KontoFehler> {
        Ok(self.sitzungsrevision()? == erwartet)
    }

    /// Das aktive Profil.
    pub fn aktiv(&self) -> Result<Profil, KontoFehler> {
        let datei = self.lesen()?;
        datei
            .profile
            .into_iter()
            .find(|p| p.id == datei.aktiv)
            .ok_or(KontoFehler::NichtGefunden(datei.aktiv))
    }

    pub fn profil_anlegen(&self, eingabe: ProfilEingabe) -> Result<Profil, KontoFehler> {
        self.aendern(|d| {
            let profil = neues_profil(eingabe, d.profile.len())?;
            d.profile.push(profil.clone());
            Ok(profil)
        })
    }

    /// Ändert Felder eines Profils. Felder aus `anmeldung.idpFelder` dürfen sich nicht
    /// ändern (Fehler `Schreibgeschuetzt`); unveränderte Werte desselben Felds sind erlaubt,
    /// damit ein Formular das ganze Profil zurückschicken kann.
    pub fn profil_aendern(
        &self,
        id: &str,
        aenderung: ProfilAenderung,
    ) -> Result<Profil, KontoFehler> {
        self.aendern(|d| {
            let profil = d
                .profile
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| KontoFehler::NichtGefunden(id.into()))?;
            anwenden(profil, aenderung)?;
            profil.geaendert_ms = jetzt_ms();
            Ok(profil.clone())
        })
    }

    pub fn profil_entfernen(&self, id: &str) -> Result<(), KontoFehler> {
        self.aendern(|d| {
            if !d.profile.iter().any(|p| p.id == id) {
                return Err(KontoFehler::NichtGefunden(id.into()));
            }
            if d.aktiv == id {
                return Err(KontoFehler::NichtEntfernbar("aktives Profil"));
            }
            if d.profile.len() <= 1 {
                return Err(KontoFehler::NichtEntfernbar("letztes Profil"));
            }
            d.profile.retain(|p| p.id != id);
            Ok(())
        })?;
        // Erst nach erfolgreichem Schreiben der JSON-Datei; ein Bildrest ist harmlos.
        self.bild_loeschen(id)
    }

    fn bild_loeschen(&self, id: &str) -> Result<(), KontoFehler> {
        match fs::remove_file(self.bild_pfad(id)?) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    pub fn aktivieren(&self, id: &str) -> Result<(), KontoFehler> {
        self.aendern(|d| {
            if !d.profile.iter().any(|p| p.id == id) {
                return Err(KontoFehler::NichtGefunden(id.into()));
            }
            d.aktiv = id.into();
            d.sitzungsrevision += 1;
            Ok(())
        })
    }

    /// Setzt das Profilbild (nur PNG, höchstens 256 KB).
    pub fn bild_setzen(&self, id: &str, png: &[u8]) -> Result<(), KontoFehler> {
        if png.len() <= PNG_SIGNATUR.len() || !png.starts_with(&PNG_SIGNATUR) {
            return Err(KontoFehler::Bild("kein PNG"));
        }
        if png.len() > BILD_MAX_BYTES {
            return Err(KontoFehler::Bild("größer als 256 KB"));
        }
        self.aendern(|d| {
            let profil = d
                .profile
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| KontoFehler::NichtGefunden(id.into()))?;
            fs::create_dir_all(self.bilder_pfad())?;
            atomar_schreiben(&self.bild_pfad(id)?, png)?;
            profil.hat_bild = true;
            profil.geaendert_ms = jetzt_ms();
            Ok(())
        })
    }

    pub fn bild_entfernen(&self, id: &str) -> Result<(), KontoFehler> {
        self.aendern(|d| {
            let profil = d
                .profile
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or_else(|| KontoFehler::NichtGefunden(id.into()))?;
            self.bild_loeschen(id)?;
            profil.hat_bild = false;
            profil.geaendert_ms = jetzt_ms();
            Ok(())
        })
    }

    pub fn bild_lesen(&self, id: &str) -> Result<Option<Vec<u8>>, KontoFehler> {
        let Ok(pfad) = self.bild_pfad(id) else {
            return Ok(None);
        };
        match fs::read(pfad) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Meldet Änderungen anderer (und eigener) Schreiber: prüft jede Sekunde `generation`
    /// und ruft `callback` mit der neuen Datei auf. Stoppt, wenn das Handle fällt.
    ///
    /// Nur ein Hinweis: Polling kann Änderungen verspätet melden. Sicherheitsrelevante
    /// Entscheidungen müssen vor der Aktion `pruefe_revision` nutzen.
    pub fn beobachten<F>(&self, callback: F) -> Result<Beobachter, KontoFehler>
    where
        F: Fn(&ProfilDatei) + Send + 'static,
    {
        self.beobachten_mit(POLL_INTERVALL, callback)
    }

    #[doc(hidden)]
    pub fn beobachten_mit<F>(
        &self,
        intervall: Duration,
        callback: F,
    ) -> Result<Beobachter, KontoFehler>
    where
        F: Fn(&ProfilDatei) + Send + 'static,
    {
        Beobachter::starten(self.clone(), intervall, callback)
    }

    fn datei_pfad(&self) -> PathBuf {
        self.wurzel.join("profile.json")
    }

    fn bilder_pfad(&self) -> PathBuf {
        self.wurzel.join("bilder")
    }

    /// Nur geprüfte IDs führen zu einem Pfad; sonst wäre `..` im Namen möglich.
    fn bild_pfad(&self, id: &str) -> Result<PathBuf, KontoFehler> {
        if !pruefen::id_ok(id) {
            return Err(KontoFehler::Ungueltig {
                feld: "id",
                grund: "ungültige Profil-ID",
            });
        }
        Ok(self.bilder_pfad().join(format!("{id}.png")))
    }

    /// Exklusive Sperre über Prozesse hinweg; wird mit dem Handle freigegeben.
    /// `konto.lock` ist stabil und wird nie per rename ersetzt (anders als `profile.json`).
    fn sperren(&self) -> Result<File, KontoFehler> {
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.wurzel.join("konto.lock"))?;
        f.lock()?;
        Ok(f)
    }

    /// Sperren, frisch lesen, ändern, `generation` erhöhen, atomar schreiben.
    fn aendern<T>(
        &self,
        f: impl FnOnce(&mut ProfilDatei) -> Result<T, KontoFehler>,
    ) -> Result<T, KontoFehler> {
        let _sperre = self.sperren()?;
        let mut datei = self.lesen()?;
        let ergebnis = f(&mut datei)?;
        datei.generation += 1;
        self.schreiben(&datei)?;
        Ok(ergebnis)
    }

    fn schreiben(&self, datei: &ProfilDatei) -> Result<(), KontoFehler> {
        let json = serde_json::to_vec_pretty(datei)?;
        atomar_schreiben(&self.datei_pfad(), &json)?;
        Ok(())
    }
}

#[derive(serde::Deserialize)]
struct Kopf {
    version: u32,
}

/// Erst nur die Version prüfen (neuere Formate dürfen unbekannte Felder haben), dann vollständig.
fn pruefen_datei(bytes: &[u8]) -> Result<ProfilDatei, KontoFehler> {
    let kopf: Kopf = serde_json::from_slice(bytes)?;
    if kopf.version != DATEI_VERSION {
        return Err(KontoFehler::Version(kopf.version));
    }
    let datei: ProfilDatei = serde_json::from_slice(bytes)?;
    let ids_ok =
        pruefen::id_ok(&datei.aktiv) && datei.profile.iter().all(|p| pruefen::id_ok(&p.id));
    if !ids_ok {
        return Err(KontoFehler::Ungueltig {
            feld: "id",
            grund: "ungültige Profil-ID in profile.json",
        });
    }
    Ok(datei)
}

/// Temp-Datei im selben Ordner schreiben, auf Platte bringen, dann ersetzen.
fn atomar_schreiben(ziel: &Path, inhalt: &[u8]) -> io::Result<()> {
    let ordner = ziel.parent().unwrap_or(Path::new("."));
    let name = ziel.file_name().and_then(|n| n.to_str()).unwrap_or("datei");
    let tmp = ordner.join(format!(".{name}.{}.tmp", std::process::id()));
    let ergebnis = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(inhalt)?;
        f.sync_all()?;
        let mut versuch = 0;
        loop {
            match fs::rename(&tmp, ziel) {
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied && versuch < 20 => {
                    versuch += 1;
                    std::thread::sleep(Duration::from_millis(10));
                }
                r => break r,
            }
        }
    })();
    if ergebnis.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    ergebnis
}

fn jetzt_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn neue_id() -> Result<String, KontoFehler> {
    let mut b = [0u8; 26];
    getrandom::fill(&mut b).map_err(|_| KontoFehler::Zufall)?;
    Ok(b.iter()
        .map(|x| ALPHABET[(x & 31) as usize] as char)
        .collect())
}

fn neues_profil(e: ProfilEingabe, vorhanden: usize) -> Result<Profil, KontoFehler> {
    let farbe = e.farbe.unwrap_or((vorhanden % FARBEN as usize) as u8);
    if farbe >= FARBEN {
        return Err(KontoFehler::Ungueltig {
            feld: "farbe",
            grund: "Index muss 0 bis 15 sein",
        });
    }
    let jetzt = jetzt_ms();
    Ok(Profil {
        id: neue_id()?,
        anzeigename: pruefen::pflicht("anzeigename", &e.anzeigename)?,
        vorname: opt("vorname", e.vorname)?,
        nachname: opt("nachname", e.nachname)?,
        email: opt("email", e.email)?,
        telefon: opt("telefon", e.telefon)?,
        mobil: opt("mobil", e.mobil)?,
        position: opt("position", e.position)?,
        abteilung: opt("abteilung", e.abteilung)?,
        organisation: opt("organisation", e.organisation)?,
        standort: opt("standort", e.standort)?,
        farbe,
        hat_bild: false,
        anmeldung: None,
        gesperrt: false,
        erstellt_ms: jetzt,
        geaendert_ms: jetzt,
    })
}

fn opt(feld: &'static str, wert: Option<String>) -> Result<Option<String>, KontoFehler> {
    wert.map_or(Ok(None), |w| pruefen::text(feld, &w))
}

fn anwenden(p: &mut Profil, a: ProfilAenderung) -> Result<(), KontoFehler> {
    let gesperrt: Vec<String> = p
        .anmeldung
        .as_ref()
        .map(|m| m.idp_felder.clone())
        .unwrap_or_default();

    macro_rules! feld {
        ($name:literal, $slot:expr, $neu:expr) => {
            if let Some(roh) = $neu {
                let neu = pruefen::text($name, &roh)?;
                if neu != $slot {
                    if gesperrt.iter().any(|g| g == $name) {
                        return Err(KontoFehler::Schreibgeschuetzt($name.into()));
                    }
                    $slot = neu;
                }
            }
        };
    }

    if let Some(roh) = a.anzeigename {
        let neu = pruefen::pflicht("anzeigename", &roh)?;
        if neu != p.anzeigename {
            if gesperrt.iter().any(|g| g == "anzeigename") {
                return Err(KontoFehler::Schreibgeschuetzt("anzeigename".into()));
            }
            p.anzeigename = neu;
        }
    }
    feld!("vorname", p.vorname, a.vorname);
    feld!("nachname", p.nachname, a.nachname);
    feld!("email", p.email, a.email);
    feld!("telefon", p.telefon, a.telefon);
    feld!("mobil", p.mobil, a.mobil);
    feld!("position", p.position, a.position);
    feld!("abteilung", p.abteilung, a.abteilung);
    feld!("organisation", p.organisation, a.organisation);
    feld!("standort", p.standort, a.standort);
    if let Some(f) = a.farbe {
        if f >= FARBEN {
            return Err(KontoFehler::Ungueltig {
                feld: "farbe",
                grund: "Index muss 0 bis 15 sein",
            });
        }
        p.farbe = f;
    }
    Ok(())
}
