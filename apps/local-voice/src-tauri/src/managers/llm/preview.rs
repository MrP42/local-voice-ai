//! Abo-CLIs (Claude Code, Codex) im Buch-Gespraech: nur als gekennzeichnete
//! VORSCHAU (lva#334, Entscheidung des Pruefers).
//!
//! Grundsatz: was nicht belegt ist, SPERRT (fail-closed); „unbekannt“ ergibt
//! nie Erfolg. Fuer ein Abo gibt es keinen Vorab-Beweis des Tokenverbrauchs:
//! die CLI schickt eigene, versteckte Anweisungen und kompaktiert still. Es
//! gibt deshalb KEINE eingebauten Zahlen und Faehigkeiten; zulaessig ist nur,
//! was auf diesem Geraet fuer genau diese CLI-Version belegt ist
//! ([`super::evidence`]: Faehigkeitstabelle `IsolationCaps` und Messreihe).
//!
//! * Freigabe: eigener Schalter `book_talk_subscription_preview`, ein
//!   gebundenes Tupel ([`PreviewBinding`]: Anbieter, Modell, CLI-Version,
//!   SHA-256 des TATSAECHLICH gestarteten Programms, [`payload_path`]), alle
//!   Isolationsfaehigkeiten dieser Version belegt UND eine passende Messung
//!   ([`gate`], [`proof_for`]). Fehlt eines, ist die Vorschau nicht freigegeben.
//! * Vorab ([`precheck`]): `UTF-8-Bytes(System + Prompt) + Overhead * 1,2 +
//!   gesamte Ausgabereserve <= PREVIEW_MAX_CONTEXT_SHARE (0,5) * Kontext`; der
//!   Overhead ist GEMESSEN, die Schwelle fest im Code (nicht vom Bediener).
//! * Nur Claude (Phase 1): Codex ist in der Vorschau IMMER gesperrt
//!   (`codex_preview_not_available`), Phase 2 folgt als eigenes Issue.
//! * Nachtraeglich ([`verify`]): Nutzungsangabe vorhanden und plausibel,
//!   gemeldetes Modell EXAKT das gemessene, Kompaktierung und Abschneiden
//!   je Tri-State ([`Tri`]) „nein“ (und der Ereignisvertrag der Version
//!   belegt), Eingabe + Ausgabe <= Kontext. Sonst wird der Text nie an
//!   Oberflaeche oder Sprache gegeben (Aufrufer: `llm_versand::transport::cli`, früher `cli::call_with_options`).
//! * Bindung: gebunden wird die installierte ORIGINALDATEI (keine Kopie). Alle
//!   Starts der Vorschau, auch `--version`, laufen ueber [`Pinned`]: Datei
//!   offen halten, Hash aus dem Handle, Start ueber das Handle. Windows:
//!   lokales festes Laufwerk; JEDE Elternkomponente (Wurzel bis Zielordner)
//!   nacheinander mit Handle geoeffnet und am Handle auf Reparse-Punkte geprueft,
//!   die Datei mit FILE_FLAG_OPEN_REPARSE_POINT und FILE_SHARE_READ,
//!   `GetFinalPathNameByHandleW` gleich dem erwarteten Pfad; Start mit genau
//!   diesem Pfad. Geht eines davon nicht, ist Windows gesperrt.
//!   ERKENNUNG statt Verhinderung: Windows verhindert das Umbenennen gehaltener
//!   Komponenten NICHT zuverlaessig (Umbenennen mit POSIX-Semantik gelingt trotz
//!   fehlendem FILE_SHARE_DELETE). Das Handle folgt aber dem Umbenennen, sein
//!   Endpfad aendert sich. [`Pinned::check_paths`] vergleicht deshalb den
//!   Endpfad des Datei-Handles und aller Elternhandles mit dem erwarteten Pfad:
//!   (1) direkt vor dem Start, (2) direkt nach dem Spawn (Abweichung beendet den
//!   Prozessbaum), (3) nach dem Lauf in [`Pinned::verify`]. Abweichung:
//!   `binding_changed`, Antwort verworfen.
//!   Die Erkennung ist KEINE Garantie, sondern ein Restrisiko, das fuer die
//!   persoenliche Vorschau ausdruecklich akzeptiert ist:
//!   * ein Start per Handle ist mit CreateProcessW nicht moeglich; der Start
//!     bleibt pfadbasiert, das gepruefte Handle wird nicht atomar uebergeben;
//!   * ein Austausch mit Rueckbenennung zwischen zwei Pruefungen bleibt unsichtbar;
//!   * zwischen der Pruefung (1) und dem Start bleibt ein Fenster; nach dem Spawn
//!     ist das Image bereits gemappt, eine spaetere Erkennung aendert daran nichts;
//!   * Kindprozesse, die vor der Job-Zuordnung gestartet werden, koennen entkommen;
//!   * das Beenden des Prozessbaums begrenzt die weitere Wirkung, macht eine
//!     bereits erfolgte Datenuebertragung aber nicht rueckgaengig (nach einer
//!     Abweichung wird deshalb nichts mehr auf stdin geschrieben);
//!   * Inhaltsaenderungen durch denselben Nutzer liegen ausserhalb der Garantie;
//!     der Hash nach dem Lauf verwirft dann die Antwort.
//!
//!   Linux: Start ueber `/proc/self/fd/<fd>`. macOS und andere Unix: gesperrt (`platform_unsupported`).
//!   Vor dem Start und nach dem Ende wird der Hash neu gebildet ([`Pinned::verify`]).
//!   Grenze (ausserhalb der Garantie): die FD-Bindung verhindert Pfadaustausch,
//!   aber keine Inhaltsaenderung zwischen Hash und `exec` durch denselben
//!   Nutzer; der Hash nach dem Lauf verwirft die Antwort dann.
//! * Schalter: [`PreviewSwitch`] wird vor, waehrend und nach dem Aufruf
//!   geprueft; bei „aus“ wird der Prozessbaum beendet und das Ergebnis verworfen.
//!
//! Kein Frage-, Antwort- oder Buchtext im Log: [`audit_line`] kennt nur Tupel,
//! Modellnamen, Methode und Status.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;

use super::cli::{split_spec, Cli, CliReply, CLAUDE_MODELS};
use super::evidence::{Evidence, IsolationCaps, Measurement};

/// Budgetmethode in Bericht und Audit.
pub const BUDGET_METHOD: &str = "empirical";

/// Kontextfenster der Claude-Modelle in Token (Vorgabe des Pruefers).
pub const CLAUDE_CONTEXT_TOKENS: u32 = 200_000;

/// Sicherheitszuschlag auf den GEMESSENEN Overhead, in Prozent (1,2-fach).
/// Kein Messwert und keine Freigabebasis, nur der Aufschlag auf einen solchen.
pub const OVERHEAD_FACTOR_PERCENT: u64 = 120;
/// Hoechster Anteil des Kontexts, den ein Aufruf ausschoepfen darf (Eingabe mit
/// gemessenem Overhead * 1,2 plus gesamte Ausgabereserve). Fest im Code,
/// konservativ: einmaliger Aufruf in frischer Sitzung, weit unter der
/// Schwelle, ab der Claude Code still kompaktiert. Kompaktierung wird
/// zusaetzlich ueber `compact_boundary` erkannt und sperrt. Nicht
/// einstellbar und nicht Teil der Messdatei.
pub const PREVIEW_MAX_CONTEXT_SHARE: f64 = 0.5;
/// Eine Nutzungsangabe mit weniger Eingabetoken als `Bytes / MIN_BYTES_PER_TOKEN`
/// ist unplausibel (kein Tokenizer packt mehr als ein paar Bytes in ein Token).
const MIN_BYTES_PER_TOKEN: usize = 16;

/// Drei Zustaende statt ja/nein: „unbekannt“ ist nie „nein“.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tri {
    No,
    Yes,
    /// Nicht erkennbar (z. B. Ausgabe ohne `stop_reason`, JSON statt Ereignisstrom).
    #[default]
    Unknown,
}

pub fn provider_of(cli: Cli) -> &'static str {
    match cli {
        Cli::Claude => "claude_cli",
        Cli::Codex => "codex_cli",
    }
}

/// Belegter Kontext des Modells. Claude: 200000 (nie mehr, auch wenn die
/// Konfiguration mehr nennt). Codex: gibt es in der Vorschau nicht (`None`).
pub fn context_tokens(cli: Cli, configured: Option<u32>) -> Option<u32> {
    match cli {
        Cli::Claude => {
            Some(configured.map_or(CLAUDE_CONTEXT_TOKENS, |c| c.clamp(1, CLAUDE_CONTEXT_TOKENS)))
        }
        Cli::Codex => None,
    }
}

/// Planungsgrenze: [`PREVIEW_MAX_CONTEXT_SHARE`] des Kontexts.
pub fn limit_tokens(context: u32) -> u64 {
    (f64::from(context) * PREVIEW_MAX_CONTEXT_SHARE) as u64
}

/// Eingabe in Token-Einheiten: UTF-8-Bytes (eine Obergrenze der Token) plus
/// gemessener Overhead mit Zuschlag.
pub fn input_estimate(overhead_tokens: u64, text_bytes: usize) -> u64 {
    text_bytes as u64 + overhead_tokens * OVERHEAD_FACTOR_PERCENT / 100
}

/// Maximale Ausgabe einer Antwort mit der gemessenen Reserve fuer Denk-Token.
pub fn max_output(answer_tokens: u32, reserve_tokens: u32) -> u32 {
    answer_tokens.saturating_add(reserve_tokens)
}

// ---------------------------------------------------------------- Bindung --

/// Das Tupel, an das die Freigabe gebunden ist. Aendert sich eines der Felder,
/// ist die Freigabe ungueltig und muss erneut bestaetigt werden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PreviewBinding {
    /// `claude_cli` oder `codex_cli`.
    pub provider: String,
    /// Modell, wie in der Konfiguration eingetragen (inkl. Effort-Endung).
    pub model_id: String,
    /// Konkrete Modell-ID statt Alias, soweit bekannt (nur Anzeige; die
    /// Pruefung nutzt das in der Messung gemeldete Modell).
    #[serde(default)]
    pub concrete_model: Option<String>,
    /// Ausgabe von `<cli> --version` (erste Zeile).
    pub cli_version: String,
    /// SHA-256 (hex) des tatsaechlich gestarteten Programms ([`payload_path`]).
    pub binary_sha256: String,
}

/// Konkrete Modell-ID zu einem Claude-Alias (`sonnet` -> `claude-sonnet-5-5`),
/// nach der Liste der App ([`CLAUDE_MODELS`]: erst Aliase, dann volle Namen in
/// gleicher Reihenfolge). Sonst die Angabe selbst; `None`, wenn nicht bekannt.
pub fn concrete_model(cli: Cli, model_id: &str) -> Option<String> {
    let base = split_spec(model_id).0;
    match cli {
        Cli::Codex => None,
        Cli::Claude => {
            let half = CLAUDE_MODELS.len() / 2;
            match CLAUDE_MODELS.iter().position(|m| *m == base) {
                Some(i) if i < half => Some(CLAUDE_MODELS[i + half].to_string()),
                Some(_) => Some(base.to_string()),
                None => None,
            }
        }
    }
}

/// Version und Pruefsumme der installierten CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub version: String,
    pub sha256: String,
}

pub fn binding_for(cli: Cli, model_id: &str, identity: &Identity) -> PreviewBinding {
    PreviewBinding {
        provider: provider_of(cli).to_string(),
        model_id: model_id.to_string(),
        concrete_model: concrete_model(cli, model_id),
        cli_version: identity.version.clone(),
        binary_sha256: identity.sha256.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingState {
    /// Gespeichertes Tupel stimmt mit dem aktuellen ueberein.
    Valid,
    /// Fuer dieses Modell wurde nie bestaetigt.
    Missing,
    /// Version, Programm oder Modell weichen ab: erneut bestaetigen.
    Changed,
}

pub fn binding_state(stored: &[PreviewBinding], current: &PreviewBinding) -> BindingState {
    match stored
        .iter()
        .find(|b| b.provider == current.provider && b.model_id == current.model_id)
    {
        None => BindingState::Missing,
        Some(b) if b == current => BindingState::Valid,
        Some(_) => BindingState::Changed,
    }
}

/// Belege, die zu einem Tupel gehoeren: Faehigkeiten (vollstaendig) und Messung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proof {
    pub caps: IsolationCaps,
    pub measurement: Measurement,
}

/// Warum die Vorschau (noch) nicht freigegeben ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Blocked {
    /// Keine CLI zu finden oder nicht lesbar.
    NoCli,
    /// Das tatsaechlich gestartete Programm ist nicht bestimmbar.
    PayloadUnknown,
    /// Fuer dieses Modell wurde nie bestaetigt.
    BindingMissing,
    /// Version, Programm oder Modell weichen vom bestaetigten Tupel ab.
    BindingChanged,
    /// Fuer diese CLI-Version sind nicht alle Isolationsfaehigkeiten belegt.
    IsolationUnverified,
    /// Keine Messung fuer (CLI, Version, Programm, Modell) auf diesem Geraet.
    MeasurementMissing,
    /// Der Kontext des Modells ist nicht belegt.
    ContextUnknown,
    /// Codex gibt es in der Vorschau nicht (Phase 2, eigenes Issue).
    CodexPreviewNotAvailable,
    /// Das Claude-Programm ist keine native Binaerdatei (npm-, Node- oder
    /// Skript-Launcher): nicht bindbar.
    ClaudeNotNative,
    /// Dieses System kann das geprueft gehaltene Programm nicht ohne
    /// Austauschfenster starten (kein `/proc`, macOS, andere Unix): gesperrt.
    PlatformUnsupported,
}

/// Belege fuer ein Tupel; die erste Luecke gibt den Grund.
pub fn proof_for(evidence: &Evidence, binding: &PreviewBinding) -> Result<Proof, Blocked> {
    if binding.provider != provider_of(Cli::Claude) {
        return Err(Blocked::CodexPreviewNotAvailable);
    }
    let caps = evidence
        .caps_for(&binding.provider, &binding.cli_version)
        .filter(IsolationCaps::complete)
        .ok_or(Blocked::IsolationUnverified)?;
    let measurement = evidence
        .measurement_for(
            &binding.provider,
            &binding.cli_version,
            &binding.binary_sha256,
            &binding.model_id,
        )
        .ok_or(Blocked::MeasurementMissing)?;
    Ok(Proof { caps, measurement })
}

/// Wirkt die Vorschau fuer diesen Aufruf?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PreviewState {
    /// Schalter aus.
    Off,
    /// Schalter an, Tupel gueltig, Isolation und Messung belegt.
    Active,
    /// Schalter an, aber das Tupel fehlt oder weicht ab (oder die CLI ist nicht
    /// zu pruefen): der Schalter wirkt nicht, die Oberflaeche fragt erneut.
    Reconfirm,
    /// Schalter an, Tupel gueltig, aber Isolation oder Messung auf diesem
    /// Geraet fehlen: „Noch nicht freigegeben“.
    NotReleased,
}

/// Ergebnis von [`gate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub state: PreviewState,
    pub blocked: Option<Blocked>,
    pub proof: Option<Proof>,
}

impl Gate {
    fn state(state: PreviewState, blocked: Option<Blocked>) -> Self {
        Self {
            state,
            blocked,
            proof: None,
        }
    }
}

/// Alle Voraussetzungen: Schalter, gueltiges Tupel, vollstaendige Isolation,
/// gueltige Messung. Nur dann `Active` mit Belegen.
pub fn gate(
    enabled: bool,
    stored: &[PreviewBinding],
    current: Option<&PreviewBinding>,
    evidence: &Evidence,
) -> Gate {
    if !enabled {
        return Gate::state(PreviewState::Off, None);
    }
    if !platform_supported() {
        return Gate::state(
            PreviewState::NotReleased,
            Some(Blocked::PlatformUnsupported),
        );
    }
    let Some(current) = current else {
        return Gate::state(PreviewState::Reconfirm, Some(Blocked::NoCli));
    };
    if current.provider != provider_of(Cli::Claude) {
        return Gate::state(
            PreviewState::NotReleased,
            Some(Blocked::CodexPreviewNotAvailable),
        );
    }
    match binding_state(stored, current) {
        BindingState::Missing => {
            Gate::state(PreviewState::Reconfirm, Some(Blocked::BindingMissing))
        }
        BindingState::Changed => {
            Gate::state(PreviewState::Reconfirm, Some(Blocked::BindingChanged))
        }
        BindingState::Valid => match proof_for(evidence, current) {
            Ok(proof) => Gate {
                state: PreviewState::Active,
                blocked: None,
                proof: Some(proof),
            },
            Err(b) => Gate::state(PreviewState::NotReleased, Some(b)),
        },
    }
}

/// Grund aus dem Fehlertext von [`current_binding`] bzw. [`identity`].
pub fn blocked_of_error(err: &str) -> Blocked {
    if err.starts_with("platform_unsupported") {
        Blocked::PlatformUnsupported
    } else if err.starts_with("payload_unknown") {
        Blocked::PayloadUnknown
    } else if err.starts_with("claude_not_native") {
        Blocked::ClaudeNotNative
    } else if err.starts_with("codex_preview_not_available") {
        Blocked::CodexPreviewNotAvailable
    } else {
        Blocked::NoCli
    }
}

// ------------------------------------------------- Programm-Identitaet (IO) --

/// Version je Inhaltshash: der Start von `--version` passiert nicht je Frage.
/// Der Hash selbst wird nie gemerkt, sondern bei jeder Pruefung neu gebildet.
static VERSION_CACHE: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 (hex) aus einem Leser, gelesen in Bloecken.
fn hash_reader(r: &mut impl std::io::Read) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// SHA-256 (hex) einer Datei, gelesen in Bloecken.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    hash_reader(&mut std::fs::File::open(path)?)
}

/// Erste nichtleere Zeile einer `--version`-Ausgabe, auf 120 Zeichen gekuerzt.
pub fn parse_version(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(120).collect())
}

#[cfg(test)]
thread_local! {
    /// Nur Tests: Fake-CLIs sind Skripte; sie duerfen die Pruefung „nativ“ umgehen.
    pub(crate) static TEST_ALLOW_SCRIPT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Hat der Dateikopf die Magic-Bytes von ELF, PE oder Mach-O?
fn native_head(head: &[u8]) -> bool {
    head.starts_with(&[0x7f, b'E', b'L', b'F'])
        || head.starts_with(b"MZ")
        || [
            [0xfe, 0xed, 0xfa, 0xce],
            [0xfe, 0xed, 0xfa, 0xcf],
            [0xce, 0xfa, 0xed, 0xfe],
            [0xcf, 0xfa, 0xed, 0xfe],
            [0xca, 0xfe, 0xba, 0xbe],
        ]
        .iter()
        .any(|m| head.starts_with(m))
}

/// Nur eine NATIVE Claude-Binaerdatei ist bindbar: kein npm-/Node-/Skript-
/// Launcher (Shebang, `.cmd`, `.bat`, `.js`, `cli.js`, ...). Erkannt an
/// Endung und Dateikopf (ELF, PE, Mach-O). Sonst `claude_not_native`.
pub fn check_native(path: &Path) -> Result<(), String> {
    #[cfg(test)]
    if TEST_ALLOW_SCRIPT.with(std::cell::Cell::get) {
        return Ok(());
    }
    let not_native = |why: &str| Err(format!("claude_not_native: {why}"));
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ["cmd", "bat", "js", "mjs", "cjs", "ps1", "sh", "py", "ts"].contains(&ext.as_str()) {
        return not_native("Skript-Endung");
    }
    let mut head = [0u8; 4];
    let read = std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
        .map_err(|e| format!("cli_missing: {e}"))?;
    let head = &head[..read];
    if head.starts_with(b"#!") {
        return not_native("Skript (Shebang)");
    }
    let native = native_head(head);
    if native {
        Ok(())
    } else {
        not_native("kein ELF-, PE- oder Mach-O-Kopf")
    }
}

/// Das Programm, das wirklich laeuft und gehasht wird: die aufgeloeste Datei
/// (Verweise aufgeloest). Nicht bestimmbar: `cli_missing`.
pub fn payload_path(binary: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(binary).map_err(|e| format!("cli_missing: {e}"))
}

/// Kann dieses System genau das geprueft gehaltene Programm starten?
/// Windows: ja (das Handle teilt nur zum Lesen, ein Austausch ist gesperrt).
/// Linux: ja, wenn `/proc/self/fd` da ist (Start ueber den Deskriptor des
/// gehaltenen Handles, [`Pinned::exec_path`]). macOS und andere Unix: nein,
/// ein Start per Pfad liesse ein Austauschfenster zwischen Pruefung und `exec`.
pub fn platform_supported() -> bool {
    #[cfg(windows)]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc/self/fd").is_dir()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// Windows: nur lokale, feste Laufwerke (`DRIVE_FIXED`); keine UNC-Pfade und
/// keine Netzlaufwerke.
#[cfg(windows)]
fn assert_local_fixed(path: &Path) -> Result<(), String> {
    use std::path::{Component, Prefix};
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    // Wert aus WinBase.h; im windows-Crate unter Win32_System_WindowsProgramming.
    const DRIVE_FIXED: u32 = 3;
    let bad = |why: &str| Err(format!("payload_unknown: {why}"));
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return bad("kein Laufwerkspfad");
    };
    let letter = match prefix.kind() {
        Prefix::Disk(l) | Prefix::VerbatimDisk(l) => l,
        _ => return bad("UNC- oder Geraetepfad"),
    };
    let root: Vec<u16> = format!("{}:\\", letter as char)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `root` ist eine nullterminierte UTF-16-Zeichenkette, die den Aufruf ueberlebt.
    let kind = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
    if kind != DRIVE_FIXED {
        return bad("kein festes lokales Laufwerk");
    }
    Ok(())
}

/// Windows: keine Reparse-Komponente (Symlink, Junction, Mount) im Pfad.
#[cfg(windows)]
fn assert_no_reparse(path: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    for part in path.ancestors() {
        if part.file_name().is_none() {
            continue;
        }
        let meta = std::fs::symlink_metadata(part)
            .map_err(|e| format!("payload_unknown: Pfad nicht lesbar ({e})"))?;
        if meta.file_type().is_symlink()
            || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err("payload_unknown: Reparse-Komponente im Pfad".to_string());
        }
    }
    Ok(())
}

/// Normalisierter Vergleichswert eines Windows-Pfads (auch der Rueckgabe von
/// `GetFinalPathNameByHandleW`): `/` zu `\`, Praefix `\\?\` entfernt, ohne
/// abschliessenden `\`, klein geschrieben (Windows vergleicht ohne Gross-/
/// Kleinschreibung). `\\?\UNC\...` bleibt als `unc\...` stehen und gleicht nie
/// einem Laufwerkspfad.
#[cfg(any(windows, test))]
fn normalize_final_path(p: &str) -> String {
    let s = p.replace('/', "\\");
    let s = s.strip_prefix("\\\\?\\").unwrap_or(&s);
    s.trim_end_matches('\\').to_lowercase()
}

/// Windows: Handles auf JEDE Elternkomponente und die Pruefung am Handle.
///
/// Ein Pfad, den man zuerst auf Reparse-Punkte prueft und danach oeffnet, laesst
/// ein Austauschfenster (Junction dazwischen schieben). Deshalb hier: jede
/// Komponente vom Laufwerkswurzel bis zum Zielordner nacheinander oeffnen, am
/// HANDLE pruefen (kein `FILE_ATTRIBUTE_REPARSE_POINT`, Verzeichnis, Endpfad
/// gleich dem erwarteten) und die Handles halten. Geteilt wird nur
/// Lesen/Schreiben, NICHT Loeschen (erschwert Loeschen und Ersetzen). Umbenennen
/// wird dadurch NICHT zuverlaessig verhindert (POSIX-Semantik); es wird ueber
/// den Endpfad der gehaltenen Handles ERKANNT ([`Pinned::check_paths`]).
#[cfg(windows)]
mod winpin {
    use std::fs::File;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};

    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;

    fn unknown(why: &str) -> String {
        format!("payload_unknown: {why}")
    }

    /// Endpfad des Handles (`GetFinalPathNameByHandleW`, normalisiert, DOS-Name).
    pub fn final_path(file: &File) -> Result<String, String> {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED,
        };
        let handle = HANDLE(file.as_raw_handle());
        let mut buf = vec![0u16; 1024];
        for _ in 0..4 {
            // SAFETY: `handle` gehoert `file` und lebt waehrend des Aufrufs; `buf`
            // ist ein beschreibbarer Puffer der angegebenen Laenge.
            // Flags: FILE_NAME_NORMALIZED (0) und VOLUME_NAME_DOS (0, der Standard).
            let n = unsafe { GetFinalPathNameByHandleW(handle, &mut buf, FILE_NAME_NORMALIZED) }
                as usize;
            if n == 0 {
                return Err(unknown("Endpfad des Handles nicht lesbar"));
            }
            if n < buf.len() {
                return Ok(String::from_utf16_lossy(&buf[..n]));
            }
            buf.resize(n + 1, 0);
        }
        Err(unknown("Endpfad des Handles zu lang"))
    }

    /// Endpfad des Handles gleich dem erwarteten Pfad (normalisiert).
    pub fn assert_final(file: &File, expected: &Path) -> Result<(), String> {
        let actual = super::normalize_final_path(&final_path(file)?);
        let want = super::normalize_final_path(&expected.to_string_lossy());
        if actual == want {
            Ok(())
        } else {
            Err(unknown("Endpfad des Handles weicht vom erwarteten Pfad ab"))
        }
    }

    /// Eine Verzeichniskomponente oeffnen und am Handle pruefen.
    pub fn hold_dir(dir: &Path) -> Result<(File, PathBuf), String> {
        let handle = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(dir)
            .map_err(|e| unknown(&format!("Verzeichnis nicht oeffenbar ({e})")))?;
        let attrs = handle
            .metadata()
            .map_err(|e| unknown(&format!("Attribute nicht lesbar ({e})")))?
            .file_attributes();
        if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(unknown("Reparse-Komponente im Pfad"));
        }
        if attrs & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err(unknown("Pfadkomponente ist kein Verzeichnis"));
        }
        assert_final(&handle, dir)?;
        Ok((handle, dir.to_path_buf()))
    }

    /// Alle Elternkomponenten von `file` (Wurzel zuerst) nacheinander halten.
    pub fn hold_chain(file: &Path) -> Result<Vec<(File, PathBuf)>, String> {
        let mut dirs: Vec<&Path> = file.ancestors().skip(1).collect();
        if dirs.is_empty() {
            return Err(unknown("kein Elternordner"));
        }
        dirs.reverse();
        dirs.into_iter().map(hold_dir).collect()
    }

    /// Die Zieldatei am Handle pruefen: weder Reparse-Punkt noch Verzeichnis,
    /// Endpfad gleich dem erwarteten Pfad.
    pub fn verify_file(file: &File, expected: &Path) -> Result<(), String> {
        let attrs = file
            .metadata()
            .map_err(|e| unknown(&format!("Attribute nicht lesbar ({e})")))?
            .file_attributes();
        if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(unknown("Reparse-Punkt als Programmdatei"));
        }
        if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
            return Err(unknown("Programmdatei ist ein Verzeichnis"));
        }
        assert_final(file, expected)
    }
}

/// Datei zum Lesen oeffnen; Windows: GENERIC_READ, Teilen AUSSCHLIESSLICH zum
/// Lesen (FILE_SHARE_READ) und FILE_FLAG_OPEN_REPARSE_POINT (ein Reparse-Punkt
/// wird selbst geoeffnet, nie durchlaufen; [`winpin::verify_file`] lehnt ihn ab).
/// Solange das Handle lebt, scheitern Schreiben und Loeschen; ein Umbenennen
/// wird ueber den Endpfad erkannt ([`Pinned::check_paths`]).
fn open_pinned(path: &Path) -> std::io::Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.share_mode(1); // FILE_SHARE_READ
        opts.custom_flags(winpin::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    opts.open(path)
}

fn hash_handle(mut file: &std::fs::File) -> std::io::Result<String> {
    use std::io::Seek;
    file.seek(std::io::SeekFrom::Start(0))?;
    hash_reader(&mut file)
}

/// Das installierte Original, offen gehalten und gehasht AUS DEM HANDLE. Alle
/// Starts der Vorschau (auch `--version`) laufen ueber diese Stelle:
///
/// * Windows: lokales festes Laufwerk; Handles auf alle Elternkomponenten
///   (ohne FILE_SHARE_DELETE, am Handle auf Reparse-Punkte geprueft, [`winpin`])
///   und auf die Datei (FILE_SHARE_READ, OPEN_REPARSE_POINT, Endpfad gleich dem
///   erwarteten) solange die Bindung lebt; Start mit genau dem geprueften Pfad
///   (Umbenennen wird erkannt, nicht verhindert: [`Pinned::check_paths`]).
/// * Linux: Start ueber `/proc/self/fd/<fd>` des gehaltenen Handles; der Kernel
///   loest den Pfad in `execve` auf, bevor `O_CLOEXEC` den Deskriptor schliesst.
///   Ein Austausch der Datei am Pfad wirkt nicht mehr.
/// * macOS und andere Unix: gesperrt ([`platform_supported`]).
///
/// Grenze (ausserhalb der Garantie): Die Bindung an das Handle verhindert
/// Pfadaustausch, aber keine Aenderung des INHALTS zwischen Hash und `exec`
/// durch denselben Nutzer (Unix: `chmod` und Schreiben in dieselbe Datei). Der
/// Hash nach dem Lauf ([`Pinned::verify`]) verwirft die Antwort dann.
#[derive(Debug)]
pub struct Pinned {
    /// Aufgeloester Pfad der Originaldatei.
    pub path: PathBuf,
    sha256: String,
    handle: std::fs::File,
    /// Windows: Handles auf alle Elternkomponenten (Wurzel bis Zielordner),
    /// gehalten, solange die Bindung lebt.
    #[cfg(windows)]
    dirs: Vec<(std::fs::File, PathBuf)>,
}

impl Pinned {
    /// Oeffnet, prueft (nativ, Plattform) und hasht aus dem Handle.
    pub fn open(source: &Path) -> Result<Self, String> {
        if !platform_supported() {
            return Err("platform_unsupported: Start nicht austauschsicher moeglich".to_string());
        }
        let path = payload_path(source)?;
        #[cfg(windows)]
        let dirs = {
            let absolute = std::path::absolute(source).map_err(|e| format!("cli_missing: {e}"))?;
            assert_no_reparse(&absolute)?;
            assert_local_fixed(&path)?;
            // Zuerst jede Elternkomponente halten und am Handle pruefen, erst dann
            // die Datei oeffnen: zwischen Pruefung und Oeffnen laesst sich nichts
            // mehr austauschen.
            winpin::hold_chain(&path)?
        };
        check_native(&path)?;
        let handle = open_pinned(&path).map_err(|e| format!("cli_missing: {e}"))?;
        #[cfg(windows)]
        winpin::verify_file(&handle, &path)?;
        // Kopf aus dem Handle: es ist genau die Datei, die laeuft.
        #[cfg(test)]
        let skip = TEST_ALLOW_SCRIPT.with(std::cell::Cell::get);
        #[cfg(not(test))]
        let skip = false;
        if !skip {
            use std::io::{Read, Seek};
            let mut head = [0u8; 4];
            let mut h = &handle;
            let n = h
                .seek(std::io::SeekFrom::Start(0))
                .and_then(|_| h.read(&mut head))
                .map_err(|e| format!("cli_missing: {e}"))?;
            if !native_head(&head[..n]) {
                return Err("claude_not_native: kein ELF-, PE- oder Mach-O-Kopf".to_string());
            }
        }
        let sha256 = hash_handle(&handle).map_err(|e| format!("cli_failed: {e}"))?;
        Ok(Self {
            path,
            sha256,
            handle,
            #[cfg(windows)]
            dirs,
        })
    }

    /// SHA-256 (hex) des gehaltenen Programms (aus dem Handle gebildet).
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Der Pfad, der an `exec` geht (siehe Typbeschreibung).
    pub fn exec_path(&self) -> PathBuf {
        #[cfg(test)]
        if TEST_ALLOW_SCRIPT.with(std::cell::Cell::get) {
            // Nur Tests: Fake-CLIs sind Skripte; ein Shebang-Start ueber einen
            // `O_CLOEXEC`-Deskriptor ginge nicht (der Interpreter oeffnet den Pfad
            // erst nach dem Schliessen).
            return self.path.clone();
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            PathBuf::from(format!("/proc/self/fd/{}", self.handle.as_raw_fd()))
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.path.clone()
        }
    }

    /// Version (`--version`), gestartet ueber das gehaltene Handle, je
    /// Inhaltshash gemerkt. Blockiert.
    pub fn version(&self) -> Result<String, String> {
        let cached = VERSION_CACHE.lock().ok().and_then(|c| {
            c.iter()
                .find(|(hash, _)| *hash == self.sha256)
                .map(|(_, v)| v.clone())
        });
        if let Some(version) = cached {
            return Ok(version);
        }
        self.check_paths().map_err(|r| r.error())?;
        let mut cmd = std::process::Command::new(self.exec_path());
        cmd.arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = cmd.spawn().map_err(|e| format!("cli_missing: {e}"))?;
        let out = super::cli::wait_with_timeout(child, std::time::Duration::from_secs(15))?;
        let version = parse_version(&String::from_utf8_lossy(&out.stdout))
            .or_else(|| parse_version(&String::from_utf8_lossy(&out.stderr)))
            .filter(|_| out.status.success())
            .ok_or_else(|| "cli_failed: Version der CLI nicht lesbar".to_string())?;
        if let Ok(mut c) = VERSION_CACHE.lock() {
            c.retain(|(hash, _)| *hash != self.sha256);
            c.push((self.sha256.clone(), version.clone()));
        }
        Ok(version)
    }

    /// Bereitet einen Aufruf vor: das installierte Original, gehalten, mit dem
    /// Hash und der Version der Bindung. Jede Abweichung sperrt.
    pub fn prepare(source: &Path, check: &PreviewCheck) -> Result<Self, Rejection> {
        let fail = |reason| Rejection::llm(reason);
        let pinned = Self::open(source).map_err(|e| {
            if e.starts_with("platform_unsupported") {
                fail("platform_unsupported")
            } else if e.starts_with("claude_not_native") {
                fail("claude_not_native")
            } else {
                fail("payload_unknown")
            }
        })?;
        if !pinned
            .sha256
            .eq_ignore_ascii_case(&check.binding.binary_sha256)
        {
            return Err(fail("binding_changed"));
        }
        match pinned.version() {
            Ok(v) if v == check.binding.cli_version => Ok(pinned),
            Ok(_) => Err(fail("binding_changed")),
            Err(_) => Err(fail("payload_unknown")),
        }
    }

    /// Windows: Endpfad des gehaltenen Datei-Handles und aller Elternhandles gleich
    /// dem erwarteten Pfad? Ein Umbenennen einer Komponente aendert den Endpfad
    /// (das Handle folgt) und wird so ERKANNT (`binding_changed`). Andere
    /// Plattformen: bindet das Handle selbst (Linux), nichts zu pruefen.
    pub fn check_paths(&self) -> Result<(), Rejection> {
        #[cfg(windows)]
        {
            let same = winpin::assert_final(&self.handle, &self.path).is_ok()
                && self
                    .dirs
                    .iter()
                    .all(|(file, expected)| winpin::assert_final(file, expected).is_ok());
            if !same {
                return Err(Rejection::llm("binding_changed"));
            }
        }
        Ok(())
    }

    /// Nach dem Lauf: Endpfade (Windows), Hash aus dem Handle UND frisch vom Pfad.
    pub fn verify(&self) -> Result<(), Rejection> {
        self.check_paths()?;
        let same = hash_handle(&self.handle).is_ok_and(|h| h == self.sha256)
            && sha256_file(&self.path).is_ok_and(|h| h == self.sha256);
        if same {
            Ok(())
        } else {
            Err(Rejection::llm("binding_changed"))
        }
    }
}

/// Version und Pruefsumme der CLI, gebildet und gestartet ueber [`Pinned`].
/// Hasht bei JEDEM Aufruf neu; nur die Version wird je Inhaltshash gemerkt.
/// Nur native Programme. Blockiert: auf einem Blocking-Thread ausfuehren.
pub fn identity(binary: &Path) -> Result<Identity, String> {
    let pinned = Pinned::open(binary)?;
    let version = pinned.version()?;
    pinned
        .verify()
        .map_err(|_| "payload_unknown: Programm waehrend der Pruefung veraendert".to_string())?;
    Ok(Identity {
        version,
        sha256: pinned.sha256().to_string(),
    })
}

/// Aktuelles Tupel fuer ein Modell: sucht die CLI und liest ihre Identitaet.
/// Nur Claude; fuer Codex gibt es in der Vorschau kein Tupel. Blockiert.
pub fn current_binding(cli: Cli, model_id: &str) -> Result<PreviewBinding, String> {
    if cli != Cli::Claude {
        return Err("codex_preview_not_available: Codex gibt es in der Vorschau nicht".to_string());
    }
    let binary = super::cli::locate_native_claude()?;
    Ok(binding_for(cli, model_id, &identity(&binary)?))
}

// ------------------------------------------------------------ Schalter --

/// Liest den Vorschau-Schalter LIVE (nicht den Stand zu Beginn der Frage).
#[derive(Clone)]
pub struct PreviewSwitch(std::sync::Arc<dyn Fn() -> bool + Send + Sync>);

impl PreviewSwitch {
    pub fn new(read: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(read))
    }

    pub fn is_on(&self) -> bool {
        (self.0)()
    }
}

impl std::fmt::Debug for PreviewSwitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewSwitch")
    }
}

// ------------------------------------------------------------- Aufrufpruefung --

/// Alles, was ein Aufruf in der Vorschau einhalten muss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewCheck {
    pub binding: PreviewBinding,
    /// Belegter Kontext des Modells.
    pub context_tokens: u32,
    /// Hoechste Ausgabe inklusive gemessener Reserve ([`max_output`]).
    pub max_output_tokens: u32,
    /// Faehigkeiten und Messung dieser Version.
    pub proof: Proof,
}

impl PreviewCheck {
    pub fn new(
        binding: PreviewBinding,
        context_tokens: u32,
        answer_tokens: u32,
        proof: Proof,
    ) -> Self {
        let max_output_tokens =
            max_output(answer_tokens, proof.measurement.reasoning_reserve_tokens);
        Self {
            binding,
            context_tokens,
            max_output_tokens,
            proof,
        }
    }

    /// Eingabe in Token-Einheiten (gemessener Overhead mit Zuschlag).
    pub fn input_estimate(&self, text_bytes: usize) -> u64 {
        input_estimate(self.proof.measurement.max_overhead_tokens, text_bytes)
    }

    /// Planungsgrenze: [`PREVIEW_MAX_CONTEXT_SHARE`] des Kontexts.
    pub fn limit(&self) -> u64 {
        limit_tokens(self.context_tokens)
    }

    /// Die Vorab-Pruefung: passt der Aufruf unter die Schwelle?
    pub fn fits(&self, text_bytes: usize) -> bool {
        self.input_estimate(text_bytes) + u64::from(self.max_output_tokens) <= self.limit()
    }
}

/// Ablehnung: `code` ist der Code des Buch-Gespraechs (`budget`, `llm_failed`
/// oder `cancelled`), `reason` ein fester Text fuer das Audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejection {
    pub code: &'static str,
    pub reason: &'static str,
}

/// Praefix des Fehlertexts einer Ablehnung: `cli_preview_rejected:<code>:<grund>`.
pub const REJECTED_PREFIX: &str = "cli_preview_rejected";

impl Rejection {
    /// Ablehnung mit dem Code `llm_failed`.
    pub fn llm(reason: &'static str) -> Self {
        Self {
            code: LLM_FAILED,
            reason,
        }
    }

    /// Ablehnung mit dem Code `budget`.
    pub fn budget(reason: &'static str) -> Self {
        Self {
            code: BUDGET,
            reason,
        }
    }

    pub fn error(&self) -> String {
        format!("{REJECTED_PREFIX}:{}:{}", self.code, self.reason)
    }
}

/// Code aus einem Fehlertext einer Ablehnung (`budget`, `cancelled` oder
/// `llm_failed`).
pub fn rejection_code(err: &str) -> Option<&'static str> {
    let rest = err.strip_prefix(REJECTED_PREFIX)?.strip_prefix(':')?;
    if rest.starts_with("budget") {
        Some("budget")
    } else if rest.starts_with("cancelled") {
        Some("cancelled")
    } else {
        Some("llm_failed")
    }
}

const BUDGET: &str = "budget";
const LLM_FAILED: &str = "llm_failed";
const CANCELLED: &str = "cancelled";

/// Codex gibt es in der Vorschau nicht (Phase 2): immer gesperrt.
pub const CODEX_BLOCKED: Rejection = Rejection {
    code: LLM_FAILED,
    reason: "codex_preview_not_available",
};

/// Der Schalter wurde waehrend der Frage ausgeschaltet: Ergebnis verwerfen.
pub const SWITCH_OFF: Rejection = Rejection {
    code: CANCELLED,
    reason: "switch_off",
};

/// Vorab: Passt der Aufruf in das gemessene Budget?
pub fn precheck(check: &PreviewCheck, text_bytes: usize) -> Result<(), Rejection> {
    if check.fits(text_bytes) {
        return Ok(());
    }
    Err(Rejection {
        code: BUDGET,
        reason: "budget_precheck",
    })
}

/// Ist das gemeldete Modell GENAU das gemessene? Kein Teilstring, keine
/// Gross-/Kleinschreibung, kein Alias: nur Gleichheit nach Trimmen.
pub fn model_is_exact(expected: &str, actual: &str) -> bool {
    let (expected, actual) = (expected.trim(), actual.trim());
    !expected.is_empty() && expected == actual
}

/// Nachtraeglich: darf die Antwort verwendet werden? Fail-closed.
pub fn verify(check: &PreviewCheck, reply: &CliReply, input_bytes: usize) -> Result<(), Rejection> {
    let reject = |code, reason| Err(Rejection { code, reason });
    if !check.proof.caps.event_contract {
        return reject(LLM_FAILED, "event_contract");
    }
    if reply.prompt_tokens == 0 || reply.completion_tokens == 0 {
        return reject(LLM_FAILED, "usage_missing");
    }
    let plausible_min = (input_bytes / MIN_BYTES_PER_TOKEN) as u64;
    if reply.prompt_tokens < plausible_min {
        return reject(LLM_FAILED, "usage_implausible");
    }
    let Some(actual) = reply.model.as_deref() else {
        return reject(LLM_FAILED, "model_missing");
    };
    if !model_is_exact(&check.proof.measurement.reported_model, actual) {
        return reject(LLM_FAILED, "model_mismatch");
    }
    match reply.compacted {
        Tri::No => {}
        Tri::Yes => return reject(BUDGET, "compaction"),
        Tri::Unknown => return reject(BUDGET, "compaction_unknown"),
    }
    match reply.truncated {
        Tri::No => {}
        Tri::Yes => return reject(BUDGET, "truncated"),
        Tri::Unknown => return reject(BUDGET, "truncated_unknown"),
    }
    if reply.prompt_tokens + reply.completion_tokens > u64::from(check.context_tokens) {
        return reject(BUDGET, "context_exceeded");
    }
    Ok(())
}

// ------------------------------------------------------------------ Audit --

/// Audit-Zeile ohne Inhalte: Tupel, Modell angefragt und tatsaechlich,
/// Budgetmethode, Status. Gleiches Muster wie die uebrigen `book_talk`-Zeilen.
pub fn audit_line(
    binding: &PreviewBinding,
    requested_model: &str,
    actual_model: Option<&str>,
    status: &str,
) -> String {
    format!(
        "book_talk_preview: provider={} model_requested={} model_actual={} cli_version={} cli_sha256={} method={} status={}",
        binding.provider,
        clean(requested_model),
        actual_model.map_or_else(|| "unknown".to_string(), clean),
        clean(&binding.cli_version),
        binding.binary_sha256,
        BUDGET_METHOD,
        status
    )
}

/// Nur Zeichen, die in einer Modell- oder Versionsangabe vorkommen (kein
/// Zeilenumbruch, keine Leerzeichen): ein fremder Wert kann das Log nicht
/// umbrechen.
fn clean(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric()
                || matches!(c, '.' | '-' | '_' | '@' | '~' | '/' | '(' | ')')
            {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect()
}

/// Bausteine fuer Tests anderer Module: vollstaendige Faehigkeiten und eine
/// passende Messung zu einem Tupel.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn caps() -> IsolationCaps {
        IsolationCaps {
            instructions_blocked: true,
            project_instructions_blocked: true,
            hooks_blocked: true,
            memory_blocked: true,
            tools_blocked: true,
            event_contract: true,
        }
    }

    pub fn measurement(binding: &PreviewBinding, reported: &str) -> Measurement {
        Measurement {
            cli: binding.provider.clone(),
            cli_version: binding.cli_version.clone(),
            binary_sha256: binding.binary_sha256.clone(),
            model: binding.model_id.clone(),
            reported_model: reported.to_string(),
            runs: 10,
            max_overhead_tokens: 30_000,
            reasoning_reserve_tokens: 4_000,
            measured_at: "2026-11-07T10:00:00Z".into(),
            kind: "empirisch".into(),
        }
    }

    pub fn proof(binding: &PreviewBinding, reported: &str) -> Proof {
        Proof {
            caps: caps(),
            measurement: measurement(binding, reported),
        }
    }

    /// Belege fuer alle Tupel (Modell `reported` je Tupel).
    pub fn evidence(bindings: &[(&PreviewBinding, &str)]) -> Evidence {
        let caps_entries: Vec<(&str, &str, IsolationCaps)> = bindings
            .iter()
            .map(|(b, _)| (b.provider.as_str(), b.cli_version.as_str(), caps()))
            .collect();
        Evidence::from_parts(
            &caps_entries,
            bindings.iter().map(|(b, r)| measurement(b, r)).collect(),
        )
    }

    pub fn check(binding: PreviewBinding, reported: &str, context: u32) -> PreviewCheck {
        let proof = proof(&binding, reported);
        PreviewCheck::new(binding, context, 600, proof)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures;
    use super::*;

    const V: &str = "2.1.318 (Claude Code)";

    fn ident(version: &str, tag: &str) -> Identity {
        Identity {
            version: version.into(),
            sha256: tag.repeat(32),
        }
    }

    fn binding() -> PreviewBinding {
        binding_for(Cli::Claude, "sonnet", &ident(V, "ab"))
    }

    fn check(context: u32) -> PreviewCheck {
        fixtures::check(binding(), "claude-sonnet-5-5", context)
    }

    fn reply(input: u64, output: u64, model: Option<&str>) -> CliReply {
        CliReply {
            text: "synthetisch".into(),
            prompt_tokens: input,
            completion_tokens: output,
            model: model.map(str::to_string),
            compacted: Tri::No,
            truncated: Tri::No,
        }
    }

    #[test]
    fn the_budget_uses_the_measured_overhead_and_a_fixed_half_of_the_context() {
        // Overhead 30000 * 1,2 = 36000, Schwelle fest 50 % von 200000, Reserve 4000.
        let c = check(200_000);
        assert_eq!(PREVIEW_MAX_CONTEXT_SHARE, 0.5);
        assert_eq!(c.input_estimate(1_000), 37_000);
        assert_eq!(c.limit(), 100_000);
        assert_eq!(c.max_output_tokens, 600 + 4_000);
        let room = 100_000 - 36_000 - u64::from(c.max_output_tokens);
        assert!(c.fits(room as usize));
        assert!(!c.fits(room as usize + 1));
        // Ein anderer Overhead ergibt eine andere Grenze; die Schwelle bleibt.
        let mut other = check(200_000);
        other.proof.measurement.max_overhead_tokens = 10_000;
        assert_eq!(other.limit(), 100_000);
        assert_eq!(other.input_estimate(0), 12_000);
        // Nie mehr als die Haelfte, auch nicht bei grossem Kontext.
        assert_eq!(limit_tokens(1_000_000), 500_000);
        let mut small = check(32_000);
        assert!(!small.fits(0));
        small.proof.measurement.max_overhead_tokens = 1_000;
        assert!(small.fits(0));
    }

    #[test]
    fn the_reasoning_reserve_is_part_of_the_output() {
        assert_eq!(max_output(600, 3_100), 3_700);
        assert_eq!(max_output(u32::MAX, 5), u32::MAX);
    }

    #[test]
    fn only_a_proven_context_counts() {
        assert_eq!(context_tokens(Cli::Claude, None), Some(200_000));
        assert_eq!(context_tokens(Cli::Claude, Some(1_000_000)), Some(200_000));
        assert_eq!(context_tokens(Cli::Claude, Some(100_000)), Some(100_000));
        assert_eq!(context_tokens(Cli::Codex, None), None);
        assert_eq!(context_tokens(Cli::Codex, Some(272_000)), None);
    }

    #[test]
    fn aliases_get_a_concrete_model_where_known() {
        assert_eq!(
            concrete_model(Cli::Claude, "sonnet@high").as_deref(),
            Some("claude-sonnet-5-5")
        );
        assert_eq!(
            concrete_model(Cli::Claude, "claude-opus-5-5").as_deref(),
            Some("claude-opus-5-5")
        );
        assert_eq!(concrete_model(Cli::Claude, "unbekannt"), None);
        assert_eq!(concrete_model(Cli::Codex, "gpt-6.1-sol~fast"), None);
    }

    #[test]
    fn a_changed_tuple_invalidates_the_approval() {
        let stored = vec![binding()];
        assert_eq!(binding_state(&stored, &binding()), BindingState::Valid);
        let mut newer = binding();
        newer.cli_version = "2.1.319 (Claude Code)".into();
        assert_eq!(binding_state(&stored, &newer), BindingState::Changed);
        let mut swapped = binding();
        swapped.binary_sha256 = "cd".repeat(32);
        assert_eq!(binding_state(&stored, &swapped), BindingState::Changed);
        let mut other_model = binding();
        other_model.model_id = "opus".into();
        assert_eq!(binding_state(&stored, &other_model), BindingState::Missing);
        assert_eq!(binding_state(&[], &binding()), BindingState::Missing);
    }

    #[test]
    fn ready_needs_switch_binding_isolation_and_measurement() {
        let b = binding();
        let stored = vec![b.clone()];
        let full = fixtures::evidence(&[(&b, "claude-sonnet-5-5")]);
        let blocked = |g: Gate| (g.state, g.blocked);
        // Alles belegt.
        let g = gate(true, &stored, Some(&b), &full);
        assert_eq!(blocked(g.clone()), (PreviewState::Active, None));
        assert!(g.proof.is_some());
        // Schalter aus.
        assert_eq!(
            blocked(gate(false, &stored, Some(&b), &full)),
            (PreviewState::Off, None)
        );
        // Kein Tupel / CLI nicht pruefbar.
        assert_eq!(
            blocked(gate(true, &stored, None, &full)),
            (PreviewState::Reconfirm, Some(Blocked::NoCli))
        );
        assert_eq!(
            blocked(gate(true, &[], Some(&b), &full)),
            (PreviewState::Reconfirm, Some(Blocked::BindingMissing))
        );
        let mut newer = b.clone();
        newer.cli_version = "9".into();
        assert_eq!(
            blocked(gate(true, &stored, Some(&newer), &full)),
            (PreviewState::Reconfirm, Some(Blocked::BindingChanged))
        );
        // Ohne Belege: gesperrt, mit Grund.
        assert_eq!(
            blocked(gate(true, &stored, Some(&b), &Evidence::default())),
            (
                PreviewState::NotReleased,
                Some(Blocked::IsolationUnverified)
            )
        );
        // Faehigkeiten nur teilweise: gesperrt.
        let mut partial = fixtures::caps();
        partial.hooks_blocked = false;
        let ev = Evidence::from_parts(
            &[(b.provider.as_str(), b.cli_version.as_str(), partial)],
            vec![fixtures::measurement(&b, "claude-sonnet-5-5")],
        );
        assert_eq!(
            blocked(gate(true, &stored, Some(&b), &ev)),
            (
                PreviewState::NotReleased,
                Some(Blocked::IsolationUnverified)
            )
        );
        // Faehigkeiten da, Messung fehlt.
        let ev = Evidence::from_parts(
            &[(
                b.provider.as_str(),
                b.cli_version.as_str(),
                fixtures::caps(),
            )],
            vec![],
        );
        assert_eq!(
            blocked(gate(true, &stored, Some(&b), &ev)),
            (PreviewState::NotReleased, Some(Blocked::MeasurementMissing))
        );
        // Messung fuer ein anderes Modell oder anderes Programm zaehlt nicht.
        let mut other = b.clone();
        other.model_id = "opus".into();
        let ev = Evidence::from_parts(
            &[(
                b.provider.as_str(),
                b.cli_version.as_str(),
                fixtures::caps(),
            )],
            vec![fixtures::measurement(&other, "claude-opus-5-5")],
        );
        assert_eq!(
            blocked(gate(true, &stored, Some(&b), &ev)),
            (PreviewState::NotReleased, Some(Blocked::MeasurementMissing))
        );
        let mut other_hash = b.clone();
        other_hash.binary_sha256 = "cd".repeat(32);
        let ev = fixtures::evidence(&[(&other_hash, "claude-sonnet-5-5")]);
        assert_eq!(
            blocked(gate(true, &stored, Some(&b), &ev)),
            (PreviewState::NotReleased, Some(Blocked::MeasurementMissing))
        );
    }

    #[test]
    fn precheck_rejects_with_the_budget_code() {
        let c = check(50_000);
        let err = precheck(&c, 1).unwrap_err();
        assert_eq!(err.code, "budget");
        assert_eq!(rejection_code(&err.error()), Some("budget"));
        assert!(precheck(&check(200_000), 10_000).is_ok());
    }

    #[test]
    fn a_good_reply_passes() {
        let r = reply(40_000, 300, Some("claude-sonnet-5-5"));
        assert_eq!(verify(&check(200_000), &r, 10_000), Ok(()));
    }

    #[test]
    fn missing_or_implausible_usage_fails_closed() {
        let c = check(200_000);
        for r in [
            reply(0, 300, Some("claude-sonnet-5-5")),
            reply(40_000, 0, Some("claude-sonnet-5-5")),
        ] {
            let e = verify(&c, &r, 10_000).unwrap_err();
            assert_eq!((e.code, e.reason), ("llm_failed", "usage_missing"));
        }
        // 10 Eingabetoken bei 160000 Bytes Prompt kann nicht stimmen.
        let r = reply(10, 300, Some("claude-sonnet-5-5"));
        let e = verify(&c, &r, 160_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("llm_failed", "usage_implausible"));
    }

    #[test]
    fn a_missing_or_inexact_model_is_rejected() {
        let c = check(200_000);
        // Ohne gemeldetes Modell: abgelehnt (frueher erlaubt).
        let e = verify(&c, &reply(40_000, 300, None), 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("llm_failed", "model_missing"));
        let e = verify(&c, &reply(40_000, 300, Some("  ")), 10_000).unwrap_err();
        assert_eq!(e.reason, "model_mismatch");
        for wrong in [
            "claude-haiku-4-5-20251001",
            // Teilstring-Treffer zaehlen nicht mehr.
            "claude-sonnet-5-5-preview",
            "xclaude-sonnet-5-5",
            "sonnet",
            "CLAUDE-SONNET-5-5",
        ] {
            let e = verify(&c, &reply(40_000, 300, Some(wrong)), 10_000).unwrap_err();
            assert_eq!(
                (e.code, e.reason),
                ("llm_failed", "model_mismatch"),
                "{wrong}"
            );
        }
        assert!(model_is_exact("claude-sonnet-5-5", " claude-sonnet-5-5\n"));
        assert!(!model_is_exact("", ""));
        assert!(!model_is_exact("gpt-6.1-sol", "gpt-6.1-sol-mini"));
    }

    #[test]
    fn unknown_is_never_success() {
        let c = check(200_000);
        let mut r = reply(40_000, 300, Some("claude-sonnet-5-5"));
        r.compacted = Tri::Unknown;
        assert_eq!(
            verify(&c, &r, 10_000).unwrap_err().reason,
            "compaction_unknown"
        );
        r.compacted = Tri::Yes;
        let e = verify(&c, &r, 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("budget", "compaction"));
        r.compacted = Tri::No;
        r.truncated = Tri::Unknown;
        assert_eq!(
            verify(&c, &r, 10_000).unwrap_err().reason,
            "truncated_unknown"
        );
        r.truncated = Tri::Yes;
        let e = verify(&c, &r, 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("budget", "truncated"));
        // Ohne belegten Ereignisvertrag der Version ist „nein“ nichts wert.
        let mut c2 = check(200_000);
        c2.proof.caps.event_contract = false;
        let ok = reply(40_000, 300, Some("claude-sonnet-5-5"));
        assert_eq!(
            verify(&c2, &ok, 10_000).unwrap_err().reason,
            "event_contract"
        );
        assert_eq!(Tri::default(), Tri::Unknown);
    }

    #[test]
    fn exceeding_the_context_is_a_budget_failure() {
        let c = check(100_000);
        let over = reply(99_800, 300, Some("claude-sonnet-5-5"));
        let e = verify(&c, &over, 10_000).unwrap_err();
        assert_eq!((e.code, e.reason), ("budget", "context_exceeded"));
    }

    #[test]
    fn the_switch_rejection_maps_to_cancelled() {
        assert_eq!(
            SWITCH_OFF.error(),
            "cli_preview_rejected:cancelled:switch_off"
        );
        assert_eq!(rejection_code(&SWITCH_OFF.error()), Some("cancelled"));
        assert_eq!(rejection_code("cli_failed: x"), None);
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let f = flag.clone();
        let switch = PreviewSwitch::new(move || f.load(std::sync::atomic::Ordering::SeqCst));
        assert!(switch.is_on());
        flag.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(!switch.is_on());
    }

    #[test]
    fn the_audit_line_has_no_content_and_cannot_break_the_log() {
        let line = audit_line(
            &binding(),
            "sonnet\nFrage: geheim",
            Some("claude-sonnet-5-5"),
            "ok",
        );
        assert!(!line.contains('\n'));
        assert!(!line.contains("Frage:"));
        for part in [
            "provider=claude_cli",
            "model_requested=sonnet_Frage__geheim",
            "model_actual=claude-sonnet-5-5",
            "cli_version=2.1.318_(Claude_Code)",
            "method=empirical",
            "status=ok",
        ] {
            assert!(line.contains(part), "{part} fehlt in {line}");
        }
        assert!(line.contains(&"ab".repeat(32)));
    }

    #[test]
    fn version_and_hash_are_read() {
        assert_eq!(
            parse_version("\n 2.1.318 (Claude Code)\nmehr").as_deref(),
            Some("2.1.318 (Claude Code)")
        );
        assert_eq!(parse_version("  \n"), None);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("cli");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            sha256_file(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn a_plain_program_is_hashed_itself_through_links() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("versions").join("claude");
        write(&real, b"native");
        let got = payload_path(&real).unwrap();
        assert_eq!(got, std::fs::canonicalize(&real).unwrap());
        #[cfg(unix)]
        {
            let link = dir.path().join("bin").join("claude");
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert_eq!(payload_path(&link).unwrap(), got);
        }
        assert!(payload_path(&dir.path().join("fehlt")).is_err());
    }

    #[cfg(unix)]
    fn fake_version_cli(dir: &Path, version: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!("#!/bin/sh\n# {body}\n[ \"$1\" = \"--version\" ] && echo \"{version}\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[cfg(unix)]
    fn rebind(bin: &Path, expected: &PreviewBinding) -> Result<(), Rejection> {
        let check = fixtures::check(expected.clone(), "claude-sonnet-5-5", 200_000);
        Pinned::prepare(bin, &check).map(|_| ())
    }

    #[cfg(unix)]
    #[test]
    fn the_hash_is_recomputed_and_a_swapped_program_breaks_the_binding() {
        TEST_ALLOW_SCRIPT.with(|f| f.set(true));
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_version_cli(dir.path(), "7.7.7 (Claude Code)", "eins");
        let first = identity(&bin).unwrap();
        assert_eq!(first.version, "7.7.7 (Claude Code)");
        let bound = binding_for(Cli::Claude, "sonnet", &first);
        assert_eq!(rebind(&bin, &bound), Ok(()));
        // Anderer Inhalt, gleiche Groesse, gleicher Pfad: der Hash aendert sich
        // (kein Cache ueber Pfad/Zeit/Groesse).
        fake_version_cli(dir.path(), "7.7.7 (Claude Code)", "zwei");
        let second = identity(&bin).unwrap();
        assert_ne!(first.sha256, second.sha256);
        let err = rebind(&bin, &bound).unwrap_err();
        assert_eq!((err.code, err.reason), ("llm_failed", "binding_changed"));
        // Neue Version bei gleichem Pfad: ebenfalls abgewichen.
        fake_version_cli(dir.path(), "7.7.8 (Claude Code)", "eins");
        assert_eq!(rebind(&bin, &bound).unwrap_err().reason, "binding_changed");
        // Programm weg: nicht bestimmbar.
        std::fs::remove_file(&bin).unwrap();
        assert_eq!(rebind(&bin, &bound).unwrap_err().reason, "payload_unknown");
    }

    #[test]
    fn only_native_claude_programs_are_bindable() {
        let dir = tempfile::tempdir().unwrap();
        let mut cases: Vec<(&str, &str, &[u8], bool)> = vec![
            ("elf", "claude", b"\x7fELF\x02\x01\x01\x00rest", true),
            ("pe", "claude.exe", b"MZ\x90\x00rest", true),
            ("macho", "claude", b"\xcf\xfa\xed\xferest", true),
            ("shebang", "claude", b"#!/usr/bin/env node\n", false),
            ("cmd", "claude.cmd", b"@echo off", false),
            ("js", "cli.js", b"\x7fELF", false),
            ("zufall", "claude", b"irgendwas", false),
        ];
        cases.push(("leer", "claude", b"", false));
        for (name, file, bytes, ok) in cases {
            let path = dir.path().join(name).join(file);
            write(&path, bytes);
            let got = check_native(&path);
            assert_eq!(got.is_ok(), ok, "{name}: {got:?}");
            if let Err(e) = got {
                assert!(e.starts_with("claude_not_native"), "{e}");
                assert_eq!(blocked_of_error(&e), Blocked::ClaudeNotNative);
            }
        }
    }

    #[cfg(unix)]
    fn bound(bin: &Path) -> (Identity, PreviewCheck) {
        let id = identity(bin).unwrap();
        let binding = binding_for(Cli::Claude, "sonnet", &id);
        let check = fixtures::check(binding, "claude-sonnet-5-5", 200_000);
        (id, check)
    }

    #[cfg(unix)]
    #[test]
    fn the_preview_binds_and_starts_the_installed_original() {
        TEST_ALLOW_SCRIPT.with(|f| f.set(true));
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_version_cli(dir.path(), "7.7.7 (Claude Code)", "eins");
        let (id, check) = bound(&bin);
        assert_eq!(id.sha256, sha256_file(&bin).unwrap());
        let pinned = Pinned::prepare(&bin, &check).unwrap();
        assert_eq!(pinned.path, std::fs::canonicalize(&bin).unwrap());
        assert_eq!(pinned.sha256(), id.sha256);
        assert_eq!(pinned.verify(), Ok(()));
        // Keine Kopie, kein Cache-Ordner.
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "nur das Original");
    }

    #[cfg(unix)]
    #[test]
    fn a_changed_or_updated_original_blocks_the_start() {
        TEST_ALLOW_SCRIPT.with(|f| f.set(true));
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_version_cli(dir.path(), "7.7.7 (Claude Code)", "eins");
        let (_, check) = bound(&bin);
        let reason = |c: &PreviewCheck| Pinned::prepare(&bin, c).unwrap_err().reason;
        // Anderer Inhalt, gleiche Version: Hash weicht ab.
        fake_version_cli(dir.path(), "7.7.7 (Claude Code)", "geaendert");
        assert_eq!(reason(&check), "binding_changed");
        // Update: neue Version.
        fake_version_cli(dir.path(), "7.7.8 (Claude Code)", "update");
        assert_eq!(reason(&check), "binding_changed");
        // Fehlt: nicht bestimmbar.
        std::fs::remove_file(&bin).unwrap();
        assert_eq!(reason(&check), "payload_unknown");
    }

    #[cfg(unix)]
    #[test]
    fn a_change_of_the_original_after_the_start_is_detected() {
        TEST_ALLOW_SCRIPT.with(|f| f.set(true));
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_version_cli(dir.path(), "7.7.7 (Claude Code)", "eins");
        let (_, check) = bound(&bin);
        let pinned = Pinned::prepare(&bin, &check).unwrap();
        std::fs::write(&bin, "anders").unwrap();
        assert_eq!(pinned.verify().unwrap_err().reason, "binding_changed");
    }

    /// Natives Programm (Systemkopie) als `Pinned`, ohne Versionsabfrage.
    #[cfg(target_os = "linux")]
    fn native_pinned(dir: &Path, system: &str) -> Pinned {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("claude");
        std::fs::copy(system, &path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Pinned::open(&path).unwrap()
    }

    #[cfg(target_os = "linux")]
    fn system_binary(name: &str) -> String {
        ["/bin", "/usr/bin"]
            .iter()
            .map(|d| format!("{d}/{name}"))
            .find(|p| Path::new(p).is_file())
            .unwrap_or_else(|| panic!("{name} fehlt"))
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_starts_the_held_handle_not_the_path_even_after_a_swap() {
        use std::os::unix::process::CommandExt;
        let dir = tempfile::tempdir().unwrap();
        // Gehalten und gehasht: `true` (Exit 0).
        let launch = native_pinned(dir.path(), &system_binary("true"));
        let exec = launch.exec_path();
        assert!(exec.starts_with("/proc/self/fd/"), "{exec:?}");
        // Austauschfenster: am Kopiepfad liegt jetzt `false` (Exit 1).
        std::fs::remove_file(&launch.path).unwrap();
        std::fs::copy(system_binary("false"), &launch.path).unwrap();
        // Per Pfad wuerde das ausgetauschte Programm laufen ...
        let by_path = std::process::Command::new(&launch.path).status().unwrap();
        assert!(!by_path.success(), "Gegenprobe: Pfadstart startet false");
        // ... ueber den Deskriptor (O_CLOEXEC am Handle) das gehaltene `true`.
        let by_fd = std::process::Command::new(&exec).status().unwrap();
        assert!(by_fd.success(), "der gehaltene Inhalt muss laufen");
        // Auch mit eigener Prozessgruppe, wie im Aufruf (anderer Startweg).
        let mut cmd = std::process::Command::new(&exec);
        cmd.process_group(0);
        assert!(cmd.status().unwrap().success());
        // Und mehrfach nacheinander (der Deskriptor bleibt im Elternprozess).
        for _ in 0..3 {
            assert!(std::process::Command::new(&exec)
                .status()
                .unwrap()
                .success());
        }
        // Der ausgetauschte Pfad faellt bei der Pruefung nach dem Lauf auf.
        assert_eq!(launch.verify().unwrap_err().reason, "binding_changed");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_held_descriptor_is_close_on_exec() {
        let dir = tempfile::tempdir().unwrap();
        let launch = native_pinned(dir.path(), &system_binary("true"));
        let fd = std::os::fd::AsRawFd::as_raw_fd(&launch.handle);
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert!(
            flags >= 0 && flags & libc::FD_CLOEXEC != 0,
            "O_CLOEXEC erwartet"
        );
    }

    #[test]
    fn the_gate_blocks_where_the_held_program_cannot_be_started_safely() {
        let b = binding_for(Cli::Claude, "sonnet", &ident("2.2.2 (Claude Code)", "ab"));
        let stored = vec![b.clone()];
        let ev = fixtures::evidence(&[(&b, "sonnet")]);
        let g = gate(true, &stored, Some(&b), &ev);
        if platform_supported() {
            assert_eq!(g.state, PreviewState::Active);
        } else {
            assert_eq!(
                (g.state, g.blocked),
                (
                    PreviewState::NotReleased,
                    Some(Blocked::PlatformUnsupported)
                )
            );
        }
        assert_eq!(
            blocked_of_error("platform_unsupported: x"),
            Blocked::PlatformUnsupported
        );
        // Windows und Linux (mit /proc) gelten, macOS und andere Unix nicht.
        assert_eq!(
            cfg!(windows) || (cfg!(target_os = "linux") && Path::new("/proc/self/fd").is_dir()),
            platform_supported()
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_holding_the_handle_blocks_replacing_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude.exe");
        std::fs::write(&path, b"MZ\x90\x00 programm").unwrap();
        let held = open_pinned(&path).unwrap();
        // Schreiben, Loeschen und Umbenennen scheitern, solange das Handle lebt.
        assert!(std::fs::write(&path, b"MZ anders").is_err());
        assert!(std::fs::remove_file(&path).is_err());
        assert_eq!(hash_handle(&held).unwrap(), sha256_file(&path).unwrap());
    }

    #[test]
    fn final_paths_are_compared_normalized_and_case_insensitive() {
        let a = normalize_final_path(r"\\?\C:\Users\Wolff\.local\bin\Claude.exe");
        assert_eq!(
            a,
            normalize_final_path("c:/users/wolff/.local/bin/claude.exe")
        );
        assert_eq!(
            normalize_final_path(r"\\?\C:\"),
            normalize_final_path(r"C:\")
        );
        assert_eq!(normalize_final_path(r"\\?\C:\a\"), "c:\\a");
        // Ein anderer Ordner oder ein UNC-Pfad gleicht nie.
        assert_ne!(
            normalize_final_path(r"\\?\C:\a\b"),
            normalize_final_path(r"C:\a\c")
        );
        assert_ne!(
            normalize_final_path(r"\\?\UNC\server\share\x"),
            normalize_final_path(r"C:\server\share\x")
        );
    }

    #[cfg(windows)]
    fn mklink_junction(link: &Path, target: &Path) -> bool {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[cfg(windows)]
    #[test]
    fn windows_renaming_a_held_parent_is_detected() {
        let root = tempfile::tempdir().unwrap();
        let outer = root.path().join("aussen");
        let inner = outer.join("innen");
        std::fs::create_dir_all(&inner).unwrap();
        let exe = inner.join("claude.exe");
        std::fs::write(&exe, b"MZ\x90\x00 programm").unwrap();
        let exe = std::fs::canonicalize(&exe).unwrap();
        let chain = winpin::hold_chain(&exe).unwrap();
        assert!(chain.len() >= 3, "Wurzel bis Zielordner");
        let intact = |chain: &[(std::fs::File, PathBuf)]| {
            chain
                .iter()
                .all(|(f, p)| winpin::assert_final(f, p).is_ok())
        };
        assert!(intact(&chain));
        // Umbenennen wird nicht verhindert (POSIX-Semantik), aber erkannt: das
        // Handle folgt, sein Endpfad weicht ab. Scheitert es doch, bleibt alles heil.
        let renamed = std::fs::rename(&outer, root.path().join("anders")).is_ok();
        assert_eq!(intact(&chain), !renamed);
    }

    #[cfg(windows)]
    #[test]
    fn windows_a_reparse_component_or_target_is_blocked() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("echt");
        std::fs::create_dir_all(&real).unwrap();
        let exe = real.join("claude.exe");
        std::fs::write(&exe, b"MZ\x90\x00 programm").unwrap();
        // Junction als Elternkomponente: am Handle erkannt, auch ohne vorherige Pfadpruefung.
        let link = root.path().join("verweis");
        assert!(mklink_junction(&link, &real), "mklink /J nicht moeglich");
        let err = winpin::hold_chain(&link.join("claude.exe")).unwrap_err();
        assert!(err.contains("Reparse"), "{err}");
        let err = Pinned::open(&link.join("claude.exe")).unwrap_err();
        assert!(err.starts_with("payload_unknown"), "{err}");
        // Symbolische Verknuepfung als Zieldatei (braucht ggf. ein Recht; sonst nur die Junction).
        let file_link = real.join("link.exe");
        if std::os::windows::fs::symlink_file(&exe, &file_link).is_ok() {
            let file = open_pinned(&file_link).unwrap();
            let err = winpin::verify_file(&file, &file_link).unwrap_err();
            assert!(err.contains("Reparse"), "{err}");
        }
        // Die echte Datei besteht die Pruefung am Handle, ihr Endpfad passt.
        let canon = std::fs::canonicalize(&exe).unwrap();
        let file = open_pinned(&canon).unwrap();
        winpin::verify_file(&file, &canon).unwrap();
        assert!(winpin::verify_file(&file, &real.join("anders.exe")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_pinned_detects_a_renamed_parent_in_verify() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("programme");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("claude.exe");
        std::fs::write(&exe, b"MZ\x90\x00 programm").unwrap();
        let pinned = Pinned::open(&exe).unwrap();
        assert_eq!(pinned.check_paths(), Ok(()));
        assert_eq!(pinned.verify(), Ok(()));
        let renamed = std::fs::rename(&dir, root.path().join("anders")).is_ok();
        if renamed {
            assert_eq!(pinned.check_paths().unwrap_err().reason, "binding_changed");
            assert_eq!(pinned.verify().unwrap_err().reason, "binding_changed");
        } else {
            assert_eq!(pinned.verify(), Ok(()));
        }
    }

    #[test]
    fn codex_is_blocked_at_every_preview_entry() {
        let b = binding_for(Cli::Codex, "gpt-6.1-sol", &ident("codex-cli 9", "cd"));
        let stored = vec![b.clone()];
        // Tor: auch mit Schalter, Tupel, Faehigkeiten und Messung.
        let ev = fixtures::evidence(&[(&b, "gpt-6.1-sol")]);
        let g = gate(true, &stored, Some(&b), &ev);
        assert_eq!(
            (g.state, g.blocked),
            (
                PreviewState::NotReleased,
                Some(Blocked::CodexPreviewNotAvailable)
            )
        );
        assert!(g.proof.is_none());
        assert_eq!(
            proof_for(&ev, &b).unwrap_err(),
            Blocked::CodexPreviewNotAvailable
        );
        // Tupel und Kontext gibt es fuer Codex nicht.
        assert!(current_binding(Cli::Codex, "gpt-6.1-sol")
            .unwrap_err()
            .starts_with("codex_preview_not_available"));
        assert_eq!(context_tokens(Cli::Codex, Some(272_000)), None);
        assert_eq!(
            blocked_of_error("codex_preview_not_available: x"),
            Blocked::CodexPreviewNotAvailable
        );
    }
}
