//! Sperrklinke zu lva#872: Verhalten, das Endpoint-Schutzsoftware (EDR) als
//! schadsoftwareaehnlich einstuft, darf nicht unbemerkt neu entstehen.
//!
//! Die Tests lesen den Produktionsquelltext der Crate und vergleichen ihn mit einer
//! Positivliste. Jede Abweichung laesst den Test scheitern, in BEIDE Richtungen:
//!
//! - mehr Fundstellen als erlaubt: neues EDR-relevantes Verhalten. Erst die
//!   Alternative pruefen (Win32-API statt Systemwerkzeug, kein Hook, kein Zugriff
//!   auf fremde Prozesse). Nur wenn es keine gibt, den Eintrag mit Issue-Verweis
//!   und Begruendung ergaenzen (Regel: apps/local-voice/AGENTS.md, Abschnitt
//!   "EDR-relevantes Verhalten").
//! - weniger Fundstellen als erlaubt: gut. Den Eintrag in derselben Aenderung
//!   kuerzen oder loeschen, damit die Liste nur schrumpft und nie wieder waechst.
//!
//! Testcode zaehlt nicht mit: Testdateien werden uebersprungen, `#[cfg(test)]`-
//! Bloecke in Produktionsdateien ausgeblendet. Kommentarzeilen zaehlen nicht.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// (Datei relativ zu `src-tauri`, Stichwort, erlaubte Anzahl Zeilen, Grund mit Issue).
type Erlaubt = (&'static str, &'static str, usize, &'static str);

/// Windows-Systemwerkzeuge, die ein unsignierter Prozess nicht starten soll.
/// Geprueft wird der Programmname in `Command::new("<name>")`, mit und ohne `.exe`.
const SYSTEMWERKZEUGE: &[&str] = &[
    "arp",
    "at",
    "attrib",
    "bcdedit",
    "bitsadmin",
    "cacls",
    "certutil",
    "cipher",
    "cmd",
    "cmdkey",
    "cscript",
    "curl",
    "dsquery",
    "esentutl",
    "explorer",
    "findstr",
    "forfiles",
    "fsutil",
    "ftp",
    "icacls",
    "ipconfig",
    "mshta",
    "msiexec",
    "net",
    "net1",
    "netsh",
    "netstat",
    "nltest",
    "nslookup",
    "ping",
    "quser",
    "qwinsta",
    "reg",
    "regedit",
    "regsvr32",
    "route",
    "runas",
    "rundll32",
    "sc",
    "schtasks",
    "systeminfo",
    "takeown",
    "taskkill",
    "tasklist",
    "tftp",
    "vssadmin",
    "wevtutil",
    "where",
    "whoami",
    "wmic",
    "wscript",
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
        1,
        "lva#877: bleibt als einzige Ausnahme, Batch-Starter eines CLI nach Klick auf Anmelden",
    ),
    (
        "src/managers/llm/cli.rs",
        "taskkill",
        1,
        "lva#877: Prozessbaum beenden, wird Job-Objekt",
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
];

/// Win32- und Bibliotheksaufrufe, auf die Verhaltensregeln gezielt achten.
/// Gross-/Kleinschreibung zaehlt, Kommentarzeilen nicht.
const HEIKLE_AUFRUFE: &[&str] = &[
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
    "PROCESS_VM_WRITE",
    "PROCESS_VM_OPERATION",
    "PROCESS_ALL_ACCESS",
    "PROCESS_CREATE_THREAD",
    "PROCESS_DUP_HANDLE",
    "ReadProcessMemory",
    "WriteProcessMemory",
    "VirtualAllocEx",
    "CreateRemoteThread",
    "QueueUserAPC",
    "MiniDumpWriteDump",
    // Rechteausweitung.
    "AdjustTokenPrivileges",
    "SeDebugPrivilege",
    // sysinfo oeffnet beim Auflisten ALLER Prozesse jeden einzelnen mit
    // PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, auch geschuetzte Systemprozesse.
    "ProcessesToUpdate::All",
    "System::new_all",
    "refresh_all(",
    // Eingriffe in den Virenschutz.
    "MpPreference",
    "ExclusionPath",
];

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
        "ProcessesToUpdate::All",
        1,
        "lva#873: Verbrauchsanzeige, wird eigene Prozessliste ohne Lesezugriff",
    ),
    (
        "src/managers/meeting_detect/mod.rs",
        "ProcessesToUpdate::All",
        1,
        "lva#873: Besprechungserkennung, wird eigene Prozessliste ohne Lesezugriff",
    ),
];

/// Crates, die systemweite Eingabe-Hooks setzen. Abbau: lva#874 (E2).
const HOOK_CRATES: &[&str] = &[
    "device_query",
    "handy-keys",
    "inputbot",
    "livesplit-hotkey",
    "multiinput",
    "rdev",
    "willhook",
];

/// `handy-keys`: heutiger Standard fuer Tastenkuerzel unter Windows, wird mit E2 optional.
/// `rdev`: im Quelltext unbenutzt, wird mit E2 entfernt.
const ERLAUBTE_HOOK_CRATES: &[&str] = &["handy-keys", "rdev"];

fn wurzel() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn ist_testdatei(pfad: &Path) -> bool {
    let name = pfad
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    name == "tests.rs"
        || name == "test_support.rs"
        || name.starts_with("tests_")
        || name.ends_with("_tests.rs")
        || name.ends_with("test.rs")
        || pfad.components().any(|teil| teil.as_os_str() == "tests")
}

fn produktionsdateien() -> Vec<PathBuf> {
    fn sammeln(ordner: &Path, dateien: &mut Vec<PathBuf>) {
        for eintrag in std::fs::read_dir(ordner).expect("Quellordner nicht lesbar") {
            let eintrag = eintrag.expect("Verzeichniseintrag nicht lesbar");
            let pfad = eintrag.path();
            let typ = eintrag.file_type().expect("Dateityp nicht lesbar");
            if typ.is_dir() {
                sammeln(&pfad, dateien);
            } else if typ.is_file()
                && pfad.extension().is_some_and(|endung| endung == "rs")
                && !ist_testdatei(&pfad)
            {
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

fn relativ(pfad: &Path) -> String {
    pfad.strip_prefix(wurzel())
        .unwrap_or(pfad)
        .components()
        .map(|teil| teil.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn nennt_cfg_test(zeile: &str) -> bool {
    let zeile: String = zeile.split_whitespace().collect();
    zeile.starts_with("#[cfg(")
        && !zeile.contains("not(test")
        && (zeile.contains("(test)") || zeile.contains("(test,") || zeile.contains(",test"))
}

fn klammern(zeile: &str) -> (i64, bool) {
    let auf = zeile.matches('{').count() as i64;
    let zu = zeile.matches('}').count() as i64;
    (auf - zu, auf > 0)
}

/// Blendet `#[cfg(test)]`-Elemente aus, die Zeilennummern bleiben erhalten.
/// Die Klammerzaehlung ist bewusst einfach: Findet sie das Ende eines Blocks nicht,
/// gilt der Rest der Datei als Testcode. Zaehlt sie zu frueh zu Ende, landet Testcode
/// in der Pruefung und der Test scheitert sichtbar statt etwas zu uebersehen.
fn ohne_testcode(text: &str) -> Vec<&str> {
    #[derive(PartialEq)]
    enum Lage {
        Produktion,
        VorElement,
        ImBlock,
    }
    let mut lage = Lage::Produktion;
    let mut tiefe = 0_i64;
    let mut zeilen = Vec::new();
    for zeile in text.lines() {
        let kern = zeile.trim();
        match lage {
            Lage::Produktion => {
                if nennt_cfg_test(kern) {
                    lage = Lage::VorElement;
                    zeilen.push("");
                } else {
                    zeilen.push(zeile);
                }
            }
            Lage::VorElement => {
                zeilen.push("");
                if kern.is_empty() || kern.starts_with("#[") || kern.starts_with("//") {
                    continue;
                }
                let (differenz, geoeffnet) = klammern(kern);
                if geoeffnet {
                    tiefe = differenz;
                    lage = if tiefe > 0 {
                        Lage::ImBlock
                    } else {
                        Lage::Produktion
                    };
                } else if kern.ends_with(';') {
                    lage = Lage::Produktion;
                }
            }
            Lage::ImBlock => {
                zeilen.push("");
                tiefe += klammern(kern).0;
                if tiefe <= 0 {
                    lage = Lage::Produktion;
                }
            }
        }
    }
    zeilen
}

fn werkzeug_in_zeile(zeile: &str) -> Option<&'static str> {
    let zeile = zeile.to_lowercase();
    let (_, rest) = zeile.split_once("new(\"")?;
    let (name, rest) = rest.split_once('"')?;
    if !rest.starts_with(')') {
        return None;
    }
    let name = name.strip_suffix(".exe").unwrap_or(name);
    SYSTEMWERKZEUGE
        .iter()
        .copied()
        .find(|werkzeug| *werkzeug == name)
}

fn aufrufe_in_zeile(zeile: &str) -> Vec<&'static str> {
    if zeile.trim_start().starts_with("//") {
        return Vec::new();
    }
    HEIKLE_AUFRUFE
        .iter()
        .copied()
        .filter(|stichwort| zeile.contains(stichwort))
        .collect()
}

type Zaehlung = BTreeMap<(String, &'static str), usize>;

fn zaehlen(je_zeile: impl Fn(&str) -> Vec<&'static str>) -> Zaehlung {
    let mut zaehlung = Zaehlung::new();
    for pfad in produktionsdateien() {
        let text = std::fs::read_to_string(&pfad)
            .unwrap_or_else(|_| panic!("Quelldatei nicht lesbar: {}", pfad.display()));
        for zeile in ohne_testcode(&text) {
            for stichwort in je_zeile(zeile) {
                *zaehlung.entry((relativ(&pfad), stichwort)).or_default() += 1;
            }
        }
    }
    zaehlung
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
                "NEU {datei}: `{stichwort}` in {anzahl} Zeilen, erlaubt {erlaubt}"
            ));
        }
    }
    for ((datei, stichwort), erlaubt) in &soll {
        let anzahl = ist.get(&(datei.clone(), *stichwort)).copied().unwrap_or(0);
        if anzahl < *erlaubt {
            meldungen.push(format!(
                "ENTFALLEN {datei}: `{stichwort}` nur noch in {anzahl} Zeilen, Positivliste nennt {erlaubt} (Eintrag kuerzen)"
            ));
        }
    }
    meldungen
}

fn hook_crates(cargo_toml: &str) -> Vec<&'static str> {
    HOOK_CRATES
        .iter()
        .copied()
        .filter(|name| {
            cargo_toml.lines().any(|zeile| {
                let zeile = zeile.trim_start();
                zeile
                    .strip_prefix(name)
                    .is_some_and(|rest| rest.trim_start().starts_with('=') || rest.starts_with('.'))
            })
        })
        .collect()
}

const HINWEIS: &str = "Regel und Vorgehen: apps/local-voice/AGENTS.md, Abschnitt \
\"EDR-relevantes Verhalten\" (lva#872).";

#[test]
fn kein_neuer_start_von_windows_systemwerkzeugen() {
    let ist = zaehlen(|zeile| werkzeug_in_zeile(zeile).into_iter().collect());
    let meldungen = abweichungen(&ist, ERLAUBTE_SYSTEMWERKZEUGE);
    assert!(
        meldungen.is_empty(),
        "Systemwerkzeuge weichen von der Positivliste ab:\n{}\n{HINWEIS}",
        meldungen.join("\n")
    );
}

#[test]
fn keine_neuen_hooks_und_kein_neuer_zugriff_auf_fremde_prozesse() {
    let ist = zaehlen(aufrufe_in_zeile);
    let meldungen = abweichungen(&ist, ERLAUBTE_AUFRUFE);
    assert!(
        meldungen.is_empty(),
        "Heikle Aufrufe weichen von der Positivliste ab:\n{}\n{HINWEIS}",
        meldungen.join("\n")
    );
}

#[test]
fn keine_neue_abhaengigkeit_mit_systemweiten_eingabe_hooks() {
    let text = std::fs::read_to_string(wurzel().join("Cargo.toml")).expect("Cargo.toml");
    assert_eq!(
        hook_crates(&text),
        ERLAUBTE_HOOK_CRATES,
        "Hook-Crates in Cargo.toml weichen von der Positivliste ab. {HINWEIS}"
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
        assert!(
            SYSTEMWERKZEUGE.contains(stichwort) || HEIKLE_AUFRUFE.contains(stichwort),
            "{datei}: `{stichwort}` steht in keiner Pruefliste"
        );
    }
}

#[test]
fn die_pruefung_erkennt_die_gegenproben() {
    // Systemwerkzeuge: mit und ohne .exe, in jeder Schreibweise, nicht als Teilwort.
    assert_eq!(
        werkzeug_in_zeile("Command::new(\"taskkill\")"),
        Some("taskkill")
    );
    assert_eq!(
        werkzeug_in_zeile("let mut c = Command::new(\"CMD.EXE\");"),
        Some("cmd")
    );
    assert_eq!(werkzeug_in_zeile("Command::new(\"netzwerk\")"), None);
    assert_eq!(werkzeug_in_zeile("Command::new(ffmpeg_pfad)"), None);

    // Heikle Aufrufe: Treffer im Code, nicht im Kommentar.
    assert_eq!(
        aufrufe_in_zeile("SetWindowsHookExW(WH_KEYBOARD_LL, Some(haken), None, 0)"),
        vec!["SetWindowsHookEx", "WH_KEYBOARD_LL"]
    );
    assert!(aufrufe_in_zeile("// kein SetWindowsHookExW mehr").is_empty());
    assert!(aufrufe_in_zeile("PROCESS_QUERY_LIMITED_INFORMATION").is_empty());

    // Testcode in Produktionsdateien zaehlt nicht, Code davor und danach schon.
    let text = "fn a() { Command::new(\"cmd\"); }\n\
                #[cfg(test)]\n\
                mod tests {\n\
                    fn t() { Command::new(\"taskkill\"); }\n\
                }\n\
                #[cfg(test)]\n\
                mod weitere_tests;\n\
                #[cfg(all(test, windows))]\n\
                fn hilfe() {\n\
                    Command::new(\"netstat\");\n\
                }\n\
                #[cfg(not(test))]\n\
                fn b() { Command::new(\"icacls\"); }\n";
    let funde: Vec<_> = ohne_testcode(text)
        .into_iter()
        .filter_map(werkzeug_in_zeile)
        .collect();
    assert_eq!(funde, vec!["cmd", "icacls"]);
    assert_eq!(ohne_testcode(text).len(), text.lines().count());

    // Abweichungen in beide Richtungen.
    let mut ist = Zaehlung::new();
    ist.insert(("src/a.rs".to_string(), "cmd"), 2);
    let erlaubt: &[Erlaubt] = &[("src/a.rs", "cmd", 1, "lva#1 Grund")];
    assert!(abweichungen(&ist, erlaubt)[0].starts_with("NEU src/a.rs"));
    let erlaubt: &[Erlaubt] = &[("src/a.rs", "cmd", 3, "lva#1 Grund")];
    assert!(abweichungen(&ist, erlaubt)[0].starts_with("ENTFALLEN src/a.rs"));
    let erlaubt: &[Erlaubt] = &[("src/a.rs", "cmd", 2, "lva#1 Grund")];
    assert!(abweichungen(&ist, erlaubt).is_empty());
    assert!(abweichungen(&Zaehlung::new(), erlaubt)[0].starts_with("ENTFALLEN"));

    // Hook-Crates: einfache und Tabellen-Schreibweise, keine Teilnamen.
    assert_eq!(hook_crates("rdev = \"0.5\"\n"), vec!["rdev"]);
    assert_eq!(
        hook_crates("handy-keys.workspace = true\n"),
        vec!["handy-keys"]
    );
    assert!(hook_crates("rdev-fork = \"1\"\n# rdev = \"0.5\"\n").is_empty());
}
