//! Quelltext-Tests zu lva#739: nichts, was ein EDR als schadsoftwareaehnlich einstuft.
//! Sie lesen die Quellen der Crate und scheitern mit Datei und Zeile.

use std::path::{Path, PathBuf};

fn quelldateien() -> Vec<PathBuf> {
    fn sammeln(ordner: &Path, dateien: &mut Vec<PathBuf>) {
        for eintrag in std::fs::read_dir(ordner).expect("Quellordner nicht lesbar") {
            let eintrag = eintrag.expect("Verzeichniseintrag nicht lesbar");
            let pfad = eintrag.path();
            let typ = eintrag.file_type().expect("Dateityp nicht lesbar");
            if typ.is_dir() {
                sammeln(&pfad, dateien);
            } else if typ.is_file()
                && pfad.extension().is_some_and(|endung| endung == "rs")
                && !matches!(
                    pfad.file_name().and_then(|name| name.to_str()),
                    Some("firmentauglich_tests.rs" | "ziele.rs")
                )
            {
                dateien.push(pfad);
            }
        }
    }

    let wurzel = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut dateien = Vec::new();
    sammeln(&wurzel.join("src"), &mut dateien);
    for name in [
        "build.rs",
        "nsis/installer.nsi",
        "tauri.conf.json",
        "tauri.windows.conf.json",
    ] {
        let pfad = wurzel.join(name);
        if pfad.is_file() {
            dateien.push(pfad);
        }
    }
    dateien.sort();
    dateien
}

fn fundzeilen(text: &str, muster: &[&str]) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter_map(|(index, zeile)| {
            let zeile = zeile.to_lowercase();
            muster
                .iter()
                .any(|muster| zeile.contains(muster))
                .then_some(index + 1)
        })
        .collect()
}

fn treffer(muster: &[&str]) -> Vec<String> {
    let mut fundstellen = Vec::new();
    for pfad in quelldateien() {
        let text = std::fs::read_to_string(&pfad)
            .unwrap_or_else(|_| panic!("Quelldatei nicht lesbar: {}", pfad.display()));
        for zeile in fundzeilen(&text, muster) {
            fundstellen.push(format!("{}:{zeile}", pfad.display()));
        }
    }
    fundstellen
}

fn ist_testdatei(pfad: &Path) -> bool {
    let name = pfad
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    name == "tests.rs"
        || name.ends_with("_tests.rs")
        || pfad.components().any(|teil| teil.as_os_str() == "tests")
}

fn listener_ohne_loopback(text: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter_map(|(index, zeile)| {
            let zeile = zeile.to_lowercase();
            let bindet = zeile.contains("tcplistener::bind(") || zeile.contains("udpsocket::bind(");
            let loopback = ["127.0.0.1", "localhost", "ipv4addr::localhost", "[::1]"]
                .iter()
                .any(|adresse| zeile.contains(adresse));
            (bindet && !loopback).then_some(index + 1)
        })
        .collect()
}

#[test]
fn kein_codepfad_laedt_skripte_zur_ausfuehrung_oder_umgeht_die_ausfuehrungsrichtlinie() {
    // Nachgeladene Skripte und Richtlinienumgehungen sind fuer EDR ein Red Flag.
    let muster = [
        concat!("-encoded", "command"),
        concat!("-execution", "policy"),
        concat!("executionpolicy ", "bypass"),
        concat!("| ", "iex"),
        concat!("|", "iex"),
        concat!("invoke-", "expression"),
        concat!("download", "string("),
        concat!("irm ", "http"),
        concat!("iwr ", "http"),
        concat!("invoke-", "webrequest"),
        concat!("curl ", "-fssl"),
        concat!("| ", "bash"),
        concat!("| ", "sh -"),
        concat!("frombase64", "string"),
    ];
    // Gegenprobe: Jedes verbotene Muster muss auch in gemischter Schreibweise anschlagen.
    for muster in muster {
        assert_eq!(fundzeilen(&muster.to_uppercase(), &[muster]), vec![1]);
    }
    let fundstellen = treffer(&muster);
    assert!(fundstellen.is_empty(), "Skriptausfuehrung: {fundstellen:?}");
}

#[test]
fn powershell_wird_von_keinem_produktionscode_gestartet() {
    // Ein eigener PowerShell-Start erschwert die Freigabe durch die IT-Sicherheit.
    let muster = [concat!("new(\"power", "shell"), concat!("new(\"pw", "sh")];
    for programm in ["powershell", "powershell.exe", "pwsh", "pwsh.exe"] {
        let text = format!("Command::new(\"{programm}\")");
        assert_eq!(fundzeilen(&text, &muster), vec![1]);
    }
    assert!(ist_testdatei(Path::new("src/tests.rs")));
    assert!(ist_testdatei(Path::new("src/proxy_tests.rs")));
    assert!(ist_testdatei(Path::new("src/tests/proxy.rs")));
    assert!(!ist_testdatei(Path::new("src/proxy.rs")));
    let mut fundstellen = Vec::new();
    for pfad in quelldateien()
        .into_iter()
        .filter(|pfad| !ist_testdatei(pfad))
    {
        let text = std::fs::read_to_string(&pfad)
            .unwrap_or_else(|_| panic!("Quelldatei nicht lesbar: {}", pfad.display()));
        for zeile in fundzeilen(&text, &muster) {
            fundstellen.push(format!("{}:{zeile}", pfad.display()));
        }
    }
    assert!(fundstellen.is_empty(), "PowerShell-Start: {fundstellen:?}");
}

#[test]
fn lokale_listener_binden_nur_an_loopback() {
    // Lokale Hilfsdienste duerfen im Firmennetz nicht von anderen Rechnern erreichbar sein.
    assert_eq!(
        listener_ohne_loopback("TcpListener::bind(\"0.0.0.0:80\")"),
        vec![1]
    );
    assert_eq!(listener_ohne_loopback("UdpSocket::bind(adresse)"), vec![1]);
    assert!(listener_ohne_loopback("TcpListener::bind(\"127.0.0.1:0\")").is_empty());
    let mut fundstellen = Vec::new();
    for pfad in quelldateien() {
        let text = std::fs::read_to_string(&pfad)
            .unwrap_or_else(|_| panic!("Quelldatei nicht lesbar: {}", pfad.display()));
        for zeile in listener_ohne_loopback(&text) {
            fundstellen.push(format!("{}:{zeile}", pfad.display()));
        }
    }
    assert!(
        fundstellen.is_empty(),
        "Listener ohne Loopback: {fundstellen:?}"
    );
}

#[test]
fn der_weiterleiter_fuer_die_proxy_anmeldung_bindet_127_0_0_1_mit_zufallsport() {
    // Der SSPI-Weiterleiter darf nur lokal und ohne festen Serverport erreichbar sein.
    let text = include_str!("proxy_auth.rs");
    let bind = "TcpListener::bind(\"127.0.0.1:0\")";
    assert!(
        text.contains(bind),
        "src/netz/proxy_auth.rs:1: Loopback-Bindung fehlt"
    );
    let fundstellen = fundzeilen(text, &["0.0.0.0"]);
    assert!(
        fundstellen.is_empty(),
        "src/netz/proxy_auth.rs: unzulaessige Bindung in Zeilen {fundstellen:?}"
    );
    // Gegenprobe: Fester Port und Wildcard duerfen die Bindung nicht erfuellen.
    assert!(!"TcpListener::bind(\"127.0.0.1:8080\")".contains(bind));
    assert_eq!(fundzeilen("0.0.0.0", &["0.0.0.0"]), vec![1]);
}

#[derive(Default)]
struct Paket {
    name: String,
    version: String,
    zeile: usize,
    abhaengigkeiten: Vec<(String, usize)>,
}

fn lock_pakete(text: &str) -> Vec<Paket> {
    let mut pakete = Vec::new();
    let mut aktuell: Option<Paket> = None;
    let mut in_abhaengigkeiten = false;
    for (index, zeile) in text.lines().enumerate() {
        let zeile = zeile.trim();
        if zeile == "[[package]]" {
            if let Some(paket) = aktuell.take() {
                pakete.push(paket);
            }
            aktuell = Some(Paket {
                zeile: index + 1,
                ..Paket::default()
            });
            in_abhaengigkeiten = false;
        } else if let Some(paket) = aktuell.as_mut() {
            if in_abhaengigkeiten {
                if zeile == "]" {
                    in_abhaengigkeiten = false;
                } else if let Some(name) = zeile.strip_prefix('"') {
                    let name = name.split('"').next().unwrap().split_whitespace().next();
                    if let Some(name) = name {
                        paket.abhaengigkeiten.push((name.to_string(), index + 1));
                    }
                }
            } else if zeile == "dependencies = [" {
                in_abhaengigkeiten = true;
            } else if let Some(name) = zeile.strip_prefix("name = \"") {
                paket.name = name.trim_end_matches('"').to_string();
            } else if let Some(version) = zeile.strip_prefix("version = \"") {
                paket.version = version.trim_end_matches('"').to_string();
            }
        }
    }
    if let Some(paket) = aktuell {
        pakete.push(paket);
    }
    pakete
}

fn tls_fundstellen(text: &str) -> Vec<String> {
    let mut fundstellen = Vec::new();
    for paket in lock_pakete(text) {
        let hat = |name: &str| paket.abhaengigkeiten.iter().any(|(dep, _)| dep == name);
        for (name, zeile) in &paket.abhaengigkeiten {
            if (name == "webpki-roots" || name == "webpki-root-certs")
                && !["ureq", "rustls-platform-verifier"].contains(&paket.name.as_str())
            {
                fundstellen.push(format!("Cargo.lock:{zeile}: {} nutzt {name}", paket.name));
            }
        }
        if paket.name == "reqwest" {
            if hat("webpki-roots") {
                fundstellen.push(format!(
                    "Cargo.lock:{}: reqwest nutzt webpki-roots",
                    paket.zeile
                ));
            }
            let fehlt = if paket.version.starts_with("0.13.") {
                !hat("rustls-platform-verifier")
            } else if paket.version.starts_with("0.12.") {
                !hat("native-tls") && !hat("hyper-tls")
            } else {
                false
            };
            if fehlt {
                fundstellen.push(format!(
                    "Cargo.lock:{}: reqwest {} ohne Betriebssystem-Pruefung",
                    paket.zeile, paket.version
                ));
            }
        }
    }
    fundstellen
}

#[test]
fn kein_netzweg_nutzt_eingebaute_webpki_wurzeln() {
    // Firmen-CAs muessen ueber den Windows-Speicher greifen, auch bei TLS-Inspektion.
    // Nur ureq ist erlaubt: ungenutzter synchroner hf-hub-Zweig und Build von ort-sys.
    // rustls-platform-verifier haengt im Lockfile an webpki-root-certs, nutzt unter Windows aber den
    // Systemspeicher (`cargo tree -i webpki-root-certs --target x86_64-pc-windows-msvc`: nur ureq).
    let text = include_str!("../../Cargo.lock");
    let fundstellen = tls_fundstellen(text);
    assert!(fundstellen.is_empty(), "TLS-Konfiguration: {fundstellen:?}");
}

#[test]
fn tls_pruefung_erkennt_die_gegenproben() {
    // Gegenproben verhindern, dass neue Lockfile-Versionen die Firmen-CA-Pruefung aushebeln.
    let paket = |name: &str, version: &str, deps: &str| {
        format!(
            "[[package]]\nname = \"{name}\"\nversion = \"{version}\"\ndependencies = [\n{deps}\n]\n"
        )
    };
    for name in ["webpki-roots", "webpki-root-certs"] {
        let deps = format!(" \"{name} 1.0.0\",");
        assert!(!tls_fundstellen(&paket("anderes-paket", "1.0.0", &deps)).is_empty());
        assert!(tls_fundstellen(&paket("ureq", "3.2.0", &deps)).is_empty());
    }
    for (version, dep) in [
        ("0.13.0", "rustls-platform-verifier"),
        ("0.12.0", "native-tls"),
        ("0.12.0", "hyper-tls"),
    ] {
        assert!(!tls_fundstellen(&paket("reqwest", version, "")).is_empty());
        let deps = format!(" \"{dep}\",");
        assert!(tls_fundstellen(&paket("reqwest", version, &deps)).is_empty());
        let deps = format!("{deps}\n \"webpki-roots\",");
        assert!(!tls_fundstellen(&paket("reqwest", version, &deps)).is_empty());
    }
}

#[test]
fn hf_hub_wird_nur_asynchron_genutzt() {
    // Der synchrone hf-hub-Zweig nutzt ureq mit Mozilla-Wurzeln statt der Firmen-CAs.
    let muster = concat!("hf_hub::api::", "sync");
    assert_eq!(fundzeilen(muster, &[muster]), vec![1]);
    let fundstellen = treffer(&[muster]);
    assert!(fundstellen.is_empty(), "Synchrones hf-hub: {fundstellen:?}");
}

#[test]
fn abo_download_startet_weder_installer_noch_paketmanager_und_loggt_keine_antworten() {
    let text = include_str!("../managers/llm/cli_install.rs");
    let verboten = ["command::", "log::", ".extract("];
    for muster in verboten {
        assert_eq!(fundzeilen(muster, &[muster]), vec![1]);
        assert_eq!(fundzeilen(&format!("harmlos\n{}", muster.to_uppercase()), &[muster]), vec![2]);
        assert!(fundzeilen("harmlos", &[muster]).is_empty());
        assert!(fundzeilen(text, &[muster]).is_empty(), "Abo-Download: {muster}");
    }
    // Gegenprobe: das Entfernen eines Hosts darf die Freigabeliste nicht erfuellen.
    for host in crate::managers::llm::cli_install::HOSTS_CLAUDE.iter()
        .chain(crate::managers::llm::cli_install::HOSTS_CODEX)
    {
        assert!(super::ziele::ZIELE.iter().any(|z| z.host == *host));
        assert!(!super::ziele::ZIELE.iter().filter(|z| z.host != *host).any(|z| z.host == *host));
    }
}

#[test]
fn kein_npm_start_im_produktionscode() {
    let muster = [concat!("new(\"n", "pm\""), concat!("new(\"n", "pm.cmd\"")];
    for m in muster {
        assert_eq!(fundzeilen(m, &[m]), vec![1]);
    }
    assert!(treffer(&muster).is_empty());
}
