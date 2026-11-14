//! Compile-Zeit-Zusicherungen des Versandvertrags (lva#604, Teilpaket 4). Nur unter `cfg(test)`.
//!
//! Der Versandvertrag lebt von Dingen, die **nicht übersetzbar** sein müssen: der Rohtransport
//! ist von außen unerreichbar, eine `Versandfreigabe` lässt sich weder bauen noch vervielfältigen
//! noch zweimal einlösen, `Quellen` hat kein stilles „leer“. Dieses Modul beweist das mit dem
//! **tatsächlichen Fehlercode des Compilers**, nicht nur damit, dass etwas nicht kompiliert.
//!
//! # Warum keine `compile_fail`-Doctests
//!
//! - `mod llm_versand;` ist in `lib.rs` privat, Doctests sind aber **externe** Crates: sie sehen
//!   `llm_versand` (und damit alles, was geprüft werden soll) schon deshalb nicht. Der Fehler
//!   wäre immer derselbe („Modul privat“), gleich ob der Vertrag hält oder nicht, und eine
//!   positive Kontrolle wäre unmöglich.
//! - Auf stabilem Rust prüft `rustdoc` den Fehlercode hinter `compile_fail,E0603` nicht; ohne
//!   Code ist „kompiliert nicht“ auch bei einem Tippfehler im Snippet wahr.
//! - Gewünscht ist außerdem der Blick **aus dem Crate**: „auch nicht aus `crate::managers::llm`“.
//!   Das kann nur ein Modul im Crate zeigen. `trybuild` scheidet aus denselben Gründen aus (externes
//!   Crate) und wäre eine neue Abhängigkeit.
//!
//! # Mechanismus: Proben im Crate, übersetzt mit `--cfg lva_sonde`
//!
//! Die Probendateien liegen unter `llm_versand/sonden/` (innerhalb von `llm_versand`) und
//! `llm_versand/sonden/aussen/` (eingehängt als `crate::managers::llm::versand_sonden`, also von
//! außerhalb). Ohne `--cfg lva_sonde` kennt kein Build sie. Der Test [`compile_zeit_zusicherungen`]
//! startet `cargo rustc --lib --profile check -- --cfg lva_sonde --cfg lva_sonde="<lauf>"` (nur
//! Prüfung, keine Codeerzeugung), liest die JSON-Diagnosen des Compilers und vergleicht je Probe
//! den Fehlercode mit der Tabelle [`SONDEN`].
//!
//! Jede Probendatei hat **einen** Verstoß und daneben denselben Code ohne ihn
//! (`#[cfg(lva_sonde = "kontrolle")]`). Läufe:
//!
//! | Lauf | Inhalt | Prüfung |
//! |---|---|---|
//! | `kontrolle` | alle Proben ohne Verstoß | übersetzt ohne Fehler; **jede** Probe meldet ihre Kontrollmarke (belegt, dass sie übersetzt wurde) |
//! | `aufloesung` | Verstöße gegen die Namensauflösung | je Probe genau Fehler `E0603` in der eigenen Datei |
//! | `typ` | Verstöße gegen Methoden und Funktionen | `E0599`, `E0624` |
//! | `feld` | Verstoß gegen private Felder | `E0451` |
//! | `ausleihe` | Verstöße gegen Besitz | `E0382` |
//!
//! Die Läufe sind getrennt, weil `rustc` je Phase abbricht: Besitzfehler (`E0382`) meldet der
//! Borrowchecker erst, wenn die Typprüfung fehlerfrei war; Feldsichtbarkeit (`E0451`) prüft die
//! Datenschutzphase erst danach. Ein Fehler in einer fremden Datei oder mit anderem Code lässt
//! den Lauf scheitern: eine Probe, die aus einem anderen Grund fällt, besteht nicht. Die
//! Auswertung selbst ist mit künstlichen Diagnosen getestet (`auswertung_*`).
//!
//! Dauer: etwa zwei Minuten beim ersten Lauf (die Prüfeinheit des Crates wird mit anderen cfg
//! neu übersetzt), danach je Lauf rund 15 bis 20 Sekunden. Der Lauf überschreibt die
//! **Prüfartefakte** (`cargo check`) des Crates in `target`; der nächste `cargo check` oder
//! `clippy` übersetzt das Crate deshalb einmal neu. Testartefakte bleiben unberührt. Wer den
//! Lauf nicht braucht (etwa bei knappem Platz), setzt `LVA_SONDEN_UEBERSPRINGEN=1`; das Überspringen
//! wird auf `stderr` gemeldet.
//!
//! # Zusätzlich ohne Prozess
//!
//! `quellen_hat_kein_default` ergänzt die Trait-Beweise des Prüfers (`Versandfreigabe` ohne
//! `Clone`, `Copy`, `Default`, `Serialize`, `Deserialize`, im Modul `pruefer`) um `Quellen` ohne
//! `Default`. Sie laufen bei jedem Testbuild und scheitern beim Übersetzen.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Eine Probe: Datei (ohne Verzeichnis), erwarteter Fehlercode, Lauf.
struct Sonde {
    datei: &'static str,
    code: &'static str,
    lauf: &'static str,
}

const fn sonde(datei: &'static str, code: &'static str, lauf: &'static str) -> Sonde {
    Sonde { datei, code, lauf }
}

/// Alle Proben mit dem Fehlercode, den der Compiler **genau** melden muss.
const SONDEN: &[Sonde] = &[
    // Rohtransport und Interna sind nicht erreichbar (privat).
    sonde("aussen_e0603_transport.rs", "E0603", "aufloesung"),
    sonde("aussen_e0603_versand.rs", "E0603", "aufloesung"),
    sonde("aussen_e0603_freigabe.rs", "E0603", "aufloesung"),
    sonde("aussen_e0603_ausstellen.rs", "E0603", "aufloesung"),
    sonde("innen_e0603_cli.rs", "E0603", "aufloesung"),
    sonde("innen_e0603_pruefer_app.rs", "E0603", "aufloesung"),
    // Ausstellen ist privat, Quellen und Freigabe haben keine Standardwerte und keinen Klon.
    sonde("innen_e0624_ausstellen.rs", "E0624", "typ"),
    sonde("innen_e0599_klon.rs", "E0599", "typ"),
    sonde("innen_e0599_default.rs", "E0599", "typ"),
    sonde("aussen_e0599_quellen_default.rs", "E0599", "typ"),
    // Felder privat.
    sonde("innen_e0451_literal.rs", "E0451", "feld"),
    // Eine eingelöste Freigabe ist verbraucht.
    sonde("innen_e0382_zweimal.rs", "E0382", "ausleihe"),
    sonde("innen_e0382_kopie.rs", "E0382", "ausleihe"),
];

/// Läufe mit Fehlern, in der Reihenfolge, in der sie gestartet werden.
const FEHLERLAEUFE: &[&str] = &["aufloesung", "typ", "feld", "ausleihe"];

/// Meldung des Compilers über eine Fehlerstelle.
#[derive(Debug, PartialEq, Eq)]
struct Meldung {
    /// Dateiname ohne Verzeichnis; leer ohne Quellstelle.
    datei: String,
    code: String,
    text: String,
}

/// Ausgewertete Diagnosen eines Compilerlaufs.
#[derive(Debug, Default)]
struct Ergebnis {
    erfolg: bool,
    fehler: Vec<Meldung>,
    /// Dateien, die die Kontrollmarke gemeldet haben.
    kontrollmarken: BTreeSet<String>,
}

fn dateiname(pfad: &str) -> String {
    pfad.rsplit(['/', '\\']).next().unwrap_or(pfad).to_string()
}

/// Liest die JSON-Zeilen von `cargo --message-format=json`.
fn auswerten(erfolg: bool, ausgabe: &str) -> Ergebnis {
    let mut ergebnis = Ergebnis {
        erfolg,
        ..Ergebnis::default()
    };
    for zeile in ausgabe.lines() {
        let Ok(wert) = serde_json::from_str::<Value>(zeile) else {
            continue;
        };
        if wert["reason"] != "compiler-message" {
            continue;
        }
        let meldung = &wert["message"];
        let text = meldung["message"].as_str().unwrap_or_default();
        let datei = meldung["spans"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|stelle| stelle["is_primary"] == true)
            .and_then(|stelle| stelle["file_name"].as_str())
            .map(dateiname)
            .unwrap_or_default();
        match meldung["level"].as_str() {
            Some("error") if text.starts_with("aborting due to") => {}
            Some("error") => ergebnis.fehler.push(Meldung {
                datei,
                code: meldung["code"]["code"].as_str().unwrap_or_default().into(),
                text: text.into(),
            }),
            Some("warning") if text.contains("deprecated") && text.contains("Kontrollmarke") => {
                ergebnis.kontrollmarken.insert(datei);
            }
            _ => {}
        }
    }
    ergebnis
}

fn proben_des_laufs(lauf: &str) -> Vec<&'static Sonde> {
    SONDEN.iter().filter(|sonde| sonde.lauf == lauf).collect()
}

fn beschreiben(fehler: &[Meldung]) -> String {
    fehler
        .iter()
        .map(|f| format!("  {} [{}] {}", f.datei, f.code, f.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Der Kontrolllauf: übersetzt ohne Fehler, und jede Probe wurde tatsächlich übersetzt.
fn kontrolle_pruefen(ergebnis: &Ergebnis) -> Result<(), String> {
    if !ergebnis.erfolg || !ergebnis.fehler.is_empty() {
        return Err(format!(
            "Kontrolle (Code ohne Verstoß) muss übersetzen:\n{}",
            beschreiben(&ergebnis.fehler)
        ));
    }
    let fehlend: Vec<&str> = SONDEN
        .iter()
        .map(|sonde| sonde.datei)
        .filter(|datei| !ergebnis.kontrollmarken.contains(*datei))
        .collect();
    if fehlend.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Proben ohne Kontrollmarke (nicht übersetzt?): {fehlend:?}"
    ))
}

/// Ein Fehlerlauf: scheitert, jede Probe des Laufs meldet ihren Code, und es gibt **nichts**
/// anderes (keine fremde Datei, keinen anderen Code).
fn fehlerlauf_pruefen(lauf: &str, ergebnis: &Ergebnis) -> Result<(), String> {
    let proben = proben_des_laufs(lauf);
    if ergebnis.erfolg {
        return Err(format!("Lauf {lauf}: der Verstoß hat übersetzt"));
    }
    for fehler in &ergebnis.fehler {
        let Some(sonde) = proben.iter().find(|sonde| sonde.datei == fehler.datei) else {
            return Err(format!(
                "Lauf {lauf}: Fehler außerhalb der Proben:\n{}",
                beschreiben(std::slice::from_ref(fehler))
            ));
        };
        if fehler.code != sonde.code {
            return Err(format!(
                "Lauf {lauf}: {} soll {} melden, meldet aber:\n{}",
                sonde.datei,
                sonde.code,
                beschreiben(std::slice::from_ref(fehler))
            ));
        }
    }
    for sonde in &proben {
        if !ergebnis.fehler.iter().any(|f| f.datei == sonde.datei) {
            return Err(format!(
                "Lauf {lauf}: {} meldet gar keinen Fehler (soll {})",
                sonde.datei, sonde.code
            ));
        }
    }
    Ok(())
}

fn manifest_verzeichnis() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Startet `cargo rustc` für die Bibliothek im Prüfmodus mit den Probenschaltern.
fn lauf_starten(lauf: &str) -> Ergebnis {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let ausgabe = Command::new(cargo)
        .arg("rustc")
        .arg("--manifest-path")
        .arg(manifest_verzeichnis().join("Cargo.toml"))
        .args(["--lib", "--profile", "check", "--message-format=json", "--"])
        .args(["--cfg", "lva_sonde", "--cfg"])
        .arg(format!("lva_sonde=\"{lauf}\""))
        .output()
        .expect("cargo rustc ließ sich nicht starten");
    auswerten(
        ausgabe.status.success(),
        &String::from_utf8_lossy(&ausgabe.stdout),
    )
}

fn dateien_in(verzeichnis: &Path) -> BTreeSet<String> {
    std::fs::read_dir(verzeichnis)
        .unwrap_or_else(|e| panic!("{}: {e}", verzeichnis.display()))
        .map(|eintrag| eintrag.expect("Verzeichniseintrag").path())
        .filter(|pfad| pfad.extension().is_some_and(|endung| endung == "rs"))
        .map(|pfad| dateiname(&pfad.to_string_lossy()))
        .filter(|name| name != "mod.rs")
        .collect()
}

fn sondenverzeichnis() -> PathBuf {
    manifest_verzeichnis().join("src/llm_versand/sonden")
}

#[test]
fn proben_tabelle_ist_vollstaendig() {
    let auf_platte: BTreeSet<String> = dateien_in(&sondenverzeichnis())
        .union(&dateien_in(&sondenverzeichnis().join("aussen")))
        .cloned()
        .collect();
    let in_tabelle: BTreeSet<String> = SONDEN.iter().map(|s| s.datei.to_string()).collect();
    assert_eq!(auf_platte, in_tabelle, "Probendateien und Tabelle SONDEN");

    for sonde in SONDEN {
        assert!(
            sonde.datei.contains(&sonde.code.to_lowercase()),
            "{}: Dateiname nennt den Code {} nicht",
            sonde.datei,
            sonde.code
        );
        assert!(
            FEHLERLAEUFE.contains(&sonde.lauf),
            "{}: unbekannter Lauf {}",
            sonde.datei,
            sonde.lauf
        );
        let verzeichnis = if sonde.datei.starts_with("aussen_") {
            sondenverzeichnis().join("aussen")
        } else {
            sondenverzeichnis()
        };
        let quelltext = std::fs::read_to_string(verzeichnis.join(sonde.datei)).expect("Probe");
        assert!(
            quelltext.contains("#[cfg(not(lva_sonde = \"kontrolle\"))]"),
            "{}: der Verstoß fehlt",
            sonde.datei
        );
        assert!(
            quelltext.contains("#[cfg(lva_sonde = \"kontrolle\")]")
                && quelltext.contains("super::kontrollmarke()"),
            "{}: Kontrolle oder Kontrollmarke fehlt",
            sonde.datei
        );
    }
}

#[test]
fn proben_mods_nennen_jede_probe() {
    for (verzeichnis, vorsatz) in [
        (sondenverzeichnis(), "innen_"),
        (sondenverzeichnis().join("aussen"), "aussen_"),
    ] {
        let mod_rs = std::fs::read_to_string(verzeichnis.join("mod.rs")).expect("mod.rs");
        for sonde in SONDEN.iter().filter(|s| s.datei.starts_with(vorsatz)) {
            let modul = sonde.datei.trim_end_matches(".rs");
            assert!(
                mod_rs.contains(&format!("mod {modul};")),
                "{modul} fehlt in mod.rs"
            );
            let schalter = format!("lva_sonde = \"{}\"", sonde.lauf);
            let zeilen: Vec<&str> = mod_rs.lines().collect();
            let stelle = zeilen
                .iter()
                .position(|z| *z == format!("mod {modul};"))
                .expect("Modulzeile");
            assert!(
                zeilen[stelle - 1].contains(&schalter)
                    && zeilen[stelle - 1].contains("lva_sonde = \"kontrolle\""),
                "{modul}: cfg-Zeile muss `kontrolle` und `{}` nennen",
                sonde.lauf
            );
        }
    }
}

fn diagnose(ebene: &str, code: Option<&str>, text: &str, datei: Option<&str>) -> String {
    serde_json::json!({
        "reason": "compiler-message",
        "message": {
            "level": ebene,
            "message": text,
            "code": code.map(|c| serde_json::json!({ "code": c })),
            "spans": datei.map(|d| vec![serde_json::json!({
                "is_primary": true,
                "file_name": format!("src/managers/llm/../../llm_versand/sonden/{d}"),
            })]).unwrap_or_default(),
        }
    })
    .to_string()
}

fn ausgabe(zeilen: &[String]) -> String {
    zeilen.join("\n")
}

#[test]
fn auswertung_nimmt_den_richtigen_code_an() {
    let zeilen = [
        diagnose(
            "error",
            Some("E0603"),
            "module `transport` is private",
            Some("aussen_e0603_transport.rs"),
        ),
        diagnose(
            "error",
            Some("E0603"),
            "module `versand` is private",
            Some("aussen_e0603_versand.rs"),
        ),
        diagnose(
            "error",
            Some("E0603"),
            "struct `Versandfreigabe` is private",
            Some("aussen_e0603_freigabe.rs"),
        ),
        diagnose(
            "error",
            Some("E0603"),
            "function `freigabe_ausstellen` is private",
            Some("aussen_e0603_ausstellen.rs"),
        ),
        diagnose(
            "error",
            Some("E0603"),
            "module `cli` is private",
            Some("innen_e0603_cli.rs"),
        ),
        diagnose(
            "error",
            Some("E0603"),
            "module `app` is private",
            Some("innen_e0603_pruefer_app.rs"),
        ),
        diagnose("error", None, "aborting due to 6 previous errors", None),
    ];
    let ergebnis = auswerten(false, &ausgabe(&zeilen));
    assert_eq!(ergebnis.fehler.len(), 6, "{:?}", ergebnis.fehler);
    assert_eq!(fehlerlauf_pruefen("aufloesung", &ergebnis), Ok(()));
}

#[test]
fn auswertung_weist_falschen_code_ab() {
    // Die Probe scheitert, aber aus einem anderen Grund (etwa Tippfehler): E0432 statt E0603.
    let mut zeilen: Vec<String> = proben_des_laufs("aufloesung")
        .iter()
        .map(|s| diagnose("error", Some(s.code), "x", Some(s.datei)))
        .collect();
    zeilen[0] = diagnose(
        "error",
        Some("E0432"),
        "unresolved import",
        Some(proben_des_laufs("aufloesung")[0].datei),
    );
    let ergebnis = auswerten(false, &ausgabe(&zeilen));
    let fehler = fehlerlauf_pruefen("aufloesung", &ergebnis).unwrap_err();
    assert!(
        fehler.contains("E0603") && fehler.contains("E0432"),
        "{fehler}"
    );
}

#[test]
fn auswertung_weist_fehler_ausserhalb_der_proben_ab() {
    let mut zeilen: Vec<String> = proben_des_laufs("feld")
        .iter()
        .map(|s| diagnose("error", Some(s.code), "x", Some(s.datei)))
        .collect();
    zeilen.push(diagnose(
        "error",
        Some("E0425"),
        "cannot find value",
        Some("fremd.rs"),
    ));
    let ergebnis = auswerten(false, &ausgabe(&zeilen));
    assert!(fehlerlauf_pruefen("feld", &ergebnis)
        .unwrap_err()
        .contains("außerhalb"));

    let ohne_stelle = [diagnose("error", None, "linking failed", None)];
    let ergebnis = auswerten(false, &ausgabe(&ohne_stelle));
    assert!(fehlerlauf_pruefen("feld", &ergebnis).is_err());
}

#[test]
fn auswertung_weist_fehlende_oder_uebersetzte_proben_ab() {
    // Probe meldet gar keinen Fehler.
    let leer = auswerten(
        false,
        &diagnose("error", None, "aborting due to 1 previous error", None),
    );
    assert!(fehlerlauf_pruefen("ausleihe", &leer)
        .unwrap_err()
        .contains("gar keinen Fehler"));
    // Der Verstoß übersetzt: Erfolg ist ein Fehlschlag der Probe.
    let erfolg = auswerten(true, "");
    assert!(fehlerlauf_pruefen("ausleihe", &erfolg)
        .unwrap_err()
        .contains("übersetzt"));
    // Eine zweite Datei der Probe darf nur den eigenen Code melden.
    let sonden = proben_des_laufs("typ");
    let mut zeilen: Vec<String> = sonden
        .iter()
        .map(|s| diagnose("error", Some(s.code), "x", Some(s.datei)))
        .collect();
    zeilen.push(diagnose(
        "error",
        Some("E0308"),
        "mismatched types",
        Some(sonden[0].datei),
    ));
    let gemischt = auswerten(false, &ausgabe(&zeilen));
    assert!(fehlerlauf_pruefen("typ", &gemischt).is_err());
}

#[test]
fn auswertung_der_kontrolle_verlangt_jede_marke() {
    let marken: Vec<String> = SONDEN
        .iter()
        .map(|s| {
            diagnose(
                "warning",
                None,
                "use of deprecated function `x`: Kontrollmarke",
                Some(s.datei),
            )
        })
        .collect();
    let vollstaendig = auswerten(true, &ausgabe(&marken));
    assert_eq!(kontrolle_pruefen(&vollstaendig), Ok(()));

    let unvollstaendig = auswerten(true, &ausgabe(&marken[1..]));
    let fehler = kontrolle_pruefen(&unvollstaendig).unwrap_err();
    assert!(fehler.contains(SONDEN[0].datei), "{fehler}");

    let mut mit_fehler = marken;
    mit_fehler.push(diagnose("error", Some("E0599"), "x", Some(SONDEN[0].datei)));
    assert!(kontrolle_pruefen(&auswerten(false, &ausgabe(&mit_fehler))).is_err());
}

/// Der echte Lauf: Kontrolle, dann jede Fehlergruppe mit dem Compiler dieses Crates.
#[test]
fn compile_zeit_zusicherungen() {
    if std::env::var_os("LVA_SONDEN_UEBERSPRINGEN").is_some() {
        eprintln!("compile_zeit_zusicherungen übersprungen (LVA_SONDEN_UEBERSPRINGEN)");
        return;
    }
    let kontrolle = lauf_starten("kontrolle");
    kontrolle_pruefen(&kontrolle).unwrap_or_else(|fehler| panic!("{fehler}"));
    for lauf in FEHLERLAEUFE {
        let ergebnis = lauf_starten(lauf);
        fehlerlauf_pruefen(lauf, &ergebnis).unwrap_or_else(|fehler| panic!("{fehler}"));
    }
}

/// Compile-Zeit-Beweis wie im Prüfer: `Quellen` hat kein `Default`. Implementierte es das
/// Merkmal, wäre der Aufruf von `pruefen` mehrdeutig und der Test würde nicht übersetzen.
#[test]
fn quellen_hat_kein_default() {
    trait Mehrdeutig<A> {
        fn pruefen() {}
    }
    impl<T> Mehrdeutig<()> for T {}
    impl<T: Default> Mehrdeutig<u8> for T {}
    <super::quellen::Quellen as Mehrdeutig<_>>::pruefen();
}
