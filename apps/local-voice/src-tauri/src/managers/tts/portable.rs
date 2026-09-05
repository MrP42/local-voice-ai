//! Stimmen teilen: eine Stimme als einzelnes `.lvvoice`-Archiv exportieren
//! und wieder einspielen.
//!
//! Ein `.lvvoice` ist ein ZIP mit flacher Struktur — genau die Dateien, die
//! ein Stimmenordner ausmacht: `meta.json`, `sample.wav`, `sample.lab` und,
//! falls vorhanden, `avatar.<ext>` und `seed.txt`. Kein eigenes Format, kein
//! neues Werkzeug: das `zip`-Crate wird im Meeting-Export schon so benutzt
//! (siehe `managers/meetings/export.rs`).
//!
//! Bewusst ohne Tauri: reine Funktionen ueber `&Path`, damit die drei Regeln,
//! auf die es ankommt, ohne App testbar sind —
//!
//! 1. **Zip-Slip**: ein Archiv mit `../` oder absoluten Pfaden im
//!    Eintragsnamen wird ABGELEHNT, nicht etwa teilweise ausgepackt. Ein
//!    fremdes Archiv ist eine Nutzereingabe wie jede andere.
//! 2. **Nur bekannte Dateinamen** landen auf der Platte; alles andere wird
//!    stillschweigend ignoriert (ein Archiv aus einer neueren Version darf
//!    nicht am Zusatzinhalt scheitern).
//! 3. **Vollstaendig oder gar nicht**: bricht der Import auf halber Strecke
//!    ab, wird das Zielverzeichnis wieder entfernt — dieselbe Regel wie in
//!    `save_seed_voice_v2`, denn ein halbes Verzeichnis meldet beim naechsten
//!    Versuch faelschlich „existiert bereits".

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::{registry, voices};

/// Dateiendung des Stimmen-Archivs (ohne Punkt) — der Dateidialog bietet sie
/// an, die Logik haengt nicht daran.
pub const ARCHIVE_EXTENSION: &str = "lvvoice";

const META_FILE: &str = "meta.json";
const SAMPLE_WAV: &str = "sample.wav";
const SAMPLE_LAB: &str = "sample.lab";
const SEED_FILE: &str = "seed.txt";
const AVATAR_FILES: [&str; 3] = ["avatar.png", "avatar.webp", "avatar.jpg"];

/// Obergrenze je entpacktem Eintrag: 64 MiB. Eine Referenzaufnahme sind
/// wenige Megabyte; die Grenze ist keine Qualitaetsregel, sondern der Deckel
/// gegen ein Archiv, das beim Auspacken ins Unermessliche waechst.
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

/// Ist dieser Eintragsname einer der bekannten Stimmen-Dateien?
fn is_known_entry(name: &str) -> bool {
    name == META_FILE
        || name == SAMPLE_WAV
        || name == SAMPLE_LAB
        || name == SEED_FILE
        || AVATAR_FILES.contains(&name)
}

/// Ein Eintragsname, der aus dem Zielordner herausfuehren koennte. Geprueft
/// wird der ROHE Name aus dem Archiv, nicht ein bereits normalisierter Pfad:
/// die Abwehr muss vor jeder Pfad-Auswertung greifen.
fn escapes_target(name: &str) -> bool {
    if name.starts_with('/') || name.starts_with('\\') {
        return true;
    }
    // Laufwerksbuchstabe oder ADS unter Windows (`C:\…`, `datei:stream`).
    if name.contains(':') {
        return true;
    }
    if Path::new(name).is_absolute() {
        return true;
    }
    name.split(['/', '\\']).any(|part| part == "..")
}

/// Alle Eintragsnamen pruefen, BEVOR irgendetwas geschrieben wird. Ein
/// einziger boesartiger Eintrag verwirft das ganze Archiv — nicht nur den
/// Eintrag: wer so ein Archiv baut, hat nichts Gutes vor, und der Rest ist
/// dann ebenso wenig vertrauenswuerdig.
fn reject_unsafe_entries<R: Read + std::io::Seek>(zip: &zip::ZipArchive<R>) -> Result<(), String> {
    for name in zip.file_names() {
        if escapes_target(name) {
            return Err(format!(
                "Das Archiv enthaelt einen Eintrag ausserhalb des Zielordners ('{name}') und wird nicht eingespielt"
            ));
        }
    }
    Ok(())
}

fn open_archive(archive: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(archive)
        .map_err(|e| format!("could not open {}: {e}", archive.display()))?;
    let zip = zip::ZipArchive::new(file)
        .map_err(|e| format!("{} ist kein gueltiges Archiv: {e}", archive.display()))?;
    reject_unsafe_entries(&zip)?;
    Ok(zip)
}

/// Einen Eintrag vollstaendig lesen — mit Deckel gegen Zip-Bomben.
fn read_entry(
    zip: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<Option<Vec<u8>>, String> {
    let mut entry = match zip.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(format!("could not read {name}: {e}")),
    };
    if entry.size() > MAX_ENTRY_BYTES {
        return Err(format!(
            "Der Eintrag '{name}' ist mit {} Bytes zu gross (erlaubt sind {MAX_ENTRY_BYTES})",
            entry.size()
        ));
    }
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut buf)
        .map_err(|e| format!("could not read {name}: {e}"))?;
    Ok(Some(buf))
}

// ------------------------------------------------------------------ Export --

/// Eine Stimme als ZIP: `meta.json`, `sample.wav`, `sample.lab` und, falls
/// vorhanden, der Avatar und `seed.txt`.
///
/// Die Referenzaufnahme kommt ueber [`voices::voice_sample`] — dieselbe
/// Auswahlregel wie Stimmenliste und Hoerprobe — und wird im Archiv immer
/// `sample.wav`/`sample.lab` genannt, egal wie sie im Ordner heisst. Das
/// Archiv hat damit genau eine Form, unabhaengig davon, wie die Stimme
/// entstanden ist.
///
/// `meta.json` schreibt immer [`registry::read_meta`] — auch fuer eine
/// Bestandsstimme ohne eigene Datei. Ein Archiv ohne Metadaten waere sonst
/// beim Import namenlos.
pub fn export_voice(fish_dir: &Path, id: &str, out_path: &Path) -> Result<(), String> {
    let (wav, transcript) = voices::voice_sample(fish_dir, id)
        .ok_or_else(|| format!("Die Stimme '{id}' hat keine vollstaendige Referenzaufnahme"))?;
    let meta = registry::read_meta(fish_dir, id);
    let meta_json = serde_json::to_string_pretty(&meta)
        .map_err(|e| format!("could not serialize meta for {id}: {e}"))?;
    let wav_bytes =
        std::fs::read(&wav).map_err(|e| format!("could not read {}: {e}", wav.display()))?;

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
    }
    let file = std::fs::File::create(out_path)
        .map_err(|e| format!("could not create {}: {e}", out_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
        zip.start_file(name, options)
            .map_err(|e| format!("could not add {name}: {e}"))?;
        zip.write_all(bytes)
            .map_err(|e| format!("could not write {name}: {e}"))
    };

    put(META_FILE, meta_json.as_bytes())?;
    put(SAMPLE_WAV, &wav_bytes)?;
    put(SAMPLE_LAB, transcript.as_bytes())?;

    let dir = voices::voice_dir(fish_dir, id);
    if let Some(avatar) = voices::avatar_path(fish_dir, id) {
        let name = avatar
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if AVATAR_FILES.contains(&name.as_str()) {
            let bytes = std::fs::read(&avatar)
                .map_err(|e| format!("could not read {}: {e}", avatar.display()))?;
            put(&name, &bytes)?;
        }
    }
    let seed = dir.join(SEED_FILE);
    if seed.is_file() {
        let bytes =
            std::fs::read(&seed).map_err(|e| format!("could not read {}: {e}", seed.display()))?;
        put(SEED_FILE, &bytes)?;
    }

    zip.finish()
        .map_err(|e| format!("could not finalize {}: {e}", out_path.display()))?;
    Ok(())
}

// ----------------------------------------------------------------- Vorschau --

/// Was ein Archiv enthaelt, ohne es auszupacken — fuer die Vorschau vor dem
/// Import (Anzeigename, und ob dieser Name schon vergeben ist).
///
/// Prueft dabei bereits alles, was den Import scheitern liesse: Zip-Slip,
/// fehlende `meta.json`, fehlende `sample.wav`. Die Oberflaeche kann sich
/// darauf verlassen, dass ein Archiv, das hier durchkommt, auch einspielbar
/// ist — bis auf Namenskollision und Schreibfehler.
pub fn inspect_archive(archive: &Path) -> Result<registry::VoiceMeta, String> {
    let mut zip = open_archive(archive)?;
    if !zip.file_names().any(|n| n == SAMPLE_WAV) {
        return Err(format!(
            "Das Archiv enthaelt keine '{SAMPLE_WAV}' und ist keine Stimme"
        ));
    }
    let raw = read_entry(&mut zip, META_FILE)?
        .ok_or_else(|| format!("Das Archiv enthaelt keine '{META_FILE}' und ist keine Stimme"))?;
    let text = String::from_utf8(raw)
        .map_err(|_| format!("'{META_FILE}' im Archiv ist kein gueltiges UTF-8"))?;
    serde_json::from_str(&text).map_err(|e| format!("'{META_FILE}' im Archiv ist unlesbar: {e}"))
}

// ------------------------------------------------------------------ Import --

/// Archiv als NEUE Stimme einspielen. `display_name_override` erlaubt der
/// Oberflaeche, bei Namenskollision einen anderen Namen zu setzen — der
/// Import ueberschreibt nie eine vorhandene Stimme.
///
/// Rueckgabe bei Erfolg: die vergebene `voice_id`.
pub fn import_voice(
    fish_dir: &Path,
    archive: &Path,
    display_name_override: Option<&str>,
) -> Result<String, String> {
    let mut meta = inspect_archive(archive)?;
    if let Some(name) = display_name_override {
        meta.display_name = name.trim().to_string();
    }
    let id = voices::sanitize_voice_id(&meta.display_name)
        .ok_or_else(|| "Der Name ergibt keinen brauchbaren Stimmennamen".to_string())?;
    if voices::voice_is_complete(fish_dir, &id) {
        return Err(format!(
            "Die Stimme '{id}' existiert bereits — bitte einen anderen Namen waehlen"
        ));
    }
    let others = registry::other_voice_names(fish_dir, Some(&id));
    registry::validate_meta(&meta, &others)?;

    let target = voices::voice_dir(fish_dir, &id);
    std::fs::create_dir_all(&target)
        .map_err(|e| format!("could not create {}: {e}", target.display()))?;
    match extract_into(archive, &target) {
        Ok(written) => {
            // Der Avatar aus der `meta.json` muss auch wirklich dabei sein —
            // sonst zeigt die Oberflaeche auf eine Datei, die es nicht gibt.
            if let Some(registry::Avatar::Image { file }) = meta.avatar.clone() {
                if !written.iter().any(|n| n == &file) {
                    meta.avatar = None;
                }
            }
            // `sample.lab` darf im Archiv fehlen; ohne sie waere die Stimme
            // fuer `list_voices` unsichtbar. Leer ist ein zulaessiges
            // Transkript, „gar nicht vorhanden" nicht.
            let lab = target.join(SAMPLE_LAB);
            if !lab.is_file() {
                if let Err(e) = std::fs::write(&lab, b"") {
                    let _ = std::fs::remove_dir_all(&target);
                    return Err(format!("could not write {}: {e}", lab.display()));
                }
            }
            // Metadaten zuletzt und ueber `write_meta`, damit ein
            // ueberschriebener Anzeigename auch auf der Platte steht.
            if let Err(e) = registry::write_meta(fish_dir, &id, &meta) {
                let _ = std::fs::remove_dir_all(&target);
                return Err(e);
            }
            voices::update_registry(fish_dir);
            log::info!("Stimme '{id}' aus {} eingespielt", archive.display());
            Ok(id)
        }
        Err(e) => {
            // Vollstaendig oder gar nicht — die Regel aus
            // `save_seed_voice_v2`: ein halbes Verzeichnis meldet beim
            // naechsten Versuch faelschlich „existiert bereits".
            let _ = std::fs::remove_dir_all(&target);
            Err(e)
        }
    }
}

/// Die bekannten Dateien des Archivs in `target` schreiben. Liefert die
/// geschriebenen Dateinamen. Unbekannte Eintraege werden ignoriert;
/// gefaehrliche hat [`open_archive`] bereits abgewiesen.
fn extract_into(archive: &Path, target: &Path) -> Result<Vec<String>, String> {
    let mut zip = open_archive(archive)?;
    let names: Vec<String> = zip
        .file_names()
        .filter(|n| is_known_entry(n))
        .map(|n| n.to_string())
        .collect();
    let mut written = Vec::new();
    for name in names {
        let Some(bytes) = read_entry(&mut zip, &name)? else {
            continue;
        };
        let path: PathBuf = target.join(&name);
        std::fs::write(&path, &bytes)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        written.push(name);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eine minimale, aber gueltige Stimme im Referenzordner anlegen.
    fn stimme_anlegen(fish_dir: &Path, id: &str, display_name: &str) {
        let dir = voices::voice_dir(fish_dir, id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SAMPLE_WAV), b"RIFF____WAVEfake").unwrap();
        std::fs::write(dir.join(SAMPLE_LAB), "Hallo Welt").unwrap();
        let mut meta = registry::default_meta(id, &[]);
        meta.display_name = display_name.to_string();
        registry::write_meta(fish_dir, id, &meta).unwrap();
    }

    /// Archiv aus (Name, Inhalt)-Paaren bauen — auch mit Namen, die ein
    /// ehrlicher Export nie erzeugen wuerde.
    fn archiv_bauen(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn meta_json(display_name: &str) -> Vec<u8> {
        let mut meta = registry::default_meta("egal", &[]);
        meta.display_name = display_name.to_string();
        serde_json::to_vec(&meta).unwrap()
    }

    #[test]
    fn rundlauf_export_import_unter_neuem_namen() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path();
        stimme_anlegen(fish, "anna", "Anna");
        std::fs::write(voices::voice_dir(fish, "anna").join(SEED_FILE), "4711").unwrap();

        let archiv = tmp.path().join("anna.lvvoice");
        export_voice(fish, "anna", &archiv).unwrap();
        assert!(archiv.is_file());

        // Vorschau kennt den Anzeigenamen, ohne auszupacken.
        assert_eq!(inspect_archive(&archiv).unwrap().display_name, "Anna");

        let neu = import_voice(fish, &archiv, Some("Berta")).unwrap();
        assert_eq!(neu, "berta");
        assert!(voices::voice_is_complete(fish, "berta"));
        assert_eq!(registry::read_meta(fish, "berta").display_name, "Berta");
        // Inhalte sind mitgekommen, die alte Stimme ist unangetastet.
        let dir = voices::voice_dir(fish, "berta");
        assert_eq!(
            std::fs::read(dir.join(SAMPLE_WAV)).unwrap(),
            b"RIFF____WAVEfake"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(SAMPLE_LAB)).unwrap(),
            "Hallo Welt"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(SEED_FILE)).unwrap(),
            "4711"
        );
        assert_eq!(registry::read_meta(fish, "anna").display_name, "Anna");
    }

    #[test]
    fn zip_slip_wird_abgewiesen_statt_ausgepackt() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path().join("fish");
        std::fs::create_dir_all(&fish).unwrap();
        let archiv = tmp.path().join("boese.lvvoice");
        archiv_bauen(
            &archiv,
            &[
                (META_FILE, &meta_json("Boese")),
                (SAMPLE_WAV, b"RIFF"),
                ("../../pwned.txt", b"gehackt"),
            ],
        );

        let err = inspect_archive(&archiv).unwrap_err();
        assert!(err.contains("ausserhalb"), "unerwartete Meldung: {err}");
        let err = import_voice(&fish, &archiv, None).unwrap_err();
        assert!(err.contains("ausserhalb"), "unerwartete Meldung: {err}");
        // Weder ausserhalb noch innerhalb ist etwas entstanden.
        assert!(!tmp.path().join("pwned.txt").exists());
        assert!(!voices::voice_dir(&fish, "boese").exists());
    }

    #[test]
    fn absoluter_pfad_im_archiv_wird_abgewiesen() {
        let tmp = tempfile::tempdir().unwrap();
        let archiv = tmp.path().join("absolut.lvvoice");
        archiv_bauen(
            &archiv,
            &[
                (META_FILE, &meta_json("Absolut")),
                (SAMPLE_WAV, b"RIFF"),
                ("/etc/passwd", b"root"),
            ],
        );
        assert!(inspect_archive(&archiv).unwrap_err().contains("ausserhalb"));
    }

    #[test]
    fn archiv_ohne_meta_oder_sample_wird_abgewiesen() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path().join("fish");
        std::fs::create_dir_all(&fish).unwrap();

        let ohne_meta = tmp.path().join("ohne-meta.lvvoice");
        archiv_bauen(&ohne_meta, &[(SAMPLE_WAV, b"RIFF"), (SAMPLE_LAB, b"hi")]);
        assert!(inspect_archive(&ohne_meta).unwrap_err().contains(META_FILE));
        assert!(import_voice(&fish, &ohne_meta, None).is_err());

        let ohne_wav = tmp.path().join("ohne-wav.lvvoice");
        archiv_bauen(&ohne_wav, &[(META_FILE, &meta_json("Clara"))]);
        assert!(inspect_archive(&ohne_wav).unwrap_err().contains(SAMPLE_WAV));
        assert!(import_voice(&fish, &ohne_wav, None).is_err());
        assert!(!voices::voice_dir(&fish, "clara").exists());
    }

    #[test]
    fn kollision_ueberschreibt_nie() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path();
        stimme_anlegen(fish, "anna", "Anna");
        let archiv = tmp.path().join("anna.lvvoice");
        export_voice(fish, "anna", &archiv).unwrap();

        // Gleiche id: bricht ab, bevor irgendetwas geschrieben wird.
        let err = import_voice(fish, &archiv, None).unwrap_err();
        assert!(
            err.contains("existiert bereits"),
            "unerwartete Meldung: {err}"
        );
        assert_eq!(
            std::fs::read(voices::voice_dir(fish, "anna").join(SAMPLE_WAV)).unwrap(),
            b"RIFF____WAVEfake"
        );

        // Anderer Anzeigename, der aber auf denselben Namen einer ANDEREN
        // Stimme faellt: `validate_meta` schlaegt zu.
        stimme_anlegen(fish, "berta", "Berta");
        let err = import_voice(fish, &archiv, Some("berta")).unwrap_err();
        assert!(
            err.contains("existiert bereits"),
            "unerwartete Meldung: {err}"
        );
    }

    #[test]
    fn halber_import_laesst_nichts_zurueck() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path().join("fish");
        std::fs::create_dir_all(&fish).unwrap();
        let archiv = tmp.path().join("dora.lvvoice");
        archiv_bauen(
            &archiv,
            &[(META_FILE, &meta_json("Dora")), (SAMPLE_WAV, b"RIFF")],
        );

        // `sample.wav` ist im Ziel bereits ein VERZEICHNIS — das Schreiben
        // scheitert mitten im Auspacken, nachdem meta.json schon liegt.
        let target = voices::voice_dir(&fish, "dora");
        std::fs::create_dir_all(target.join(SAMPLE_WAV)).unwrap();
        assert!(!voices::voice_is_complete(&fish, "dora"));

        assert!(import_voice(&fish, &archiv, None).is_err());
        assert!(
            !target.exists(),
            "halbes Stimmenverzeichnis stehengeblieben: {}",
            target.display()
        );
    }

    #[test]
    fn unbekannte_eintraege_werden_ignoriert() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path().join("fish");
        std::fs::create_dir_all(&fish).unwrap();
        let archiv = tmp.path().join("emil.lvvoice");
        archiv_bauen(
            &archiv,
            &[
                (META_FILE, &meta_json("Emil")),
                (SAMPLE_WAV, b"RIFF"),
                (SAMPLE_LAB, b"hallo"),
                ("autorun.exe", b"MZ"),
                ("zukunft.json", b"{}"),
            ],
        );
        let id = import_voice(&fish, &archiv, None).unwrap();
        let dir = voices::voice_dir(&fish, &id);
        assert!(dir.join(SAMPLE_WAV).is_file());
        assert!(!dir.join("autorun.exe").exists());
        assert!(!dir.join("zukunft.json").exists());
    }

    #[test]
    fn avatar_wandert_mit_und_wird_sonst_verworfen() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path();
        stimme_anlegen(fish, "anna", "Anna");
        // Gueltiges PNG (nur Signatur) — `save_avatar` prueft die Signatur.
        let png: Vec<u8> = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n']
            .into_iter()
            .chain(b"rest".iter().copied())
            .collect();
        let file = voices::save_avatar(fish, "anna", &png, "png").unwrap();
        let mut meta = registry::read_meta(fish, "anna");
        meta.avatar = Some(registry::Avatar::Image { file });
        registry::write_meta(fish, "anna", &meta).unwrap();

        let archiv = tmp.path().join("anna.lvvoice");
        export_voice(fish, "anna", &archiv).unwrap();
        let id = import_voice(fish, &archiv, Some("Frida")).unwrap();
        assert!(voices::avatar_path(fish, &id).is_some());
        assert!(matches!(
            registry::read_meta(fish, &id).avatar,
            Some(registry::Avatar::Image { .. })
        ));

        // Archiv OHNE Avatar-Datei, aber mit Avatar in der meta.json:
        // der Verweis wird beim Import verworfen statt ins Leere zu zeigen.
        let mut meta = registry::default_meta("egal", &[]);
        meta.display_name = "Gerda".to_string();
        meta.avatar = Some(registry::Avatar::Image {
            file: "avatar.png".to_string(),
        });
        let ohne = tmp.path().join("gerda.lvvoice");
        archiv_bauen(
            &ohne,
            &[
                (META_FILE, &serde_json::to_vec(&meta).unwrap()),
                (SAMPLE_WAV, b"RIFF"),
                (SAMPLE_LAB, b"hi"),
            ],
        );
        let id = import_voice(fish, &ohne, None).unwrap();
        assert!(registry::read_meta(fish, &id).avatar.is_none());
    }

    #[test]
    fn export_ohne_vollstaendige_referenz_schlaegt_fehl() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path();
        let dir = voices::voice_dir(fish, "halb");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SAMPLE_WAV), b"RIFF").unwrap(); // ohne .lab
        let archiv = tmp.path().join("halb.lvvoice");
        assert!(export_voice(fish, "halb", &archiv).is_err());
        assert!(!archiv.exists());
    }
}
