//! Geteilte Dateien des portablen Offline-Pakets wieder zusammensetzen.
//!
//! GitHub nimmt je Release-Datei hoechstens 2 GiB an. Das Offline-Paket
//! (`scripts/build-portable-zip.ps1 -Offline`) teilt deshalb jede groessere
//! Datei (das Sprachmodell, 2,7 GB) in Stuecke und verteilt sie auf mehrere
//! ZIPs. Neben den Stuecken liegt eine Beschreibung `<datei>.teile.json`:
//!
//! ```json
//! { "file": "Qwen3.5-4B-Q4_K_M.gguf", "size": 2740937888,
//!   "sha256": "00fe…", "parts": ["Qwen3.5-4B-Q4_K_M.gguf.teil001", "…teil002"] }
//! ```
//!
//! Beim Start im portablen Modus sucht [`assemble_all`] unter `Data\` nach
//! solchen Beschreibungen. Sind alle Stuecke da, werden sie in eine
//! Zwischendatei kopiert, Groesse und SHA-256 geprueft und erst dann auf den
//! Zielnamen umbenannt; danach werden Stuecke und Beschreibung entfernt. Fehlt
//! ein Stueck (zweite ZIP noch nicht entpackt), bleibt alles liegen und die
//! App startet normal: das Modell gilt dann als nicht installiert, bis der
//! naechste Start es zusammensetzt.
//!
//! | Fall | Absicherung |
//! |---|---|
//! | Stueck fehlt | nichts wird angefasst, Hinweis im Log (`missing_parts_are_left_alone`) |
//! | Stueck beschaedigt / falsche Groesse | Zwischendatei wird verworfen, Stuecke bleiben (`a_wrong_hash_keeps_the_parts`) |
//! | Zwei Starts gleichzeitig | Sperrdatei per `create_new`; der zweite ueberspringt (`a_held_lock_skips_assembly`) |
//! | Absturz mittendrin | alte Sperre (> 30 min) wird verworfen, Zwischendatei neu geschrieben |
//! | Pfade mit `..` oder Verzeichnisanteil in der Beschreibung | abgelehnt (`names_must_be_plain_file_names`) |

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MANIFEST_SUFFIX: &str = ".teile.json";
const LOCK_SUFFIX: &str = ".zusammensetzen";
const STALE_LOCK: Duration = Duration::from_secs(30 * 60);
/// Tiefe der Suche unter `Data\` (z. B. `llm\models\<id>\datei`).
const MAX_DEPTH: usize = 6;

#[derive(Debug, Deserialize)]
struct Manifest {
    file: String,
    size: u64,
    sha256: String,
    parts: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Assembled,
    AlreadyPresent,
    MissingParts,
    Busy,
}

/// Voreinstellung des Offline-Pakets (gewaehlte Spracherkennung und Sprachmodell).
/// Sie liegt unter eigenem Namen im ZIP, damit ein Entpacken ueber einen
/// bestehenden Ordner die Einstellungen der Person nie ueberschreibt: uebernommen
/// wird sie nur, solange es noch keine Einstellungen gibt.
pub const SETTINGS_SEED: &str = "settings_store.seed.json";

pub fn apply_settings_seed(root: &Path) {
    let seed = root.join(SETTINGS_SEED);
    if !seed.is_file() {
        return;
    }
    let target = root.join(crate::settings::SETTINGS_STORE_PATH);
    if target.exists() {
        let _ = fs::remove_file(&seed);
    } else if let Err(e) = fs::rename(&seed, &target) {
        eprintln!("[portable] Voreinstellung nicht uebernommen: {e}");
    }
}

/// Alle Beschreibungen unter `root` abarbeiten. Fehler einzelner Dateien
/// werden gemeldet, halten aber den Start nicht auf.
pub fn assemble_all(root: &Path) {
    apply_settings_seed(root);
    let mut manifests = Vec::new();
    collect_manifests(root, 0, &mut manifests);
    for manifest in manifests {
        match assemble(&manifest) {
            Ok(Outcome::Assembled) => {
                eprintln!("[portable] zusammengesetzt: {}", manifest.display())
            }
            Ok(Outcome::MissingParts) => eprintln!(
                "[portable] unvollstaendig (weitere ZIP-Teile in denselben Ordner entpacken): {}",
                manifest.display()
            ),
            Ok(_) => {}
            Err(e) => eprintln!("[portable] {}: {e}", manifest.display()),
        }
    }
}

fn collect_manifests(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_manifests(&path, depth + 1, out);
        } else if kind.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(MANIFEST_SUFFIX))
        {
            out.push(path);
        }
    }
}

fn plain_name(name: &str) -> Result<&str, String> {
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && Path::new(name).file_name().and_then(|n| n.to_str()) == Some(name);
    if ok {
        Ok(name)
    } else {
        Err(format!(
            "unzulaessiger Dateiname in der Beschreibung: {name:?}"
        ))
    }
}

pub fn assemble(manifest_path: &Path) -> Result<Outcome, String> {
    let dir = manifest_path
        .parent()
        .ok_or_else(|| "Beschreibung ohne Ordner".to_string())?;
    let text = fs::read_to_string(manifest_path).map_err(|e| e.to_string())?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let target = dir.join(plain_name(&manifest.file)?);
    let parts = manifest
        .parts
        .iter()
        .map(|p| plain_name(p).map(|n| dir.join(n)))
        .collect::<Result<Vec<_>, _>>()?;
    if parts.is_empty() {
        return Err("Beschreibung nennt keine Teile".to_string());
    }

    if target.is_file() && fs::metadata(&target).map(|m| m.len()).ok() == Some(manifest.size) {
        cleanup(&parts, manifest_path);
        return Ok(Outcome::AlreadyPresent);
    }
    if parts.iter().any(|p| !p.is_file()) {
        return Ok(Outcome::MissingParts);
    }
    let have: u64 = parts
        .iter()
        .map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum();
    if have != manifest.size {
        return Err(format!(
            "Teile ergeben {have} Bytes, erwartet {} (ZIP vollstaendig entpackt?)",
            manifest.size
        ));
    }

    let lock = with_suffix(&target, LOCK_SUFFIX);
    if !take_lock(&lock)? {
        return Ok(Outcome::Busy);
    }
    let result = join_and_verify(&parts, &target, &manifest);
    let _ = fs::remove_file(&lock);
    result?;
    cleanup(&parts, manifest_path);
    Ok(Outcome::Assembled)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn take_lock(lock: &Path) -> Result<bool, String> {
    for _ in 0..2 {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock)
        {
            Ok(_) => return Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(lock)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > STALE_LOCK);
                if !stale {
                    return Ok(false);
                }
                let _ = fs::remove_file(lock);
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(false)
}

fn join_and_verify(parts: &[PathBuf], target: &Path, manifest: &Manifest) -> Result<(), String> {
    let tmp = with_suffix(target, ".partial");
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    {
        let mut out = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        for part in parts {
            let mut input = fs::File::open(part).map_err(|e| e.to_string())?;
            loop {
                let n = input.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                out.write_all(&buf[..n]).map_err(|e| {
                    let _ = fs::remove_file(&tmp);
                    format!("Schreiben fehlgeschlagen (Datentraeger voll?): {e}")
                })?;
                written += n as u64;
            }
        }
        out.sync_all().map_err(|e| e.to_string())?;
    }
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if written != manifest.size || !actual.eq_ignore_ascii_case(manifest.sha256.trim()) {
        let _ = fs::remove_file(&tmp);
        return Err(format!(
            "Pruefsumme stimmt nicht (erwartet {}, erhalten {actual}); Teile bleiben liegen",
            manifest.sha256
        ));
    }
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

fn cleanup(parts: &[PathBuf], manifest_path: &Path) {
    for p in parts {
        let _ = fs::remove_file(p);
    }
    let _ = fs::remove_file(manifest_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(data: &[u8]) -> String {
        Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn setup(name: &str, data: &[u8], split: usize, write_parts: usize) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("lva_parts_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("llm").join("models")).unwrap();
        let sub = dir.join("llm").join("models");
        let chunks: Vec<&[u8]> = data.chunks(split).collect();
        let names: Vec<String> = (1..=chunks.len())
            .map(|i| format!("m.gguf.teil{i:03}"))
            .collect();
        for (i, chunk) in chunks.iter().enumerate().take(write_parts) {
            fs::write(sub.join(&names[i]), chunk).unwrap();
        }
        let manifest = serde_json::json!({
            "file": "m.gguf", "size": data.len(), "sha256": sha(data), "parts": names,
        });
        let mpath = sub.join(format!("m.gguf{MANIFEST_SUFFIX}"));
        fs::write(&mpath, manifest.to_string()).unwrap();
        (dir, mpath)
    }

    #[test]
    fn all_parts_are_joined_verified_and_removed() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let (dir, mpath) = setup("ok", &data, 3000, 4);
        assemble_all(&dir);
        let sub = mpath.parent().unwrap();
        assert_eq!(fs::read(sub.join("m.gguf")).unwrap(), data);
        assert!(!mpath.exists());
        assert!(!sub.join("m.gguf.teil001").exists());
        assert!(!sub.join(format!("m.gguf{LOCK_SUFFIX}")).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_parts_are_left_alone() {
        let data = vec![7u8; 5000];
        let (dir, mpath) = setup("missing", &data, 2000, 2);
        assert_eq!(assemble(&mpath).unwrap(), Outcome::MissingParts);
        let sub = mpath.parent().unwrap();
        assert!(!sub.join("m.gguf").exists());
        assert!(sub.join("m.gguf.teil001").exists());
        assert!(mpath.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_wrong_hash_keeps_the_parts() {
        let data = vec![1u8; 4000];
        let (dir, mpath) = setup("hash", &data, 2000, 2);
        let sub = mpath.parent().unwrap().to_path_buf();
        fs::write(sub.join("m.gguf.teil002"), vec![2u8; 2000]).unwrap();
        assert!(assemble(&mpath).is_err());
        assert!(!sub.join("m.gguf").exists());
        assert!(!sub.join("m.gguf.partial").exists());
        assert!(sub.join("m.gguf.teil001").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_held_lock_skips_assembly() {
        let data = vec![3u8; 3000];
        let (dir, mpath) = setup("lock", &data, 1000, 3);
        let sub = mpath.parent().unwrap();
        fs::write(sub.join(format!("m.gguf{LOCK_SUFFIX}")), b"").unwrap();
        assert_eq!(assemble(&mpath).unwrap(), Outcome::Busy);
        assert!(!sub.join("m.gguf").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_existing_target_only_cleans_up() {
        let data = vec![4u8; 3000];
        let (dir, mpath) = setup("present", &data, 1000, 3);
        let sub = mpath.parent().unwrap();
        fs::write(sub.join("m.gguf"), &data).unwrap();
        assert_eq!(assemble(&mpath).unwrap(), Outcome::AlreadyPresent);
        assert!(!mpath.exists());
        assert!(!sub.join("m.gguf.teil002").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_seed_never_overwrites_existing_settings() {
        let dir = std::env::temp_dir().join(format!("lva_seed_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join(crate::settings::SETTINGS_STORE_PATH);
        fs::write(dir.join(SETTINGS_SEED), b"seed").unwrap();
        apply_settings_seed(&dir);
        assert_eq!(fs::read(&target).unwrap(), b"seed");
        fs::write(&target, b"eigene").unwrap();
        fs::write(dir.join(SETTINGS_SEED), b"seed2").unwrap();
        apply_settings_seed(&dir);
        assert_eq!(fs::read(&target).unwrap(), b"eigene");
        assert!(!dir.join(SETTINGS_SEED).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn names_must_be_plain_file_names() {
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "C:x"] {
            assert!(plain_name(bad).is_err(), "{bad:?}");
        }
        assert!(plain_name("Qwen3.5-4B-Q4_K_M.gguf.teil001").is_ok());
    }
}
