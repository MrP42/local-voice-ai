//! Sperrklinke zu lva#872: Verhalten, das Endpoint-Schutzsoftware (EDR) als
//! schadsoftwareaehnlich einstuft, darf nicht unbemerkt neu entstehen.
//!
//! Die Tests zerlegen den Produktionsquelltext der Crate in Woerter und
//! Zeichenketten (Kommentare fallen weg) und vergleichen die Funde mit einer
//! Positivliste. Jede Abweichung laesst den Test scheitern, in BEIDE Richtungen:
//!
//! - mehr Funde als erlaubt (`NEU`): neues EDR-relevantes Verhalten. Erst die
//!   Alternative pruefen (Win32-API statt Systemwerkzeug, kein Hook, kein Zugriff
//!   auf fremde Prozesse). Nur wenn es keine gibt, den Eintrag mit Issue-Verweis
//!   und Begruendung ergaenzen (Regel: apps/local-voice/AGENTS.md, Abschnitt
//!   "EDR-relevantes Verhalten").
//! - weniger Funde als erlaubt (`ENTFALLEN`): gut. Den Eintrag in derselben
//!   Aenderung kuerzen oder loeschen, damit die Liste nur schrumpft.
//!
//! Testcode zaehlt nicht mit. Was Testcode ist, folgt aus dem Quelltext selbst:
//! Elemente hinter `#[cfg(test)]`, `#[cfg(all(test, ..))]` oder `#[test]` und alle
//! Dateien, deren `mod`-Zeile so markiert ist. `#[cfg(any(test, ..))]` ist auch im
//! Release aktiv und zaehlt als Produktionscode.
//!
//! Grenzen: Die Pruefung ist ein Stolperdraht fuer die ueblichen Schreibweisen,
//! kein Beweis. Zur Laufzeit zusammengesetzte Programmnamen, Zahlen statt
//! Win32-Konstanten und Verhalten in Abhaengigkeiten sieht sie nicht. Dafuer sind
//! das Review und die Pruefung der fertigen Programmdateien zustaendig (lva#878).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// (Datei relativ zu `src-tauri`, Stichwort, erlaubte Anzahl Funde, Grund mit Issue).
type Erlaubt = (&'static str, &'static str, usize, &'static str);

/// Windows-Systemwerkzeuge mit unverwechselbarem Namen. Eine Zeichenkette zaehlt,
/// wenn sie als Ganzes das Werkzeug nennt (auch mit Pfad oder `.exe`) oder eine
/// Befehlszeile damit beginnt (`"taskkill /PID .."`), egal wo sie steht.
const EINDEUTIGE_WERKZEUGE: &[&str] = &[
    "bcdedit",
    "bitsadmin",
    "cacls",
    "certutil",
    "cmdkey",
    "comspec",
    "cscript",
    "dsquery",
    "esentutl",
    "findstr",
    "forfiles",
    "fsutil",
    "icacls",
    "ipconfig",
    "mshta",
    "msiexec",
    "netsh",
    "netstat",
    "nltest",
    "nslookup",
    "powershell",
    "pwsh",
    "quser",
    "qwinsta",
    "regedit",
    "regsvr32",
    "runas",
    "rundll32",
    "schtasks",
    "systeminfo",
    "takeown",
    "taskkill",
    "tasklist",
    "tftp",
    "vssadmin",
    "wevtutil",
    "whoami",
    "wmic",
    "wscript",
];

/// Werkzeuge, deren Name auch ein gewoehnliches Wort ist. Sie zaehlen mit `.exe`,
/// mit Windows-Systempfad, als Befehlszeile mit Schalter (`"cmd /C .."`), direkt
/// als Argument von `..::new("cmd")` und als ganze Zeichenkette in Dateien, die
/// selbst Prozesse starten.
const MEHRDEUTIGE_WERKZEUGE: &[&str] = &[
    "arp", "at", "attrib", "cipher", "cmd", "curl", "explorer", "ftp", "net", "net1", "ping",
    "reg", "route", "sc", "where",
];

/// Woerter, an denen eine Datei erkannt wird, die selbst Prozesse startet.
const PROZESSSTART: &[&str] = &[
    "Command",
    "CreateProcessA",
    "CreateProcessW",
    "ShellExecuteA",
    "ShellExecuteExW",
    "ShellExecuteW",
    "WinExec",
];

/// Heutige Ausnahmen. Abbau: lva#877 (E3). Die Liste darf nur schrumpfen.
const ERLAUBTE_SYSTEMWERKZEUGE: &[Erlaubt] = &[
    (
        "src/commands/audio.rs",
        "cmd",
        1,
        "lva#877: Einstellungsseite oeffnen, wird ShellExecuteW",
    ),
    (
        "src/commands/pages.rs",
        "explorer",
        1,
        "lva#877: Ordner anzeigen, wird ShellExecuteW",
    ),
    (
        "src/managers/llm/cli.rs",
        "cmd",
        2,
        "lva#877: 1 Start bleibt (Batch-Starter eines CLI nach Klick auf Anmelden), 1 Vergleich der Dateiendung",
    ),
    (
        "src/managers/llm/cli.rs",
        "taskkill",
        1,
        "lva#877: Prozessbaum beenden, wird Job-Objekt",
    ),
    (
        "src/managers/llm/preview.rs",
        "cmd",
        1,
        "lva#872: bleibt, Liste von Dateiendungen, kein Prozessstart",
    ),
    (
        "src/managers/llm/server.rs",
        "taskkill",
        1,
        "lva#877: Prozessbaum beenden, wird Job-Objekt",
    ),
    (
        "src/managers/tts/mod.rs",
        "netstat",
        1,
        "lva#877: Port-Besitzer finden, wird GetExtendedTcpTable",
    ),
    (
        "src/managers/tts/mod.rs",
        "taskkill",
        1,
        "lva#877: Prozessbaum beenden, wird Job-Objekt",
    ),
    (
        "src/managers/tts/piper.rs",
        "taskkill",
        1,
        "lva#877: Prozessbaum beenden, wird Job-Objekt",
    ),
    (
        "src/sync/account.rs",
        "icacls",
        1,
        "lva#877: Datei nur fuer den Benutzer, wird SetNamedSecurityInfoW",
    ),
    (
        "src/sync/curl.rs",
        "curl",
        1,
        "lva#877: Hub-Sync ueber curl.exe aus System32, wird ein Netzweg im eigenen Prozess",
    ),
];

/// Win32-Namen, auf die Verhaltensregeln gezielt achten. Ein Name zaehlt als Wort
/// im Code (auch mit Endung A, W, Ex). Als Zeichenkette (`GetProcAddress`) ist er
/// nie erlaubt.
const HEIKLE_WIN32_NAMEN: &[&str] = &[
    // Systemweite Eingabe-Hooks und Tastaturzustand (Keylogger-Muster).
    "SetWindowsHookEx",
    "WH_KEYBOARD_LL",
    "WH_MOUSE_LL",
    "GetAsyncKeyState",
    "GetKeyboardState",
    "RegisterRawInputDevices",
    "GetRawInputData",
    // Zugriff auf Speicher und Threads fremder Prozesse (Injektion, Ausspaehen).
    "PROCESS_VM_READ",
    "PROCESS_QUERY_INFORMATION",
    "PROCESS_VM_WRITE",
    "PROCESS_VM_OPERATION",
    "PROCESS_ALL_ACCESS",
    "PROCESS_CREATE_THREAD",
    "PROCESS_DUP_HANDLE",
    "ReadProcessMemory",
    "NtQueryInformationProcess",
    "WriteProcessMemory",
    "VirtualAllocEx",
    "CreateRemoteThread",
    "QueueUserAPC",
    "MiniDumpWriteDump",
    // Rechteausweitung.
    "AdjustTokenPrivileges",
    "SeDebugPrivilege",
    // Eingriffe in den Virenschutz.
    "MpPreference",
    "ExclusionPath",
];

/// `sysinfo`: Beim Auflisten von Prozessen oeffnet die Bibliothek jeden Prozess mit
/// PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, bei `All` auch geschuetzte
/// Systemprozesse. Jede Erwaehnung zaehlt, auch in `use`.
const HEIKLE_SYSINFO_NAMEN: &[&str] = &[
    "ProcessesToUpdate",
    "new_all",
    "refresh_all",
    "refresh_processes",
    "refresh_processes_specifics",
    "with_processes",
];

/// Zusaetzliches Stichwort fuer die Vollabfrage `ProcessesToUpdate::All`.
const VOLLABFRAGE: &str = "ProcessesToUpdate::All";

/// Heutige Ausnahmen. Abbau: lva#873 (E1) und lva#874 (E2). Die Liste darf nur schrumpfen.
/// `GetAsyncKeyState` bleibt fuer einzelne Zustandsabfragen, nie als Schleife ueber alle Tasten.
const ERLAUBTE_AUFRUFE: &[Erlaubt] = &[
    (
        "src/actions.rs",
        "GetAsyncKeyState",
        2,
        "lva#872: bleibt, einmalige Abfrage der AltGr-Taste beim Ausloesen, kein Dauerabruf",
    ),
    (
        "src/input.rs",
        "GetAsyncKeyState",
        2,
        "lva#872: bleibt, einmalige Abfrage der Zusatztasten vor dem Einfuegen, kein Dauerabruf",
    ),
    (
        "src/input.rs",
        "SetWindowsHookEx",
        3,
        "lva#874: Eingabewaechter, wird GetLastInputInfo",
    ),
    (
        "src/input.rs",
        "WH_KEYBOARD_LL",
        2,
        "lva#874: Eingabewaechter, wird GetLastInputInfo",
    ),
    (
        "src/input.rs",
        "WH_MOUSE_LL",
        2,
        "lva#874: Eingabewaechter, wird GetLastInputInfo",
    ),
    (
        "src/managers/llm/app_usage.rs",
        "ProcessesToUpdate",
        3,
        "lva#873: nur macOS/Linux; Windows nutzt prozessliste (separates strenges Tor)",
    ),
    (
        "src/managers/llm/app_usage.rs",
        "ProcessesToUpdate::All",
        1,
        "lva#873: nur macOS/Linux; Windows nutzt prozessliste (separates strenges Tor)",
    ),
    (
        "src/managers/llm/app_usage.rs",
        "refresh_processes_specifics",
        2,
        "lva#873: nur macOS/Linux; Windows nutzt prozessliste (separates strenges Tor)",
    ),
    (
        "src/managers/meeting_detect/mod.rs",
        "ProcessesToUpdate",
        2,
        "lva#873: nur macOS/Linux; Windows nutzt prozessliste (separates strenges Tor)",
    ),
    (
        "src/managers/meeting_detect/mod.rs",
        "ProcessesToUpdate::All",
        1,
        "lva#873: nur macOS/Linux; Windows nutzt prozessliste (separates strenges Tor)",
    ),
    (
        "src/managers/meeting_detect/mod.rs",
        "refresh_processes_specifics",
        1,
        "lva#873: nur macOS/Linux; Windows nutzt prozessliste (separates strenges Tor)",
    ),
    (
        "src/managers/meetings/search/indexer.rs",
        "ProcessesToUpdate",
        2,
        "lva#873: nur macOS/Linux; Windows nutzt prozessliste (separates strenges Tor)",
    ),
    (
        "src/managers/meetings/search/indexer.rs",
        "refresh_processes_specifics",
        1,
        "lva#873: nur macOS/Linux; Windows nutzt prozessliste (separates strenges Tor)",
    ),
];

/// Crates, die systemweite Eingabe-Hooks setzen. Abbau: lva#874 (E2).
const HOOK_CRATES: &[&str] = &[
    "device_query",
    "handy-keys",
    "hookmap",
    "inputbot",
    "livesplit-hotkey",
    "mki",
    "multiinput",
    "rdev",
    "willhook",
    "winput",
];

/// `handy-keys`: heutiger Standard fuer Tastenkuerzel unter Windows, wird mit E2 optional.
/// `rdev`: im Quelltext unbenutzt, wird mit E2 entfernt.
const ERLAUBTE_HOOK_CRATES: &[&str] = &["handy-keys", "rdev"];

// ---------------------------------------------------------------------------
// Zerlegen: Woerter, Zeichenketten und Sonderzeichen, ohne Kommentare.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Marke {
    Wort(String),
    Text(String),
    Sonder(u8),
}
use Marke::{Sonder, Text, Wort};

fn ist_wort(marke: Option<&Marke>, wort: &str) -> bool {
    matches!(marke, Some(Wort(w)) if w == wort)
}

fn zerlegen(quelle: &str) -> Vec<Marke> {
    let b = quelle.as_bytes();
    let text =
        |von: usize, bis: usize| String::from_utf8_lossy(&b[von..bis.min(b.len())]).into_owned();
    let mut marken = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let mut tiefe = 1;
            i += 2;
            while i < b.len() && tiefe > 0 {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    tiefe += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    tiefe -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let wort = &quelle[start..i];
            if matches!(wort, "r" | "br" | "cr") {
                let mut j = i;
                while b.get(j) == Some(&b'#') {
                    j += 1;
                }
                let rauten = j - i;
                if b.get(j) == Some(&b'"') {
                    let anfang = j + 1;
                    j = anfang;
                    while j < b.len()
                        && !(b[j] == b'"' && (1..=rauten).all(|k| b.get(j + k) == Some(&b'#')))
                    {
                        j += 1;
                    }
                    marken.push(Text(text(anfang, j)));
                    i = j + 1 + rauten;
                    continue;
                }
            }
            let vorsatz = (matches!(wort, "b" | "c") && b.get(i) == Some(&b'"'))
                || (wort == "b" && b.get(i) == Some(&b'\''));
            if !vorsatz {
                marken.push(Wort(wort.to_string()));
            }
        } else if c == b'"' {
            let anfang = i + 1;
            i = anfang;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            marken.push(Text(text(anfang, i)));
            i += 1;
        } else if c == b'\'' {
            if b.get(i + 1) == Some(&b'\\') {
                i += 3;
                while i < b.len() && b[i] != b'\'' {
                    i += 1;
                }
                i += 1;
            } else {
                // Zeichenliteral ('x', auch mehrere Bytes) oder Lebenszeit ('a).
                let laenge = match b.get(i + 1) {
                    Some(erstes) if *erstes >= 0xF0 => 4,
                    Some(erstes) if *erstes >= 0xE0 => 3,
                    Some(erstes) if *erstes >= 0xC0 => 2,
                    _ => 1,
                };
                if b.get(i + 1 + laenge) == Some(&b'\'') {
                    i += laenge + 2;
                } else {
                    i += 1;
                }
            }
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
        } else {
            marken.push(Sonder(c));
            i += 1;
        }
    }
    marken
}

// ---------------------------------------------------------------------------
// Testcode ausblenden.
// ---------------------------------------------------------------------------

/// Index der schliessenden Klammer zu `m[auf]`, sonst `m.len()`.
fn klammer_ende(m: &[Marke], auf: usize) -> usize {
    let mut tiefe = 0_i32;
    for (k, marke) in m.iter().enumerate().skip(auf) {
        match marke {
            Sonder(b'(' | b'[' | b'{') => tiefe += 1,
            Sonder(b')' | b']' | b'}') => {
                tiefe -= 1;
                if tiefe == 0 {
                    return k;
                }
            }
            _ => {}
        }
    }
    m.len()
}

/// Verlangt das cfg-Praedikat zwingend `test`? Nur `test` selbst und `all(.., test, ..)`.
/// `any(test, ..)` und `not(test)` gelten auch ohne Testlauf, also nein.
fn verlangt_test(praedikat: &[Marke]) -> bool {
    match praedikat {
        [Wort(w)] => w == "test",
        [Wort(all), Sonder(b'('), innen @ .., Sonder(b')')] if all == "all" => {
            let mut tiefe = 0_i32;
            let mut teil_anfang = 0;
            let mut teile = Vec::new();
            for (k, marke) in innen.iter().enumerate() {
                match marke {
                    Sonder(b'(') => tiefe += 1,
                    Sonder(b')') => tiefe -= 1,
                    Sonder(b',') if tiefe == 0 => {
                        teile.push(&innen[teil_anfang..k]);
                        teil_anfang = k + 1;
                    }
                    _ => {}
                }
            }
            teile.push(&innen[teil_anfang..]);
            teile
                .iter()
                .any(|teil| matches!(teil, [Wort(w)] if w == "test"))
        }
        _ => false,
    }
}

/// `inhalt` ist der Teil zwischen `#[` und `]`.
fn ist_testattribut(inhalt: &[Marke]) -> bool {
    if let [Wort(cfg), Sonder(b'('), praedikat @ .., Sonder(b')')] = inhalt {
        if cfg == "cfg" {
            return verlangt_test(praedikat);
        }
    }
    // #[test], #[tokio::test], #[tokio::test(flavor = "..")]
    let kopf = inhalt
        .iter()
        .position(|marke| *marke == Sonder(b'('))
        .map_or(inhalt, |klammer| &inhalt[..klammer]);
    match kopf {
        [Wort(w)] => w == "test",
        [.., Sonder(b':'), Sonder(b':'), Wort(w)] => w == "test",
        _ => false,
    }
}

const ELEMENTWOERTER: &[&str] = &[
    "async",
    "const",
    "default",
    "enum",
    "extern",
    "fn",
    "impl",
    "let",
    "macro_rules",
    "mod",
    "static",
    "struct",
    "trait",
    "type",
    "union",
    "unsafe",
    "use",
];

fn nach_sichtbarkeit(m: &[Marke], von: usize) -> usize {
    if !ist_wort(m.get(von), "pub") {
        return von;
    }
    if m.get(von + 1) == Some(&Sonder(b'(')) {
        klammer_ende(m, von + 1) + 1
    } else {
        von + 1
    }
}

/// Index hinter dem Element, das bei `von` beginnt. Bewusst vorsichtig: Im Zweifel
/// endet das Element zu frueh. Dann landet Testcode in der Pruefung und der Test
/// scheitert sichtbar, statt Produktionscode zu uebersehen.
fn element_ende(m: &[Marke], von: usize) -> usize {
    let kopf = nach_sichtbarkeit(m, von);
    let als_element = matches!(m.get(kopf), Some(Wort(w)) if ELEMENTWOERTER.contains(&w.as_str()));
    let mut tiefe = 0_i32;
    for (k, marke) in m.iter().enumerate().skip(von) {
        match marke {
            Sonder(b'(' | b'[' | b'{') => tiefe += 1,
            Sonder(b')' | b']') => {
                tiefe -= 1;
                if tiefe < 0 {
                    return k;
                }
            }
            Sonder(b'}') => {
                tiefe -= 1;
                if tiefe < 0 {
                    return k;
                }
                if tiefe == 0 {
                    return k + 1;
                }
            }
            Sonder(b';') if tiefe == 0 => return k + 1,
            Sonder(b',') if tiefe == 0 && !als_element => return k + 1,
            _ => {}
        }
    }
    m.len()
}

/// Index der Klammer, die den umgebenden Block schliesst, sonst `m.len()`.
fn block_ende(m: &[Marke], von: usize) -> usize {
    let mut tiefe = 0_i32;
    for (k, marke) in m.iter().enumerate().skip(von) {
        match marke {
            Sonder(b'(' | b'[' | b'{') => tiefe += 1,
            Sonder(b')' | b']' | b'}') => {
                tiefe -= 1;
                if tiefe < 0 {
                    return k;
                }
            }
            _ => {}
        }
    }
    m.len()
}

/// Ein Modul mit eigener Datei (`mod x;`): Name, optional `#[path]` und die Module
/// derselben Datei, in denen die Zeile steht (`mod a { mod b { mod x; } }`).
#[derive(Debug, PartialEq)]
struct Dateimodul {
    umgebung: Vec<String>,
    name: String,
    pfad: Option<String>,
}

/// `#[path = ".."]` direkt vor `mod` bei `mod_index` (Sichtbarkeit dazwischen erlaubt).
fn pfad_davor(m: &[Marke], mod_index: usize) -> Option<String> {
    let mut k = mod_index;
    if k >= 1 && m[k - 1] == Sonder(b')') {
        while k > 0 && m[k - 1] != Sonder(b'(') {
            k -= 1;
        }
        k = k.saturating_sub(1);
    }
    if k >= 1 && ist_wort(m.get(k - 1), "pub") {
        k -= 1;
    }
    if k < 6 {
        return None;
    }
    match &m[k - 6..k] {
        [Sonder(b'#'), Sonder(b'['), Wort(w), Sonder(b'='), Text(wert), Sonder(b']')]
            if w == "path" =>
        {
            Some(wert.clone())
        }
        _ => None,
    }
}

/// Alle `mod name;` in `m`. `aussen` sind die Module, in denen `m` selbst steht.
fn dateimodule(m: &[Marke], aussen: &[String]) -> Vec<Dateimodul> {
    let mut funde = Vec::new();
    // Offene `mod name {`-Bloecke, je mit der Klammertiefe davor.
    let mut offen: Vec<(String, i32)> = Vec::new();
    let mut tiefe = 0_i32;
    for (i, marke) in m.iter().enumerate() {
        let nach_mod = i >= 2 && ist_wort(m.get(i - 2), "mod");
        match (marke, m.get(i.wrapping_sub(1))) {
            (Sonder(b'{'), davor) => {
                if let (true, Some(Wort(name))) = (nach_mod, davor) {
                    offen.push((name.clone(), tiefe));
                }
                tiefe += 1;
            }
            (Sonder(b'}'), _) => {
                tiefe -= 1;
                if offen.last().is_some_and(|(_, vorher)| *vorher == tiefe) {
                    offen.pop();
                }
            }
            (Sonder(b';'), Some(Wort(name))) if nach_mod => {
                let mut umgebung = aussen.to_vec();
                umgebung.extend(offen.iter().map(|(name, _)| name.clone()));
                funde.push(Dateimodul {
                    umgebung,
                    name: name.clone(),
                    pfad: pfad_davor(m, i - 2),
                });
            }
            _ => {}
        }
    }
    funde
}

/// Produktionscode der Datei und die Dateimodule, die nur im Testlauf existieren.
/// Ein Testmodul im Quelltext (`#[cfg(test)] mod x { .. }`) hat selbst keine Datei:
/// Eine gleichnamige `x.rs` kann zu einem Produktionsmodul gehoeren. Es zaehlen nur
/// die Dateimodule, die in seinem Block stehen.
fn ohne_testcode(m: &[Marke]) -> (Vec<Marke>, Vec<Dateimodul>) {
    let mut produktion = Vec::new();
    let mut testmodule = Vec::new();
    // Offene `mod name {`-Bloecke im Produktionscode, je mit der Klammertiefe davor.
    let mut umgebung: Vec<(String, i32)> = Vec::new();
    let mut tiefe = 0_i32;
    let mut i = 0;
    while i < m.len() {
        let innen = m.get(i + 1) == Some(&Sonder(b'!'));
        let auf = if innen { i + 2 } else { i + 1 };
        if m[i] != Sonder(b'#') || m.get(auf) != Some(&Sonder(b'[')) {
            match &m[i] {
                Sonder(b'{') => {
                    if let (true, Some(Wort(name))) = (
                        i >= 2 && ist_wort(m.get(i - 2), "mod"),
                        m.get(i.wrapping_sub(1)),
                    ) {
                        umgebung.push((name.clone(), tiefe));
                    }
                    tiefe += 1;
                }
                Sonder(b'}') => {
                    tiefe -= 1;
                    if umgebung.last().is_some_and(|(_, offen)| *offen == tiefe) {
                        umgebung.pop();
                    }
                }
                _ => {}
            }
            produktion.push(m[i].clone());
            i += 1;
            continue;
        }
        let zu = klammer_ende(m, auf);
        if !ist_testattribut(&m[(auf + 1).min(zu)..zu]) {
            produktion.extend_from_slice(&m[i..(zu + 1).min(m.len())]);
            i = zu + 1;
            continue;
        }
        if innen {
            // #![cfg(test)]: der Rest des umgebenden Blocks ist Testcode.
            i = block_ende(m, zu + 1);
            continue;
        }
        // Weitere Attribute des Elements ueberspringen, `#[path = ".."]` merken.
        let mut k = zu + 1;
        let mut pfad = None;
        while m.get(k) == Some(&Sonder(b'#')) && m.get(k + 1) == Some(&Sonder(b'[')) {
            let ende = klammer_ende(m, k + 1);
            if let [Wort(w), Sonder(b'='), Text(wert)] = &m[(k + 2).min(ende)..ende] {
                if w == "path" {
                    pfad = Some(wert.clone());
                }
            }
            k = ende + 1;
        }
        let kopf = nach_sichtbarkeit(m, k);
        if let (true, Some(Wort(name))) = (ist_wort(m.get(kopf), "mod"), m.get(kopf + 1)) {
            let mut aussen: Vec<String> = umgebung.iter().map(|(name, _)| name.clone()).collect();
            if m.get(kopf + 2) == Some(&Sonder(b';')) {
                testmodule.push(Dateimodul {
                    umgebung: aussen,
                    name: name.clone(),
                    pfad,
                });
            } else if m.get(kopf + 2) == Some(&Sonder(b'{')) {
                let zu = klammer_ende(m, kopf + 2);
                aussen.push(name.clone());
                testmodule.extend(dateimodule(&m[(kopf + 3).min(zu)..zu], &aussen));
            }
        }
        i = element_ende(m, k);
    }
    (produktion, testmodule)
}

// ---------------------------------------------------------------------------
// Quelltext der Crate einlesen.
// ---------------------------------------------------------------------------

fn wurzel() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn relativ(pfad: &Path) -> String {
    pfad.strip_prefix(wurzel())
        .unwrap_or(pfad)
        .components()
        .map(|teil| teil.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn rust_dateien() -> Vec<PathBuf> {
    fn sammeln(ordner: &Path, dateien: &mut Vec<PathBuf>) {
        for eintrag in std::fs::read_dir(ordner).expect("Quellordner nicht lesbar") {
            let eintrag = eintrag.expect("Verzeichniseintrag nicht lesbar");
            let pfad = eintrag.path();
            let typ = eintrag.file_type().expect("Dateityp nicht lesbar");
            if typ.is_dir() {
                sammeln(&pfad, dateien);
            } else if typ.is_file() && pfad.extension().is_some_and(|endung| endung == "rs") {
                dateien.push(pfad);
            }
        }
    }

    let mut dateien = Vec::new();
    sammeln(&wurzel().join("src"), &mut dateien);
    let build = wurzel().join("build.rs");
    if build.is_file() {
        dateien.push(build);
    }
    dateien.sort();
    dateien
}

/// Ordner, in dem die Untermodule einer Datei liegen.
fn modulordner(datei: &Path) -> PathBuf {
    let ordner = datei.parent().unwrap_or(Path::new("")).to_path_buf();
    match datei.file_stem().and_then(|stamm| stamm.to_str()) {
        Some("mod" | "lib" | "main") | None => ordner,
        Some(stamm) => ordner.join(stamm),
    }
}

/// Datei und Ordner, die zu einem Dateimodul gehoeren. Module, die in der Datei
/// selbst um die `mod`-Zeile liegen, sind Ordner auf dem Weg dorthin.
fn modulpfade(datei: &Path, modul: &Dateimodul) -> Vec<PathBuf> {
    let mut basis = modulordner(datei);
    basis.extend(&modul.umgebung);
    if let Some(pfad) = &modul.pfad {
        // `#[path]` gilt ohne umgebendes Modul relativ zum Ordner der Datei.
        let ordner = if modul.umgebung.is_empty() {
            datei.parent().unwrap_or(Path::new("")).to_path_buf()
        } else {
            basis
        };
        let ziel = ordner.join(pfad);
        let unterordner = ziel.with_extension("");
        return vec![ziel, unterordner];
    }
    let ordner = basis.join(&modul.name);
    vec![ordner.with_extension("rs"), ordner]
}

/// Je Datei: ihre Dateimodule, die nur im Testlauf gelten, und die im Produktionscode.
type Modulzeilen = (PathBuf, Vec<Dateimodul>, Vec<Dateimodul>);

/// Dateien und Ordner, die nur im Testlauf uebersetzt werden. Was auch eine
/// Produktionszeile einbindet (`#[cfg(test)] mod x;` neben `#[cfg(not(test))] mod x;`),
/// bleibt Produktionscode.
fn nur_testpfade(dateien: &[Modulzeilen]) -> Vec<PathBuf> {
    let produktion: BTreeSet<PathBuf> = dateien
        .iter()
        .flat_map(|(datei, _, module)| module.iter().flat_map(move |m| modulpfade(datei, m)))
        .collect();
    dateien
        .iter()
        .flat_map(|(datei, module, _)| module.iter().flat_map(move |m| modulpfade(datei, m)))
        .filter(|pfad| !produktion.contains(pfad))
        .collect()
}

/// Produktionscode je Datei (Pfad relativ zu `src-tauri`), einmal je Testlauf gelesen.
fn produktionscode() -> &'static Vec<(String, Vec<Marke>)> {
    static CODE: OnceLock<Vec<(String, Vec<Marke>)>> = OnceLock::new();
    CODE.get_or_init(|| {
        let mut gelesen = Vec::new();
        let mut modulzeilen: Vec<Modulzeilen> = Vec::new();
        for pfad in rust_dateien() {
            let text = std::fs::read_to_string(&pfad)
                .unwrap_or_else(|_| panic!("Quelldatei nicht lesbar: {}", pfad.display()));
            let (produktion, testmodule) = ohne_testcode(&zerlegen(&text));
            for modul in &testmodule {
                assert!(
                    modulpfade(&pfad, modul).iter().any(|p| p.exists()),
                    "{}: Datei zum Testmodul `{}` nicht gefunden",
                    pfad.display(),
                    modul.name
                );
            }
            modulzeilen.push((pfad.clone(), testmodule, dateimodule(&produktion, &[])));
            gelesen.push((pfad, produktion));
        }
        let nur_test = nur_testpfade(&modulzeilen);
        gelesen
            .into_iter()
            .filter(|(pfad, _)| !nur_test.iter().any(|test| pfad.starts_with(test)))
            .map(|(pfad, produktion)| (relativ(&pfad), produktion))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Funde.
// ---------------------------------------------------------------------------

/// Nennt die Zeichenkette ein Systemwerkzeug? `im_start`: steht direkt in
/// `..::new("..")` oder in einer Datei, die selbst Prozesse startet.
fn werkzeug_im_text(text: &str, im_start: bool) -> Option<&'static str> {
    let text = text.trim().to_lowercase();
    let mut teile = text.split_whitespace();
    let erstes = teile
        .next()?
        .trim_matches(|c| c == '"' || c == '\'' || c == '\\');
    let schalter = teile
        .next()
        .map(|zweites| zweites.starts_with('/') || zweites.starts_with('-'));
    let mit_pfad = erstes.contains('\\') || erstes.contains('/');
    let datei = erstes.rsplit(['\\', '/']).next().unwrap_or(erstes);
    let (stamm, mit_endung) = match datei.strip_suffix(".exe").or(datei.strip_suffix(".com")) {
        Some(stamm) => (stamm, true),
        None => (datei, false),
    };
    // Ganze Zeichenkette oder Befehlszeile mit Schalter, kein Fliesstext.
    let als_befehl = schalter.unwrap_or(true);
    if let Some(name) = EINDEUTIGE_WERKZEUGE.iter().find(|name| **name == stamm) {
        return (als_befehl || mit_endung).then_some(name);
    }
    let name = MEHRDEUTIGE_WERKZEUGE.iter().find(|name| **name == stamm)?;
    let systempfad = ["system32", "syswow64", "sysnative", "windows\\", "windows/"]
        .iter()
        .any(|ordner| erstes.contains(ordner));
    let treffer = mit_endung
        || (mit_pfad && systempfad)
        || schalter == Some(true)
        || (schalter.is_none() && !mit_pfad && im_start);
    treffer.then_some(name)
}

fn win32_name(wort: &str) -> Option<&'static str> {
    HEIKLE_WIN32_NAMEN.iter().copied().find(|name| {
        wort.strip_prefix(name)
            .is_some_and(|rest| matches!(rest, "" | "A" | "W" | "Ex" | "ExA" | "ExW" | "ExNuma"))
    })
}

fn heikles_wort(wort: &str) -> Option<&'static str> {
    win32_name(wort).or_else(|| {
        HEIKLE_SYSINFO_NAMEN
            .iter()
            .copied()
            .find(|name| *name == wort)
    })
}

const GRUNDTYPEN: &[&str] = &[
    "bool", "char", "f32", "f64", "i128", "i16", "i32", "i64", "i8", "isize", "u128", "u16", "u32",
    "u64", "u8", "usize", "_",
];

type Zaehlung = BTreeMap<(String, &'static str), usize>;

fn werkzeuge_zaehlen(code: &[(String, Vec<Marke>)]) -> Zaehlung {
    let mut zaehlung = Zaehlung::new();
    for (datei, m) in code {
        let startet = m
            .iter()
            .any(|marke| matches!(marke, Wort(w) if PROZESSSTART.contains(&w.as_str())));
        for (k, marke) in m.iter().enumerate() {
            let Text(text) = marke else { continue };
            let in_new = k >= 2 && m[k - 1] == Sonder(b'(') && ist_wort(m.get(k - 2), "new");
            if let Some(name) = werkzeug_im_text(text, startet || in_new) {
                *zaehlung.entry((datei.clone(), name)).or_default() += 1;
            }
        }
    }
    zaehlung
}

/// Zaehlt heikle Namen im Code. Zweiter Wert: Schreibweisen, die nie erlaubt sind,
/// weil sie den Namen an der Aufrufstelle verbergen oder eine entfallene Fundstelle
/// in der Zaehlung ersetzen koennten: umbenannte Importe (`use .. X as Y`) und
/// Win32-Namen als Zeichenkette (`GetProcAddress(.., "X")`).
fn aufrufe_zaehlen(code: &[(String, Vec<Marke>)]) -> (Zaehlung, Vec<String>) {
    let mut zaehlung = Zaehlung::new();
    let mut verdeckt = Vec::new();
    for (datei, m) in code {
        for (k, marke) in m.iter().enumerate() {
            let wort = match marke {
                Wort(wort) => wort,
                Text(text) => {
                    if let Some(name) = win32_name(text.trim()) {
                        verdeckt.push(format!("{datei}: `{name}` als Zeichenkette"));
                    }
                    continue;
                }
                Sonder(_) => continue,
            };
            let Some(name) = heikles_wort(wort) else {
                continue;
            };
            *zaehlung.entry((datei.clone(), name)).or_default() += 1;
            if let (true, Some(Wort(neu))) = (ist_wort(m.get(k + 1), "as"), m.get(k + 2)) {
                if !GRUNDTYPEN.contains(&neu.as_str()) {
                    verdeckt.push(format!("{datei}: `{name}` umbenannt in `{neu}`"));
                }
            }
            let vollabfrage = name == "ProcessesToUpdate"
                && m.get(k + 1) == Some(&Sonder(b':'))
                && m.get(k + 2) == Some(&Sonder(b':'))
                && !ist_wort(m.get(k + 3), "Some");
            if vollabfrage {
                *zaehlung.entry((datei.clone(), VOLLABFRAGE)).or_default() += 1;
            }
        }
    }
    (zaehlung, verdeckt)
}

fn abweichungen(ist: &Zaehlung, erlaubt: &[Erlaubt]) -> Vec<String> {
    let mut soll = BTreeMap::new();
    for (datei, stichwort, anzahl, _) in erlaubt {
        soll.insert((datei.to_string(), *stichwort), *anzahl);
    }
    let mut meldungen = Vec::new();
    for ((datei, stichwort), anzahl) in ist {
        let erlaubt = soll.get(&(datei.clone(), *stichwort)).copied().unwrap_or(0);
        if *anzahl > erlaubt {
            meldungen.push(format!(
                "NEU {datei}: `{stichwort}` {anzahl} mal, erlaubt {erlaubt}"
            ));
        }
    }
    for ((datei, stichwort), erlaubt) in &soll {
        let anzahl = ist.get(&(datei.clone(), *stichwort)).copied().unwrap_or(0);
        if anzahl < *erlaubt {
            meldungen.push(format!(
                "ENTFALLEN {datei}: `{stichwort}` nur noch {anzahl} mal, Positivliste nennt {erlaubt} (Eintrag kuerzen)"
            ));
        }
    }
    meldungen
}

fn gleicher_name(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x == y || (matches!(x, b'-' | b'_') && matches!(y, b'-' | b'_')))
}

/// Hook-Crates im Manifest: als Schluessel, als Tabelle `[dependencies.x]`, als
/// `package = "x"` oder in einer Quelle. Kommentare zaehlen nicht.
fn hook_crates_im_manifest(cargo_toml: &str) -> BTreeSet<&'static str> {
    let namenszeichen = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let mut funde = BTreeSet::new();
    for zeile in cargo_toml.lines() {
        let zeile = zeile.split('#').next().unwrap_or("");
        for wort in zeile.split(|c: char| !namenszeichen(c)) {
            if let Some(name) = HOOK_CRATES.iter().find(|name| gleicher_name(name, wort)) {
                funde.insert(*name);
            }
        }
    }
    funde
}

/// Hook-Crates im Lockfile, also auch ueber andere Abhaengigkeiten hereingezogene.
fn hook_crates_im_lockfile(cargo_lock: &str) -> BTreeSet<&'static str> {
    cargo_lock
        .lines()
        .filter_map(|zeile| zeile.trim().strip_prefix("name = \""))
        .filter_map(|rest| {
            let paket = rest.trim_end_matches('"');
            HOOK_CRATES
                .iter()
                .copied()
                .find(|name| gleicher_name(name, paket))
        })
        .collect()
}

const HINWEIS: &str = "Regel und Vorgehen: apps/local-voice/AGENTS.md, Abschnitt \
\"EDR-relevantes Verhalten\" (lva#872).";

// ---------------------------------------------------------------------------
// Pruefungen am Quelltext der Crate.
// ---------------------------------------------------------------------------

#[test]
fn kein_neuer_start_von_windows_systemwerkzeugen() {
    let meldungen = abweichungen(
        &werkzeuge_zaehlen(produktionscode()),
        ERLAUBTE_SYSTEMWERKZEUGE,
    );
    assert!(
        meldungen.is_empty(),
        "Systemwerkzeuge weichen von der Positivliste ab:\n{}\n{HINWEIS}",
        meldungen.join("\n")
    );
}

#[test]
fn keine_neuen_hooks_und_kein_neuer_zugriff_auf_fremde_prozesse() {
    let (ist, verdeckt) = aufrufe_zaehlen(produktionscode());
    assert!(
        verdeckt.is_empty(),
        "Heikle Namen duerfen weder umbenannt noch als Zeichenkette vorkommen:\n{}\n{HINWEIS}",
        verdeckt.join("\n")
    );
    let meldungen = abweichungen(&ist, ERLAUBTE_AUFRUFE);
    assert!(
        meldungen.is_empty(),
        "Heikle Aufrufe weichen von der Positivliste ab:\n{}\n{HINWEIS}",
        meldungen.join("\n")
    );
}

/// Nur fuer das zusaetzliche Windows-Tor: exakt `cfg(not(windows))` entfernen.
/// Die bestehende plattformuebergreifende Sperrklinke bleibt unveraendert streng.
/// Unbekannte cfg-Formen bleiben sichtbar und koennen das Tor nicht umgehen.
fn windows_code(m: &[Marke]) -> Vec<Marke> {
    let attribut = zerlegen("#[cfg(not(windows))]");
    let mut result = Vec::new();
    let mut i = 0;
    while i < m.len() {
        if m[i..].starts_with(&attribut) {
            i = element_ende(m, i + attribut.len());
        } else {
            result.push(m[i].clone());
            i += 1;
        }
    }
    result
}

fn windows_prozessverstoss(m: &[Marke]) -> bool {
    windows_code(m).iter().any(|marke| match marke {
        Wort(w) | Text(w) => HEIKLE_SYSINFO_NAMEN.contains(&w.as_str())
            || matches!(w.as_str(), "PROCESS_VM_READ" | "PROCESS_QUERY_INFORMATION"
                | "ReadProcessMemory" | "NtQueryInformationProcess"),
        _ => false,
    })
}

#[test]
fn windows_prozessabfragen_ohne_fremden_speicher() {
    for (datei, code) in produktionscode() {
        assert!(!windows_prozessverstoss(code), "Windows-Prozessabfrage: {datei}");
    }
}

/// Bestehende Token-/Job-Abfragen und das zentrale E1-Modul. Importe zaehlen
/// mit, Testcode nicht. Exakte Zaehlung in beide Richtungen wie bei den Altlasten.
fn open_process_anzahl(code: &[Marke]) -> Option<usize> {
    let mut anzahl = 0;
    for (i, marke) in code.iter().enumerate() {
        match marke {
            Text(w) if w == "OpenProcess" => return None,
            Wort(w) if w == "OpenProcess" => {
                if ist_wort(code.get(i + 1), "as") { return None; }
                anzahl += 1;
            }
            _ => {}
        }
    }
    Some(anzahl)
}

#[test]
fn open_process_nur_an_den_geprueften_stellen() {
    let mut ist = BTreeMap::new();
    for (datei, code) in produktionscode() {
        let anzahl = open_process_anzahl(code).unwrap_or_else(|| panic!("Verdecktes OpenProcess: {datei}"));
        if anzahl > 0 { ist.insert(datei.as_str(), anzahl); }
    }
    let soll = BTreeMap::from([
        ("src/agent_bridge/pipe.rs", 2),
        ("src/managers/meetings/pdf.rs", 2),
        ("src/paste_guard.rs", 2),
        ("src/prozessliste.rs", 2),
    ]);
    assert_eq!(ist, soll, "OpenProcess-Positivliste (lva#873)");
}

#[test]
fn gegenprobe_open_process_auch_importiert_und_verdeckt() {
    assert_eq!(open_process_anzahl(&zerlegen("use x::OpenProcess; fn a() { OpenProcess(); }")), Some(2));
    assert_eq!(open_process_anzahl(&zerlegen("// OpenProcess\nfn a() { OpenProcessToken(); }")), Some(0));
    assert_eq!(open_process_anzahl(&zerlegen("use x::OpenProcess as Offen;")), None);
    assert_eq!(open_process_anzahl(&zerlegen("get(\"OpenProcess\");")), None);
}

#[test]
fn gegenprobe_windows_tor_laesst_nur_expliziten_fremdplattformcode_aus() {
    assert!(!windows_prozessverstoss(&zerlegen(
        "#[cfg(not(windows))] fn a() { s.refresh_processes(ProcessesToUpdate::All); }"
    )));
    for quelle in [
        "fn a() { s.refresh_processes(ProcessesToUpdate::All); }",
        "#[cfg(windows)] fn a() { s.refresh_processes_specifics(); }",
        "#[cfg(any(test, windows))] fn a() { ReadProcessMemory(); }",
        "#[cfg(not(windows))] fn a() {} fn b() { PROCESS_VM_READ; }",
        "#[cfg(not(windows))] use sysinfo::System; fn b() { NtQueryInformationProcess(); }",
        "fn a() { PROCESS_QUERY_INFORMATION; }",
    ] {
        assert!(windows_prozessverstoss(&zerlegen(quelle)), "{quelle}");
    }
}

#[test]
fn keine_neue_abhaengigkeit_mit_systemweiten_eingabe_hooks() {
    let erlaubt: BTreeSet<_> = ERLAUBTE_HOOK_CRATES.iter().copied().collect();
    let manifest = std::fs::read_to_string(wurzel().join("Cargo.toml")).expect("Cargo.toml");
    assert_eq!(
        hook_crates_im_manifest(&manifest),
        erlaubt,
        "Hook-Crates in Cargo.toml weichen von der Positivliste ab. {HINWEIS}"
    );
    let lockfile = std::fs::read_to_string(wurzel().join("Cargo.lock")).expect("Cargo.lock");
    assert_eq!(
        hook_crates_im_lockfile(&lockfile),
        erlaubt,
        "Hook-Crates in Cargo.lock weichen von der Positivliste ab. {HINWEIS}"
    );
}

#[test]
fn jede_ausnahme_nennt_grund_und_issue() {
    for (datei, stichwort, anzahl, grund) in ERLAUBTE_SYSTEMWERKZEUGE.iter().chain(ERLAUBTE_AUFRUFE)
    {
        assert!(*anzahl > 0, "{datei}: `{stichwort}` mit Anzahl 0 loeschen");
        assert!(
            grund.contains("lva#") && grund.len() > 12,
            "{datei}: `{stichwort}` braucht Grund und Issue"
        );
        assert!(
            wurzel().join(datei).is_file(),
            "{datei}: Datei der Ausnahme gibt es nicht mehr"
        );
        let bekannt = EINDEUTIGE_WERKZEUGE.contains(stichwort)
            || MEHRDEUTIGE_WERKZEUGE.contains(stichwort)
            || HEIKLE_WIN32_NAMEN.contains(stichwort)
            || HEIKLE_SYSINFO_NAMEN.contains(stichwort)
            || *stichwort == VOLLABFRAGE;
        assert!(bekannt, "{datei}: `{stichwort}` steht in keiner Pruefliste");
    }
}

#[test]
fn die_eigenen_tests_zaehlen_nicht_als_produktionscode() {
    let dateien: Vec<_> = produktionscode()
        .iter()
        .map(|(datei, _)| datei.as_str())
        .collect();
    assert!(dateien.contains(&"src/netz/mod.rs"));
    assert!(dateien.contains(&"src/input.rs"));
    assert!(!dateien.contains(&"src/netz/edr_sperrklinke_tests.rs"));
    assert!(!dateien.contains(&"src/netz/firmentauglich_tests.rs"));
}

// ---------------------------------------------------------------------------
// Gegenproben: Jede bekannte Luecke bleibt geschlossen.
// ---------------------------------------------------------------------------

fn probe(quelle: &str) -> Vec<(String, Vec<Marke>)> {
    vec![(
        "src/probe.rs".to_string(),
        ohne_testcode(&zerlegen(quelle)).0,
    )]
}

fn werkzeuge(quelle: &str) -> Vec<(&'static str, usize)> {
    werkzeuge_zaehlen(&probe(quelle))
        .into_iter()
        .map(|((_, name), anzahl)| (name, anzahl))
        .collect()
}

fn aufrufe(quelle: &str) -> Vec<(&'static str, usize)> {
    aufrufe_zaehlen(&probe(quelle))
        .0
        .into_iter()
        .map(|((_, name), anzahl)| (name, anzahl))
        .collect()
}

#[test]
fn gegenprobe_testcode_wird_nur_bei_zwingendem_cfg_test_ausgeblendet() {
    // cfg(any(test, windows)) ist im Release aktiv und zaehlt.
    for attribut in [
        "#[cfg(any(test, windows))]",
        "#[cfg(not(test))]",
        "#[cfg(all(windows, not(test)))]",
        "#[cfg(all(windows, any(test, debug_assertions)))]",
        "#[cfg(windows)]",
        "#[cfg_attr(test, allow(dead_code))]",
    ] {
        let quelle = format!("{attribut}\nfn a() {{ Command::new(\"taskkill\"); }}");
        assert_eq!(werkzeuge(&quelle), vec![("taskkill", 1)], "{attribut}");
    }
    assert_eq!(
        werkzeuge("fn a() { if cfg!(test) { Command::new(\"taskkill\"); } }"),
        vec![("taskkill", 1)]
    );
    // Zwingend Test: ausgeblendet.
    for attribut in [
        "#[cfg(test)]",
        "#[cfg(all(test, windows))]",
        "#[cfg(all(windows, test))]",
        "#[test]",
        "#[tokio::test]",
        "#[tokio::test(flavor = \"multi_thread\")]",
    ] {
        let quelle = format!("{attribut}\nfn a() {{ Command::new(\"taskkill\"); }}");
        assert!(werkzeuge(&quelle).is_empty(), "{attribut}");
    }
    assert!(werkzeuge("#![cfg(test)]\nfn a() { Command::new(\"taskkill\"); }").is_empty());
}

#[test]
fn gegenprobe_klammern_in_zeichenketten_und_kommentaren_verstecken_keinen_produktionscode() {
    // Offene Klammern in Text, Rohtext, Zeichen und Kommentaren im Testblock:
    // Der Produktionscode dahinter muss sichtbar bleiben.
    let quelle = r####"
        #[cfg(test)]
        mod tests {
            // eine offene { im Kommentar
            /* und noch eine { im /* geschachtelten */ Blockkommentar */
            fn t<'a>(x: &'a str) {
                let _ = "offen { und \" noch {";
                let _ = r#"roh { mit " darin"#;
                let _ = ('{', '\'', b'{', '\u{7B}', 'ä');
                Command::new("netstat");
            }
        }
        fn produktion() { Command::new("taskkill"); }
    "####;
    assert_eq!(werkzeuge(quelle), vec![("taskkill", 1)]);
    // Schliessende Klammern im Testblock beenden ihn nicht vorzeitig.
    let quelle = r####"
        #[cfg(test)]
        mod tests {
            fn t() { let _ = "}"; let _ = '}'; /* } */ Command::new("netstat"); }
        }
    "####;
    assert!(werkzeuge(quelle).is_empty());
    // Felder und Anweisungen mit cfg(test) schlucken den Nachbarn nicht.
    let quelle = r####"
        struct S { #[cfg(test)] probe: u8, name: &'static str }
        fn a() {
            #[cfg(test)]
            let _ = 1;
            Command::new("taskkill");
        }
    "####;
    assert_eq!(werkzeuge(quelle), vec![("taskkill", 1)]);
}

#[test]
fn gegenprobe_systemwerkzeuge_in_jeder_schreibweise() {
    for quelle in [
        "fn a() { Command::new(\"taskkill\"); }",
        "fn a() { Command :: new ( \"taskkill\" ); }",
        "fn a() { Command::new(\n    \"taskkill\",\n); }",
        "fn a() { Command::new(r\"taskkill\"); }",
        "fn a() { Command::new(r#\"taskkill.exe\"#); }",
        "fn a() { let programm = \"taskkill\"; starte(programm); }",
        "const WERKZEUG: &str = \"TASKKILL.EXE\";",
        "fn a() { Command::new(\"C:\\\\Windows\\\\System32\\\\taskkill.exe\"); }",
        "fn a() { Command::new(r\"C:\\Windows\\System32\\taskkill.exe\"); }",
        "fn a() { starte(\"taskkill /PID 4 /T /F\"); }",
        "fn a() { Cmd::new(\"cmd\").args([\"/C\", \"taskkill\"]); }",
    ] {
        let funde = werkzeuge(quelle);
        assert!(funde.contains(&("taskkill", 1)), "{quelle}: {funde:?}");
    }
    // Mehrdeutige Namen: als Programm ja, als gewoehnliches Wort nein.
    for quelle in [
        "fn a() { Command::new(\"cmd\"); }",
        "fn a() { Befehl::new(\"cmd\"); }",
        "fn a() { let shell = \"cmd\"; Command::new(shell); }",
        "fn a() { starte(\"cmd.exe\"); }",
        "fn a() { starte(\"cmd /C dir\"); }",
        "fn a() { starte(r\"C:\\Windows\\System32\\cmd.exe\"); }",
    ] {
        assert_eq!(werkzeuge(quelle), vec![("cmd", 1)], "{quelle}");
    }
    assert_eq!(
        werkzeuge("fn a() { std::env::var(\"COMSPEC\"); }"),
        vec![("comspec", 1)]
    );
    for quelle in [
        "fn a() { json[\"cmd\"] = 1; }",
        "fn a() { log(\"net income at route\"); }",
        "fn a() { get(\"/api/net\"); }",
        "fn a() { fehler(\"taskkill nicht startbar\"); }",
        "fn a() { Command::new(\"netzwerk\"); }",
        "fn a() { Command::new(ffmpeg_pfad); }",
        "// Command::new(\"taskkill\")\nfn a() {}",
    ] {
        assert!(werkzeuge(quelle).is_empty(), "{quelle}");
    }
    // Zwei Starts in einer Zeile sind zwei Funde.
    assert_eq!(
        werkzeuge("fn a() { Command::new(\"netstat\"); Command::new(\"netstat\"); }"),
        vec![("netstat", 2)]
    );
}

#[test]
fn gegenprobe_heikle_namen_auch_importiert_umbenannt_und_als_text() {
    assert_eq!(
        aufrufe("fn a() { SetWindowsHookExW(WH_KEYBOARD_LL, None, None, 0); }"),
        vec![("SetWindowsHookEx", 1), ("WH_KEYBOARD_LL", 1)]
    );
    // Kommentare und aehnliche Namen zaehlen nicht.
    assert!(
        aufrufe("// SetWindowsHookExW\nfn a() { PROCESS_QUERY_LIMITED_INFORMATION; }").is_empty()
    );
    assert!(aufrufe("fn refresh_all_models() { UnhookWindowsHookEx(h); }").is_empty());
    // Der Name als Zeichenkette (GetProcAddress) ist immer ein Fehler und zaehlt
    // nicht als Fund: Er kann eine entfallene Fundstelle nicht ersetzen.
    let (zaehlung, verdeckt) = aufrufe_zaehlen(&probe(
        "fn a() { GetProcAddress(m, \"WriteProcessMemory\"); let _ = \"WH_KEYBOARD_LL\"; }",
    ));
    assert!(zaehlung.is_empty());
    assert_eq!(verdeckt.len(), 2);
    let erlaubt: &[Erlaubt] = &[("src/probe.rs", "WH_KEYBOARD_LL", 1, "lva#1 Grund")];
    assert!(abweichungen(&zaehlung, erlaubt)[0].starts_with("ENTFALLEN"));
    // Umbenannte Importe: gezaehlt und zusaetzlich immer ein Fehler.
    for quelle in [
        "use sysinfo::ProcessesToUpdate as P;",
        "use sysinfo::{System as S, ProcessesToUpdate as P};",
        "use windows::Win32::UI::WindowsAndMessaging::SetWindowsHookExW as Haken;",
        "use windows::Win32::System::Threading::{PROCESS_VM_READ as LESEN};",
    ] {
        let (zaehlung, verdeckt) = aufrufe_zaehlen(&probe(quelle));
        assert_eq!(zaehlung.len(), 1, "{quelle}");
        assert_eq!(verdeckt.len(), 1, "{quelle}");
    }
    // Eine Typumwandlung ist keine Umbenennung.
    let (zaehlung, verdeckt) = aufrufe_zaehlen(&probe("fn a() { let _ = WH_KEYBOARD_LL as i32; }"));
    assert_eq!(zaehlung.len(), 1);
    assert!(verdeckt.is_empty());
    // sysinfo: jede Erwaehnung zaehlt, die Vollabfrage zusaetzlich.
    let voll = vec![
        ("ProcessesToUpdate", 1),
        (VOLLABFRAGE, 1),
        ("refresh_processes", 1),
    ];
    assert_eq!(
        aufrufe("fn a() { s.refresh_processes(ProcessesToUpdate::All, true); }"),
        voll
    );
    assert_eq!(
        aufrufe("fn a() { s.refresh_processes(\n    ProcessesToUpdate\n        ::All, true); }"),
        voll
    );
    assert_eq!(
        aufrufe("fn a() { s.refresh_processes(ProcessesToUpdate::Some(&[pid]), true); }"),
        vec![("ProcessesToUpdate", 1), ("refresh_processes", 1)]
    );
    // Variante ueber `use` hereingeholt: gilt als Vollabfrage.
    assert_eq!(
        aufrufe("use sysinfo::ProcessesToUpdate::{All, Some};"),
        vec![("ProcessesToUpdate", 1), (VOLLABFRAGE, 1)]
    );
    assert_eq!(
        aufrufe("fn a() { let s = System::new_all(); s.refresh_all(); }"),
        vec![("new_all", 1), ("refresh_all", 1)]
    );
}

#[test]
fn gegenprobe_hook_crates_in_jeder_manifest_schreibweise() {
    let einzeln = |text: &str| {
        hook_crates_im_manifest(text)
            .into_iter()
            .collect::<Vec<_>>()
    };
    assert_eq!(einzeln("rdev = \"0.5\"\n"), vec!["rdev"]);
    assert_eq!(einzeln("handy-keys.workspace = true\n"), vec!["handy-keys"]);
    assert_eq!(
        einzeln("tasten = { package = \"rdev\", version = \"0.5\" }\n"),
        vec!["rdev"]
    );
    assert_eq!(
        einzeln("[dependencies.rdev]\nversion = \"0.5\"\n"),
        vec!["rdev"]
    );
    assert_eq!(
        einzeln("[target.'cfg(windows)'.dependencies.handy-keys]\nversion = \"0.3\"\n"),
        vec!["handy-keys"]
    );
    assert_eq!(
        einzeln("tasten = { git = \"https://example.org/x/rdev\" }\n"),
        vec!["rdev"]
    );
    assert_eq!(einzeln("device-query = \"2\"\n"), vec!["device_query"]);
    assert!(einzeln("# rdev = \"0.5\"\nrdevx = \"1\"\nxrdev = \"1\"\n").is_empty());

    let lock =
        "[[package]]\nname = \"rdev\"\nversion = \"0.5.0\"\n\n[[package]]\nname = \"serde\"\n";
    assert_eq!(
        hook_crates_im_lockfile(lock)
            .into_iter()
            .collect::<Vec<_>>(),
        vec!["rdev"]
    );
    assert!(hook_crates_im_lockfile("name = \"rdev-sys-fremd\"\n").is_empty());
}

#[test]
fn gegenprobe_testdateien_folgen_aus_der_mod_zeile_nicht_aus_dem_namen() {
    let quelle = "mod latest;\npub mod linktest_fremd;\n\
                  #[cfg(test)]\nmod tests;\n\
                  #[cfg(test)]\npub(crate) mod linktest;\n\
                  #[cfg(test)]\n#[path = \"hilfe/probe.rs\"]\nmod probe;\n\
                  #[cfg(any(test, windows))]\nmod auch_produktion;\n\
                  #[cfg(test)]\nmod innen { fn t() {} mod hilfsdatei; }\n";
    let funde = ohne_testcode(&zerlegen(quelle)).1;
    let namen: Vec<_> = funde.iter().map(|modul| modul.name.as_str()).collect();
    // Das Testmodul im Quelltext (`innen`) zaehlt nicht selbst, nur seine Dateimodule.
    assert_eq!(namen, vec!["tests", "linktest", "probe", "hilfsdatei"]);
    assert_eq!(funde[2].pfad.as_deref(), Some("hilfe/probe.rs"));
    assert_eq!(funde[3].umgebung, vec!["innen".to_string()]);

    // Untermodule liegen neben mod.rs/lib.rs, sonst im Ordner mit dem Dateinamen.
    let modul = |name: &str| Dateimodul {
        umgebung: Vec::new(),
        name: name.to_string(),
        pfad: None,
    };
    assert_eq!(
        modulpfade(Path::new("src/a/mod.rs"), &modul("tests")),
        vec![
            PathBuf::from("src/a/tests.rs"),
            PathBuf::from("src/a/tests")
        ]
    );
    assert_eq!(
        modulpfade(Path::new("src/a/b.rs"), &modul("tests")),
        vec![
            PathBuf::from("src/a/b/tests.rs"),
            PathBuf::from("src/a/b/tests")
        ]
    );
    assert_eq!(
        modulpfade(Path::new("src/a/b.rs"), &funde[2]),
        vec![
            PathBuf::from("src/a/hilfe/probe.rs"),
            PathBuf::from("src/a/hilfe/probe")
        ]
    );
}

#[test]
fn gegenprobe_verschachteltes_testmodul_schliesst_kein_gleichnamiges_modul_aus() {
    // `mod platform { #[cfg(test)] mod process; }` meint platform/process.rs.
    // Ein gleichnamiges Modul daneben (process.rs) bleibt Produktionscode.
    let quelle = "mod process;\n\
                  mod platform {\n\
                      pub mod windows {\n\
                          fn a() { if x { y() } }\n\
                          #[cfg(test)]\n\
                          mod process;\n\
                      }\n\
                      #[cfg(test)]\n\
                      mod hilfe;\n\
                      #[cfg(test)]\n\
                      #[path = \"fern.rs\"]\n\
                      mod fern;\n\
                  }\n\
                  #[cfg(test)]\n\
                  mod tests;\n";
    let funde = ohne_testcode(&zerlegen(quelle)).1;
    let lage: Vec<_> = funde
        .iter()
        .map(|modul| (modul.umgebung.join("/"), modul.name.as_str()))
        .collect();
    assert_eq!(
        lage,
        vec![
            ("platform/windows".to_string(), "process"),
            ("platform".to_string(), "hilfe"),
            ("platform".to_string(), "fern"),
            (String::new(), "tests"),
        ]
    );
    let pfade = |datei: &str, k: usize| modulpfade(Path::new(datei), &funde[k]);
    assert_eq!(
        pfade("src/lib.rs", 0),
        vec![
            PathBuf::from("src/platform/windows/process.rs"),
            PathBuf::from("src/platform/windows/process")
        ]
    );
    assert_eq!(
        pfade("src/a/b.rs", 0),
        vec![
            PathBuf::from("src/a/b/platform/windows/process.rs"),
            PathBuf::from("src/a/b/platform/windows/process")
        ]
    );
    assert_eq!(
        pfade("src/a/b.rs", 2),
        vec![
            PathBuf::from("src/a/b/platform/fern.rs"),
            PathBuf::from("src/a/b/platform/fern")
        ]
    );
    // Keiner dieser Pfade trifft das gleichnamige Modul auf oberster Ebene.
    for k in 0..3 {
        for pfad in pfade("src/lib.rs", k) {
            assert!(!Path::new("src/process.rs").starts_with(&pfad), "{pfad:?}");
            assert!(!Path::new("src/hilfe.rs").starts_with(&pfad), "{pfad:?}");
        }
    }
    assert_eq!(pfade("src/lib.rs", 3)[0], PathBuf::from("src/tests.rs"));
}

#[test]
fn gegenprobe_testmodul_im_quelltext_schliesst_keine_gleichnamige_datei_aus() {
    // Attrappe im Quelltext neben dem echten Modul in backend.rs:
    // backend.rs und backend/ bleiben Produktionscode.
    let quelle = "#[cfg(test)]\n\
                  mod backend {\n\
                      pub fn starte() {}\n\
                  }\n\
                  #[cfg(not(test))]\n\
                  mod backend;\n";
    let (produktion, testmodule) = ohne_testcode(&zerlegen(quelle));
    assert!(testmodule.is_empty(), "{testmodule:?}");
    let echte = dateimodule(&produktion, &[]);
    assert_eq!(echte.len(), 1);
    let zeilen = vec![(PathBuf::from("src/lib.rs"), testmodule, echte)];
    assert!(nur_testpfade(&zeilen).is_empty());

    // Bindet die Attrappe eine eigene Datei ein, ist nur diese Testcode.
    let quelle = "#[cfg(test)]\n\
                  mod backend {\n\
                      mod aufzeichnung;\n\
                  }\n\
                  #[cfg(not(test))]\n\
                  mod backend;\n";
    let (produktion, testmodule) = ohne_testcode(&zerlegen(quelle));
    let zeilen = vec![(
        PathBuf::from("src/lib.rs"),
        testmodule,
        dateimodule(&produktion, &[]),
    )];
    let nur_test = nur_testpfade(&zeilen);
    assert_eq!(
        nur_test,
        vec![
            PathBuf::from("src/backend/aufzeichnung.rs"),
            PathBuf::from("src/backend/aufzeichnung")
        ]
    );
    let ist_test = |pfad: &str| {
        nur_test
            .iter()
            .any(|test| Path::new(pfad).starts_with(test))
    };
    assert!(!ist_test("src/backend.rs"));
    assert!(!ist_test("src/backend/mod.rs"));
    assert!(!ist_test("src/backend/echt.rs"));
    assert!(ist_test("src/backend/aufzeichnung.rs"));

    // Dieselbe Datei fuer Test und Produktion eingebunden: bleibt Produktionscode.
    // Ein eigener Pfad fuer die Testfassung ist dagegen Testcode.
    let quelle = "#[cfg(test)]\nmod doppelt;\n#[cfg(not(test))]\nmod doppelt;\n\
                  #[cfg(test)]\n#[path = \"attrappe.rs\"]\nmod dienst;\n\
                  #[cfg(not(test))]\nmod dienst;\n\
                  #[cfg(test)]\nmod tests;\n";
    let (produktion, testmodule) = ohne_testcode(&zerlegen(quelle));
    let zeilen = vec![(
        PathBuf::from("src/a/mod.rs"),
        testmodule,
        dateimodule(&produktion, &[]),
    )];
    assert_eq!(
        nur_testpfade(&zeilen),
        vec![
            PathBuf::from("src/a/attrappe.rs"),
            PathBuf::from("src/a/attrappe"),
            PathBuf::from("src/a/tests.rs"),
            PathBuf::from("src/a/tests")
        ]
    );
}

#[test]
fn gegenprobe_abweichungen_in_beide_richtungen() {
    let mut ist = Zaehlung::new();
    ist.insert(("src/a.rs".to_string(), "cmd"), 2);
    let erlaubt: &[Erlaubt] = &[("src/a.rs", "cmd", 1, "lva#1 Grund")];
    assert!(abweichungen(&ist, erlaubt)[0].starts_with("NEU src/a.rs"));
    let erlaubt: &[Erlaubt] = &[("src/a.rs", "cmd", 3, "lva#1 Grund")];
    assert!(abweichungen(&ist, erlaubt)[0].starts_with("ENTFALLEN src/a.rs"));
    let erlaubt: &[Erlaubt] = &[("src/a.rs", "cmd", 2, "lva#1 Grund")];
    assert!(abweichungen(&ist, erlaubt).is_empty());
    assert!(abweichungen(&Zaehlung::new(), erlaubt)[0].starts_with("ENTFALLEN"));
    assert!(abweichungen(&ist, &[])[0].starts_with("NEU"));
}
