//! Datei in OneDrive ablegen (A5, Scope `Files.ReadWrite` bzw.
//! `Files.ReadWrite.AppFolder`).
//!
//! - **Klein** (bis `SIMPLE_MAX`, 4 MiB): ein `PUT /me/drive/root:/<Pfad>:/content`.
//! - **Gross**: Upload-Sitzung (`createUploadSession`), die Datei geht in Stuecken zu
//!   je `CHUNK` (ein Vielfaches von 320 KiB, wie Graph es verlangt) an die
//!   `uploadUrl`. Es liegt nie mehr als ein Stueck im Arbeitsspeicher; die Datei
//!   wird stueckweise von der Platte gelesen. Die `uploadUrl` ist vorab beglaubigt:
//!   dorthin geht KEIN Zugriffstoken.
//! - Ein Stueck, das scheitert (Netz, 5xx, 416), wird nach Abfrage des Stands der
//!   Sitzung (`nextExpectedRanges`) bis zu `MAX_CHUNK_RETRIES`-mal wiederholt; danach
//!   wird die Sitzung abgebrochen (`DELETE uploadUrl`), damit nichts Halbes
//!   liegenbleibt. Graph legt die Datei erst nach dem letzten Stueck an: ein
//!   Abbruch (auch ein Absturz der App) hinterlaesst keine Teildatei, die Sitzung
//!   verfaellt serverseitig.
//! - Standard bei einem vorhandenen Namen: `rename` (nie stillschweigend
//!   ueberschreiben); `replace` und `fail` nur auf ausdruecklichen Wunsch.
//! - Pfade werden VOR dem Netz geprueft: keine `..`, keine Laufwerks- oder
//!   Sonderzeichen, keine reservierten Namen; jeder Abschnitt wird kodiert.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

use super::config::{FilesMode, M365Config};
use super::error::M365Error;
use super::service::{error_of, Acct, M365Service, Reply, TRANSFER_TIMEOUT};
use crate::managers::integrations::model::Capability;

/// Bis zu dieser Groesse genuegt ein einzelner PUT.
pub const SIMPLE_MAX: u64 = 4 * 1024 * 1024;
/// Stueckgroesse der Upload-Sitzung: 10 x 320 KiB.
pub const CHUNK: u64 = 10 * 320 * 1024;
/// Groesste Datei, die die App hochlaedt.
pub const MAX_UPLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_CHUNK_RETRIES: u32 = 3;
/// Arbeitsspeicher, den eine Upload-Sitzung braucht (ein Stueck, Puffer, Verbindungen).
pub const UPLOAD_NEED_MB: u64 = 64;
const MAX_PATH_CHARS: usize = 400;
const MAX_NAME_CHARS: usize = 255;

/// Was bei einem vorhandenen Namen geschieht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    Rename,
    Replace,
    Fail,
}

impl Conflict {
    pub fn as_str(self) -> &'static str {
        match self {
            Conflict::Rename => "rename",
            Conflict::Replace => "replace",
            Conflict::Fail => "fail",
        }
    }
}

#[derive(Clone, Debug)]
pub enum UploadSource {
    Bytes(Vec<u8>),
    File(PathBuf),
}

#[derive(Clone, Debug)]
pub struct UploadRequest {
    /// Dateiname in OneDrive.
    pub name: String,
    /// Weiterer Unterordner unter dem eingestellten Ordner; leer = direkt dort.
    pub subfolder: String,
    pub conflict: Conflict,
    pub source: UploadSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadVia {
    Simple,
    Session,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uploaded {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub web_url: Option<String>,
    pub via: UploadVia,
}

fn invalid<T>(msg: impl Into<String>) -> Result<T, M365Error> {
    Err(M365Error::Invalid(msg.into()))
}

const RESERVED: [&str; 24] = [
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Ein Name (Datei oder Ordner) in OneDrive: nichts, was Windows oder OneDrive
/// anders deuten wuerden.
pub fn clean_segment(raw: &str) -> Result<String, M365Error> {
    let name = raw.trim_end_matches([' ', '.']).trim_start();
    if name.is_empty() {
        return invalid("Ein Datei- oder Ordnername ist leer oder besteht nur aus Punkten.");
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return invalid(format!(
            "Ein Datei- oder Ordnername ist zu lang (höchstens {MAX_NAME_CHARS} Zeichen)."
        ));
    }
    if name
        .chars()
        .any(|c| c.is_control() || "\"*:<>?/\\|".contains(c))
    {
        return invalid(format!(
            "Der Name „{name}“ enthält Zeichen, die OneDrive nicht erlaubt (\" * : < > ? / \\ |)."
        ));
    }
    let lower = name.to_lowercase();
    let stem = lower.split('.').next().unwrap_or("");
    if RESERVED.contains(&stem)
        || lower == "desktop.ini"
        || lower == ".lock"
        || lower.starts_with("~$")
        || lower.contains("_vti_")
    {
        return invalid(format!("Der Name „{name}“ ist in OneDrive reserviert."));
    }
    Ok(name.to_string())
}

/// Ein relativer Ordnerpfad: Abschnitte mit `/` (auch `\` wird angenommen),
/// ohne fuehrenden oder abschliessenden Trenner; leer = Wurzel. `..` ist ungueltig
/// (endet auf einen Punkt, siehe `clean_segment`).
pub fn clean_folder(raw: &str) -> Result<String, M365Error> {
    let normalized = raw.replace('\\', "/");
    let segments: Vec<String> = normalized
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(clean_segment)
        .collect::<Result<_, _>>()?;
    let joined = segments.join("/");
    if joined.chars().count() > MAX_PATH_CHARS {
        return invalid("Der Ordnerpfad ist zu lang.");
    }
    Ok(joined)
}

/// Kodiert einen Pfadabschnitt (nur `A-Za-z0-9-._~` bleiben stehen).
pub fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Der Pfad der Datei in OneDrive als Abschnitte: Konfigurationsordner,
/// Unterordner, Name.
fn remote_segments(
    cfg: &M365Config,
    subfolder: &str,
    name: &str,
) -> Result<Vec<String>, M365Error> {
    let name = clean_segment(name)?;
    let sub = clean_folder(subfolder)?;
    let mut segments: Vec<String> = Vec::new();
    for part in [cfg.files_folder.as_str(), sub.as_str()] {
        segments.extend(
            part.split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
    }
    segments.push(name);
    let total: usize = segments.iter().map(|s| s.chars().count() + 1).sum();
    if total > MAX_PATH_CHARS {
        return invalid("Der Pfad in OneDrive ist zu lang.");
    }
    Ok(segments)
}

/// Adressteil `root:/a/b:` bzw. `special/approot:/a/b:` (kodiert).
fn item_path(cfg: &M365Config, segments: &[String]) -> String {
    let base = match cfg.files_mode {
        FilesMode::Full => "root",
        FilesMode::AppFolder => "special/approot",
    };
    let path: Vec<String> = segments.iter().map(|s| encode_segment(s)).collect();
    format!("{base}:/{}:", path.join("/"))
}

impl UploadRequest {
    /// Anzeigepfad fuer Audit und Freigabe, z. B. `OneDrive:/Local Voice AI/x.txt`.
    pub fn display_path(&self, cfg: &M365Config) -> Result<String, M365Error> {
        let segments = remote_segments(cfg, &self.subfolder, &self.name)?;
        let root = match cfg.files_mode {
            FilesMode::Full => "OneDrive:",
            FilesMode::AppFolder => "OneDrive (App-Ordner):",
        };
        Ok(format!("{root}/{}", segments.join("/")))
    }

    fn size(&self) -> Result<u64, M365Error> {
        match &self.source {
            UploadSource::Bytes(b) => Ok(b.len() as u64),
            UploadSource::File(p) => std::fs::metadata(p)
                .map_err(|e| M365Error::Invalid(format!("Die Datei ist nicht lesbar: {e}")))
                .and_then(|m| {
                    if m.is_file() {
                        Ok(m.len())
                    } else {
                        invalid("Das ist keine Datei.")
                    }
                }),
        }
    }

    /// Argumente fuer Freigabe-Vorschau und Bindung.
    pub fn gate_args(&self, cfg: &M365Config) -> Result<Value, M365Error> {
        let source = match &self.source {
            UploadSource::File(p) => p.display().to_string(),
            UploadSource::Bytes(_) => "(Inhalt aus der App)".to_string(),
        };
        Ok(json!({
            "path": self.display_path(cfg)?,
            "source": source,
            "bytes": self.size()?,
            "conflict": self.conflict.as_str(),
        }))
    }
}

/// Liest ein Stueck der Quelle (Datei: von der Platte, ohne den Rest zu laden).
async fn read_chunk(source: &UploadSource, offset: u64, len: u64) -> Result<Vec<u8>, M365Error> {
    match source {
        UploadSource::Bytes(b) => {
            let start = offset as usize;
            let end = start + len as usize;
            b.get(start..end)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| M365Error::Invalid("Der Inhalt ist kürzer als angekündigt.".into()))
        }
        UploadSource::File(path) => {
            let path = path.clone();
            tokio::task::spawn_blocking(move || {
                use std::io::{Read, Seek, SeekFrom};
                let mut f = std::fs::File::open(&path)?;
                f.seek(SeekFrom::Start(offset))?;
                let mut buf = vec![0u8; len as usize];
                f.read_exact(&mut buf)?;
                Ok::<_, std::io::Error>(buf)
            })
            .await
            .map_err(|e| M365Error::Store(e.to_string()))?
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    M365Error::Invalid("Die Datei hat sich während des Hochladens geändert.".into())
                } else {
                    M365Error::Invalid(format!("Die Datei ist nicht lesbar: {e}"))
                }
            })
        }
    }
}

fn uploaded_from(reply: &Reply, via: UploadVia) -> Result<Uploaded, M365Error> {
    let v = reply.json()?;
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| M365Error::Parse("Antwort ohne Kennung der Datei".to_string()))?;
    Ok(Uploaded {
        id: id.to_string(),
        name: v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        size: v.get("size").and_then(Value::as_u64).unwrap_or_default(),
        web_url: v.get("webUrl").and_then(Value::as_str).map(str::to_string),
        via,
    })
}

/// Der naechste erwartete Anfang aus `nextExpectedRanges` (`"26-"` oder `"26-99"`).
fn next_expected_start(v: &Value) -> Option<u64> {
    v.get("nextExpectedRanges")?
        .as_array()?
        .first()?
        .as_str()?
        .split('-')
        .next()?
        .parse()
        .ok()
}

impl M365Service {
    /// Legt die Datei in OneDrive ab (siehe Moduldoku). `cancel`: wird zwischen den
    /// Stuecken geprueft; dann endet die Sitzung mit `Cancelled`.
    pub async fn upload(
        &self,
        a: &Acct,
        req: &UploadRequest,
        cancel: Option<&AtomicBool>,
    ) -> Result<Uploaded, M365Error> {
        a.require(Capability::FilesWrite)?;
        let segments = remote_segments(&a.cfg, &req.subfolder, &req.name)?;
        let name = segments.last().cloned().unwrap_or_default();
        let total = req.size()?;
        if total > MAX_UPLOAD_BYTES {
            return invalid("Die Datei ist zu groß (höchstens 2 GB).");
        }
        let item = item_path(&a.cfg, &segments);
        if total <= SIMPLE_MAX {
            self.upload_simple(a, req, &item, total).await
        } else {
            // Ein Stueck plus Puffer: bei knappem Speicher lieber melden als anfangen.
            crate::process_guard::check_ram_for_start(UPLOAD_NEED_MB)
                .map_err(M365Error::MemoryLow)?;
            self.upload_session(a, req, &item, &name, total, cancel)
                .await
        }
    }

    async fn upload_simple(
        &self,
        a: &Acct,
        req: &UploadRequest,
        item: &str,
        total: u64,
    ) -> Result<Uploaded, M365Error> {
        let bytes = read_chunk(&req.source, 0, total).await?;
        let url = format!(
            "{}/me/drive/{item}/content?@microsoft.graph.conflictBehavior={}",
            self.graph_base(),
            req.conflict.as_str()
        );
        let reply = self
            .send_authed(a, true, |c, token| {
                c.put(&url)
                    .bearer_auth(token)
                    .timeout(TRANSFER_TIMEOUT)
                    .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                    .body(bytes.clone())
            })
            .await?;
        if reply.is_success() {
            uploaded_from(&reply, UploadVia::Simple)
        } else {
            Err(error_of(&reply, "Upload"))
        }
    }

    async fn upload_session(
        &self,
        a: &Acct,
        req: &UploadRequest,
        item: &str,
        name: &str,
        total: u64,
        cancel: Option<&AtomicBool>,
    ) -> Result<Uploaded, M365Error> {
        let url = format!("{}/me/drive/{item}/createUploadSession", self.graph_base());
        let body = json!({ "item": {
            "@microsoft.graph.conflictBehavior": req.conflict.as_str(),
            "name": name,
        }});
        let reply = self
            .send_authed(a, true, |c, token| {
                c.post(&url).bearer_auth(token).json(&body)
            })
            .await?;
        if !reply.is_success() {
            return Err(error_of(&reply, "Upload-Sitzung"));
        }
        let upload_url = self.checked_upload_url(&reply.json()?)?;
        let result = self
            .push_chunks(&upload_url, &req.source, total, cancel)
            .await;
        if let Err(e) = &result {
            // Die Sitzung ist weg, wenn sie abgelaufen ist; sonst aufraeumen (best effort).
            if !matches!(e, M365Error::NotFound(_)) {
                let _ = self.send_plain(true, |c| c.delete(&upload_url)).await;
            }
        }
        result
    }

    /// Die `uploadUrl` muss `https` sein (in Tests gegen einen lokalen Server: `http`
    /// nur, wenn auch die Graph-Adresse `http` ist) und darf keine Zugangsdaten tragen.
    pub(super) fn checked_upload_url(&self, v: &Value) -> Result<String, M365Error> {
        let raw = v
            .get("uploadUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| M365Error::Parse("Antwort ohne uploadUrl".to_string()))?;
        let url = url::Url::parse(raw)
            .map_err(|_| M365Error::Parse("ungültige uploadUrl".to_string()))?;
        let test_mode = self.ep.graph.starts_with("http://");
        let scheme_ok = url.scheme() == "https" || (test_mode && url.scheme() == "http");
        if !scheme_ok || url.host_str().is_none() || !url.username().is_empty() {
            return Err(M365Error::Parse("unzulässige uploadUrl".to_string()));
        }
        Ok(raw.to_string())
    }

    async fn push_chunks(
        &self,
        upload_url: &str,
        source: &UploadSource,
        total: u64,
        cancel: Option<&AtomicBool>,
    ) -> Result<Uploaded, M365Error> {
        let mut offset: u64 = 0;
        let mut retries: u32 = 0;
        loop {
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(M365Error::Cancelled);
            }
            let len = CHUNK.min(total - offset);
            let end = offset + len - 1;
            let chunk = read_chunk(source, offset, len).await?;
            let range = format!("bytes {offset}-{end}/{total}");
            let reply = self
                .send_plain(true, |c| {
                    c.put(upload_url)
                        .timeout(TRANSFER_TIMEOUT)
                        .header("Content-Range", range.clone())
                        .body(chunk)
                })
                .await;
            let failure = match reply {
                Ok(r) if r.status == 202 => {
                    retries = 0;
                    let next = next_expected_start(&r.json().unwrap_or(Value::Null));
                    offset = match next {
                        Some(n) if n <= total => n,
                        Some(_) => return Err(M365Error::Parse("unerwarteter Stand".into())),
                        None => offset + len,
                    };
                    continue;
                }
                Ok(r) if r.status == 200 || r.status == 201 => {
                    if end + 1 != total {
                        return Err(M365Error::Parse("Sitzung vorzeitig beendet".to_string()));
                    }
                    return uploaded_from(&r, UploadVia::Session);
                }
                Ok(r) if r.status == 404 || r.status == 410 => {
                    return Err(M365Error::NotFound(
                        "Die Upload-Sitzung ist abgelaufen".to_string(),
                    ))
                }
                Ok(r) if r.status == 416 || (r.status >= 500 && r.status != 507) => {
                    error_of(&r, "Upload")
                }
                Ok(r) => return Err(error_of(&r, "Upload")),
                Err(e @ (M365Error::Network(_) | M365Error::Timeout)) => e,
                Err(e) => return Err(e),
            };
            retries += 1;
            if retries > MAX_CHUNK_RETRIES {
                return Err(failure);
            }
            tokio::time::sleep(self.retry_pause * retries).await;
            // Wo steht die Sitzung wirklich? Ein Stueck kann angekommen sein, obwohl die
            // Antwort fehlte.
            if let Ok(status) = self.send_plain(true, |c| c.get(upload_url)).await {
                if status.is_success() {
                    if let Some(n) = next_expected_start(&status.json().unwrap_or(Value::Null)) {
                        if n <= total {
                            offset = n;
                        }
                    }
                }
            }
        }
    }
}

/// Pause zwischen zwei Versuchen eines Stuecks (mal Versuchsnummer).
pub const DEFAULT_RETRY_PAUSE: Duration = Duration::from_millis(800);
