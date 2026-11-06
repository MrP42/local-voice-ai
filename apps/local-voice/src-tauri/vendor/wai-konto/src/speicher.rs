use crate::beobachter::Beobachter;
use crate::modell::{Profil, ProfilAenderung, ProfilDatei, ProfilEingabe};
use crate::{BILD_MAX_BYTES, BILD_MAX_KANTE, DATEI_VERSION, FARBEN, KontoFehler, pruefen};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
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

    /// Legt ein Profil an und setzt es im selben Schreibvorgang unter derselben Sperre als
    /// aktiv (erhöht `sitzungsrevision` wie [`Self::aktivieren`]). Scheitert die Anlage,
    /// bleibt alles unverändert; es gibt keinen Zwischenzustand mit inaktivem neuem Profil.
    pub fn profil_anlegen_und_aktivieren(
        &self,
        eingabe: ProfilEingabe,
    ) -> Result<Profil, KontoFehler> {
        self.aendern(|d| {
            let profil = neues_profil(eingabe, d.profile.len())?;
            d.profile.push(profil.clone());
            d.aktiv = profil.id.clone();
            d.sitzungsrevision += 1;
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
        self.profil_aendern_intern(id, aenderung, false)
    }

    /// Wie [`Self::profil_aendern`], prüft aber unter derselben Dateisperre und vor dem
    /// Schreiben, dass `id` das aktive Profil ist. Sonst `NichtAktiv`; dann wird nichts
    /// geschrieben. Schließt das Fenster zwischen „ist X aktiv?“ und dem Schreiben, in dem
    /// eine andere Suite-App das aktive Profil wechseln kann.
    pub fn profil_aendern_wenn_aktiv(
        &self,
        id: &str,
        aenderung: ProfilAenderung,
    ) -> Result<Profil, KontoFehler> {
        self.profil_aendern_intern(id, aenderung, true)
    }

    /// Gemeinsame Umsetzung; `nur_aktiv` schaltet die Aktiv-Prüfung unter der Sperre ein.
    fn profil_aendern_intern(
        &self,
        id: &str,
        aenderung: ProfilAenderung,
        nur_aktiv: bool,
    ) -> Result<Profil, KontoFehler> {
        self.aendern(|d| {
            aktiv_pruefen(d, id, nur_aktiv)?;
            let profil = profil_suchen(d, id)?;
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

    /// Setzt das Profilbild (nur PNG, höchstens 256 KB, Kantenlänge 1 bis
    /// [`BILD_MAX_KANTE`] px laut IHDR).
    ///
    /// Reihenfolge unter der Sperre: (1) Bild in Temp-Datei schreiben, auf Platte bringen und
    /// atomar an den Zielort umbenennen, (2) erst danach `profile.json` mit `hat_bild = true`
    /// atomar schreiben. So zeigt `hat_bild = true` in jedem Fehlerfall auf eine vollständige,
    /// gültige Datei: Scheitert Schritt 1, ist nichts verändert; scheitert Schritt 2, bleibt
    /// höchstens eine verwaiste Bilddatei ohne Flag (harmlos). Die umgekehrte Reihenfolge
    /// (Flag zuerst, Umbenennen danach) hätte ein Fenster, in dem das Flag auf eine fehlende
    /// Datei zeigt. Bei einem Fehler im Schritt 2 wird der Zustand zurückgesetzt: Das vorher
    /// gelesene alte Bild (im Speicher gehalten, höchstens 256 KB) wird wiederhergestellt,
    /// ein neues Bild eines zuvor bildlosen Profils wird gelöscht. Nur wenn auch das
    /// Wiederherstellen scheitert, kann ein anderes gültiges Bild oder eine Waise bleiben.
    pub fn bild_setzen(&self, id: &str, png: &[u8]) -> Result<(), KontoFehler> {
        self.bild_setzen_intern(id, png, false)
    }

    /// Wie [`Self::bild_setzen`], prüft aber unter derselben Dateisperre und vor jedem
    /// Schreiben, dass `id` das aktive Profil ist. Sonst `NichtAktiv`; dann bleiben
    /// `profile.json`, `generation` und das vorhandene Bild unverändert und es entsteht
    /// keine Bilddatei. Ein ungültiges Bild wird weiterhin zuerst abgelehnt.
    pub fn bild_setzen_wenn_aktiv(&self, id: &str, png: &[u8]) -> Result<(), KontoFehler> {
        self.bild_setzen_intern(id, png, true)
    }

    /// Gemeinsame Umsetzung; `nur_aktiv` schaltet die Aktiv-Prüfung unter der Sperre ein.
    fn bild_setzen_intern(&self, id: &str, png: &[u8], nur_aktiv: bool) -> Result<(), KontoFehler> {
        bild_pruefen(png)?;
        let _sperre = self.sperren()?;
        let mut datei = self.lesen()?;
        aktiv_pruefen(&datei, id, nur_aktiv)?;
        let profil = profil_suchen(&mut datei, id)?;
        let ziel = self.bild_pfad(id)?;
        let alt = if profil.hat_bild {
            self.bild_lesen_roh(&ziel)?
        } else {
            None
        };
        fs::create_dir_all(self.bilder_pfad())?;
        atomar_schreiben(&ziel, png)?;
        profil.hat_bild = true;
        profil.geaendert_ms = jetzt_ms();
        datei.generation += 1;
        if let Err(e) = self.schreiben(&datei) {
            // Zustand wie vorher herstellen: altes Bild zurück oder neues Bild löschen.
            match alt {
                Some(bytes) => {
                    let _ = atomar_schreiben(&ziel, &bytes);
                }
                None => {
                    let _ = fs::remove_file(&ziel);
                }
            }
            return Err(e);
        }
        Ok(())
    }

    /// Entfernt das Profilbild. Erst `profile.json` (`hat_bild = false`) schreiben, dann die
    /// Datei löschen: Scheitert das Löschen, bleibt nur eine verwaiste Datei ohne Flag (kein
    /// Fehler; [`Self::bild_lesen`] liefert sie nicht aus); das Flag zeigt nie auf eine
    /// bereits gelöschte Datei.
    pub fn bild_entfernen(&self, id: &str) -> Result<(), KontoFehler> {
        self.bild_entfernen_intern(id, false)
    }

    /// Wie [`Self::bild_entfernen`], prüft aber unter derselben Dateisperre und vor dem
    /// Schreiben, dass `id` das aktive Profil ist. Sonst `NichtAktiv`; dann bleiben
    /// `profile.json`, `generation` und die Bilddatei unverändert.
    pub fn bild_entfernen_wenn_aktiv(&self, id: &str) -> Result<(), KontoFehler> {
        self.bild_entfernen_intern(id, true)
    }

    /// Gemeinsame Umsetzung; `nur_aktiv` schaltet die Aktiv-Prüfung unter der Sperre ein.
    fn bild_entfernen_intern(&self, id: &str, nur_aktiv: bool) -> Result<(), KontoFehler> {
        self.aendern_dann(
            |d| {
                aktiv_pruefen(d, id, nur_aktiv)?;
                let profil = profil_suchen(d, id)?;
                profil.hat_bild = false;
                profil.geaendert_ms = jetzt_ms();
                Ok(())
            },
            // Ein gescheitertes Löschen ist kein Fehler: Der Zustand ist schon konsistent.
            |()| {
                let _ = self.bild_loeschen(id);
                Ok(())
            },
        )
    }

    /// Liest das Profilbild, höchstens `BILD_MAX_BYTES` + 1 Byte (eine manipulierte Datei im
    /// geteilten Ordner wird nie vollständig eingelesen). `Ok(None)`: kein Bild, ungültige
    /// ID, `hat_bild` ist im Profil `false` (eine verwaiste Datei, etwa nach gescheitertem
    /// Löschen, wird nicht ausgeliefert), die Datei ist zu groß oder kein gültiges PNG (die UI
    /// zeigt dann die Initialen). Echte E-/A-Fehler bleiben `Err`.
    pub fn bild_lesen(&self, id: &str) -> Result<Option<Vec<u8>>, KontoFehler> {
        let Ok(pfad) = self.bild_pfad(id) else {
            return Ok(None);
        };
        let hat_bild = self
            .lesen()?
            .profile
            .iter()
            .any(|p| p.id == id && p.hat_bild);
        if !hat_bild {
            return Ok(None);
        }
        self.bild_lesen_roh(&pfad)
    }

    /// Liest und prüft die Bilddatei am Pfad, ohne das Flag zu beachten.
    fn bild_lesen_roh(&self, pfad: &Path) -> Result<Option<Vec<u8>>, KontoFehler> {
        let datei = match File::open(pfad) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let bytes = begrenzt_lesen(datei, BILD_MAX_BYTES)?;
        Ok(bild_pruefen(&bytes).is_ok().then_some(bytes))
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

    /// Wie [`Self::aendern`], führt danach aber noch unter derselben Sperre `danach` aus
    /// (nur nach erfolgreichem Schreiben von `profile.json`).
    fn aendern_dann<T>(
        &self,
        f: impl FnOnce(&mut ProfilDatei) -> Result<T, KontoFehler>,
        danach: impl FnOnce(&T) -> Result<(), KontoFehler>,
    ) -> Result<(), KontoFehler> {
        let _sperre = self.sperren()?;
        let mut datei = self.lesen()?;
        let ergebnis = f(&mut datei)?;
        datei.generation += 1;
        self.schreiben(&datei)?;
        danach(&ergebnis)
    }

    fn schreiben(&self, datei: &ProfilDatei) -> Result<(), KontoFehler> {
        let json = serde_json::to_vec_pretty(datei)?;
        atomar_schreiben(&self.datei_pfad(), &json)?;
        Ok(())
    }
}

/// Mit `nur_aktiv`: Fehler `NichtAktiv`, wenn `id` nicht das aktive Profil der Datei ist.
fn aktiv_pruefen(datei: &ProfilDatei, id: &str, nur_aktiv: bool) -> Result<(), KontoFehler> {
    if nur_aktiv && datei.aktiv != id {
        return Err(KontoFehler::NichtAktiv(id.into()));
    }
    Ok(())
}

/// Sucht das Profil `id` veränderbar; sonst `NichtGefunden`.
fn profil_suchen<'a>(datei: &'a mut ProfilDatei, id: &str) -> Result<&'a mut Profil, KontoFehler> {
    datei
        .profile
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| KontoFehler::NichtGefunden(id.into()))
}

/// Liest höchstens `max` + 1 Byte aus `leser`; das eine Byte mehr zeigt „zu groß“ an,
/// ohne dass eine große Datei ganz eingelesen wird.
fn begrenzt_lesen(leser: impl Read, max: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    leser.take(max as u64 + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Prüft ein Profilbild von Hand (ohne Bibliothek): Größe, PNG-Signatur und den IHDR-Chunk
/// direkt danach (Länge 13, Typ `IHDR`, Breite und Höhe je 1 bis [`BILD_MAX_KANTE`]).
/// Die Kantengrenze schützt Dekoder vor kleinen, aber riesig entpackenden PNGs
/// (Dekompressionsbombe); die Pixelzahl ist so auf 1 Mio. begrenzt.
fn bild_pruefen(png: &[u8]) -> Result<(), KontoFehler> {
    const IHDR_ENDE: usize = 8 + 4 + 4 + 13 + 4;
    if png.len() > BILD_MAX_BYTES {
        return Err(KontoFehler::Bild("größer als 256 KB"));
    }
    if !png.starts_with(&PNG_SIGNATUR) {
        return Err(KontoFehler::Bild("kein PNG"));
    }
    if png.len() < IHDR_ENDE || png[8..12] != 13u32.to_be_bytes() || &png[12..16] != b"IHDR" {
        return Err(KontoFehler::Bild("IHDR fehlt oder ist ungültig"));
    }
    let kante = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let (breite, hoehe) = (kante(&png[16..20]), kante(&png[20..24]));
    if !(1..=BILD_MAX_KANTE).contains(&breite) || !(1..=BILD_MAX_KANTE).contains(&hoehe) {
        return Err(KontoFehler::Bild("Breite/Höhe außerhalb 1 bis 1024 px"));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::begrenzt_lesen;
    use std::cell::Cell;
    use std::io::{self, Read};

    struct Zaehler<'a>(&'a Cell<usize>);

    impl Read for Zaehler<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            buf.fill(1);
            self.0.set(self.0.get() + buf.len());
            Ok(buf.len())
        }
    }

    #[test]
    fn begrenzt_lesen_liest_hoechstens_max_plus_eins() {
        let gelesen = Cell::new(0);
        let bytes = begrenzt_lesen(Zaehler(&gelesen), 1000).unwrap();
        assert_eq!(bytes.len(), 1001);
        // Der Leser wurde nur so weit gezogen, wie der Puffer es verlangte (kein unbegrenztes Lesen).
        assert!(gelesen.get() < 1001 + 64 * 1024);
        assert_eq!(begrenzt_lesen(&b"abc"[..], 1000).unwrap(), b"abc");
        assert_eq!(begrenzt_lesen(&b"abcd"[..], 3).unwrap().len(), 4);
    }
}
