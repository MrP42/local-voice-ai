//! Nur für das gleichnamige Example. Produktive Dateien werden ausschließlich gelesen.
use super::*;
use super::voice_sync::zeit;
use crate::ablage::zusammenfuehren::Datei;
use crate::sync::ablage::{konto_kennung, Konto};
use crate::sync::account::SyncConfig;
use std::collections::BTreeSet;

const MODELLE: [&str; 2] = ["phonon-40m-de", "phonon-31m-en"];
type Res<T> = Result<T, &'static str>;

fn hochladen(args: &[String]) -> Res<bool> {
    match args {
        [] => Ok(false),
        [arg] if arg == "--hochladen" => Ok(true),
        _ => Err("Aufruf: phonon_stimmen_hochladen [--hochladen]"),
    }
}

struct Listenzeile {
    name: String,
    sprache: String,
    sha256: String,
    emb: Vec<(String, usize)>,
    objekt_bytes: usize,
}

impl Listenzeile {
    fn ausgabe(&self) -> String {
        let emb = self.emb.iter().map(|(id, n)| format!("{id}: {n} B")).collect::<Vec<_>>().join(", ");
        format!("{} | {} | {} | {} | Objekt: {} B", self.name, self.sprache, self.sha256, emb, self.objekt_bytes)
    }
}

fn listenzeile(r: &Record, bytes: &[u8], d: &Datei) -> Res<Listenzeile> {
    let sha = sha256_hex(bytes);
    let mut emb = Vec::new();
    for id in MODELLE {
        let v = d.extra.get(&format!("phonon_emb_{id}")).ok_or("emb-Fassung fehlt.")?;
        if v[1] != sha { return Err("emb-Fassung gehört zu einem anderen Master."); }
        let size = v[2]["size"].as_u64().ok_or("emb-Größe fehlt.")? as usize;
        emb.push((id.into(), size));
    }
    Ok(Listenzeile { name: r.name.clone(), sprache: if r.language.is_empty() { "und".into() } else { r.language.clone() },
        sha256: sha[..8].into(), emb, objekt_bytes: crate::sync::ablage::stimmobjekt_groesse(d) })
}

// Dieselben Known Folders wie Tauri/dirs; kein Tauri-Runtime-Start, keine Verzeichnisanlage.
#[cfg(windows)]
fn app_pfade() -> Res<(PathBuf, PathBuf)> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath, FOLDERID_LocalAppData, FOLDERID_RoamingAppData, KF_FLAG_DONT_VERIFY};
    fn folder(id: &windows::core::GUID) -> Res<PathBuf> {
        // SAFETY: Known-Folder-Kennung ist gültig; der von Windows allozierte Puffer wird freigegeben.
        unsafe {
            let p = SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None).map_err(|_| "AppData-Pfad nicht verfügbar.")?;
            let path = PathBuf::from(std::ffi::OsString::from_wide(p.as_wide()));
            CoTaskMemFree(Some(p.0.cast()));
            let config: Value = serde_json::from_str(include_str!("../../../../tauri.conf.json"))
                .map_err(|_| "App-Kennung nicht lesbar.")?;
            Ok(path.join(config["identifier"].as_str().ok_or("App-Kennung fehlt.")?))
        }
    }
    Ok((folder(&FOLDERID_LocalAppData)?, folder(&FOLDERID_RoamingAppData)?))
}

#[cfg(not(windows))]
fn app_pfade() -> Res<(PathBuf, PathBuf)> { Err("Dieses einmalige Werkzeug unterstützt die installierte Windows-App.") }

struct Stimme { record: Record, bytes: Vec<u8>, datei: Datei }

fn vorbereiten(root: &Path, idx: Index, konto: &Konto<'_>, weights: &[(crate::catalog::TtsCatalogEntry, Vec<f32>)]) -> Res<Vec<Stimme>> {
    if idx.version > 1 || idx.sync_receive.is_some() { return Err("Stimmenindex neuer oder Empfang offen; App-Abgleich zuerst abschließen."); }
    if idx.voices.len() > MAX_OWN_VOICES { return Err("Zu viele Stimmen im Index."); }
    let mut ids = BTreeSet::new();
    let mut uids = BTreeSet::new();
    let mut out = Vec::new();
    let kennung = konto_kennung(konto.user_id, konto.key, konto.hub_url);
    for r in idx.voices {
        if idx.sync_removed.contains(&r.uid) { continue; }
        let parsed = parse_name(&r.name).map_err(|_| "Ungültiger Stimmname im Index.")?;
        if parsed.display != r.name || !parsed.id.eq_ignore_ascii_case(&r.id) || !origin_is_own(&r.origin)
            || !matches!(r.language.as_str(), "" | "und" | "de" | "en" | "fr" | "es" | "pt")
            || r.uid.len() != 32 || !r.uid.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            || !ids.insert(r.id.to_ascii_lowercase()) || !uids.insert(r.uid.clone())
            || chrono::DateTime::parse_from_rfc3339(&r.created_at).is_err() {
            return Err("Stimmenindex ungültig oder UID fehlt; zuerst den Stimmenabgleich der App ausführen.");
        }
        // Auch die Dateikennung selbst prüfen, bevor sie als Pfad verwendet wird.
        if parse_name(&r.id).map_err(|_| "Ungültige Stimmkennung.")?.id != r.id { return Err("Ungültige Stimmkennung."); }
        let bytes = read_voice_file(&root.join(STORE_DIR).join(format!("{}.safetensors", r.id)))
            .map_err(|_| "Stimmdatei nicht lesbar oder ungültig.")?;
        if r.sha256.as_ref().is_some_and(|s| s != &sha256_hex(&bytes)) { return Err("Master geändert; zuerst den App-Abgleich abschließen."); }
        let generation = r.sync_file.as_ref().filter(|l| l.konto == kennung).map(|l| l.generation).unwrap_or(0);
        let mut datei = voice_sync::stimmdatei(&r, &bytes, konto, generation, None)
            .map_err(|_| "Stimmobjekt nicht erzeugbar.")?;
        for (entry, w) in weights {
            let emb = embedding::encode(&bytes, w).map_err(|_| "emb-Fassung nicht erzeugbar.")?;
            voice_sync::emb_eintragen(&mut datei, entry, &sha256_hex(&bytes), zeit(&r), &emb)
                .map_err(|_| "emb-Metadaten ungültig.")?;
        }
        crate::ablage::zusammenfuehren::phonon_emb_begrenzen(&mut datei, crate::sync::collect::MAX_PLAINTEXT);
        listenzeile(&r, &bytes, &datei)?;
        out.push(Stimme { record: r, bytes, datei });
    }
    Ok(out)
}

fn ausfuehren(args: &[String]) -> Res<()> {
    let upload = hochladen(args)?;
    let exe = std::env::current_exe().map_err(|_| "Programmverzeichnis nicht verfügbar.")?;
    if exe.parent().is_some_and(|p| p.join("portable").exists()) {
        return Err("Portable-Modus wird nicht automatisch geöffnet; dieses Werkzeug gilt für die installierte App.");
    }
    let (local, roaming) = app_pfade()?;
    let dirs = super::super::phonon_models::PhononDirs::new(&local);
    let root = dirs.root();
    let cfg_path = roaming.join("sync.json");
    let cfg_raw = fs::read(&cfg_path).map_err(|_| "Sync-Konto nicht lesbar.")?;
    let cfg: SyncConfig = serde_json::from_slice(&cfg_raw).map_err(|_| "Sync-Konto ungültig.")?;
    let key = cfg.key().map_err(|_| "Sync-Schlüssel ungültig.")?;
    let konto = Konto { key: &key, user_id: cfg.user_id, device_id: &cfg.device_id, hub_url: &cfg.url };
    if cfg.user_id <= 0 || cfg.token.is_empty() || !crate::sync::account::valid_device_id(&cfg.device_id) {
        return Err("Sync-Konto unvollständig.");
    }
    let idx_path = root.join(STORE_DIR).join(INDEX_FILE);
    // Absichtlich nicht read_index(): dessen Reparaturpfade schreiben in den App-Bestand.
    let idx_raw = fs::read(&idx_path).map_err(|_| "Stimmenindex nicht lesbar.")?;
    let idx: Index = serde_json::from_slice(&idx_raw).map_err(|_| "Stimmenindex ungültig.")?;
    let installed = super::super::phonon_models::installed_model_dirs(&dirs);
    let katalog = crate::catalog::tts_entries(crate::catalog::Purpose::PhononModel);
    let mut weights = Vec::new();
    for id in MODELLE {
        let (_, dir) = installed.iter().find(|(name, _)| name == id).ok_or("Beide Phonon-Modelle müssen vollständig installiert sein.")?;
        let entry = katalog.iter().find(|e| e.id == id).ok_or("Modell fehlt im Katalog.")?;
        let file = entry.files.iter().find(|f| f.filename.ends_with(".gguf")).ok_or("Modell ohne GGUF.")?;
        weights.push((entry.clone(), embedding::load_weight(&dir.join(&file.filename)).map_err(|_| "Speaker-Projektion nicht lesbar.")?));
    }
    let stimmen = vorbereiten(root, idx, &konto, &weights)?;
    for s in &stimmen { println!("{}", listenzeile(&s.record, &s.bytes, &s.datei)?.ausgabe()); }
    if !upload { return Ok(()); }
    let aktuell = || fs::read(&cfg_path).is_ok_and(|b| b == cfg_raw)
        && fs::read(&idx_path).is_ok_and(|b| b == idx_raw)
        && stimmen.iter().all(|s| read_voice_file(&root.join(STORE_DIR).join(format!("{}.safetensors", s.record.id)))
            .is_ok_and(|b| b == s.bytes));
    let client = crate::sync::client::HubClient::new(&cfg.url, &cfg.token);
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|_| "Async-Laufzeit nicht verfügbar.")?;
    runtime.block_on(async {
        let objekte = stimmen.iter().map(|s| {
            let generation = s.record.sync_file.as_ref().filter(|l| l.konto == konto_kennung(konto.user_id, konto.key, konto.hub_url))
                .map(|l| l.generation).unwrap_or(0);
            (voice_sync::stimmordner(&key, generation), s.datei.clone(), s.bytes.clone())
        }).collect::<Vec<_>>();
        match crate::sync::ablage::stimmen_einmalig::hochladen(&client, &konto, objekte, aktuell).await {
            Ok(n) => { eprintln!("Hochgeladen: {n} Stimmen."); Ok(()) }
            Err((n, text)) => { eprintln!("Teilergebnis: {n} Stimmen bestätigt; weitere Objekte/Blobs können bereits angekommen sein."); Err(text) }
        }
    })
}

pub fn haupt(args: &[String]) -> i32 {
    match ausfuehren(args) {
        Ok(()) => 0,
        Err(text) => { eprintln!("Abbruch: {text}"); 1 }
    }
}

#[cfg(test)]
#[path = "einmalig_tests.rs"]
mod tests;
