//! Prüfstand für den Sprachdienst: führt dessen Tests aus, ohne die App zu bauen.
//!
//! Dieses Crate enthält **keinen** Dienstcode. Es bindet die Originalquellen aus
//! `src-tauri/src/sprachdienst` per `#[path]` ein, unter demselben Modulpfad wie in der App
//! (`crate::sprachdienst::…`, `crate::testlog`). Was hier grün ist, ist derselbe Quelltext.
//!
//! Abweichungen vom echten Crate, siehe README: ohne `app.rs` (Tauri-Anbindung) und ohne
//! `pipe.rs` (Windows), nur unter Unix, eigene Abhängigkeitsliste. Maßgeblich bleibt das Rust-Tor
//! im App-Crate.
#![cfg(unix)]

#[cfg(test)]
#[path = "../../../src-tauri/src/testlog.rs"]
mod testlog;

pub mod sprachdienst;

/// Wächter gegen Drift zwischen Prüfstand und App. Sie prüfen nicht den Dienst, sondern dass der
/// Prüfstand noch das abbildet, was die App baut.
#[cfg(test)]
mod waechter {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn app() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri")
    }

    fn lies(p: PathBuf) -> String {
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    /// Module aus `mod x;` / `pub mod x;` einer Datei, ohne Kommentarzeilen.
    fn module(text: &str) -> BTreeSet<String> {
        text.lines()
            .map(str::trim)
            .filter_map(|z| {
                z.strip_prefix("pub mod ")
                    .or_else(|| z.strip_prefix("mod "))
            })
            .filter_map(|z| z.strip_suffix(';'))
            .map(str::to_string)
            .collect()
    }

    /// Bekommt der Dienst ein neues Modul, muss hier entschieden werden, ob der Prüfstand es
    /// einbindet. Ohne diesen Test liefe er still an neuem Code vorbei.
    #[test]
    fn module_folgen_dem_dienst() {
        let dienst = module(&lies(app().join("src/sprachdienst/mod.rs")));
        // Bewusst nicht eingebunden: `app` (Tauri), `pipe` (Windows).
        let ausgelassen = BTreeSet::from(["app".to_string(), "pipe".to_string()]);
        let eingebunden = module(include_str!("sprachdienst/mod.rs"));
        let erwartet: BTreeSet<String> = dienst.difference(&ausgelassen).cloned().collect();
        assert_eq!(
            eingebunden, erwartet,
            "sprachdienst/mod.rs und der Prüfstand nennen nicht dieselben Module"
        );
        for m in &ausgelassen {
            assert!(
                dienst.contains(m),
                "ausgelassenes Modul gibt es nicht mehr: {m}"
            );
        }
    }

    /// Paare (Name, Version) aller Pakete einer `Cargo.lock`.
    fn pakete(sperre: &str) -> BTreeSet<(String, String)> {
        let feld = |block: &str, name: &str| {
            let anfang = format!("{name} = \"");
            block
                .lines()
                .find_map(|z| z.strip_prefix(anfang.as_str()))
                .and_then(|z| z.strip_suffix('"'))
                .map(str::to_string)
        };
        sperre
            .split("[[package]]")
            .skip(1)
            .filter_map(|b| Some((feld(b, "name")?, feld(b, "version")?)))
            .collect()
    }

    /// Der Prüfstand baut mit genau den Paketversionen, die auch die App sperrt. Nach einer
    /// Änderung an `src-tauri/Cargo.lock` neu ableiten, siehe README.
    #[test]
    fn sperrdatei_folgt_der_app() {
        let der_app = pakete(&lies(app().join("Cargo.lock")));
        let meine = pakete(include_str!("../Cargo.lock"));
        assert!(
            meine.len() > 20,
            "Cargo.lock des Prüfstands ist nicht lesbar"
        );
        let fremd: Vec<_> = meine
            .difference(&der_app)
            .filter(|(name, _)| name != env!("CARGO_PKG_NAME"))
            .collect();
        assert!(
            fremd.is_empty(),
            "nicht in der Sperrdatei der App: {fremd:?}"
        );
    }

    /// Versionsanforderungen eines Pakets in einer `Cargo.toml` (jede Zeile `name = …`).
    fn anforderungen(toml: &str, name: &str) -> BTreeSet<String> {
        let anfang = format!("{name} = ");
        toml.lines()
            .map(str::trim)
            .filter_map(|z| z.strip_prefix(anfang.as_str()))
            .filter_map(|wert| {
                let rest = match wert.strip_prefix('"') {
                    Some(r) => r,
                    None => wert.split("version = \"").nth(1)?,
                };
                Some(rest.split('"').next()?.to_string())
            })
            .collect()
    }

    /// Jede Abhängigkeit des Prüfstands fordert eine Version an, die auch die App anfordert.
    #[test]
    fn abhaengigkeiten_folgen_der_app() {
        let app = lies(app().join("Cargo.toml"));
        let hier = include_str!("../Cargo.toml");
        for name in [
            "serde",
            "serde_json",
            "log",
            "tokio",
            "rand",
            "hmac",
            "sha2",
            "tempfile",
        ] {
            let meine = anforderungen(hier, name);
            assert_eq!(
                meine.len(),
                1,
                "{name}: genau eine Anforderung im Prüfstand"
            );
            let der_app = anforderungen(&app, name);
            assert!(
                meine.is_subset(&der_app),
                "{name}: Prüfstand fordert {meine:?}, die App {der_app:?}"
            );
        }
        assert!(
            hier.contains(
                "wai-voice-dienst = { path = \"../../src-tauri/vendor/wai-voice-dienst\" }"
            ),
            "der Vertrag kommt aus der vendorten Crate der App"
        );
    }
}
