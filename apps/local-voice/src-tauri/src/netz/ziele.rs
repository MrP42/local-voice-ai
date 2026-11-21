//! Zieladressen der App fuer die IT-Freigabe (lva#739).
//! Quelle der Wahrheit fuer docs/FIRMENNETZ.md und den Befehl `netz_ziele`.
//! Tests binden die Dokumentation und die Anbieterliste
//! (`managers::compliance::facts::FACTS`) an diese Konstante.
//! Alle Ziele sind HTTPS auf Port 443; die App oeffnet keine anderen Ports nach aussen.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct NetzZiel {
    pub funktion: &'static str,
    pub host: &'static str,
    pub port: u16,
    pub zweck: &'static str,
}

const fn z(funktion: &'static str, host: &'static str, zweck: &'static str) -> NetzZiel {
    NetzZiel {
        funktion,
        host,
        port: 443,
        zweck,
    }
}

pub const ZIELE: &[NetzZiel] = &[
    z(
        "Programm-Updates",
        "github.com",
        "Update-Manifest und Installer (Release-Dateien von MrP42/local-voice-ai)",
    ),
    z(
        "Programm-Updates",
        "objects.githubusercontent.com",
        "Auslieferung der GitHub-Release-Dateien (Weiterleitung)",
    ),
    z(
        "Programm-Updates",
        "release-assets.githubusercontent.com",
        "Auslieferung der GitHub-Release-Dateien (Weiterleitung)",
    ),
    z(
        "Sprach- und Sprachmodelle",
        "huggingface.co",
        "Modell-Downloads (Hugging Face)",
    ),
    z(
        "Sprach- und Sprachmodelle",
        "cas-bridge.xethub.hf.co",
        "Weiterleitung der Hugging-Face-Dateien",
    ),
    z(
        "Sprach- und Sprachmodelle",
        "cdn-lfs.hf.co",
        "Weiterleitung der Hugging-Face-Dateien",
    ),
    z(
        "Sprach- und Sprachmodelle",
        "blob.handy.computer",
        "Spracherkennungsmodelle (Direktdownload)",
    ),
    z(
        "KI-Anbieter (API, Schluessel des Nutzers)",
        "api.anthropic.com",
        "Claude API",
    ),
    z(
        "KI-Anbieter (API, Schluessel des Nutzers)",
        "api.openai.com",
        "OpenAI API",
    ),
    z(
        "KI-Anbieter (API, Schluessel des Nutzers)",
        "eu.api.openai.com",
        "OpenAI API mit EU-Datenresidenz",
    ),
    z(
        "KI-Anbieter (API, Schluessel des Nutzers)",
        "api.mistral.ai",
        "Mistral API",
    ),
    z(
        "KI-Anbieter (API, Schluessel des Nutzers)",
        "generativelanguage.googleapis.com",
        "Google Gemini API",
    ),
    z(
        "KI-Anbieter (API, Schluessel des Nutzers)",
        "bedrock-mantle.eu-central-1.api.aws",
        "Amazon Bedrock, Region Frankfurt",
    ),
    z(
        "KI-Anbieter (API, Schluessel des Nutzers)",
        "bedrock-mantle.eu-west-1.api.aws",
        "Amazon Bedrock, Region Irland",
    ),
    z(
        "KI-Anbieter (API, Schluessel des Nutzers)",
        "bedrock-mantle.eu-south-1.api.aws",
        "Amazon Bedrock, Region Mailand",
    ),
    z(
        "KI-Anbieter (API, Schluessel des Nutzers)",
        "bedrock-mantle.eu-north-1.api.aws",
        "Amazon Bedrock, Region Stockholm",
    ),
    z(
        "KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert)",
        "api.anthropic.com",
        "Claude Code (Programm von Anthropic, nicht von der App installiert)",
    ),
    z(
        "KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert)",
        "claude.ai",
        "Anmeldung von Claude Code",
    ),
    z(
        "KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert)",
        "chatgpt.com",
        "Codex mit ChatGPT-Konto (Programm von OpenAI, nicht von der App installiert)",
    ),
    z(
        "KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert)",
        "api.openai.com",
        "Codex (Programm von OpenAI)",
    ),
    z(
        "KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert)",
        "registry.npmjs.org",
        "Codex-Update per npm (nur bei eingeschalteter automatischer Aktualisierung)",
    ),
    z(
        "Optionale Dienste (nur wenn der Nutzer sie einrichtet)",
        "graph.microsoft.com",
        "Microsoft 365: Kalender und Dateien",
    ),
    z(
        "Optionale Dienste (nur wenn der Nutzer sie einrichtet)",
        "login.microsoftonline.com",
        "Microsoft 365: Anmeldung",
    ),
    z(
        "Optionale Dienste (nur wenn der Nutzer sie einrichtet)",
        "api.gradium.ai",
        "Stimmerzeugung (nur mit Einwilligung)",
    ),
    z(
        "Optionale Dienste (nur wenn der Nutzer sie einrichtet)",
        "openlibrary.org",
        "Buchdaten (Metadaten)",
    ),
];

pub fn markdown_tabelle() -> String {
    let mut zeilen = vec![
        "| Funktion | Adresse | Port | Zweck |".to_string(),
        "|---|---|---|---|".to_string(),
    ];
    zeilen.extend(ZIELE.iter().map(|ziel| {
        format!(
            "| {} | {} | {} | {} |",
            ziel.funktion, ziel.host, ziel.port, ziel.zweck
        )
    }));
    zeilen.join("\n")
}

pub fn klartext() -> String {
    ZIELE
        .iter()
        .map(|ziel| {
            format!(
                "{}:{}  ({}: {})",
                ziel.host, ziel.port, ziel.funktion, ziel.zweck
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alle_ziele_sind_443_und_ohne_dopplung_je_funktion() {
        let mut gesehen = std::collections::HashSet::new();
        for ziel in ZIELE {
            assert_eq!(ziel.port, 443, "Falscher Port: {}", ziel.host);
            assert!(
                gesehen.insert((ziel.funktion, ziel.host)),
                "Doppeltes Ziel: {} / {}",
                ziel.funktion,
                ziel.host
            );
            assert!(
                !ziel.host.contains(['/', ':', ' ']),
                "Ungueltiger Host: {}",
                ziel.host
            );
            assert_eq!(ziel.host, ziel.host.to_lowercase());
        }
    }

    #[test]
    fn jeder_host_der_anbieterliste_steht_in_der_zielliste() {
        for p in crate::managers::compliance::facts::FACTS {
            if let Some(host) = p.host {
                if host != "claude" && host != "codex" {
                    assert!(
                        ZIELE.iter().any(|ziel| ziel.host == host),
                        "Anbieter-Host fehlt: {host}"
                    );
                }
            }
        }
    }

    #[test]
    fn der_updater_host_steht_in_der_zielliste() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        let endpoint = config["plugins"]["updater"]["endpoints"][0]
            .as_str()
            .expect("Erster Updater-Endpunkt fehlt");
        let url = url::Url::parse(endpoint).unwrap();
        let host = url.host_str().expect("Updater-Endpunkt hat keinen Host");
        assert!(
            ZIELE
                .iter()
                .any(|ziel| ziel.funktion == "Programm-Updates" && ziel.host == host),
            "Updater-Host fehlt: {host}"
        );
    }

    #[test]
    fn die_dokumentation_enthaelt_die_erzeugte_tabelle() {
        let pfad =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/FIRMENNETZ.md");
        let dokument = std::fs::read_to_string(pfad).unwrap().replace("\r\n", "\n");
        let zeilen: Vec<_> = dokument.lines().collect();
        let anfang = zeilen
            .iter()
            .position(|zeile| *zeile == "<!-- ZIELE:BEGIN -->")
            .expect("Beginn-Marker fehlt");
        let ende = zeilen
            .iter()
            .position(|zeile| *zeile == "<!-- ZIELE:END -->")
            .expect("Ende-Marker fehlt");
        assert!(ende > anfang, "Ziel-Marker stehen in falscher Reihenfolge");
        let tabelle = markdown_tabelle();
        assert_eq!(
            zeilen[anfang + 1..ende].join("\n").trim(),
            tabelle,
            "Erwartete Tabelle zum Kopieren:\n{tabelle}"
        );
    }

    #[test]
    fn klartext_hat_eine_zeile_je_ziel() {
        let text = klartext();
        assert_eq!(text.lines().count(), ZIELE.len());
        assert!(!text.ends_with('\n'));
        for (zeile, ziel) in text.lines().zip(ZIELE) {
            assert_eq!(
                zeile,
                format!(
                    "{}:{}  ({}: {})",
                    ziel.host, ziel.port, ziel.funktion, ziel.zweck
                )
            );
        }
    }
}
