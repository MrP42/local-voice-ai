//! Sichtbarer Direktdownload nach Klick (lva#831). Keine Installer oder Skripte.
//! Fehler enthalten nur feste Kennungen und den Zielhost, nie Antworten oder URLs mit Token.
use super::cli::Cli;
use futures_util::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::time::Duration;
use tempfile::NamedTempFile;

pub const HOSTS_CLAUDE: &[&str] = &["downloads.claude.ai"];
pub const HOSTS_CODEX: &[&str] = &[
    "api.github.com", "github.com", "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];
const MAX_META: u64 = 1024 * 1024;
const MAX_BINARY: u64 = 512 * 1024 * 1024;
const GESAMTFRIST: Duration = Duration::from_secs(300);
const CLAUDE_BASE: &str = "https://downloads.claude.ai/claude-code-releases";
const CODEX_RELEASE: &str = "https://api.github.com/repos/openai/codex/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum InstallFehler {
    NetzGesperrt { ziel: String },
    Blockiert,
    Pruefsumme,
    Speicher,
    Abgebrochen,
    Richtlinie,
    Firmenrichtlinie { meldung: String },
    Pfad,
    Ungueltig,
    Beschaeftigt,
    Anmelden,
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct InstallFortschritt {
    pub kind: String,
    pub phase: String,
    pub received: u64,
    pub total: Option<u64>,
    pub error: Option<InstallFehler>,
}

pub fn io_fehler(error: std::io::Error) -> InstallFehler {
    match error.raw_os_error() {
        Some(28 | 39 | 112) => InstallFehler::Speicher,
        Some(5 | 32 | 33 | 225 | 226 | 1260) => InstallFehler::Blockiert,
        _ if error.kind() == std::io::ErrorKind::PermissionDenied => InstallFehler::Blockiert,
        _ => InstallFehler::Pfad,
    }
}

#[derive(Default)]
pub struct Abbruch {
    cancelled: AtomicBool,
    notify: tokio::sync::Notify,
    login_started: Mutex<bool>,
}

impl Abbruch {
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        let Ok(started) = self.login_started.lock() else { return; };
        if *started { return; }
        self.cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }

    /// Abbruch und Prozessstart sind gegenseitig ausgeschlossen; danach ist Abbruch beendet.
    pub(super) fn start_login(&self, start: impl FnOnce() -> Result<(), InstallFehler>) -> Result<(), InstallFehler> {
        let mut started = self.login_started.lock().map_err(|_| InstallFehler::Blockiert)?;
        self.check()?;
        *started = true;
        start()
    }

    fn check(&self) -> Result<(), InstallFehler> {
        if self.cancelled.load(Ordering::SeqCst) {
            Err(InstallFehler::Abgebrochen)
        } else {
            Ok(())
        }
    }

    async fn warten(&self) {
        if self.check().is_err() { return; }
        self.notify.notified().await;
    }
}

type Registry = Mutex<Vec<(Cli, Arc<Abbruch>)>>;
static AUFTRAEGE: Registry = Mutex::new(Vec::new());

/// Bleibt bis nach Anmeldung aktiv; Drop gibt den Anbieter auch bei Fehler/Abbruch frei.
pub struct Auftrag<'a> {
    registry: &'a Registry,
    pub cancel: Arc<Abbruch>,
}

impl Auftrag<'static> {
    pub fn begin(cli: Cli) -> Result<Self, InstallFehler> {
        Self::begin_in(cli, &AUFTRAEGE)
    }
}

impl<'a> Auftrag<'a> {
    fn begin_in(cli: Cli, registry: &'a Registry) -> Result<Self, InstallFehler> {
        let mut active = registry.lock().map_err(|_| InstallFehler::Blockiert)?;
        if active.iter().any(|(c, _)| *c == cli) { return Err(InstallFehler::Beschaeftigt); }
        let cancel = Arc::new(Abbruch::default());
        active.push((cli, cancel.clone()));
        Ok(Self { registry, cancel })
    }
}

impl Drop for Auftrag<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.registry.lock() {
            active.retain(|(_, token)| !Arc::ptr_eq(token, &self.cancel));
        }
    }
}

pub fn abbrechen(cli: Cli) {
    if let Ok(active) = AUFTRAEGE.lock() {
        if let Some((_, cancel)) = active.iter().find(|(c, _)| *c == cli) { cancel.cancel(); }
    }
}

pub fn aktiv(cli: Cli) -> bool {
    AUFTRAEGE.lock().map(|a| a.iter().any(|(c, _)| *c == cli)).unwrap_or(true)
}

fn netzfehler(url: &str) -> InstallFehler {
    InstallFehler::NetzGesperrt {
        ziel: url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default(),
    }
}

fn erlaubte_url(cli: Cli, url: &str) -> Result<url::Url, InstallFehler> {
    let parsed = url::Url::parse(url).map_err(|_| InstallFehler::Ungueltig)?;
    let hosts = match cli { Cli::Claude => HOSTS_CLAUDE, Cli::Codex => HOSTS_CODEX };
    if parsed.scheme() != "https" || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty() || parsed.password().is_some()
        || !hosts.contains(&parsed.host_str().unwrap_or(""))
    {
        return Err(netzfehler(url));
    }
    Ok(parsed)
}

struct Antwort {
    total: Option<u64>,
    chunks: Pin<Box<dyn Stream<Item = Result<Vec<u8>, InstallFehler>> + Send>>,
}

#[async_trait::async_trait]
trait Quelle: Sync {
    async fn get(&self, url: &str) -> Result<Antwort, InstallFehler>;
}

struct Http { cli: Cli }

#[async_trait::async_trait]
impl Quelle for Http {
    async fn get(&self, url: &str) -> Result<Antwort, InstallFehler> {
        http_get(self.cli, url, |current| async move {
            let builder = crate::netz::proxy::client_fuer(current.as_str()).await;
            let client = builder.redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(180))
                .user_agent("Local-Voice-CLI-Setup")
                .build().map_err(|_| netzfehler(current.as_str()))?;
            client.get(current.clone()).send().await.map_err(|_| netzfehler(current.as_str()))
        }).await
    }
}

async fn http_get<F, Fut>(cli: Cli, url: &str, mut send: F) -> Result<Antwort, InstallFehler>
where
    F: FnMut(url::Url) -> Fut,
    Fut: std::future::Future<Output = Result<reqwest::Response, InstallFehler>>,
{
    let mut current = erlaubte_url(cli, url)?;
    // Jede Weiterleitung vor der Anfrage pruefen UND den Proxy fuer das neue Ziel aufloesen.
    for _ in 0..=5 {
        let response = send(current.clone()).await?;
        if response.status().is_redirection() {
            let location = response.headers().get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok()).ok_or_else(|| netzfehler(current.as_str()))?;
            let next = current.join(location).map_err(|_| netzfehler(current.as_str()))?;
            current = erlaubte_url(cli, next.as_str())?;
            continue;
        }
        if !response.status().is_success() { return Err(netzfehler(current.as_str())); }
        let total = response.content_length();
        let failure = netzfehler(current.as_str());
        return Ok(Antwort {
            total,
            chunks: Box::pin(response.bytes_stream().map(move |part|
                part.map(|b| b.to_vec()).map_err(|_| failure.clone()))),
        });
    }
    Err(netzfehler(current.as_str()))
}

fn plattform(arch: &str) -> Result<(&'static str, &'static str), InstallFehler> {
    match arch {
        "x86_64" => Ok(("win32-x64", "x86_64-pc-windows-msvc")),
        "aarch64" => Ok(("win32-arm64", "aarch64-pc-windows-msvc")),
        _ => Err(InstallFehler::Ungueltig),
    }
}

struct Asset { url: String, checksum: String, member: Option<String> }

fn checksum(text: &str) -> Result<String, InstallFehler> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(InstallFehler::Pruefsumme);
    }
    Ok(text.to_ascii_lowercase())
}

fn version_pruefen(version: &str) -> Result<(), InstallFehler> {
    if version.is_empty() || version.len() > 64 || !version.bytes().all(|b|
        b.is_ascii_alphanumeric() || b".-".contains(&b)) || version.contains("..")
    { return Err(InstallFehler::Ungueltig); }
    Ok(())
}

fn claude_asset(version: &str, manifest: &serde_json::Value, arch: &str) -> Result<Asset, InstallFehler> {
    let (platform, _) = plattform(arch)?;
    version_pruefen(version)?;
    let hash = manifest["platforms"][platform]["checksum"].as_str().ok_or(InstallFehler::Pruefsumme)?;
    Ok(Asset { url: format!("{CLAUDE_BASE}/{version}/{platform}/claude.exe"),
        checksum: checksum(hash)?, member: None })
}

fn codex_asset(release: &serde_json::Value, arch: &str) -> Result<Asset, InstallFehler> {
    let (_, triple) = plattform(arch)?;
    let name = format!("codex-{triple}.zip");
    let assets = release["assets"].as_array().ok_or(InstallFehler::Ungueltig)?;
    let mut matches = assets.iter().filter(|a| a["name"].as_str() == Some(name.as_str()));
    let asset = matches.next().ok_or(InstallFehler::Ungueltig)?;
    if matches.next().is_some() { return Err(InstallFehler::Ungueltig); }
    let url = asset["browser_download_url"].as_str().ok_or(InstallFehler::Ungueltig)?;
    let parsed = erlaubte_url(Cli::Codex, url)?;
    if parsed.host_str() != Some("github.com") || !parsed.path().starts_with("/openai/codex/releases/download/") {
        return Err(InstallFehler::Ungueltig);
    }
    let hash = asset["digest"].as_str().and_then(|s| s.strip_prefix("sha256:"))
        .ok_or(InstallFehler::Pruefsumme)?;
    Ok(Asset { url: url.to_string(), checksum: checksum(hash)?,
        member: Some(format!("codex-{triple}.exe")) })
}

async fn metadata(source: &impl Quelle, url: &str) -> Result<Vec<u8>, InstallFehler> {
    let read = async {
        let mut response = source.get(url).await?;
        if response.total.is_some_and(|n| n > MAX_META) { return Err(InstallFehler::Ungueltig); }
        let mut bytes = Vec::new();
        while let Some(part) = response.chunks.next().await {
            let part = part?;
            if bytes.len() as u64 + part.len() as u64 > MAX_META { return Err(InstallFehler::Ungueltig); }
            bytes.extend(part);
        }
        Ok(bytes)
    };
    tokio::time::timeout(Duration::from_secs(30), read).await.map_err(|_| netzfehler(url))?
}

async fn laden(source: &impl Quelle, url: &str, temp: &mut NamedTempFile, limit: u64,
    cancel: &Abbruch, progress: &impl Fn(u64, Option<u64>)) -> Result<String, InstallFehler>
{
    cancel.check()?;
    let mut response = tokio::select! {
        biased;
        _ = cancel.warten() => return Err(InstallFehler::Abgebrochen),
        response = source.get(url) => response?,
    };
    if response.total.is_some_and(|n| n > limit) { return Err(InstallFehler::Ungueltig); }
    let mut count = 0;
    let mut hash = Sha256::new();
    loop {
        cancel.check()?;
        let part = tokio::select! {
            biased;
            _ = cancel.warten() => return Err(InstallFehler::Abgebrochen),
            part = response.chunks.next() => part,
        };
        let Some(part) = part else { break; };
        let part = part?;
        count += part.len() as u64;
        if count > limit { return Err(InstallFehler::Ungueltig); }
        temp.write_all(&part).map_err(io_fehler)?;
        hash.update(&part);
        progress(count, response.total);
    }
    cancel.check()?;
    if response.total.is_some_and(|n| n != count) { return Err(netzfehler(url)); }
    Ok(format!("{:x}", hash.finalize()))
}

fn entpacken(archive: &mut std::fs::File, exe: &mut NamedTempFile, member: &str,
    cancel: &Abbruch) -> Result<String, InstallFehler>
{
    archive.seek(SeekFrom::Start(0)).map_err(io_fehler)?;
    let mut zip = zip::ZipArchive::new(archive).map_err(|_| InstallFehler::Ungueltig)?;
    // Nie einen Archivpfad ans Dateisystem reichen, nur genau diesen einen Eintrag lesen.
    if zip.file_names().filter(|n| *n == member).count() != 1 { return Err(InstallFehler::Ungueltig); }
    let mut entry = zip.by_name(member).map_err(|_| InstallFehler::Ungueltig)?;
    if !entry.is_file() || entry.is_symlink() || entry.size() > MAX_BINARY { return Err(InstallFehler::Ungueltig); }
    let mut buffer = [0; 64 * 1024];
    let mut count = 0;
    let mut hash = Sha256::new();
    loop {
        cancel.check()?;
        let n = entry.read(&mut buffer).map_err(|_| InstallFehler::Ungueltig)?;
        if n == 0 { break; }
        count += n as u64;
        if count > MAX_BINARY { return Err(InstallFehler::Ungueltig); }
        exe.write_all(&buffer[..n]).map_err(io_fehler)?;
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Haelt die endgueltige Datei und auf Windows ihre Vorfahren bis nach dem Prozessstart fest.
#[derive(Debug)]
pub struct Geprueft {
    _handles: Vec<std::fs::File>,
}

fn endpruefung(target: &Path, expected: &str) -> Result<Geprueft, InstallFehler> {
    let check = || {
        lokaler_pfad(target)?;
        let mut handles = Vec::new();
        // Von der Wurzel her sperren: ein gepruefter Vorfahr darf danach nicht umbenannt werden.
        for path in target.ancestors().collect::<Vec<_>>().into_iter().rev() {
            #[cfg(windows)]
            {
                use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
                let file = std::fs::OpenOptions::new().read(true)
                    .share_mode(1) // FILE_SHARE_READ, kein Schreiben oder Loeschen
                    .custom_flags(0x0220_0000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
                    .open(path).map_err(io_fehler)?;
                let meta = file.metadata().map_err(io_fehler)?;
                if meta.file_attributes() & 0x400 != 0
                    || (if path == target { !meta.is_file() } else { !meta.is_dir() }) {
                    return Err(InstallFehler::Pfad);
                }
                handles.push(file);
            }
            #[cfg(not(windows))]
            if path == target {
                sichere_ablage(target)?;
                handles.push(std::fs::File::open(target).map_err(io_fehler)?);
            }
        }
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        let file = handles.last_mut().ok_or(InstallFehler::Pfad)?;
        let mut count = 0u64;
        loop {
            let n = file.read(&mut buffer).map_err(io_fehler)?;
            if n == 0 { break; }
            count += n as u64;
            if count > MAX_BINARY { return Err(InstallFehler::Pruefsumme); }
            hash.update(&buffer[..n]);
        }
        if format!("{:x}", hash.finalize()) != expected { return Err(InstallFehler::Pruefsumme); }
        Ok(Geprueft { _handles: handles })
    };
    // Bei Fehler ist der Pfad nicht vertrauenswuerdig und kann auf eine fremde Datei zeigen.
    // Ohne nachweislich eigenes Handle nichts bereinigen; keine EXE zur Anmeldung freigeben.
    check().map_err(|_| InstallFehler::Pruefsumme)
}

fn kontrollieren(cancel: &Abbruch, disabled: &impl Fn() -> bool) -> Result<(), InstallFehler> {
    cancel.check()?;
    if disabled() { return Err(InstallFehler::Richtlinie); }
    Ok(())
}

async fn provision(cli: Cli, arch: &str, target: &Path, source: &impl Quelle,
    cancel: &Abbruch, disabled: &impl Fn() -> bool,
    progress: &impl Fn(&str, u64, Option<u64>)) -> Result<Geprueft, InstallFehler>
{
    provision_inner(cli, arch, target, source, cancel, disabled, progress, &|_| {}).await
}

async fn provision_inner(cli: Cli, arch: &str, target: &Path, source: &impl Quelle,
    cancel: &Abbruch, disabled: &impl Fn() -> bool,
    progress: &impl Fn(&str, u64, Option<u64>),
    after_publish: &impl Fn(&Path)) -> Result<Geprueft, InstallFehler>
{
    kontrollieren(cancel, disabled)?;
    progress("pruefen", 0, None);
    plattform(arch)?;
    let asset = match cli {
        Cli::Claude => {
            let version = metadata(source, &format!("{CLAUDE_BASE}/latest")).await?;
            let version = std::str::from_utf8(&version).map_err(|_| InstallFehler::Ungueltig)?.trim();
            // Version pruefen, bevor sie in eine Adresse eingeht.
            version_pruefen(version)?;
            let manifest = metadata(source, &format!("{CLAUDE_BASE}/{version}/manifest.json")).await?;
            let manifest = serde_json::from_slice(&manifest).map_err(|_| InstallFehler::Ungueltig)?;
            claude_asset(version, &manifest, arch)?
        }
        Cli::Codex => {
            let release = metadata(source, CODEX_RELEASE).await?;
            codex_asset(&serde_json::from_slice(&release).map_err(|_| InstallFehler::Ungueltig)?, arch)?
        }
    };
    kontrollieren(cancel, disabled)?;
    let parent = target.parent().ok_or(InstallFehler::Pfad)?;
    sichere_ablage(target)?;
    std::fs::create_dir_all(parent).map_err(io_fehler)?;
    sichere_ablage(target)?;
    let mut temp = NamedTempFile::new_in(parent).map_err(io_fehler)?;
    progress("laden", 0, None);
    let actual = laden(source, &asset.url, &mut temp, MAX_BINARY, cancel,
        &|n, total| progress("laden", n, total)).await?;
    progress("pruefsumme", 0, None);
    if actual != asset.checksum { return Err(InstallFehler::Pruefsumme); }
    let mut expected_exe = asset.checksum;
    if let Some(member) = asset.member {
        // Entpacken aus genau dem geprueften, gegen Schreiben gesperrten Archivhandle.
        let archive_path = temp.into_temp_path();
        let mut archive = endpruefung(&archive_path, &expected_exe)?;
        let mut exe = NamedTempFile::new_in(parent).map_err(io_fehler)?;
        expected_exe = entpacken(archive._handles.last_mut().ok_or(InstallFehler::Pfad)?, &mut exe, &member, cancel)?;
        temp = exe;
    }
    temp.seek(SeekFrom::Start(0)).map_err(io_fehler)?;
    let mut head = [0; 2];
    temp.read_exact(&mut head).map_err(|_| InstallFehler::Ungueltig)?;
    if &head != b"MZ" { return Err(InstallFehler::Ungueltig); }
    temp.as_file().sync_all().map_err(io_fehler)?;
    kontrollieren(cancel, disabled)?;
    progress("ablegen", 0, None);
    kontrollieren(cancel, disabled)?;
    sichere_ablage(target)?;
    // Kein vorheriges Loeschen: ein gesperrtes/laufendes Ziel bleibt bei Rename-Fehler erhalten.
    // Tempdatei liegt im Zielordner, persist ersetzt atomar auf demselben Volume.
    drop(temp.persist(target).map_err(|e| io_fehler(e.error))?);
    after_publish(target);
    endpruefung(target, &expected_exe)
}

pub fn ziel(cli: Cli) -> Result<PathBuf, InstallFehler> {
    if !cfg!(windows) { return Err(InstallFehler::Ungueltig); }
    let path = match cli {
        Cli::Claude => std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".local/bin/claude.exe")),
        Cli::Codex => std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Programs/codex/codex.exe")),
    }.ok_or(InstallFehler::Pfad)?;
    lokaler_pfad(&path)?;
    Ok(path)
}

pub async fn download(cli: Cli, target: &Path, cancel: &Abbruch,
    disabled: impl Fn() -> bool, progress: impl Fn(&str, u64, Option<u64>)) -> Result<Geprueft, InstallFehler>
{
    if !cfg!(windows) { return Err(InstallFehler::Ungueltig); }
    crate::richtlinie::programm_download_pruefen(target.exists())
        .map_err(|meldung| InstallFehler::Firmenrichtlinie { meldung })?;
    let source = Http { cli };
    let operation = provision(cli, std::env::consts::ARCH, target, &source, cancel, &disabled, &progress);
    tokio::select! {
        biased;
        _ = cancel.warten() => Err(InstallFehler::Abgebrochen),
        result = tokio::time::timeout(GESAMTFRIST, operation) =>
            result.map_err(|_| netzfehler(match cli {
                Cli::Claude => CLAUDE_BASE, Cli::Codex => CODEX_RELEASE,
            }))?,
    }
}

/// Explizit gewaehlt: nur lokale regulaere Programmdateien, niemals UNC/Geraetepfade.
pub fn valid_path(cli: Cli, path: &Path) -> Result<(), InstallFehler> {
    sichere_ablage(path)?;
    let name = match cli { Cli::Claude => "claude.exe", Cli::Codex => "codex.exe" };
    if !path.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
    { return Err(InstallFehler::Pfad); }
    let meta = std::fs::symlink_metadata(path).map_err(io_fehler)?;
    if !meta.is_file() || meta.file_type().is_symlink() { return Err(InstallFehler::Pfad); }
    let mut file = std::fs::File::open(path).map_err(io_fehler)?;
    let mut head = [0; 2];
    file.read_exact(&mut head).map_err(|_| InstallFehler::Pfad)?;
    if &head != b"MZ" { return Err(InstallFehler::Pfad); }
    Ok(())
}

fn lokaler_pfad(path: &Path) -> Result<(), InstallFehler> {
    let text = path.to_string_lossy();
    if !path.is_absolute() || text.starts_with("\\\\") || text.starts_with("//") {
        return Err(InstallFehler::Pfad);
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        if !matches!(path.components().next(), Some(std::path::Component::Prefix(p))
            if matches!(p.kind(), std::path::Prefix::Disk(_))) {
            return Err(InstallFehler::Pfad);
        }
        // Auch gemappte Netzlaufwerke ablehnen, bevor die Datei geoeffnet wird.
        let root = path.ancestors().last().ok_or(InstallFehler::Pfad)?;
        let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        let drive = unsafe { windows::Win32::Storage::FileSystem::GetDriveTypeW(windows::core::PCWSTR(wide.as_ptr())) };
        if !matches!(drive, 2 | 3 | 5 | 6) {
            return Err(InstallFehler::Pfad);
        }
    }
    Ok(())
}

pub(super) fn sichere_ablage(target: &Path) -> Result<(), InstallFehler> {
    lokaler_pfad(target)?;
    // Vorfahren zuerst: keine Datei durch eine Junction zu einem Netzpfad befragen.
    for path in target.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match std::fs::symlink_metadata(path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() { return Err(InstallFehler::Pfad); }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 { return Err(InstallFehler::Pfad); }
                }
                if path == target && !meta.is_file() || path != target && !meta.is_dir() {
                    return Err(InstallFehler::Pfad);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_fehler(e)),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "cli_install_tests.rs"]
mod tests;
