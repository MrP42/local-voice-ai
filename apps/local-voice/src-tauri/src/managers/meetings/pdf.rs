//! M6-P6b: PDF aus der HTML-Ansicht einer Besprechung über ein verstecktes
//! WebView2-Fenster (`NavigateToString` → `ICoreWebView2_7::PrintToPdf`).
//!
//! Warum ein eigener Thread mit eigener Nachrichtenschleife und nicht das
//! Tauri-Fenster: WebView2 verlangt einen Thread mit Einzelthread-Apartment
//! (STA) und laufender Nachrichtenschleife, weil es seine Rückrufe per
//! `PostMessage` zustellt. Der Hauptthread gehört Tauri, ein
//! `spawn_blocking`-Thread hat keine Schleife, und im Test (`cargo test`) und
//! im Headless-Lauf (`--export-meeting`) gibt es gar keine Tauri-Fenster.
//! Ein kurzlebiger Thread je Export ist deshalb der einzige Weg, der überall
//! gleich funktioniert. Der Aufrufer wartet nur auf ein Ergebnis mit
//! Zeitgrenze und ist nie an die Schleife gebunden.
//!
//! Sicherheit und Aufräumen:
//! - Skripte sind in der Seite abgeschaltet: Protokolltext stammt aus
//!   Transkripten und Kalendereinträgen und ist nie vertrauenswürdig
//!   (`render_meeting_html` maskiert zwar, das Abschalten ist der zweite Riegel).
//! - Jeder Lauf bekommt ein eigenes Benutzerdatenverzeichnis: zwei Exporte
//!   gleichzeitig teilen sich keinen Browserprozess, und ein hängender Lauf
//!   kann keinen anderen mitreißen. Das Verzeichnis wird am Ende gelöscht;
//!   Reste eines abgestürzten Laufs räumt der nächste Start ab.
//! - Der Browserprozess hängt in einem Job-Objekt (Speicherdeckel,
//!   niedrige Priorität, `KILL_ON_JOB_CLOSE`). Er wird nicht von uns per
//!   `Command` gestartet, sondern vom WebView2-Lader; `process_guard::attach`
//!   nimmt nur ein `Child`, deshalb steht die Zuweisung hier. Kinder, die
//!   der Browser vor der Zuweisung startet, sind nicht im Job — sie enden
//!   mit dem Browserprozess.
//! - Geschrieben wird nach `<ziel>.<pid>-<n>.part` und erst nach der Prüfung
//!   (`%PDF-`) auf das Ziel umbenannt: eine abgebrochene oder fehlgeschlagene
//!   Ausgabe lässt ein vorhandenes Ziel unangetastet.
//! - RAM-Gate vor dem Start: unter Bedarf plus Notgrenze des Speicherwächters
//!   wird nicht gestartet, sondern gemeldet.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::process_guard::{available_ram_mb, RAM_EMERGENCY_MB};

/// Gesamtzeit für einen Export (Umgebung, Laden, Drucken).
pub const PDF_TIMEOUT: Duration = Duration::from_secs(20);
/// Was ein Druckvorgang an Arbeitsspeicher braucht (MB), gemessen mit Luft.
const PDF_NEED_MB: u64 = 700;
/// Speicherdeckel des Browserbaums (MB, festgeschriebener Speicher).
#[cfg(windows)]
const PDF_JOB_LIMIT_MB: usize = 3072;
/// `NavigateToString` nimmt höchstens 2 MB; darüber wird eine Datei geladen.
const NAVIGATE_TO_STRING_MAX: usize = 1_500_000;
/// So lange darf das Aufräumen auf das Ende des Browserprozesses warten.
#[cfg(windows)]
const EXIT_GRACE: Duration = Duration::from_secs(4);
/// Zusätzliche Wartezeit des Aufrufers über die Laufzeit hinaus (Aufräumen).
const JOIN_SLACK: Duration = Duration::from_secs(10);
/// Laufordner, die älter sind, gelten als Rest eines abgestürzten Laufs.
const STALE_AFTER: Duration = Duration::from_secs(3600);
/// Titelpräfix des versteckten Fensters (Tests suchen danach).
#[cfg(windows)]
const WINDOW_TITLE_PREFIX: &str = "LVA-PDF-";

static RUN_COUNTER: AtomicU64 = AtomicU64::new(0);
/// Testzähler: wie oft der Browserprozess ohne Job-Objekt weiterlief.
#[cfg(test)]
static JOB_ATTACH_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Fehler eines PDF-Exports. Die Kennung vor dem Doppelpunkt (`code`) ist
/// stabil: die Oberfläche bietet bei jedem `pdf_…` „Drucken…" als Rückfall an.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// Keine WebView2-Laufzeit (oder kein Windows).
    Unavailable(String),
    /// Nicht fertig innerhalb der Zeitgrenze.
    Timeout,
    /// Zu wenig freier Arbeitsspeicher für den Start.
    LowMemory { free_mb: u64 },
    /// Alles andere: Ziel nicht beschreibbar, Browser abgestürzt, Druck fehlgeschlagen.
    Failed(String),
}

impl PdfError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "pdf_unavailable",
            Self::Timeout => "pdf_timeout",
            Self::LowMemory { .. } => "pdf_low_memory",
            Self::Failed(_) => "pdf_failed",
        }
    }
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(why) => write!(f, "{}: {why}", self.code()),
            Self::Timeout => write!(
                f,
                "{}: Das PDF war nach {} Sekunden nicht fertig.",
                self.code(),
                PDF_TIMEOUT.as_secs()
            ),
            Self::LowMemory { free_mb } => write!(
                f,
                "{}: Zu wenig freier Arbeitsspeicher für den PDF-Export ({:.1} GB frei). Andere Programme schließen und erneut versuchen.",
                self.code(),
                *free_mb as f64 / 1024.0
            ),
            Self::Failed(why) => write!(f, "{}: {why}", self.code()),
        }
    }
}

/// Einstellungen eines Laufs. Der Standard ist der Produktivweg; Tests setzen
/// eigene Zeitgrenzen und ein eigenes Verzeichnis.
#[derive(Clone)]
pub struct PdfOptions {
    pub timeout: Duration,
    /// Wurzel der Laufordner (`run-<sekunden>-<pid>-<n>`).
    pub user_data_root: PathBuf,
    /// Testhaken: wird mit der Prozess-ID des Browsers gerufen, sobald sie feststeht.
    #[cfg(test)]
    pub on_browser_pid: Option<Arc<dyn Fn(u32) + Send + Sync>>,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            timeout: PDF_TIMEOUT,
            user_data_root: default_user_data_root(),
            #[cfg(test)]
            on_browser_pid: None,
        }
    }
}

/// `%LOCALAPPDATA%\<Kennung>\pdf-webview2` (im portablen Modus `Data\pdf-webview2`).
/// Ein Wegwerf-Profil, nie Nutzerdaten.
pub fn default_user_data_root() -> PathBuf {
    if let Some(dir) = crate::portable::data_dir() {
        return dir.join("pdf-webview2");
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("de.wolffappliedai.localvoiceai")
        .join("pdf-webview2")
}

/// Schreibt `html` als PDF (A4, Ränder 15 mm, ohne Kopf-/Fußzeile) nach `path`.
/// Fehlertexte beginnen mit `pdf_unavailable`, `pdf_timeout`, `pdf_low_memory`
/// oder `pdf_failed`.
pub fn write_pdf(path: &Path, html: &str) -> Result<(), String> {
    write_pdf_with(path, html, &PdfOptions::default()).map_err(|e| e.to_string())
}

pub fn write_pdf_with(path: &Path, html: &str, opts: &PdfOptions) -> Result<(), PdfError> {
    check_target(path)?;
    #[cfg(not(windows))]
    {
        let _ = (html, opts);
        Err(PdfError::Unavailable(
            "PDF-Export nutzt WebView2 und ist nur unter Windows verfügbar.".to_string(),
        ))
    }
    #[cfg(windows)]
    {
        ram_gate(available_ram_mb())?;
        sweep_stale(&opts.user_data_root, unix_secs());

        let run = RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let job = win::RenderJob {
            html: html.to_string(),
            part: part_path(path, pid, run),
            target: path.to_path_buf(),
            udf: opts
                .user_data_root
                .join(format!("run-{}-{pid}-{run}", unix_secs())),
            deadline: Instant::now() + opts.timeout,
            title: format!("{WINDOW_TITLE_PREFIX}{pid}-{run}"),
            abandoned: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            on_browser_pid: opts.on_browser_pid.clone(),
        };
        let abandoned = Arc::clone(&job.abandoned);
        let part = job.part.clone();

        let (done_tx, done_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("pdf-webview2".into())
            .spawn(move || {
                // Ein Absturz des Threads meldet der getrennte Kanal unten;
                // die Aufräum-Wächter laufen beim Abwickeln trotzdem.
                let _ = done_tx.send(win::run(job));
            })
            .map_err(|e| PdfError::Failed(format!("PDF-Thread nicht startbar: {e}")))?;

        match done_rx.recv_timeout(opts.timeout + JOIN_SLACK) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Der Thread hängt in einem COM-Aufruf. Er darf nichts mehr
                // aufs Ziel schreiben, wenn er doch noch fertig wird.
                abandoned.store(true, Ordering::SeqCst);
                let _ = std::fs::remove_file(&part);
                Err(PdfError::Timeout)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = std::fs::remove_file(&part);
                Err(PdfError::Failed(
                    "Der PDF-Thread ist unerwartet beendet worden.".to_string(),
                ))
            }
        }
    }
}

/// Zielprüfung vor jedem Start: kein Ordner, Elternordner vorhanden. Spart
/// den Browserstart, wenn ohnehin nichts geschrieben werden kann.
fn check_target(path: &Path) -> Result<(), PdfError> {
    if path.is_dir() {
        return Err(PdfError::Failed(format!(
            "{} ist ein Ordner, keine Datei.",
            path.display()
        )));
    }
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() && !parent.is_dir() => {
            Err(PdfError::Failed(format!(
                "Der Zielordner {} existiert nicht.",
                parent.display()
            )))
        }
        _ => Ok(()),
    }
}

/// Start-Gate: genug freier RAM für den Browserbaum, ohne die Notgrenze des
/// Speicherwächters zu berühren. Nicht messbar (0) blockiert nicht.
pub(crate) fn ram_gate(free_mb: u64) -> Result<(), PdfError> {
    if free_mb == 0 || free_mb >= PDF_NEED_MB + RAM_EMERGENCY_MB {
        Ok(())
    } else {
        Err(PdfError::LowMemory { free_mb })
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `<ziel>.<pid>-<n>.part` neben dem Ziel (gleiches Laufwerk, damit das
/// Umbenennen atomar ist).
fn part_path(target: &Path, pid: u32, run: u64) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "export.pdf".into());
    name.push(format!(".{pid}-{run}.part"));
    target.with_file_name(name)
}

/// Löscht Laufordner (`run-<sekunden>-…`), die älter als `STALE_AFTER` sind.
/// Alles andere im Ordner bleibt unberührt. Fehler werden ignoriert: ein
/// Rest kostet nur Platz.
pub(crate) fn sweep_stale(root: &Path, now_secs: u64) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix("run-") else {
            continue;
        };
        let Some(started) = rest.split('-').next().and_then(|s| s.parse::<u64>().ok()) else {
            continue;
        };
        if now_secs.saturating_sub(started) >= STALE_AFTER.as_secs() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `file:///`-Adresse für einen lokalen Pfad; alles außer Buchstaben, Ziffern
/// und `-._~/:` wird prozentkodiert (Benutzernamen mit Leerzeichen oder Umlauten).
pub(crate) fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text);
    let mut url = String::from("file:///");
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                url.push(byte as char)
            }
            other => url.push_str(&format!("%{other:02X}")),
        }
    }
    url
}

/// Die fertige Datei prüfen und aufs Ziel umbenennen.
#[cfg(windows)]
fn finalize(part: &Path, target: &Path, abandoned: &AtomicBool) -> Result<(), PdfError> {
    use std::io::Read;
    if abandoned.load(Ordering::SeqCst) {
        return Err(PdfError::Timeout);
    }
    let mut head = [0u8; 5];
    let read = std::fs::File::open(part)
        .and_then(|mut f| f.read_exact(&mut head))
        .map(|_| head);
    match read {
        Ok(head) if &head == b"%PDF-" => {}
        Ok(_) => {
            return Err(PdfError::Failed(
                "Die erzeugte Datei ist kein PDF.".to_string(),
            ))
        }
        Err(e) => {
            return Err(PdfError::Failed(format!(
                "Die erzeugte PDF-Datei ist nicht lesbar: {e}"
            )))
        }
    }
    std::fs::rename(part, target).map_err(|e| {
        PdfError::Failed(format!(
            "Die Datei {} kann nicht geschrieben werden (in einem Programm geöffnet?): {e}",
            target.display()
        ))
    })
}

#[cfg(windows)]
mod win {
    use super::*;

    use webview2_com::Microsoft::Web::WebView2::Win32::{
        CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2Controller,
        ICoreWebView2Environment, ICoreWebView2Environment6, ICoreWebView2EnvironmentOptions,
        ICoreWebView2_7, COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT,
    };
    use webview2_com::{
        CreateCoreWebView2ControllerCompletedHandler,
        CreateCoreWebView2EnvironmentCompletedHandler, NavigationCompletedEventHandler,
        PrintToPdfCompletedHandler, ProcessFailedEventHandler,
    };
    use windows::core::{w, Interface, HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, E_POINTER, HANDLE, HWND, RECT, WAIT_OBJECT_0};
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_JOB_MEMORY,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PRIORITY_CLASS,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::{
        OpenProcess, BELOW_NORMAL_PRIORITY_CLASS, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW,
        TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT, WM_QUIT, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_POPUP,
    };

    /// Alles, was ein Lauf braucht. Wird in den PDF-Thread verschoben.
    pub(super) struct RenderJob {
        pub html: String,
        pub part: PathBuf,
        pub target: PathBuf,
        pub udf: PathBuf,
        pub deadline: Instant,
        pub title: String,
        pub abandoned: Arc<AtomicBool>,
        #[cfg(test)]
        pub on_browser_pid: Option<Arc<dyn Fn(u32) + Send + Sync>>,
    }

    /// Ereignisse aus den WebView2-Rückrufen; alle kommen im selben Thread an.
    enum Event {
        Environment(windows::core::Result<ICoreWebView2Environment>),
        Controller(windows::core::Result<ICoreWebView2Controller>),
        Navigated { success: bool, status: i32 },
        Printed(windows::core::Result<()>, bool),
        ProcessFailed(String),
    }

    /// HRESULT ohne installierte Laufzeit: Datei/Pfad nicht gefunden.
    const HR_FILE_NOT_FOUND: i32 = 0x8007_0002_u32 as i32;
    const HR_PATH_NOT_FOUND: i32 = 0x8007_0003_u32 as i32;

    pub(crate) fn map_env_error(e: windows::core::Error) -> PdfError {
        let code = e.code().0;
        if code == HR_FILE_NOT_FOUND || code == HR_PATH_NOT_FOUND {
            PdfError::Unavailable(
                "Die WebView2-Laufzeit von Microsoft ist nicht installiert.".to_string(),
            )
        } else {
            PdfError::Failed(format!(
                "WebView2 konnte nicht gestartet werden (0x{:08X}): {}",
                code as u32,
                e.message()
            ))
        }
    }

    fn failed(context: &str, e: windows::core::Error) -> PdfError {
        PdfError::Failed(format!(
            "{context}: {} (0x{:08X})",
            e.message(),
            e.code().0 as u32
        ))
    }

    /// COM je Thread einschalten und am Ende wieder ausschalten.
    struct ComGuard;

    impl ComGuard {
        fn init() -> Result<Self, PdfError> {
            // SAFETY: Aufruf ohne Zeiger; das passende CoUninitialize steht im Drop.
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            hr.ok()
                .map_err(|e| failed("COM nicht initialisierbar", e))?;
            Ok(Self)
        }
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            // SAFETY: paart das erfolgreiche CoInitializeEx aus `init`.
            unsafe { CoUninitialize() };
        }
    }

    /// Nachrichten dieses Threads abarbeiten, ohne zu blockieren.
    fn pump_pending() -> bool {
        let mut msg = MSG::default();
        // SAFETY: `msg` ist initialisiert und lebt über den Aufruf.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return false;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
    }

    /// Wartet höchstens bis `deadline` auf das nächste Ereignis und bedient
    /// dabei die Nachrichtenschleife.
    fn next_event(rx: &mpsc::Receiver<Event>, deadline: Instant) -> Result<Event, PdfError> {
        loop {
            if let Ok(event) = rx.try_recv() {
                return Ok(event);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(PdfError::Timeout);
            }
            let wait_ms = (deadline - now).as_millis().min(100) as u32;
            // SAFETY: keine Handles, nur Warten auf Nachrichten oder Ablauf.
            unsafe {
                MsgWaitForMultipleObjects(None, false, wait_ms, QS_ALLINPUT);
            }
            if !pump_pending() {
                return Err(PdfError::Failed(
                    "Die Nachrichtenschleife wurde beendet.".to_string(),
                ));
            }
        }
    }

    /// Nächstes Ereignis, das `pick` haben will; ein Browserabsturz bricht
    /// sofort ab, statt bis zur Zeitgrenze zu warten.
    fn expect<T>(
        rx: &mpsc::Receiver<Event>,
        deadline: Instant,
        mut pick: impl FnMut(Event) -> Option<T>,
    ) -> Result<T, PdfError> {
        loop {
            match next_event(rx, deadline)? {
                Event::ProcessFailed(what) => return Err(PdfError::Failed(what)),
                event => {
                    if let Some(value) = pick(event) {
                        return Ok(value);
                    }
                }
            }
        }
    }

    /// Der Browserprozess samt Job-Objekt. Der Prozess-Handle dient zum
    /// Warten auf das Ende; das Job-Objekt begrenzt und beendet den Baum.
    struct BrowserProcess {
        process: HANDLE,
        job: Option<HANDLE>,
    }

    impl BrowserProcess {
        fn attach(pid: u32) -> Option<Self> {
            use windows::Win32::Foundation::CloseHandle as close;
            const SYNCHRONIZE: u32 = 0x0010_0000;
            // SAFETY: OpenProcess mit einer fremden PID; Fehler werden geprüft.
            let process = unsafe {
                OpenProcess(
                    windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS(
                        PROCESS_SET_QUOTA.0 | PROCESS_TERMINATE.0 | SYNCHRONIZE,
                    ),
                    false,
                    pid,
                )
            }
            .ok()?;

            // SAFETY: Win32-Aufrufe mit korrekt dimensionierten Strukturen;
            // die Handles schließt der Drop.
            let job = unsafe {
                (|| {
                    let job = CreateJobObjectW(None, None).ok()?;
                    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                        | JOB_OBJECT_LIMIT_PRIORITY_CLASS
                        | JOB_OBJECT_LIMIT_JOB_MEMORY
                        // Erlaubt Chromium, seine Kinder explizit zu lösen, statt
                        // beim Start mit "Zugriff verweigert" zu scheitern.
                        | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
                    limits.BasicLimitInformation.PriorityClass = BELOW_NORMAL_PRIORITY_CLASS.0;
                    limits.JobMemoryLimit = PDF_JOB_LIMIT_MB * 1024 * 1024;
                    if SetInformationJobObject(
                        job,
                        JobObjectExtendedLimitInformation,
                        &limits as *const _ as *const core::ffi::c_void,
                        std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                    )
                    .is_err()
                        || AssignProcessToJobObject(job, process).is_err()
                    {
                        let _ = close(job);
                        return None;
                    }
                    Some(job)
                })()
            };
            if job.is_none() {
                #[cfg(test)]
                JOB_ATTACH_FAILURES.fetch_add(1, Ordering::SeqCst);
                log::warn!("pdf: browser pid {pid} runs without a job object (cap not applied)");
            }
            Some(Self { process, job })
        }

        /// Wartet (mit Nachrichtenschleife) bis `until` auf das Ende des Prozesses.
        fn wait_exit(&self, until: Instant) -> bool {
            loop {
                let now = Instant::now();
                if now >= until {
                    return false;
                }
                let wait_ms = (until - now).as_millis().min(100) as u32;
                // SAFETY: gültiger Prozess-Handle, der bis zum Drop offen bleibt.
                let result = unsafe {
                    MsgWaitForMultipleObjects(Some(&[self.process]), false, wait_ms, QS_ALLINPUT)
                };
                if result == WAIT_OBJECT_0 {
                    return true;
                }
                pump_pending();
            }
        }
    }

    impl Drop for BrowserProcess {
        fn drop(&mut self) {
            // SAFETY: beide Handles gehören uns und werden genau einmal geschlossen;
            // das Schließen des Job-Handles beendet verbliebene Prozesse (KILL_ON_JOB_CLOSE).
            unsafe {
                if let Some(job) = self.job.take() {
                    let _ = CloseHandle(job);
                }
                let _ = CloseHandle(self.process);
            }
        }
    }

    /// Hält alles, was am Ende geschlossen werden muss — auch bei frühem
    /// Fehler oder Absturz. Die Reihenfolge im Drop ist die Reihenfolge des
    /// Abbaus: Controller schließen, Referenzen freigeben, Fenster zerstören,
    /// auf den Browser warten, Verzeichnis löschen.
    struct Session {
        udf: PathBuf,
        hwnd: Option<HWND>,
        env: Option<ICoreWebView2Environment>,
        controller: Option<ICoreWebView2Controller>,
        webview: Option<ICoreWebView2>,
        browser: Option<BrowserProcess>,
    }

    impl Drop for Session {
        fn drop(&mut self) {
            // SAFETY: Close/DestroyWindow im Erzeugerthread; die Objekte sind gültig.
            unsafe {
                if let Some(controller) = self.controller.take() {
                    let _ = controller.Close();
                }
                self.webview = None;
                self.env = None;
                if let Some(hwnd) = self.hwnd.take() {
                    let _ = DestroyWindow(hwnd);
                }
            }
            if let Some(browser) = self.browser.take() {
                if !browser.wait_exit(Instant::now() + EXIT_GRACE) {
                    log::warn!(
                        "pdf: browser process still running after grace period, killing job"
                    );
                }
                drop(browser);
            }
            pump_pending();
            // Dateien im Profil sind kurz nach dem Prozessende noch gesperrt.
            for _ in 0..10 {
                if !self.udf.exists() || std::fs::remove_dir_all(&self.udf).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }

    fn create_hidden_window(title: &str) -> Result<HWND, PdfError> {
        // SAFETY: Fensterklasse STATIC ist eingebaut; das Fenster wird nie
        // angezeigt und im Session-Drop zerstört.
        unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| failed("Modulhandle fehlt", e))?;
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("STATIC"),
                &HSTRING::from(title),
                WS_POPUP,
                0,
                0,
                800,
                1100,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
            .map_err(|e| failed("Verstecktes Fenster nicht erzeugbar", e))
        }
    }

    pub(super) fn run(job: RenderJob) -> Result<(), PdfError> {
        let result = render(&job);
        // Session ist beim Verlassen von `render` bereits abgebaut.
        if result.is_err() {
            let _ = std::fs::remove_file(&job.part);
        }
        result
    }

    fn render(job: &RenderJob) -> Result<(), PdfError> {
        let started = Instant::now();
        let _com = ComGuard::init()?;
        std::fs::create_dir_all(&job.udf).map_err(|e| {
            PdfError::Failed(format!(
                "Arbeitsverzeichnis {} nicht anlegbar: {e}",
                job.udf.display()
            ))
        })?;
        let mut session = Session {
            udf: job.udf.clone(),
            hwnd: None,
            env: None,
            controller: None,
            webview: None,
            browser: None,
        };
        let hwnd = create_hidden_window(&job.title)?;
        session.hwnd = Some(hwnd);
        let (tx, rx) = mpsc::channel::<Event>();

        // 1. Umgebung (startet bei Bedarf den Browserprozess).
        let env = {
            let tx = tx.clone();
            let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |code, env| {
                    let result = code
                        .and_then(|()| env.ok_or_else(|| windows::core::Error::from(E_POINTER)));
                    let _ = tx.send(Event::Environment(result));
                    Ok(())
                },
            ));
            // SAFETY: die Zeichenkette lebt über den Aufruf; der Rückruf kommt
            // in diesem Thread über die Nachrichtenschleife an.
            unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    &HSTRING::from(job.udf.as_os_str()),
                    None::<&ICoreWebView2EnvironmentOptions>,
                    &handler,
                )
            }
            .map_err(map_env_error)?;
            expect(&rx, job.deadline, |e| match e {
                Event::Environment(r) => Some(r),
                _ => None,
            })?
            .map_err(map_env_error)?
        };
        session.env = Some(env.clone());

        // 2. Controller im versteckten Fenster.
        let controller = {
            let tx = tx.clone();
            let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                move |code, controller| {
                    let result = code.and_then(|()| {
                        controller.ok_or_else(|| windows::core::Error::from(E_POINTER))
                    });
                    let _ = tx.send(Event::Controller(result));
                    Ok(())
                },
            ));
            // SAFETY: gültiges Fenster dieses Threads, Rückruf über die Schleife.
            unsafe { env.CreateCoreWebView2Controller(hwnd, &handler) }
                .map_err(|e| failed("WebView2-Controller nicht erzeugbar", e))?;
            expect(&rx, job.deadline, |e| match e {
                Event::Controller(r) => Some(r),
                _ => None,
            })?
            .map_err(|e| failed("WebView2-Controller nicht erzeugbar", e))?
        };
        session.controller = Some(controller.clone());

        // SAFETY: alle Aufrufe auf gültigen COM-Objekten im Erzeugerthread.
        let webview = unsafe {
            controller
                .SetBounds(RECT {
                    left: 0,
                    top: 0,
                    right: 800,
                    bottom: 1100,
                })
                .map_err(|e| failed("Fenstergröße nicht setzbar", e))?;
            controller
                .CoreWebView2()
                .map_err(|e| failed("WebView2 nicht erreichbar", e))?
        };
        session.webview = Some(webview.clone());

        // Browserprozess in den Job hängen und dem Test melden.
        // SAFETY: Ausgabe-Zeiger auf lokale Variable.
        let mut pid = 0u32;
        if unsafe { webview.BrowserProcessId(&mut pid) }.is_ok() && pid != 0 {
            session.browser = BrowserProcess::attach(pid);
            #[cfg(test)]
            if let Some(hook) = &job.on_browser_pid {
                hook(pid);
            }
        }

        // Skripte aus: Protokolltext ist nie vertrauenswürdig.
        // SAFETY: gültiges Settings-Objekt.
        unsafe {
            let settings = webview
                .Settings()
                .map_err(|e| failed("WebView2-Einstellungen nicht lesbar", e))?;
            settings
                .SetIsScriptEnabled(false)
                .map_err(|e| failed("Skripte nicht abschaltbar", e))?;
            let _ = settings.SetIsWebMessageEnabled(false);
            let _ = settings.SetAreDefaultScriptDialogsEnabled(false);
            let _ = settings.SetAreDefaultContextMenusEnabled(false);
            let _ = settings.SetAreDevToolsEnabled(false);
            let _ = settings.SetIsStatusBarEnabled(false);
        }

        // Ein Absturz des Browsers oder Renderers bricht sofort ab.
        {
            let tx = tx.clone();
            let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let what = args
                    .and_then(|a| {
                        let mut kind = Default::default();
                        // SAFETY: Ausgabe-Zeiger auf lokale Variable.
                        unsafe { a.ProcessFailedKind(&mut kind) }
                            .ok()
                            .map(|()| kind.0)
                    })
                    .map(|k| format!("Ein WebView2-Prozess ist ausgefallen (Art {k})."))
                    .unwrap_or_else(|| "Ein WebView2-Prozess ist ausgefallen.".to_string());
                let _ = tx.send(Event::ProcessFailed(what));
                Ok(())
            }));
            let mut token = 0i64;
            // SAFETY: gültiger Rückruf, Token-Zeiger auf lokale Variable.
            unsafe { webview.add_ProcessFailed(&handler, &mut token) }
                .map_err(|e| failed("Überwachung nicht einrichtbar", e))?;
        }

        // 3. Seite laden.
        {
            let tx = tx.clone();
            let handler = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let (mut success, mut status) = (windows::core::BOOL(0), Default::default());
                if let Some(args) = args {
                    // SAFETY: Ausgabe-Zeiger auf lokale Variablen.
                    unsafe {
                        let _ = args.IsSuccess(&mut success);
                        let _ = args.WebErrorStatus(&mut status);
                    }
                }
                let _ = tx.send(Event::Navigated {
                    success: success.as_bool(),
                    status: status.0,
                });
                Ok(())
            }));
            let mut token = 0i64;
            // SAFETY: gültiger Rückruf, Token-Zeiger auf lokale Variable.
            unsafe { webview.add_NavigationCompleted(&handler, &mut token) }
                .map_err(|e| failed("Überwachung nicht einrichtbar", e))?;
        }
        if job.html.len() <= NAVIGATE_TO_STRING_MAX {
            // SAFETY: die Zeichenkette lebt über den Aufruf.
            unsafe { webview.NavigateToString(&HSTRING::from(job.html.as_str())) }
                .map_err(|e| failed("Seite nicht ladbar", e))?;
        } else {
            let page = job.udf.join("page.html");
            std::fs::write(&page, job.html.as_bytes())
                .map_err(|e| PdfError::Failed(format!("Seite nicht zwischenspeicherbar: {e}")))?;
            // SAFETY: die Zeichenkette lebt über den Aufruf.
            unsafe { webview.Navigate(&HSTRING::from(file_url(&page))) }
                .map_err(|e| failed("Seite nicht ladbar", e))?;
        }
        let (success, status) = expect(&rx, job.deadline, |e| match e {
            Event::Navigated { success, status } => Some((success, status)),
            _ => None,
        })?;
        if !success {
            return Err(PdfError::Failed(format!(
                "Die Seite konnte nicht geladen werden (WebView2-Status {status})."
            )));
        }

        // 4. Drucken: A4, Ränder 15 mm, ohne Kopf-/Fußzeile.
        const MM: f64 = 1.0 / 25.4;
        // SAFETY: gültige COM-Objekte; alle Zeichenketten leben über den Aufruf.
        unsafe {
            let print_settings = env
                .cast::<ICoreWebView2Environment6>()
                .and_then(|e| e.CreatePrintSettings())
                .map_err(|e| failed("Druckeinstellungen nicht verfügbar (WebView2 zu alt?)", e))?;
            let set = |r: windows::core::Result<()>| {
                r.map_err(|e| failed("Druckeinstellung nicht setzbar", e))
            };
            set(print_settings.SetOrientation(COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT))?;
            set(print_settings.SetScaleFactor(1.0))?;
            set(print_settings.SetPageWidth(210.0 * MM))?;
            set(print_settings.SetPageHeight(297.0 * MM))?;
            set(print_settings.SetMarginTop(15.0 * MM))?;
            set(print_settings.SetMarginBottom(15.0 * MM))?;
            set(print_settings.SetMarginLeft(15.0 * MM))?;
            set(print_settings.SetMarginRight(15.0 * MM))?;
            set(print_settings.SetShouldPrintBackgrounds(true))?;
            set(print_settings.SetShouldPrintSelectionOnly(false))?;
            set(print_settings.SetShouldPrintHeaderAndFooter(false))?;

            let handler = {
                let tx = tx.clone();
                PrintToPdfCompletedHandler::create(Box::new(move |code, ok| {
                    let _ = tx.send(Event::Printed(code, ok));
                    Ok(())
                }))
            };
            let webview7 = webview
                .cast::<ICoreWebView2_7>()
                .map_err(|e| failed("PrintToPdf nicht verfügbar (WebView2 zu alt?)", e))?;
            webview7
                .PrintToPdf(
                    &HSTRING::from(job.part.as_os_str()),
                    &print_settings,
                    &handler,
                )
                .map_err(|e| failed("Drucken nicht startbar", e))?;
        }
        let (code, ok) = expect(&rx, job.deadline, |e| match e {
            Event::Printed(code, ok) => Some((code, ok)),
            _ => None,
        })?;
        code.map_err(|e| failed("Drucken fehlgeschlagen", e))?;
        if !ok {
            return Err(PdfError::Failed(
                "Das PDF konnte nicht geschrieben werden (Datenträger voll oder Ziel gesperrt?)."
                    .to_string(),
            ));
        }

        finalize(&job.part, &job.target, &job.abandoned)?;
        log::info!(
            "pdf: {} written in {} ms",
            job.target.display(),
            started.elapsed().as_millis()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lva-pdf-test-{}-{tag}-{}",
            std::process::id(),
            RUN_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fehlerkennungen_sind_stabil_und_stehen_vor_dem_doppelpunkt() {
        let cases = [
            (PdfError::Unavailable("x".into()), "pdf_unavailable"),
            (PdfError::Timeout, "pdf_timeout"),
            (PdfError::LowMemory { free_mb: 900 }, "pdf_low_memory"),
            (PdfError::Failed("x".into()), "pdf_failed"),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code);
            assert!(
                error.to_string().starts_with(&format!("{code}: ")),
                "{error}"
            );
        }
        assert!(PdfError::Timeout.to_string().contains("20 Sekunden"));
    }

    #[test]
    fn ram_gate_blockt_unter_bedarf_plus_notgrenze_und_laesst_nicht_messbares_durch() {
        let border = PDF_NEED_MB + RAM_EMERGENCY_MB;
        assert_eq!(ram_gate(0), Ok(()), "nicht messbar blockiert nicht");
        assert_eq!(ram_gate(border), Ok(()));
        assert_eq!(
            ram_gate(border - 1),
            Err(PdfError::LowMemory {
                free_mb: border - 1
            })
        );
        assert!(ram_gate(1).is_err());
        let text = ram_gate(1500).unwrap_err().to_string();
        assert!(text.starts_with("pdf_low_memory: "), "{text}");
    }

    #[test]
    fn zielordner_fehlt_oder_ist_ein_ordner_wird_vor_dem_browserstart_gemeldet() {
        let dir = unique_dir("ziel");
        // Wurzel bewusst ein Pfad, der bei einem Start angelegt würde.
        let opts = PdfOptions {
            user_data_root: dir.join("kein-start"),
            ..PdfOptions::default()
        };
        let missing = dir.join("gibt-es-nicht").join("m.pdf");
        let error = write_pdf_with(&missing, "<p>x</p>", &opts).unwrap_err();
        assert_eq!(error.code(), "pdf_failed");
        assert!(error.to_string().contains("existiert nicht"), "{error}");

        let error = write_pdf_with(&dir, "<p>x</p>", &opts).unwrap_err();
        assert!(error.to_string().contains("Ordner"), "{error}");

        assert!(
            !dir.join("kein-start").exists(),
            "es darf kein Laufordner angelegt worden sein"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn part_datei_liegt_neben_dem_ziel_und_ist_je_lauf_eindeutig() {
        let target = Path::new("C:/Ziel/Protokoll.pdf");
        let a = part_path(target, 10, 1);
        let b = part_path(target, 10, 2);
        assert_eq!(
            a.parent(),
            target.parent(),
            "gleicher Ordner: atomares Umbenennen"
        );
        assert_eq!(a.file_name().unwrap(), "Protokoll.pdf.10-1.part");
        assert_ne!(a, b);
    }

    #[test]
    fn nur_alte_laufordner_werden_abgeraeumt() {
        let root = unique_dir("sweep");
        let now = 10_000_000u64;
        let old = root.join(format!("run-{}-1-0", now - STALE_AFTER.as_secs() - 1));
        let border = root.join(format!("run-{}-1-1", now - STALE_AFTER.as_secs()));
        let young = root.join(format!("run-{}-1-2", now - 60));
        let foreign = root.join("anderer-ordner");
        let garbled = root.join("run-abc-1-0");
        for dir in [&old, &border, &young, &foreign, &garbled] {
            std::fs::create_dir_all(dir.join("Default")).unwrap();
        }
        sweep_stale(&root, now);
        assert!(!old.exists(), "alt: weg");
        assert!(!border.exists(), "genau an der Grenze: weg");
        assert!(young.exists(), "jung: bleibt (läuft evtl. noch)");
        assert!(foreign.exists(), "fremde Ordner bleiben");
        assert!(garbled.exists(), "nicht parsbar: bleibt");
        // Nicht vorhandene Wurzel: kein Absturz.
        sweep_stale(&root.join("nichts"), now);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn file_url_maskiert_leerzeichen_umlaute_und_prozent() {
        assert_eq!(
            file_url(Path::new(r"C:\Users\Jörg Müller\AppData\a b%c#d.html")),
            "file:///C:/Users/J%C3%B6rg%20M%C3%BCller/AppData/a%20b%25c%23d.html"
        );
        assert_eq!(
            file_url(Path::new(r"\\?\C:\x\page.html")),
            "file:///C:/x/page.html",
            "Verbatim-Präfix wird entfernt"
        );
    }

    #[cfg(windows)]
    #[test]
    fn fehlende_laufzeit_wird_als_nicht_verfuegbar_gemeldet() {
        let missing = windows::core::Error::from(windows::core::HRESULT(0x8007_0002_u32 as i32));
        let error = super::win::map_env_error(missing);
        assert_eq!(error.code(), "pdf_unavailable");
        let other = windows::core::Error::from(windows::core::HRESULT(0x8007_139F_u32 as i32));
        assert_eq!(super::win::map_env_error(other).code(), "pdf_failed");
    }

    #[cfg(not(windows))]
    #[test]
    fn ausserhalb_von_windows_ist_pdf_nicht_verfuegbar() {
        let dir = unique_dir("nowin");
        let error =
            write_pdf_with(&dir.join("m.pdf"), "<p>x</p>", &PdfOptions::default()).unwrap_err();
        assert_eq!(error.code(), "pdf_unavailable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // Ab hier: echte WebView2-Läufe. `cargo test --lib meetings::pdf -- --ignored`
    // ------------------------------------------------------------------
    #[cfg(windows)]
    mod webview {
        use super::*;
        use crate::managers::meetings::export::{
            render_meeting_html, tests::nordlicht_bundle, ExportParts,
        };

        /// Die Fensterzählung ist prozessweit: die WebView2-Läufe dieses Moduls
        /// laufen deshalb nacheinander, auch bei paralleler Testausführung.
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

        fn serial() -> std::sync::MutexGuard<'static, ()> {
            SERIAL
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }

        fn opts(root: &Path) -> PdfOptions {
            PdfOptions {
                user_data_root: root.join("udf"),
                ..PdfOptions::default()
            }
        }

        fn small_html() -> String {
            "<!DOCTYPE html><html lang=\"de\"><head><meta charset=\"utf-8\"><title>Nordlicht</title></head>\
             <body><h1>Nordlicht Kickoff</h1><p>Größe des Pilotprojekts: 3 Standorte, Preis 40 €, äöüß.</p></body></html>"
                .to_string()
        }

        fn text_of(path: &Path) -> String {
            pdf_extract::extract_text(path).expect("pdf-extract liest das PDF")
        }

        /// msedgewebview2-Prozesse, die noch mit `needle` im Kommandozeilentext laufen.
        fn browser_processes_using(needle: &str) -> usize {
            use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
            let mut sys = System::new();
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
            );
            sys.processes()
                .values()
                .filter(|p| {
                    p.cmd()
                        .iter()
                        .any(|part| part.to_string_lossy().contains(needle))
                })
                .count()
        }

        /// Fenster dieses Prozesses mit dem PDF-Titelpräfix (auch unsichtbare).
        fn pdf_windows() -> usize {
            use windows::core::BOOL;
            use windows::Win32::Foundation::{HWND, LPARAM};
            use windows::Win32::UI::WindowsAndMessaging::{
                EnumWindows, GetWindowTextW, GetWindowThreadProcessId,
            };
            unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
                let count = &mut *(lparam.0 as *mut usize);
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == std::process::id() {
                    let mut buf = [0u16; 64];
                    let n = GetWindowTextW(hwnd, &mut buf) as usize;
                    if String::from_utf16_lossy(&buf[..n]).starts_with(WINDOW_TITLE_PREFIX) {
                        *count += 1;
                    }
                }
                BOOL(1)
            }
            let mut count = 0usize;
            // SAFETY: `count` lebt über den Aufruf, der Rückruf läuft synchron.
            unsafe {
                let _ = EnumWindows(Some(each), LPARAM(&mut count as *mut usize as isize));
            }
            count
        }

        #[test]
        #[ignore = "braucht die WebView2-Laufzeit"]
        fn pdf_enthaelt_titel_groesse_und_umlaute() {
            let _serial = serial();
            let dir = unique_dir("inhalt");
            let out = dir.join("m.pdf");
            let started = Instant::now();
            let job_failures = JOB_ATTACH_FAILURES.load(Ordering::SeqCst);
            write_pdf_with(&out, &small_html(), &opts(&dir)).expect("PDF erzeugt");
            eprintln!("PDF in {} ms", started.elapsed().as_millis());
            assert_eq!(
                JOB_ATTACH_FAILURES.load(Ordering::SeqCst),
                job_failures,
                "der Browserprozess muss im Job-Objekt (Speicherdeckel) gelaufen sein"
            );
            let bytes = std::fs::read(&out).unwrap();
            assert!(bytes.starts_with(b"%PDF-"));
            let text = text_of(&out);
            assert!(text.contains("Nordlicht Kickoff"), "{text}");
            assert!(text.contains("Größe"), "{text}");
            assert!(text.contains("äöüß"), "{text}");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        #[ignore = "braucht die WebView2-Laufzeit"]
        fn pdf_der_nordlicht_besprechung_hat_titel_groesse_und_mehrere_teile() {
            let _serial = serial();
            let dir = unique_dir("nordlicht");
            let out = dir.join("nordlicht.pdf");
            let bundle = nordlicht_bundle();
            let html = render_meeting_html(&bundle, &ExportParts::all());
            write_pdf_with(&out, &html, &opts(&dir)).expect("PDF erzeugt");
            let text = text_of(&out);
            assert!(text.contains(bundle.meeting.title.trim()), "Titel: {text}");
            assert!(text.contains("Größe"), "Größe: {text}");
            assert!(text.contains("Teilnehmende"), "{text}");
            assert!(text.contains("Transkript"), "{text}");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        #[ignore = "braucht die WebView2-Laufzeit"]
        fn zwei_exporte_hintereinander_hinterlassen_weder_fenster_noch_prozess_noch_ordner() {
            let _serial = serial();
            let dir = unique_dir("zweimal");
            let root = dir.join("udf");
            for n in 0..2 {
                let out = dir.join(format!("m{n}.pdf"));
                write_pdf_with(&out, &small_html(), &opts(&dir)).expect("PDF erzeugt");
                assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF-"));
                assert_eq!(pdf_windows(), 0, "Fenster nach Lauf {n}");
                assert_eq!(
                    browser_processes_using(&root.to_string_lossy()),
                    0,
                    "Browserprozess nach Lauf {n}"
                );
                let left: Vec<_> = std::fs::read_dir(&root)
                    .map(|d| d.flatten().map(|e| e.file_name()).collect())
                    .unwrap_or_default();
                assert!(left.is_empty(), "Laufordner nach Lauf {n}: {left:?}");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        #[ignore = "braucht die WebView2-Laufzeit"]
        fn parallele_exporte_stoeren_sich_nicht() {
            let _serial = serial();
            let dir = unique_dir("parallel");
            let handles: Vec<_> = (0..3)
                .map(|n| {
                    let out = dir.join(format!("p{n}.pdf"));
                    let opts = opts(&dir);
                    std::thread::spawn(move || {
                        write_pdf_with(&out, &small_html(), &opts).map(|()| out)
                    })
                })
                .collect();
            for handle in handles {
                let out = handle.join().unwrap().expect("PDF erzeugt");
                assert!(text_of(&out).contains("Nordlicht Kickoff"));
            }
            assert_eq!(pdf_windows(), 0);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        #[ignore = "braucht die WebView2-Laufzeit"]
        fn zeitueberschreitung_meldet_pdf_timeout_und_laesst_ziel_und_umgebung_sauber() {
            let _serial = serial();
            let dir = unique_dir("timeout");
            let out = dir.join("alt.pdf");
            std::fs::write(&out, b"ALTER INHALT").unwrap();
            let mut options = opts(&dir);
            options.timeout = Duration::from_millis(1);
            let error = write_pdf_with(&out, &small_html(), &options).unwrap_err();
            assert_eq!(error, PdfError::Timeout);
            assert_eq!(
                std::fs::read(&out).unwrap(),
                b"ALTER INHALT",
                "vorhandenes Ziel bleibt bei Abbruch unverändert"
            );
            let parts: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
                .collect();
            assert!(parts.is_empty(), "keine .part-Reste: {parts:?}");
            assert_eq!(pdf_windows(), 0);
            let needle = dir.join("udf").to_string_lossy().into_owned();
            let waited = Instant::now();
            while browser_processes_using(&needle) > 0 && waited.elapsed() < Duration::from_secs(15)
            {
                std::thread::sleep(Duration::from_millis(250));
            }
            eprintln!(
                "Browserprozess nach Timeout weg nach {} ms",
                waited.elapsed().as_millis()
            );
            assert_eq!(browser_processes_using(&needle), 0);
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        #[ignore = "braucht die WebView2-Laufzeit"]
        fn seite_ueber_zwei_megabyte_wird_ueber_eine_datei_geladen() {
            let _serial = serial();
            let dir = unique_dir("gross");
            let out = dir.join("gross.pdf");
            // Unsichtbarer Ballast: sprengt NavigateToString, bleibt eine Seite.
            let ballast = "x".repeat(2_200_000);
            let html = format!(
                "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>\
                 <!-- {ballast} --><h1>Große Seite</h1></body></html>"
            );
            assert!(html.len() > NAVIGATE_TO_STRING_MAX);
            write_pdf_with(&out, &html, &opts(&dir)).expect("PDF erzeugt");
            assert!(text_of(&out).contains("Große Seite"));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        #[ignore = "braucht die WebView2-Laufzeit"]
        fn skripte_in_der_seite_laufen_nicht() {
            let _serial = serial();
            let dir = unique_dir("skript");
            let out = dir.join("skript.pdf");
            let html = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>\
                        <p id=\"a\">Sicherer Text</p>\
                        <script>document.getElementById('a').textContent='SKRIPT-LIEF';</script>\
                        </body></html>";
            write_pdf_with(&out, html, &opts(&dir)).expect("PDF erzeugt");
            let text = text_of(&out);
            assert!(text.contains("Sicherer Text"), "{text}");
            assert!(
                !text.contains("SKRIPT-LIEF"),
                "Skript wurde ausgeführt: {text}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        #[ignore = "braucht die WebView2-Laufzeit"]
        fn absturz_des_browserprozesses_wird_gemeldet_statt_bis_zur_grenze_zu_warten() {
            let _serial = serial();
            use windows::Win32::Foundation::CloseHandle;
            use windows::Win32::System::Threading::{
                OpenProcess, TerminateProcess, PROCESS_TERMINATE,
            };
            let dir = unique_dir("absturz");
            let out = dir.join("absturz.pdf");
            let mut options = opts(&dir);
            options.on_browser_pid = Some(Arc::new(|pid| {
                // SAFETY: fremde PID aus dem eigenen Lauf; Fehler werden ignoriert.
                unsafe {
                    if let Ok(process) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                        let _ = TerminateProcess(process, 1);
                        let _ = CloseHandle(process);
                    }
                }
            }));
            let started = Instant::now();
            let error = write_pdf_with(&out, &small_html(), &options).unwrap_err();
            let took = started.elapsed();
            eprintln!("Absturz gemeldet nach {} ms: {error}", took.as_millis());
            assert!(
                matches!(error, PdfError::Failed(_) | PdfError::Timeout),
                "{error}"
            );
            assert!(!out.exists(), "keine Datei nach Absturz");
            assert_eq!(pdf_windows(), 0);
            assert_eq!(
                browser_processes_using(&dir.join("udf").to_string_lossy()),
                0
            );
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        #[ignore = "braucht die WebView2-Laufzeit"]
        fn vorhandene_datei_wird_bei_erfolg_ersetzt() {
            let _serial = serial();
            let dir = unique_dir("ersetzen");
            let out = dir.join("m.pdf");
            std::fs::write(&out, b"ALT").unwrap();
            write_pdf_with(&out, &small_html(), &opts(&dir)).expect("PDF erzeugt");
            assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF-"));
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
