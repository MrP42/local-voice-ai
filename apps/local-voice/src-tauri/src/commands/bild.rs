use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rand::RngCore;
use tauri::State;

use crate::managers::llm::vision::{vision_plan, VisionPlan};
use crate::media::bild::{self, BildOptionen};

#[derive(serde::Serialize, specta::Type)]
pub struct BildBeschreibungStatus {
    pub verfuegbar: bool,
    pub code: String,
}

fn status_aus_plan(plan: VisionPlan) -> BildBeschreibungStatus {
    BildBeschreibungStatus {
        verfuegbar: matches!(&plan, VisionPlan::Ready),
        code: plan.code().to_string(),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn bild_beschreibung_status() -> BildBeschreibungStatus {
    status_aus_plan(vision_plan(true).await)
}

#[derive(Default)]
pub struct BildAuftraege {
    auftraege: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

fn id_erlaubt(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

struct AuftragWache {
    registry: Arc<BildAuftraege>,
    id: String,
    abbruch: Arc<AtomicBool>,
}

impl Drop for AuftragWache {
    fn drop(&mut self) {
        self.registry
            .auftraege
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner())
            .remove(&self.id);
    }
}

fn anmelden(registry: &Arc<BildAuftraege>, id: &str) -> Result<AuftragWache, String> {
    if !id_erlaubt(id) {
        return Err("ungueltige_auftrag_id".into());
    }
    let mut auftraege = registry
        .auftraege
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    if auftraege.contains_key(id) {
        return Err("auftrag_laeuft".into());
    }
    let abbruch = Arc::new(AtomicBool::new(false));
    auftraege.insert(id.to_string(), Arc::clone(&abbruch));
    Ok(AuftragWache {
        registry: Arc::clone(registry),
        id: id.to_string(),
        abbruch,
    })
}

fn abbrechen(registry: &BildAuftraege, id: &str) -> Result<bool, String> {
    if !id_erlaubt(id) {
        return Err("ungueltige_auftrag_id".into());
    }
    let auftraege = registry
        .auftraege
        .lock()
        .unwrap_or_else(|fehler| fehler.into_inner());
    if let Some(abbruch) = auftraege.get(id) {
        abbruch.store(true, Ordering::Release);
        Ok(true)
    } else {
        Ok(false)
    }
}

fn auswerten_angemeldet(
    wache: AuftragWache,
    pfad: &Path,
    beschreiben: bool,
    extraktor: impl FnOnce(&Path, &BildOptionen) -> Result<String, String>,
) -> Result<String, String> {
    let optionen = BildOptionen {
        beschreibung: beschreiben,
        abbruch: Arc::clone(&wache.abbruch),
    };
    // Die Wache bleibt auch bei Fehlern und beim Entrollen einer Panik aktiv.
    extraktor(pfad, &optionen).map_err(|_| "auswertung_fehlgeschlagen".into())
}

#[cfg(test)]
fn auswerten_mit(
    registry: &Arc<BildAuftraege>,
    id: &str,
    pfad: &Path,
    beschreiben: bool,
    extraktor: impl FnOnce(&Path, &BildOptionen) -> Result<String, String>,
) -> Result<String, String> {
    auswerten_angemeldet(anmelden(registry, id)?, pfad, beschreiben, extraktor)
}

#[tauri::command]
#[specta::specta]
pub async fn bild_auswerten(
    registry: State<'_, Arc<BildAuftraege>>,
    auftrag_id: String,
    pfad: String,
    beschreiben: bool,
) -> Result<String, String> {
    // Vor dem Start anmelden, damit auch wartende Aufträge abbrechbar sind.
    let wache = anmelden(registry.inner(), &auftrag_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        auswerten_angemeldet(wache, Path::new(&pfad), beschreiben, bild::extrahieren)
    })
    .await
    .map_err(|_| "auswertung_fehlgeschlagen".to_string())?
}

#[tauri::command]
#[specta::specta]
pub fn bild_abbrechen(
    registry: State<'_, Arc<BildAuftraege>>,
    auftrag_id: String,
) -> Result<bool, String> {
    abbrechen(registry.inner(), &auftrag_id)
}

fn endung_erlaubt(endung: &str) -> bool {
    matches!(
        endung,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff"
    )
}

fn magic_passt(daten: &[u8], endung: &str) -> bool {
    match endung {
        "png" => daten.starts_with(b"\x89PNG"),
        "jpg" | "jpeg" => daten.starts_with(b"\xff\xd8\xff"),
        "gif" => daten.starts_with(b"GIF8"),
        "webp" => daten.starts_with(b"RIFF") && daten.get(8..12) == Some(b"WEBP"),
        "bmp" => daten.starts_with(b"BM"),
        "tif" | "tiff" => daten.starts_with(b"II*\0") || daten.starts_with(b"MM\0*"),
        _ => false,
    }
}

fn zufallsname(endung: &str) -> String {
    let mut zufall = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut zufall);
    let hex: String = zufall.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("lva-bild-{hex}.{endung}")
}

fn temp_ablegen(daten: &[u8], endung: &str) -> Result<String, String> {
    if daten.is_empty()
        || daten.len() as u64 > bild::MAX_DATEI_BYTES
        || !endung_erlaubt(endung)
        || !magic_passt(daten, endung)
    {
        return Err("kein_bild".into());
    }
    for _ in 0..8 {
        let pfad = std::env::temp_dir().join(zufallsname(endung));
        // Ein nicht als UTF-8 darstellbarer Pfad darf keine verwaiste Datei erzeugen.
        let ausgabe = pfad.to_str().ok_or("temp_fehler")?.to_string();
        match OpenOptions::new().write(true).create_new(true).open(&pfad) {
            Ok(mut datei) => {
                if datei.write_all(daten).is_err() {
                    drop(datei);
                    let _ = fs::remove_file(&pfad);
                    return Err("temp_fehler".into());
                }
                return Ok(ausgabe);
            }
            Err(fehler) if fehler.kind() == ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("temp_fehler".into()),
        }
    }
    Err("temp_fehler".into())
}

#[tauri::command]
#[specta::specta]
pub fn bild_temp_ablegen(daten: Vec<u8>, endung: String) -> Result<String, String> {
    temp_ablegen(&daten, &endung)
}

fn name_erlaubt(name: &str) -> bool {
    let Some((stamm, endung)) = name.rsplit_once('.') else {
        return false;
    };
    let Some(hex) = stamm.strip_prefix("lva-bild-") else {
        return false;
    };
    hex.len() == 32 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) && endung_erlaubt(endung)
}

fn ist_verknuepfung(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT umfasst auch andere Windows-Verknüpfungen.
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn temp_loeschen(pfad: &Path) -> Result<(), String> {
    if pfad
        .components()
        .any(|teil| matches!(teil, Component::ParentDir))
        || !pfad
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(name_erlaubt)
    {
        return Err("nicht_erlaubt".into());
    }
    let eltern = pfad.parent().ok_or("nicht_erlaubt")?;
    let temp = fs::canonicalize(std::env::temp_dir()).map_err(|_| "nicht_erlaubt")?;
    if fs::canonicalize(eltern).map_err(|_| "nicht_erlaubt")? != temp {
        return Err("nicht_erlaubt".into());
    }
    match fs::symlink_metadata(pfad) {
        Ok(metadata) if !metadata.is_file() || ist_verknuepfung(&metadata) => {
            return Err("nicht_erlaubt".into());
        }
        Ok(_) => {}
        Err(fehler) if fehler.kind() == ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("temp_fehler".into()),
    }
    match fs::remove_file(pfad) {
        Ok(()) => Ok(()),
        Err(fehler) if fehler.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err("temp_fehler".into()),
    }
}

#[tauri::command]
#[specta::specta]
pub fn bild_temp_loeschen(pfad: String) -> Result<(), String> {
    temp_loeschen(Path::new(&pfad))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn status_off() {
        let status = status_aus_plan(VisionPlan::Off);
        assert!(!status.verfuegbar);
        assert_eq!(status.code, "off");
    }

    #[test]
    fn status_ready() {
        let status = status_aus_plan(VisionPlan::Ready);
        assert!(status.verfuegbar);
        assert_eq!(status.code, "ready");
    }

    #[test]
    fn status_unavailable() {
        let status = status_aus_plan(VisionPlan::Unavailable("kein_modell"));
        assert!(!status.verfuegbar);
        assert_eq!(status.code, "kein_modell");
    }

    #[test]
    fn registry_anmelden() {
        let registry = Arc::new(BildAuftraege::default());
        let wache = anmelden(&registry, "Auftrag-123").unwrap();
        assert!(registry
            .auftraege
            .lock()
            .unwrap()
            .contains_key("Auftrag-123"));
        assert!(!wache.abbruch.load(Ordering::Acquire));
    }

    #[test]
    fn registry_doppelte_id() {
        let registry = Arc::new(BildAuftraege::default());
        let _wache = anmelden(&registry, "id").unwrap();
        assert_eq!(anmelden(&registry, "id").err().unwrap(), "auftrag_laeuft");
    }

    #[test]
    fn registry_ungueltige_ids() {
        let registry = Arc::new(BildAuftraege::default());
        for id in ["", "a/b", "a_b", "ä", &"a".repeat(65)] {
            assert_eq!(
                anmelden(&registry, id).err().unwrap(),
                "ungueltige_auftrag_id"
            );
            assert_eq!(
                abbrechen(&registry, id),
                Err("ungueltige_auftrag_id".into())
            );
        }
        assert!(anmelden(&registry, &"a".repeat(64)).is_ok());
    }

    #[test]
    fn registry_abbruch_wiederholbar() {
        let registry = Arc::new(BildAuftraege::default());
        let wache = anmelden(&registry, "id").unwrap();
        assert_eq!(abbrechen(&registry, "id"), Ok(true));
        assert_eq!(abbrechen(&registry, "id"), Ok(true));
        assert!(wache.abbruch.load(Ordering::Acquire));
        assert_eq!(abbrechen(&registry, "unbekannt"), Ok(false));
    }

    #[test]
    fn registry_ende_und_fehler() {
        let registry = Arc::new(BildAuftraege::default());
        for fehler in [false, true] {
            let ergebnis = auswerten_mit(&registry, "id", Path::new("bild"), false, |_, opt| {
                assert!(!opt.beschreibung);
                if fehler {
                    Err("vertraulicher Pfad".into())
                } else {
                    Ok("Text".into())
                }
            });
            assert_eq!(
                ergebnis,
                if fehler {
                    Err("auswertung_fehlgeschlagen".into())
                } else {
                    Ok("Text".into())
                }
            );
            assert!(registry.auftraege.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn registry_panik() {
        let registry = Arc::new(BildAuftraege::default());
        let ergebnis = std::panic::catch_unwind(|| {
            auswerten_mit(&registry, "id", Path::new("bild"), true, |_, _| {
                panic!("Test")
            })
        });
        assert!(ergebnis.is_err());
        assert!(registry.auftraege.lock().unwrap().is_empty());
    }

    #[test]
    fn abbruch_erreicht_laufenden_extraktor() {
        let registry = Arc::new(BildAuftraege::default());
        let arbeiter_registry = Arc::clone(&registry);
        let (gestartet_tx, gestartet_rx) = mpsc::channel();
        let (weiter_tx, weiter_rx) = mpsc::channel();
        let arbeiter = std::thread::spawn(move || {
            auswerten_mit(
                &arbeiter_registry,
                "id",
                Path::new("bild"),
                true,
                |pfad, opt| {
                    assert_eq!(pfad, Path::new("bild"));
                    assert!(opt.beschreibung);
                    gestartet_tx.send(()).unwrap();
                    weiter_rx.recv().unwrap();
                    assert!(opt.abbruch.load(Ordering::Acquire));
                    Ok("abgebrochen".into())
                },
            )
        });
        gestartet_rx.recv().unwrap();
        assert_eq!(abbrechen(&registry, "id"), Ok(true));
        weiter_tx.send(()).unwrap();
        assert_eq!(arbeiter.join().unwrap(), Ok("abgebrochen".into()));
        assert!(registry.auftraege.lock().unwrap().is_empty());
    }

    #[test]
    fn temp_gueltige_formate_und_namen() {
        let formate: &[(&str, &[u8])] = &[
            ("png", b"\x89PNG"),
            ("jpg", b"\xff\xd8\xff"),
            ("jpeg", b"\xff\xd8\xff"),
            ("gif", b"GIF8"),
            ("webp", b"RIFF1234WEBP"),
            ("bmp", b"BM"),
            ("tif", b"II*\0"),
            ("tiff", b"MM\0*"),
        ];
        for (endung, daten) in formate {
            let pfad = temp_ablegen(daten, endung).unwrap();
            let pfad = Path::new(&pfad);
            assert!(name_erlaubt(pfad.file_name().unwrap().to_str().unwrap()));
            assert_eq!(pfad.extension().unwrap(), *endung);
            assert_eq!(fs::read(pfad).unwrap(), *daten);
            temp_loeschen(pfad).unwrap();
        }
    }

    #[test]
    fn temp_falsche_endung_oder_magic() {
        for (daten, endung) in [
            (&b"\x89PNG"[..], "exe"),
            (&b"\x89PNG"[..], "PNG"),
            (&b"GIF8"[..], "png"),
            (&b"RIFF1234FAIL"[..], "webp"),
            (&b"RIFF"[..], "webp"),
            (&b"\xff\xd8"[..], "jpg"),
        ] {
            assert_eq!(temp_ablegen(daten, endung), Err("kein_bild".into()));
        }
    }

    #[test]
    fn temp_leer() {
        assert_eq!(temp_ablegen(&[], "png"), Err("kein_bild".into()));
    }

    #[test]
    fn temp_groessenlimit() {
        let mut daten = vec![0; bild::MAX_DATEI_BYTES as usize + 1];
        daten[..4].copy_from_slice(b"\x89PNG");
        assert_eq!(temp_ablegen(&daten, "png"), Err("kein_bild".into()));
        daten.pop();
        let pfad = temp_ablegen(&daten, "png").unwrap();
        temp_loeschen(Path::new(&pfad)).unwrap();
    }

    #[test]
    fn temp_loeschen_erlaubt_und_fehlend() {
        let pfad = temp_ablegen(b"BM", "bmp").unwrap();
        let pfad = Path::new(&pfad);
        temp_loeschen(pfad).unwrap();
        assert!(!pfad.exists());
        assert_eq!(temp_loeschen(pfad), Ok(()));

        // Ein passender Name erlaubt weder Verzeichnisse noch Verknüpfungen.
        fs::create_dir(pfad).unwrap();
        let ergebnis = temp_loeschen(pfad);
        let vorhanden = pfad.is_dir();
        fs::remove_dir(pfad).unwrap();
        assert_eq!(ergebnis, Err("nicht_erlaubt".into()));
        assert!(vorhanden);

        #[cfg(unix)]
        {
            let ziel = temp_ablegen(b"BM", "bmp").unwrap();
            std::os::unix::fs::symlink(&ziel, pfad).unwrap();
            let ergebnis = temp_loeschen(pfad);
            let vorhanden = fs::symlink_metadata(pfad).is_ok();
            fs::remove_file(pfad).unwrap();
            let ziel_vorhanden = Path::new(&ziel).exists();
            temp_loeschen(Path::new(&ziel)).unwrap();
            assert_eq!(ergebnis, Err("nicht_erlaubt".into()));
            assert!(vorhanden);
            assert!(ziel_vorhanden);
        }
    }

    #[test]
    fn temp_fremder_name_bleibt() {
        let pfad = std::env::temp_dir().join(format!("fremd-{}", zufallsname("png")));
        fs::write(&pfad, b"fremd").unwrap();
        let ergebnis = temp_loeschen(&pfad);
        let inhalt = fs::read(&pfad).unwrap();
        fs::remove_file(&pfad).unwrap();
        assert_eq!(ergebnis, Err("nicht_erlaubt".into()));
        assert_eq!(inhalt, b"fremd");
        for name in [
            "lva-bild-a.png",
            "lva-bild-0000000000000000000000000000000g.png",
        ] {
            assert!(!name_erlaubt(name));
        }
    }

    #[test]
    fn temp_ausserhalb_bleibt() {
        let eltern = std::env::current_dir().unwrap();
        assert_ne!(
            fs::canonicalize(&eltern).unwrap(),
            fs::canonicalize(std::env::temp_dir()).unwrap()
        );
        let pfad = eltern.join(zufallsname("png"));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pfad)
            .unwrap();
        let ergebnis = temp_loeschen(&pfad);
        let vorhanden = pfad.exists();
        fs::remove_file(&pfad).unwrap();
        assert_eq!(ergebnis, Err("nicht_erlaubt".into()));
        assert!(vorhanden);
    }

    #[test]
    fn temp_elternkomponente_abgelehnt() {
        let pfad = temp_ablegen(b"BM", "bmp").unwrap();
        let original = Path::new(&pfad);
        let temp = std::env::temp_dir();
        let umweg = temp
            .join("..")
            .join(temp.file_name().unwrap())
            .join(original.file_name().unwrap());
        let ergebnis = temp_loeschen(&umweg);
        let vorhanden = original.exists();
        temp_loeschen(original).unwrap();
        assert_eq!(ergebnis, Err("nicht_erlaubt".into()));
        assert!(vorhanden);
    }
}
