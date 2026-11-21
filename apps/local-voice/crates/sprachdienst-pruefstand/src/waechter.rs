//! Wächter gegen Drift zwischen Prüfstand und App. Sie prüfen nicht den Dienst, sondern dass der
//! Prüfstand noch das abbildet, was die App baut.
//!
//! Jeder Wächter besteht aus einer reinen Funktion, die Abweichungen als Text liefert, einem Test
//! gegen die echten Dateien und Gegenproben mit erfundenen Eingaben (`gegenprobe_…`): Die
//! Gegenproben belegen bei jedem Lauf, dass der Wächter die Abweichung auch wirklich meldet.
//!
//! Grenzen, siehe README: Die Wächter vergleichen Erklärungen (Modulzeilen, Abhängigkeiten,
//! Sperrdatei). Welche Features Cargo in der App am Ende einschaltet, bilden sie nicht nach.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

fn app() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri")
}

fn lies(p: PathBuf) -> String {
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

// ---------------------------------------------------------------------------------------------
// Modulwurzeln
// ---------------------------------------------------------------------------------------------

/// Eine Zeile `mod x;` oder `pub mod x;` mit den Attributzeilen direkt davor.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Modul {
    attribute: Vec<String>,
    oeffentlich: bool,
    name: String,
}

/// Inhalt einer Modulwurzel, zeilenweise gelesen. Alles, was weder Kommentar noch Attribut noch
/// Modulzeile ist, landet in `fremd`: Der Leser versteht absichtlich nur diese Form und meldet
/// jede andere, statt sie zu deuten.
#[derive(Debug, Default, PartialEq, Eq)]
struct Wurzel {
    innen: Vec<String>,
    module: Vec<Modul>,
    fremd: Vec<String>,
}

fn modulzeile(zeile: &str) -> Option<(bool, &str)> {
    let (oeffentlich, rest) = match zeile.strip_prefix("pub mod ") {
        Some(rest) => (true, rest),
        None => (false, zeile.strip_prefix("mod ")?),
    };
    let (name, danach) = rest.split_once(';')?;
    let danach = danach.trim();
    let name_gueltig = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    (name_gueltig && (danach.is_empty() || danach.starts_with("//"))).then_some((oeffentlich, name))
}

fn wurzel(text: &str) -> Wurzel {
    let mut w = Wurzel::default();
    let mut offen: Vec<String> = Vec::new();
    for zeile in text.lines().map(str::trim) {
        if zeile.is_empty() || zeile.starts_with("//") {
            continue;
        }
        if zeile.starts_with("#![") {
            w.innen.push(zeile.to_string());
        } else if zeile.starts_with("#[") && zeile.ends_with(']') {
            offen.push(zeile.to_string());
        } else if let Some((oeffentlich, name)) = modulzeile(zeile) {
            w.module.push(Modul {
                attribute: std::mem::take(&mut offen),
                oeffentlich,
                name: name.to_string(),
            });
        } else {
            w.fremd.append(&mut offen);
            w.fremd.push(zeile.to_string());
        }
    }
    w.fremd.append(&mut offen);
    w
}

/// Verzeichnis der Originaldateien, vom Spiegel `src/sprachdienst/mod.rs` aus gesehen.
const QUELLE: &str = "../../../../src-tauri/src/sprachdienst";

/// Bewusst nicht eingebunden, jeweils mit den Attributen, die das Original dafür trägt:
/// `app` (Tauri-Anbindung) und `pipe` (Windows).
const AUSGELASSEN: [(&str, &[&str]); 2] = [("app", &[]), ("pipe", &["#[cfg(windows)]"])];

/// Vergleicht die Modulwurzel des Dienstes mit dem Spiegel im Prüfstand.
///
/// Der Spiegel wird aus dem Original **abgeleitet** und muss dem Ergebnis genau gleichen: je
/// Modul dieselbe Sichtbarkeit, dieselben Bedingungen (`cfg(unix)` entfällt, weil das ganze Crate
/// darunter steht) und der Pfad zur gleichnamigen Originaldatei. Eine Umlenkung auf eine andere
/// Datei, ein anderes `cfg` oder eine zusätzliche Zeile auf einer der beiden Seiten ist eine
/// Abweichung. Kennt der Wächter eine Form im Original nicht, meldet er auch das: Dann muss ein
/// Mensch entscheiden, wie der Prüfstand sie abbildet.
fn spiegel_abweichungen(original: &str, spiegel: &str) -> Vec<String> {
    let (o, s) = (wurzel(original), wurzel(spiegel));
    let mut ab = Vec::new();
    for z in o.innen.iter().chain(&o.fremd) {
        ab.push(format!("Original: unerwartete Zeile `{z}`"));
    }
    for z in s.innen.iter().chain(&s.fremd) {
        ab.push(format!("Spiegel: unerwartete Zeile `{z}`"));
    }

    let mut erwartet: BTreeMap<&str, Modul> = BTreeMap::new();
    let mut gesehen: BTreeSet<&str> = BTreeSet::new();
    for m in &o.module {
        if !gesehen.insert(&m.name) {
            ab.push(format!("Original: Modul {} steht mehrfach da", m.name));
        }
        if let Some((_, attribute)) = AUSGELASSEN.iter().find(|(n, _)| *n == m.name) {
            if m.attribute != *attribute {
                ab.push(format!(
                    "Original: ausgelassenes Modul {} trägt {:?} statt {:?}",
                    m.name, m.attribute, attribute
                ));
            }
            continue;
        }
        let mut attribute = Vec::new();
        for a in &m.attribute {
            match a.as_str() {
                "#[cfg(unix)]" => {}
                "#[cfg(test)]" => attribute.push(a.clone()),
                _ => ab.push(format!(
                    "Original: Modul {} trägt ein Attribut, das der Prüfstand nicht abbildet: `{a}`",
                    m.name
                )),
            }
        }
        attribute.push(format!("#[path = \"{QUELLE}/{}.rs\"]", m.name));
        erwartet.insert(
            &m.name,
            Modul {
                attribute,
                oeffentlich: m.oeffentlich,
                name: m.name.clone(),
            },
        );
    }
    for (name, _) in AUSGELASSEN {
        if !gesehen.contains(name) {
            ab.push(format!(
                "Original: ausgelassenes Modul {name} gibt es nicht mehr"
            ));
        }
    }

    let mut vorhanden: BTreeMap<&str, &Modul> = BTreeMap::new();
    for m in &s.module {
        if vorhanden.insert(&m.name, m).is_some() {
            ab.push(format!("Spiegel: Modul {} steht mehrfach da", m.name));
        }
    }
    for (name, soll) in &erwartet {
        match vorhanden.get(name) {
            None => ab.push(format!("Spiegel: Modul {name} fehlt")),
            Some(ist) if *ist != soll => ab.push(format!(
                "Spiegel: Modul {name} ist {ist:?}, erwartet {soll:?}"
            )),
            Some(_) => {}
        }
    }
    for name in vorhanden.keys() {
        if !erwartet.contains_key(name) {
            ab.push(format!(
                "Spiegel: Modul {name} steht nicht im Original oder ist dort ausgelassen"
            ));
        }
    }
    ab
}

#[test]
fn module_folgen_dem_dienst() {
    let ab = spiegel_abweichungen(
        &lies(app().join("src/sprachdienst/mod.rs")),
        include_str!("sprachdienst/mod.rs"),
    );
    assert!(ab.is_empty(), "{ab:#?}");
}

/// Die Wurzel des Prüfstands enthält nur die drei Modulzeilen. Sonst ließe sich der Spiegel dort
/// an den Wächtern vorbei umlenken oder ergänzen.
#[test]
fn wurzel_ist_unveraendert() {
    let modul = |attribute: &[&str], oeffentlich, name: &str| Modul {
        attribute: attribute.iter().map(|a| a.to_string()).collect(),
        oeffentlich,
        name: name.to_string(),
    };
    let erwartet = Wurzel {
        innen: vec!["#![cfg(unix)]".to_string()],
        module: vec![
            modul(
                &[
                    "#[cfg(test)]",
                    "#[path = \"../../../src-tauri/src/testlog.rs\"]",
                ],
                false,
                "testlog",
            ),
            modul(&[], true, "sprachdienst"),
            modul(&["#[cfg(test)]"], false, "waechter"),
        ],
        fremd: vec![],
    };
    assert_eq!(wurzel(include_str!("lib.rs")), erwartet);
}

/// So bindet die App die beiden Module ein, die der Prüfstand nachbildet. Ändert sich das (etwa
/// ein `cfg` vor `sprachdienst`), muss der Prüfstand folgen.
#[test]
fn einbindung_in_der_app_ist_unveraendert() {
    let w = wurzel(&lies(app().join("src/lib.rs")));
    let finde =
        |name: &str| -> Vec<&Modul> { w.module.iter().filter(|m| m.name == name).collect() };
    let dienst = finde("sprachdienst");
    assert_eq!(dienst.len(), 1, "{dienst:?}");
    assert!(
        dienst[0].oeffentlich && dienst[0].attribute.is_empty(),
        "{dienst:?}"
    );
    let testlog = finde("testlog");
    assert_eq!(testlog.len(), 1, "{testlog:?}");
    assert!(
        !testlog[0].oeffentlich && testlog[0].attribute == ["#[cfg(test)]"],
        "{testlog:?}"
    );
}

const ORIGINAL: &str = "//! Kopf\n\npub mod app;\npub mod dienst;\npub mod wav;\n\n#[cfg(unix)]\npub mod unix;\n\n#[cfg(windows)]\npub mod pipe;\n\n#[cfg(test)]\nmod tests;\n";

fn spiegel_zu(module: &[(&str, &str, &str)]) -> String {
    module
        .iter()
        .map(|(davor, erklaerung, datei)| {
            format!("{davor}#[path = \"{QUELLE}/{datei}\"]\n{erklaerung}\n")
        })
        .collect()
}

const SPIEGEL: [(&str, &str, &str); 4] = [
    ("", "pub mod dienst;", "dienst.rs"),
    ("", "pub mod unix;", "unix.rs"),
    ("", "pub mod wav;", "wav.rs"),
    ("#[cfg(test)]\n", "mod tests;", "tests.rs"),
];

#[test]
fn gegenprobe_module() {
    let gut = spiegel_zu(&SPIEGEL);
    assert_eq!(spiegel_abweichungen(ORIGINAL, &gut), Vec::<String>::new());

    let meldet = |original: &str, spiegel: &str, teil: &str| {
        let ab = spiegel_abweichungen(original, spiegel);
        assert!(
            ab.iter().any(|a| a.contains(teil)),
            "erwartet eine Meldung mit „{teil}“, bekommen: {ab:#?}"
        );
    };
    let mut s = SPIEGEL;

    // Umlenkung auf eine Ersatzdatei, im selben und in einem anderen Verzeichnis.
    s[0].2 = "ersatz.rs";
    meldet(ORIGINAL, &spiegel_zu(&s), "Spiegel: Modul dienst ist");
    meldet(
        ORIGINAL,
        &gut.replace(&format!("{QUELLE}/dienst.rs"), "ersatz/dienst.rs"),
        "Spiegel: Modul dienst ist",
    );
    // Ohne Pfad: Dann gälte eine Datei des Prüfstands.
    meldet(
        ORIGINAL,
        &gut.replace(&format!("#[path = \"{QUELLE}/wav.rs\"]\n"), ""),
        "Spiegel: Modul wav ist",
    );
    // Andere Bedingung oder andere Sichtbarkeit im Spiegel.
    s = SPIEGEL;
    s[2].0 = "#[cfg(test)]\n";
    meldet(ORIGINAL, &spiegel_zu(&s), "Spiegel: Modul wav ist");
    s = SPIEGEL;
    s[2].1 = "mod wav;";
    meldet(ORIGINAL, &spiegel_zu(&s), "Spiegel: Modul wav ist");
    // Modul fehlt, ist zu viel oder ist eines der ausgelassenen.
    meldet(ORIGINAL, &spiegel_zu(&SPIEGEL[1..]), "Modul dienst fehlt");
    meldet(
        ORIGINAL,
        &format!("{gut}#[path = \"neu.rs\"]\npub mod neu;\n"),
        "Modul neu steht nicht im Original",
    );
    meldet(
        ORIGINAL,
        &format!("{gut}#[path = \"{QUELLE}/app.rs\"]\npub mod app;\n"),
        "Modul app steht nicht im Original oder ist dort ausgelassen",
    );
    // Zusätzliche Verdrahtung im Spiegel.
    meldet(
        ORIGINAL,
        &format!("{gut}pub use dienst::*;\n"),
        "Spiegel: unerwartete Zeile",
    );
    meldet(
        ORIGINAL,
        &format!("#![allow(unused)]\n{gut}"),
        "Spiegel: unerwartete Zeile",
    );

    // Änderungen am Original: neues Modul, neue Bedingung, zusätzliche Verdrahtung.
    meldet(
        &format!("{ORIGINAL}pub mod neu;\n"),
        &gut,
        "Spiegel: Modul neu fehlt",
    );
    meldet(
        &ORIGINAL.replace("pub mod wav;", "#[cfg(feature = \"x\")]\npub mod wav;"),
        &gut,
        "Attribut, das der Prüfstand nicht abbildet",
    );
    meldet(
        &ORIGINAL.replace("pub mod wav;", "#[path = \"anders.rs\"]\npub mod wav;"),
        &gut,
        "Attribut, das der Prüfstand nicht abbildet",
    );
    meldet(
        &ORIGINAL.replace("#[cfg(test)]\nmod tests;", "mod tests;"),
        &gut,
        "Spiegel: Modul tests ist",
    );
    meldet(
        &format!("{ORIGINAL}pub use dienst::Dienst;\n"),
        &gut,
        "Original: unerwartete Zeile",
    );
    meldet(
        &format!("{ORIGINAL}pub mod innen {{\n}}\n"),
        &gut,
        "Original: unerwartete Zeile",
    );
    meldet(
        &ORIGINAL.replace("#[cfg(windows)]\npub mod pipe;", "pub mod pipe;"),
        &gut,
        "ausgelassenes Modul pipe trägt",
    );
    meldet(
        &ORIGINAL.replace("pub mod app;\n", ""),
        &gut,
        "ausgelassenes Modul app gibt es nicht mehr",
    );
}

// ---------------------------------------------------------------------------------------------
// Sperrdatei
// ---------------------------------------------------------------------------------------------

/// Volle Identität eines Pakets in einer Sperrdatei: Name, Version, Quelle.
type Kennung = (String, String, Option<String>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Paket {
    name: String,
    version: String,
    quelle: Option<String>,
    pruefsumme: Option<String>,
    /// Kanten, wie sie in der Sperrdatei stehen: `name`, `name version` oder
    /// `name version (quelle)`. Cargo schreibt nur so viel, wie in **dieser** Datei zur
    /// Eindeutigkeit nötig ist; verglichen wird deshalb erst nach dem Auflösen (`ziel`).
    braucht: Vec<String>,
}

impl Paket {
    fn kennung(&self) -> Kennung {
        (self.name.clone(), self.version.clone(), self.quelle.clone())
    }
}

fn pakete(sperre: &str) -> Vec<Paket> {
    let wert = |block: &str, feld: &str| {
        let anfang = format!("{feld} = \"");
        block
            .lines()
            .find_map(|z| z.strip_prefix(anfang.as_str()))
            .and_then(|z| z.strip_suffix('"'))
            .map(str::to_string)
    };
    let braucht = |block: &str| {
        block
            .lines()
            .skip_while(|z| *z != "dependencies = [")
            .skip(1)
            .take_while(|z| *z != "]")
            .filter_map(|z| z.trim().strip_prefix('"'))
            .filter_map(|z| z.strip_suffix("\","))
            .map(str::to_string)
            .collect()
    };
    sperre
        .split("[[package]]")
        .skip(1)
        .filter_map(|b| {
            Some(Paket {
                name: wert(b, "name")?,
                version: wert(b, "version")?,
                quelle: wert(b, "source"),
                pruefsumme: wert(b, "checksum"),
                braucht: braucht(b),
            })
        })
        .collect()
}

/// Löst eine Kante innerhalb ihrer eigenen Sperrdatei zur vollen Identität des Ziels auf. Eine
/// Kante, die auf kein oder auf mehr als ein Paket passt, ist ein Fehler.
fn ziel(kante: &str, alle: &[Paket]) -> Result<Kennung, String> {
    let mut teile = kante.splitn(3, ' ');
    let name = teile.next().unwrap_or_default();
    let version = teile.next();
    let quelle = teile
        .next()
        .map(|q| q.trim_start_matches('(').trim_end_matches(')'));
    let passend: Vec<&Paket> = alle
        .iter()
        .filter(|p| p.name == name)
        .filter(|p| version.is_none() || version == Some(p.version.as_str()))
        .filter(|p| quelle.is_none() || quelle == p.quelle.as_deref())
        .collect();
    match passend.as_slice() {
        [p] => Ok(p.kennung()),
        [] => Err(format!("Kante `{kante}` zeigt auf kein Paket")),
        _ => Err(format!("Kante `{kante}` ist mehrdeutig")),
    }
}

/// Die aufgelösten Ziele aller Kanten eines Pakets; nicht auflösbare Kanten kommen als Meldung.
fn ziele(p: &Paket, alle: &[Paket], wo: &str, ab: &mut Vec<String>) -> BTreeSet<Kennung> {
    let mut aus = BTreeSet::new();
    for kante in &p.braucht {
        match ziel(kante, alle) {
            Ok(k) => {
                aus.insert(k);
            }
            Err(e) => ab.push(format!("{} {} ({wo}): {e}", p.name, p.version)),
        }
    }
    aus
}

/// Jedes Paket des Prüfstands steht mit derselben Version, **derselben Quelle und derselben
/// Prüfsumme** in der Sperrdatei der App. Seine Kanten zeigen, zur vollen Identität aufgelöst
/// (Name, Version, Quelle), nur auf Pakete, auf die es auch in der App zeigt: Ein Wechsel des
/// Ziels von `b 1` auf `b 2` ist damit eine Abweichung, auch wenn beide Versionen in der App
/// vorkommen.
fn sperre_abweichungen(meine: &[Paket], der_app: &[Paket], selbst: &str) -> Vec<String> {
    let mut ab = Vec::new();
    for p in meine {
        let hier = ziele(p, meine, "Prüfstand", &mut ab);
        if p.name == selbst {
            continue;
        }
        let gleich = der_app.iter().find(|a| a.kennung() == p.kennung());
        let Some(a) = gleich else {
            ab.push(format!(
                "{} {} aus {:?} steht nicht in der Sperrdatei der App",
                p.name, p.version, p.quelle
            ));
            continue;
        };
        if a.pruefsumme != p.pruefsumme {
            ab.push(format!(
                "{} {}: andere Prüfsumme als in der App",
                p.name, p.version
            ));
        }
        let dort = ziele(a, der_app, "App", &mut ab);
        for (name, version, quelle) in hier.difference(&dort) {
            ab.push(format!(
                "{} {}: hängt hier von {name} {version} aus {quelle:?} ab, in der App nicht",
                p.name, p.version
            ));
        }
    }
    ab
}

/// Nach einer Änderung an `src-tauri/Cargo.lock` die Sperrdatei neu ableiten, siehe README.
#[test]
fn sperrdatei_folgt_der_app() {
    let der_app = pakete(&lies(app().join("Cargo.lock")));
    let meine = pakete(include_str!("../Cargo.lock"));
    assert!(
        meine.len() > 20 && meine.iter().any(|p| p.name == env!("CARGO_PKG_NAME")),
        "Cargo.lock des Prüfstands ist nicht lesbar"
    );
    assert!(
        meine.iter().filter(|p| p.pruefsumme.is_some()).count() > 20
            && meine.iter().map(|p| p.braucht.len()).sum::<usize>() > 20,
        "Quellen, Prüfsummen oder Kanten der Sperrdatei sind nicht lesbar"
    );
    // Auch jede Kante der App-Datei muss sich auflösen lassen, sonst vergliche der Wächter Lücken.
    let mut unlesbar = Vec::new();
    let kanten: usize = der_app
        .iter()
        .map(|a| ziele(a, &der_app, "App", &mut unlesbar).len())
        .sum();
    assert!(kanten > 100 && unlesbar.is_empty(), "{unlesbar:#?}");
    let ab = sperre_abweichungen(&meine, &der_app, env!("CARGO_PKG_NAME"));
    assert!(ab.is_empty(), "{ab:#?}");
}

#[test]
fn gegenprobe_sperrdatei() {
    const CRATES: &str = "registry+https://github.com/rust-lang/crates.io-index";
    const GIT: &str = "git+https://example.org/tokio#0123";
    let block = |name: &str, version: &str, quelle: &str, summe: &str, braucht: &[&str]| {
        let mut t = format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n");
        if !quelle.is_empty() {
            t += &format!("source = \"{quelle}\"\nchecksum = \"{summe}\"\n");
        }
        if !braucht.is_empty() {
            t += "dependencies = [\n";
            for b in braucht {
                t += &format!(" \"{b}\",\n");
            }
            t += "]\n";
        }
        t + "\n"
    };
    let kopf = "# Kopf\nversion = 4\n\n";
    let bytes = block("bytes", "1.0.0", CRATES, "dd", &[]);
    let log = block("log", "0.4.29", CRATES, "aa", &[]);
    let schloss = block("parking_lot", "0.12.0", CRATES, "pp", &[]);
    let tokio_alt = block("tokio", "0.2.0", CRATES, "cc", &[]);
    // In der App gibt es `tokio` zweimal; die Kanten nennen dort deshalb die Version. `voll`
    // nennt zusätzlich die Quelle, die dritte Schreibweise einer Kante.
    let voll = format!("tokio 1.49.0 ({CRATES})");
    let app = [
        kopf,
        &block("app", "1.0.0", "", "", &["log", "nutzer", &voll, "alt"]),
        &block("alt", "1.0.0", CRATES, "ll", &["tokio 0.2.0"]),
        &bytes,
        &log,
        &block("mio", "1.0.0", CRATES, "ee", &[]),
        &block("nutzer", "1.0.0", CRATES, "nn", &["tokio 1.49.0"]),
        &schloss,
        &block("tokio", "1.49.0", CRATES, "bb", &["bytes", "mio"]),
        &tokio_alt,
    ]
    .concat();
    let pruefe = |meine: &str| sperre_abweichungen(&pakete(meine), &pakete(&app), "pruefstand");
    // Der Prüfstand sperrt `tokio` nur einmal; seine Kanten nennen deshalb nur den Namen.
    let selbst = block("pruefstand", "0.0.0", "", "", &["log", "nutzer", "tokio"]);
    let nutzer = block("nutzer", "1.0.0", CRATES, "nn", &["tokio"]);
    let mit = |bloecke: &[&str]| [kopf, &selbst, &bytes, &log, &bloecke.concat()].concat();
    let tokio = |version: &str, quelle: &str, summe: &str, braucht: &[&str]| {
        mit(&[&nutzer, &block("tokio", version, quelle, summe, braucht)])
    };

    assert_eq!(
        pruefe(&tokio("1.49.0", CRATES, "bb", &["bytes"])),
        Vec::<String>::new()
    );
    let meldet = |meine: &str, teile: &[&str]| {
        let ab = pruefe(meine);
        assert!(
            ab.len() == teile.len() && teile.iter().all(|t| ab.iter().any(|a| a.contains(t))),
            "erwartet {teile:?}: {ab:#?}"
        );
    };
    // Version, die die App nicht sperrt; damit zeigt auch die Kante von `nutzer` woandershin.
    meldet(
        &tokio("1.50.0", CRATES, "bb", &["bytes"]),
        &[
            "tokio 1.50.0 aus",
            "nutzer 1.0.0: hängt hier von tokio 1.50.0",
        ],
    );
    // Gleiche Version aus anderer Quelle.
    meldet(
        &tokio("1.49.0", GIT, "bb", &["bytes"]),
        &[
            "tokio 1.49.0 aus Some(\"git+",
            "nutzer 1.0.0: hängt hier von tokio 1.49.0 aus Some(\"git+",
        ],
    );
    // Gleiche Version und Quelle, anderer Inhalt.
    meldet(
        &tokio("1.49.0", CRATES, "ff", &["bytes"]),
        &["tokio 1.49.0: andere Prüfsumme"],
    );
    // Zielwechsel: `nutzer` zeigt hier auf `tokio 0.2.0`, in der App auf `tokio 1.49.0`. Beide
    // Pakete stehen unverändert in der App; nur die aufgelöste Kante verrät den Unterschied.
    meldet(
        &mit(&[&nutzer, &tokio_alt]),
        &["nutzer 1.0.0: hängt hier von tokio 0.2.0"],
    );
    // Zusätzliche Kante: Hier wäre ein Feature an, das die App nicht einschaltet.
    meldet(
        &[
            tokio("1.49.0", CRATES, "bb", &["bytes", "parking_lot"]),
            schloss.clone(),
        ]
        .concat(),
        &["tokio 1.49.0: hängt hier von parking_lot 0.12.0"],
    );
    // Kante ohne Ziel und mehrdeutige Kante im Prüfstand.
    meldet(
        &tokio("1.49.0", CRATES, "bb", &["bytes", "fehlt"]),
        &["tokio 1.49.0 (Prüfstand): Kante `fehlt` zeigt auf kein Paket"],
    );
    meldet(
        &[tokio("1.49.0", CRATES, "bb", &["bytes"]), tokio_alt.clone()].concat(),
        &[
            "pruefstand 0.0.0 (Prüfstand): Kante `tokio` ist mehrdeutig",
            "nutzer 1.0.0 (Prüfstand): Kante `tokio` ist mehrdeutig",
        ],
    );

    // Die drei Schreibweisen einer Kante lösen sich in der App-Datei zum selben Paket auf.
    let alle = pakete(&app);
    let ziel_von = |kante: &str| ziel(kante, &alle).map(|(_, version, _)| version);
    assert_eq!(ziel_von("mio"), Ok("1.0.0".to_string()));
    assert_eq!(ziel_von("tokio 0.2.0"), Ok("0.2.0".to_string()));
    assert_eq!(ziel_von(&voll), Ok("1.49.0".to_string()));
    assert!(ziel_von("tokio").is_err() && ziel_von("mio 2.0.0").is_err());

    // Gleicher Name und gleiche Version aus zwei Quellen in derselben Sperrdatei: Nur die Kante
    // mit Quelle ist dort eindeutig, und nur die Quelle des aufgelösten Ziels verrät einen Wechsel.
    let reg = format!("b 1.0.0 ({CRATES})");
    let git = format!("b 1.0.0 ({GIT})");
    let b_reg = block("b", "1.0.0", CRATES, "rr", &[]);
    let b_git = block("b", "1.0.0", GIT, "gg", &[]);
    let a = |kante: &str| block("a", "1.0.0", CRATES, "a1", &[kante]);
    let c = |kante: &str| block("c", "1.0.0", CRATES, "c1", &[kante]);
    let wurzel = block("pruefstand", "0.0.0", "", "", &["a"]);
    let app = [
        kopf,
        &block("app", "1.0.0", "", "", &["a", "c"]),
        &a(&reg),
        &b_reg,
        &b_git,
        &c(&git),
    ]
    .concat();
    let alle = pakete(&app);
    let quelle_von = |kante: &str| ziel(kante, &alle).map(|(_, _, quelle)| quelle);
    assert_eq!(quelle_von(&reg), Ok(Some(CRATES.to_string())));
    assert_eq!(quelle_von(&git), Ok(Some(GIT.to_string())));
    assert!(quelle_von("b").is_err() && quelle_von("b 1.0.0").is_err());
    let pruefe = |bloecke: &[&str]| {
        let meine = [kopf, &wurzel, &bloecke.concat()].concat();
        sperre_abweichungen(&pakete(&meine), &alle, "pruefstand")
    };
    // Der Prüfstand sperrt `b` nur einmal und nennt in der Kante deshalb nur den Namen.
    assert_eq!(pruefe(&[&a("b"), &b_reg]), Vec::<String>::new());
    // Quellenwechsel: `a` zeigt hier auf das `b` aus Git. Dieses Paket steht unverändert in der
    // App, Name und Version der Kante sind gleich; nur die aufgelöste Quelle unterscheidet sich.
    let ab = pruefe(&[&a("b"), &b_git]);
    assert!(
        ab.len() == 1 && ab[0].contains("a 1.0.0: hängt hier von b 1.0.0 aus Some(\"git+"),
        "{ab:#?}"
    );
    // Beide Quellen auch im Prüfstand: richtig zugeordnet ohne Befund, vertauscht zwei Befunde.
    let wurzel = block("pruefstand", "0.0.0", "", "", &["a", "c"]);
    let pruefe = |bloecke: &[&str]| {
        let meine = [kopf, &wurzel, &bloecke.concat()].concat();
        sperre_abweichungen(&pakete(&meine), &alle, "pruefstand")
    };
    assert_eq!(
        pruefe(&[&a(&reg), &b_reg, &b_git, &c(&git)]),
        Vec::<String>::new()
    );
    let ab = pruefe(&[&a(&git), &b_reg, &b_git, &c(&reg)]);
    assert!(
        ab.len() == 2
            && ab[0].contains("a 1.0.0: hängt hier von b 1.0.0 aus Some(\"git+")
            && ab[1].contains("c 1.0.0: hängt hier von b 1.0.0 aus Some(\"registry+"),
        "{ab:#?}"
    );
    // Eine Kante ohne Quelle ist im Prüfstand dann selbst mehrdeutig.
    let ab = pruefe(&[&a("b 1.0.0"), &b_reg, &b_git, &c(&git)]);
    assert!(
        ab.len() == 1 && ab[0].contains("a 1.0.0 (Prüfstand): Kante `b 1.0.0` ist mehrdeutig"),
        "{ab:#?}"
    );
}

// ---------------------------------------------------------------------------------------------
// Abhängigkeiten
// ---------------------------------------------------------------------------------------------

/// Eine erklärte Abhängigkeit, so wie `cargo metadata` sie aus einer `Cargo.toml` liest.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Abhaengigkeit {
    name: String,
    /// `None` (normal), `dev` oder `build`.
    art: Option<String>,
    anforderung: String,
    quelle: Option<String>,
    pfad: Option<PathBuf>,
    umbenannt: Option<String>,
    register: Option<String>,
    optional: bool,
    standard_features: bool,
    features: BTreeSet<String>,
    ziel: Option<String>,
}

fn abhaengigkeit(v: &Value) -> Abhaengigkeit {
    let text = |feld: &str| v[feld].as_str().map(str::to_string);
    let ja = |feld: &str| {
        v[feld]
            .as_bool()
            .unwrap_or_else(|| panic!("{feld} fehlt: {v}"))
    };
    Abhaengigkeit {
        name: text("name").expect("name"),
        art: text("kind"),
        anforderung: text("req").expect("req"),
        quelle: text("source"),
        pfad: text("path").map(|p| {
            Path::new(&p)
                .canonicalize()
                .unwrap_or_else(|e| panic!("{p}: {e}"))
        }),
        umbenannt: text("rename"),
        register: text("registry"),
        optional: ja("optional"),
        standard_features: ja("uses_default_features"),
        features: v["features"]
            .as_array()
            .expect("features")
            .iter()
            .map(|f| f.as_str().expect("feature").to_string())
            .collect(),
        ziel: text("target"),
    }
}

/// Paketeintrag einer `Cargo.toml`, gelesen von Cargo selbst (`cargo metadata --no-deps`): kein
/// eigener TOML-Leser, also dieselbe Auslegung wie beim Bau.
fn paket(manifest: &Path) -> Value {
    let manifest = manifest
        .canonicalize()
        .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let aus = std::process::Command::new(cargo)
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
        ])
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .expect("cargo metadata startet nicht");
    assert!(
        aus.status.success(),
        "cargo metadata: {}",
        String::from_utf8_lossy(&aus.stderr)
    );
    let alles: Value = serde_json::from_slice(&aus.stdout).expect("cargo metadata liefert JSON");
    alles["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .find(|p| p["manifest_path"].as_str().map(Path::new) == Some(manifest.as_path()))
        .unwrap_or_else(|| panic!("{} fehlt in cargo metadata", manifest.display()))
        .clone()
}

fn abhaengigkeiten(paket: &Value) -> Vec<Abhaengigkeit> {
    paket["dependencies"]
        .as_array()
        .expect("dependencies")
        .iter()
        .map(abhaengigkeit)
        .collect()
}

/// **Jede** Abhängigkeit des Prüfstands steht in der App mit derselben Art (normal, `dev`,
/// `build`) genau einmal und stimmt in allem überein: Anforderung, Quelle (Register oder Pfad),
/// Features, Standard-Features, Umbenennung, `optional`, Zielbedingung. Die Liste kommt aus der
/// `Cargo.toml` des Prüfstands selbst; eine neue Zeile dort wird also mitgeprüft.
///
/// Beabsichtigter Unterschied, der hier nicht gemeldet wird: Der Prüfstand nennt nur einen Teil
/// der Abhängigkeiten der App. Weitere Unterschiede sind nicht vorgesehen.
fn abhaengigkeiten_abweichungen(meine: &[Abhaengigkeit], der_app: &[Abhaengigkeit]) -> Vec<String> {
    let mut ab = Vec::new();
    for (i, m) in meine.iter().enumerate() {
        let art = m.art.as_deref().unwrap_or("normal");
        if meine[..i]
            .iter()
            .any(|v| v.name == m.name && v.art == m.art)
        {
            ab.push(format!("{} ({art}): steht im Prüfstand mehrfach", m.name));
            continue;
        }
        let gleiche: Vec<_> = der_app
            .iter()
            .filter(|a| a.name == m.name && a.art == m.art)
            .collect();
        match gleiche.as_slice() {
            [] => ab.push(format!(
                "{} ({art}): die App hat keine Abhängigkeit dieser Art",
                m.name
            )),
            [a] if *a == m => {}
            [a] => ab.push(format!(
                "{} ({art}): Prüfstand {m:?}, App {a:?}",
                m.name
            )),
            _ => ab.push(format!(
                "{} ({art}): steht in der App mehrfach (Zielbedingungen); der Prüfstand bildet das nicht ab",
                m.name
            )),
        }
    }
    ab
}

#[test]
fn abhaengigkeiten_folgen_der_app() {
    let hier = paket(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"));
    let meine = abhaengigkeiten(&hier);
    assert!(
        meine.iter().any(|m| m.name == "wai-voice-dienst"
            && m.pfad
                == Some(
                    app()
                        .join("vendor/wai-voice-dienst")
                        .canonicalize()
                        .expect("vendorte Crate")
                )),
        "der Vertrag kommt aus der vendorten Crate der App: {meine:#?}"
    );
    assert_eq!(
        hier["features"],
        serde_json::json!({}),
        "der Prüfstand hat keine eigenen Features"
    );
    let ab =
        abhaengigkeiten_abweichungen(&meine, &abhaengigkeiten(&paket(&app().join("Cargo.toml"))));
    assert!(ab.is_empty(), "{ab:#?}");
}

#[test]
fn gegenprobe_abhaengigkeiten() {
    const CRATES: &str = "registry+https://github.com/rust-lang/crates.io-index";
    let neu = |name: &str, art: Option<&str>, features: &[&str]| Abhaengigkeit {
        name: name.to_string(),
        art: art.map(str::to_string),
        anforderung: "^1".to_string(),
        quelle: Some(CRATES.to_string()),
        pfad: None,
        umbenannt: None,
        register: None,
        optional: false,
        standard_features: true,
        features: features.iter().map(|f| f.to_string()).collect(),
        ziel: None,
    };
    let der_app = vec![
        neu("serde", None, &["derive"]),
        neu("serde", Some("build"), &[]),
        neu("tokio", None, &["rt", "net"]),
        neu("tokio", Some("dev"), &["rt-multi-thread"]),
        neu("sha2", None, &[]),
        Abhaengigkeit {
            ziel: Some("cfg(unix)".to_string()),
            ..neu("libc", None, &[])
        },
        Abhaengigkeit {
            ziel: Some("cfg(windows)".to_string()),
            ..neu("zweifach", None, &[])
        },
        neu("zweifach", None, &[]),
    ];
    let gut = vec![
        neu("serde", None, &["derive"]),
        neu("tokio", None, &["net", "rt"]),
        neu("tokio", Some("dev"), &["rt-multi-thread"]),
    ];
    assert_eq!(
        abhaengigkeiten_abweichungen(&gut, &der_app),
        Vec::<String>::new()
    );

    let meldet = |dazu: Abhaengigkeit, teil: &str| {
        let mut meine = gut.clone();
        meine.push(dazu);
        let ab = abhaengigkeiten_abweichungen(&meine, &der_app);
        assert!(
            ab.len() == 1 && ab[0].contains(teil),
            "erwartet „{teil}“: {ab:#?}"
        );
    };
    let sha2 = || neu("sha2", None, &[]);
    assert_eq!(
        abhaengigkeiten_abweichungen(&[sha2()], &der_app),
        Vec::<String>::new()
    );

    // Eine zusätzliche Abhängigkeit, die die App nicht hat.
    meldet(neu("regex", None, &[]), "regex (normal): die App hat keine");
    // Vorhanden, aber in anderer Art.
    meldet(
        neu("sha2", Some("dev"), &[]),
        "sha2 (dev): die App hat keine",
    );
    // Features, Standard-Features, Anforderung, Quelle, Umbenennung, optional, Zielbedingung.
    meldet(neu("sha2", None, &["asm"]), "sha2 (normal): Prüfstand");
    meldet(
        Abhaengigkeit {
            standard_features: false,
            ..sha2()
        },
        "sha2 (normal): Prüfstand",
    );
    meldet(
        Abhaengigkeit {
            anforderung: "^1.2".to_string(),
            ..sha2()
        },
        "sha2 (normal): Prüfstand",
    );
    meldet(
        Abhaengigkeit {
            quelle: Some("git+https://example.org/sha2".to_string()),
            ..sha2()
        },
        "sha2 (normal): Prüfstand",
    );
    meldet(
        Abhaengigkeit {
            quelle: None,
            pfad: Some(PathBuf::from("/ersatz/sha2")),
            ..sha2()
        },
        "sha2 (normal): Prüfstand",
    );
    meldet(
        Abhaengigkeit {
            umbenannt: Some("hash".to_string()),
            ..sha2()
        },
        "sha2 (normal): Prüfstand",
    );
    meldet(
        Abhaengigkeit {
            optional: true,
            ..sha2()
        },
        "sha2 (normal): Prüfstand",
    );
    meldet(
        Abhaengigkeit {
            ziel: Some("cfg(unix)".to_string()),
            ..sha2()
        },
        "sha2 (normal): Prüfstand",
    );
    // In der App nur unter einer Zielbedingung, hier ohne.
    meldet(neu("libc", None, &[]), "libc (normal): Prüfstand");
    // In der App mehrfach.
    meldet(neu("zweifach", None, &[]), "steht in der App mehrfach");
    // Im Prüfstand doppelt.
    meldet(
        neu("serde", None, &["derive"]),
        "steht im Prüfstand mehrfach",
    );
}
