//! Fester Testkorpus der Sprachstufe (lva#712): 18 synthetische Texte mit erlaubten
//! Korrekturen, verbotenen (erfundenen) Korrekturen und Stellen, die unveraendert bleiben
//! muessen. Der Mock korrigiert deterministisch nach einer Ersetzungstabelle je Fall; der Job
//! muss die erlaubten Korrekturen als einzeln pruefbare Gruppen liefern und die verbotenen
//! verwerfen (mit dem erwarteten Grund-Code, nur als Zaehler).

use std::sync::Arc;

use super::cache::SprachCache;
use super::fehler::GrundCode;
use super::job::{sprache_job, Abbruch, JobBericht, JobEingabe};
use super::modell::mock::{test_versand, MockVersender};
use super::modell::{Auftrag, ModellAntwort};
use super::vertrag::{Grenzen, Textstand};
use crate::aufbereitung::{
    ergebnis_text, pruefe_aenderungen, Aenderung, AenderungArt, AenderungStatus,
};
use crate::settings::{AppSettings, LlmConnection, LlmModelConfig};

struct Fall {
    name: &'static str,
    text: &'static str,
    /// Ersetzungen, die der Job uebernehmen muss.
    erlaubt: &'static [(&'static str, &'static str)],
    /// Ersetzungen, die das Modell "erfindet" und die verworfen werden muessen.
    verboten: &'static [(&'static str, &'static str, GrundCode)],
    /// Teilstrings, die im Ergebnis unveraendert vorkommen muessen.
    unveraendert: &'static [&'static str],
}

const FAELLE: &[Fall] = &[
    Fall {
        name: "ocr_verwechslung",
        text: "Das Buch ist sehr gut gesclvrieben.",
        erlaubt: &[("gesclvrieben", "geschrieben")],
        verboten: &[],
        unveraendert: &["Das Buch ist sehr gut"],
    },
    Fall {
        name: "silbentrennung",
        text: "Die Haus-aufgabe ist fertig.",
        erlaubt: &[("Haus-aufgabe", "Hausaufgabe")],
        verboten: &[],
        unveraendert: &["ist fertig."],
    },
    Fall {
        name: "tippfehler",
        text: "Wir haben das Progrmam gestartet.",
        erlaubt: &[("Progrmam", "Programm")],
        verboten: &[],
        unveraendert: &["Wir haben das"],
    },
    Fall {
        name: "gross_klein",
        text: "Ich traf einen Freund. der war nett.",
        erlaubt: &[("der war", "Der war")],
        verboten: &[],
        unveraendert: &["Ich traf einen Freund."],
    },
    Fall {
        name: "grammatik",
        text: "Das Kind spielen im Garten.",
        erlaubt: &[("spielen", "spielt")],
        verboten: &[],
        unveraendert: &["im Garten."],
    },
    Fall {
        name: "zeichensetzung",
        text: "Ich glaube dass er kommt.",
        erlaubt: &[("glaube dass", "glaube, dass")],
        verboten: &[],
        unveraendert: &["er kommt."],
    },
    Fall {
        name: "dialekt_woertliche_rede",
        text: "Er sagte: „I mog des net.“\nDanach gieng er heim.",
        erlaubt: &[("gieng", "ging")],
        verboten: &[(
            "I mog des net.",
            "Ich mag das nicht.",
            GrundCode::AnfuehrungGeaendert,
        )],
        unveraendert: &["„I mog des net.“"],
    },
    Fall {
        name: "fachbegriffe_kontrolle",
        text: "Die Kubernetes-Cluster nutzen Containerd und Prometheus.",
        erlaubt: &[],
        verboten: &[],
        unveraendert: &["Kubernetes-Cluster nutzen Containerd und Prometheus."],
    },
    Fall {
        name: "marken_geschuetzt",
        text: "Die Marke {{S.12}} und <Anna> sagten [pause] wirklich Feler.",
        erlaubt: &[("Feler", "Fehler")],
        verboten: &[],
        unveraendert: &["{{S.12}}", "<Anna>", "[pause]"],
    },
    Fall {
        name: "liste",
        text: "- Aepfel kaufen\n- Mich bestelen\n- Brot",
        erlaubt: &[("bestelen", "bestellen")],
        verboten: &[],
        unveraendert: &["- Aepfel kaufen", "- Brot"],
    },
    Fall {
        name: "zahlen_datum_einheiten",
        text: "Der Preis beträgt 1.234,56 Euro am 12.03.2024.\nWir sprachen ueber 5 km.",
        erlaubt: &[("ueber", "über")],
        verboten: &[("1.234,56", "1.234,65", GrundCode::ZahlGeaendert)],
        unveraendert: &["1.234,56 Euro am 12.03.2024.", "5 km"],
    },
    Fall {
        name: "url_und_email",
        text: "Schreib an anna@beispiel.de oder besuche https://beispiel.de/seite. Danke fuer alles.",
        erlaubt: &[("fuer", "für")],
        verboten: &[],
        unveraendert: &["anna@beispiel.de", "https://beispiel.de/seite"],
    },
    Fall {
        name: "code_und_sprecher",
        text: "<Berta> Nutze `pip install foo` fuer die Installation.",
        erlaubt: &[("fuer", "für")],
        verboten: &[],
        unveraendert: &["<Berta>", "`pip install foo`"],
    },
    Fall {
        name: "unicode_emoji",
        text: "Das ist ein Tset 🙂 mit Ümlauten ß und e\u{301}.",
        erlaubt: &[("Tset", "Test")],
        verboten: &[],
        unveraendert: &["🙂 mit Ümlauten ß und e\u{301}."],
    },
    Fall {
        name: "negation_bleibt",
        text: "Er ist nicht gekommen, weil er krnak war.\nSie ist nicht gekommen.",
        erlaubt: &[("krnak", "krank")],
        verboten: &[("nicht gekommen.", "gekommen.", GrundCode::NegationGeaendert)],
        unveraendert: &["Sie ist nicht gekommen."],
    },
    Fall {
        name: "sinnaendernde_minimalaenderung",
        text: "Wir haben zwei Angebote erhalten.\nUnd noch ein Feler.",
        erlaubt: &[("Feler", "Fehler")],
        verboten: &[("zwei", "drei", GrundCode::ZahlGeaendert)],
        unveraendert: &["Wir haben zwei Angebote erhalten."],
    },
    Fall {
        name: "fehlerfreier_kontrolltext",
        text: "Dies ist ein vollständig korrekter Satz.\nUnd noch einer.",
        erlaubt: &[],
        verboten: &[],
        unveraendert: &["Dies ist ein vollständig korrekter Satz.\nUnd noch einer."],
    },
    Fall {
        name: "mehrere_abschnitte",
        text: "Erste Zeile mit Feler.\n\nZweite Zeile ohne Fehler.\nDritte Zeile mit Feler und Tippfeler.",
        erlaubt: &[("Feler", "Fehler"), ("Tippfeler", "Tippfehler")],
        verboten: &[],
        unveraendert: &["Zweite Zeile ohne Fehler."],
    },
];

fn einstellungen() -> AppSettings {
    let mut s = crate::settings::get_default_settings();
    s.llm_connections.clear();
    s.llm_models.clear();
    s.function_models.clear();
    s.llm_active_model_id = Some("local:eins".into());
    s.llm_connections.push(LlmConnection {
        id: "local".into(),
        kind: "local".into(),
        label: "local".into(),
        base_url: "http://127.0.0.1:0/v1".into(),
        enabled: true,
        monthly_budget_usd: None,
        budget_enforced: false,
        training_opt_out: true,
    });
    s.llm_models.push(LlmModelConfig {
        id: "local:eins".into(),
        connection_id: "local".into(),
        remote_id: "eins".into(),
        label: "eins".into(),
        enabled: true,
        context_limit: None,
        max_input_tokens: None,
        max_output_tokens: None,
        price_input_per_mtok: None,
        price_output_per_mtok: None,
        tags: Vec::new(),
        effort: None,
        fast: false,
    });
    s
}

fn mock_fuer(fall: &'static Fall) -> Arc<MockVersender> {
    Arc::new(MockVersender::antworter(move |auftrag: &Auftrag, _| {
        let zeilen = auftrag
            .nutzer
            .lines()
            .filter_map(|zeile| {
                let (nr, text) = zeile.split_once('|')?;
                let mut neu = text.to_owned();
                for (alt, ersatz) in fall.erlaubt {
                    neu = neu.replace(alt, ersatz);
                }
                for (alt, ersatz, _) in fall.verboten {
                    neu = neu.replace(alt, ersatz);
                }
                (neu != text).then(|| format!("{nr}|{neu}"))
            })
            .collect::<Vec<_>>();
        Ok(ModellAntwort {
            text: if zeilen.is_empty() {
                "OK".into()
            } else {
                zeilen.join("\n")
            },
            abgeschnitten: false,
        })
    }))
}

async fn lauf(text: &str, fall: &'static Fall) -> (Vec<Aenderung>, JobBericht) {
    let stand = Textstand::aus_text(text);
    let temp = tempfile::tempdir().unwrap();
    let cache = SprachCache::neu(temp.path());
    let eingabe = JobEingabe {
        dokument_id: "korpus",
        fassung_id: None,
        textstand: &stand,
        basis_text: text,
        regeln: &[],
        cache: &cache,
        grenzen: Grenzen {
            max_zeichen: 2500,
            max_zeilen: 2,
        },
    };
    let versand = test_versand(einstellungen(), mock_fuer(fall));
    let ergebnis = sprache_job(
        &eingabe,
        &versand,
        &versand.vorschau().unwrap(),
        &Abbruch::neu(),
        &|| Ok(text.to_owned()),
        &|_, _| {},
    )
    .await;
    let aenderungen = ergebnis
        .gruppen
        .iter()
        .flat_map(|g| g.aenderungen.iter().cloned())
        .collect();
    (aenderungen, ergebnis.bericht)
}

#[test]
fn korpus_hat_18_faelle() {
    assert_eq!(FAELLE.len(), 18);
}

#[tokio::test]
async fn korpus_erlaubt_und_verboten_je_fall() {
    for fall in FAELLE {
        let (mut aenderungen, bericht) = lauf(fall.text, fall).await;
        let mut erwartet = fall.text.to_owned();
        for (alt, neu) in fall.erlaubt {
            erwartet = erwartet.replace(alt, neu);
        }
        assert_eq!(
            pruefe_aenderungen(fall.text, &aenderungen),
            Ok(()),
            "{}",
            fall.name
        );
        for a in &mut aenderungen {
            assert_eq!(a.art, AenderungArt::Sprache, "{}", fall.name);
            assert_eq!(
                a.status,
                AenderungStatus::Offen,
                "{}: nie automatisch",
                fall.name
            );
            a.status = AenderungStatus::Angenommen;
        }
        let ergebnis = ergebnis_text(fall.text, &aenderungen).unwrap();
        assert_eq!(ergebnis, erwartet, "{}", fall.name);
        for stelle in fall.unveraendert {
            assert!(
                ergebnis.contains(stelle),
                "{}: Schutzstelle veraendert",
                fall.name
            );
        }
        for (_, _, grund) in fall.verboten {
            assert!(
                bericht.verworfen.get(*grund) >= 1,
                "{}: {:?}",
                fall.name,
                grund
            );
        }
        if fall.erlaubt.is_empty() {
            assert!(aenderungen.is_empty(), "{}", fall.name);
        }
        assert!(bericht.fehler.is_none(), "{}", fall.name);
        // Der Bericht traegt nur Zahlen und Codes, nie Text.
        let debug = format!("{bericht:?}");
        assert!(!debug.contains(fall.text), "{}", fall.name);
    }
}

#[tokio::test]
async fn korpus_zweiter_lauf_auf_dem_ergebnis_ist_idempotent() {
    for fall in FAELLE {
        let (mut aenderungen, _) = lauf(fall.text, fall).await;
        for a in &mut aenderungen {
            a.status = AenderungStatus::Angenommen;
        }
        let ergebnis = ergebnis_text(fall.text, &aenderungen).unwrap();
        let (zweite, _) = lauf(&ergebnis, fall).await;
        assert!(zweite.is_empty(), "{}: Korrekturzyklus", fall.name);
    }
}
