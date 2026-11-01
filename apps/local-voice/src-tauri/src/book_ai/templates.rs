//! Vorlagen für KI-Dokumente zum Buch (lva#182).
//!
//! Eine Vorlage legt Art und Umfang eines Ergebnisses fest: Zweck (Kontext),
//! Abschnitte mit je einer Anweisung, Zielumfang in Wörtern, Zielgruppe und
//! Ausgabeform. Muster wie bei den Besprechungsvorlagen
//! (`meetings/notes/templates.rs`): mitgelieferte Vorlagen tragen die feste ID
//! `builtin:<key>` und sind schreibgeschützt; eigene Vorlagen haben eine ULID
//! und liegen in `<App-Daten>/book_ai/vorlagen.json`.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Präfix der IDs mitgelieferter Vorlagen.
pub const BUILTIN_PREFIX: &str = "builtin:";
/// Kennung im Austauschformat einer einzelnen Vorlage.
pub const FILE_FORMAT: &str = "lva-book-template@1";
/// Empfohlene Dateiendung für exportierte Vorlagen (Dateidialog der Oberfläche).
#[allow(dead_code)]
pub const FILE_EXTENSION: &str = ".lvbooktemplate.json";
/// Größte zulässige Importdatei in Bytes.
pub const MAX_FILE_BYTES: usize = 64 * 1024;

pub const MAX_SECTIONS: usize = 12;
pub const MAX_TITLE_CHARS: usize = 60;
pub const MAX_DESCRIPTION_CHARS: usize = 200;
pub const MAX_INSTRUCTION_CHARS: usize = 600;
pub const MAX_CONTEXT_CHARS: usize = 1200;
pub const MIN_WORDS: u32 = 50;
pub const MAX_WORDS: u32 = 6000;

/// Wie das Ergebnis aufgebaut ist. Fließtext eignet sich am besten zum
/// Vorlesen; die anderen Formen werden vor dem Vorlesen von Formatzeichen
/// befreit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TemplateForm {
    /// Zusammenhängender Text ohne Überschriften und Listen.
    Prose,
    /// Überschriften je Abschnitt, darunter Text.
    Structured,
    /// Aufzählung.
    List,
    /// Markdown-Tabelle.
    Table,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BookTemplateSection {
    pub title: String,
    pub instruction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BookTemplate {
    pub id: String,
    pub title: String,
    /// Gruppe in der Auswahl: "ueberblick" | "lernen" | "analyse" | "anwenden".
    pub category: String,
    /// Ein Satz für die Auswahl.
    pub description: String,
    /// Zweck und Haltung des Ergebnisses.
    pub context: String,
    /// Leer bei Fließtext ohne Gliederung.
    pub sections: Vec<BookTemplateSection>,
    /// Zielumfang des ganzen Ergebnisses in Wörtern.
    pub target_words: u32,
    pub form: TemplateForm,
    /// "allgemein" | "fachpublikum" | "management" | "einfache_sprache" | "lernende"
    pub audience: String,
    /// Mitgeliefert und schreibgeschützt.
    pub builtin: bool,
}

pub const CATEGORIES: [&str; 4] = ["ueberblick", "lernen", "analyse", "anwenden"];
pub const AUDIENCES: [&str; 5] = [
    "allgemein",
    "fachpublikum",
    "management",
    "einfache_sprache",
    "lernende",
];

pub fn is_builtin_id(id: &str) -> bool {
    id.starts_with(BUILTIN_PREFIX)
}

// ---------------------------------------------------------------------------
// Prüfung
// ---------------------------------------------------------------------------

fn chars(s: &str) -> usize {
    s.trim().chars().count()
}

/// Prüft eine Vorlage. Die Fehlertexte sind stabile Codes
/// (`template_invalid:<feld>`), die Oberfläche übersetzt sie.
pub fn validate(t: &BookTemplate) -> Result<(), String> {
    let bad = |field: &str| Err(format!("template_invalid:{field}"));
    if chars(&t.title) == 0 || chars(&t.title) > MAX_TITLE_CHARS {
        return bad("title");
    }
    if chars(&t.description) > MAX_DESCRIPTION_CHARS {
        return bad("description");
    }
    if chars(&t.context) == 0 || chars(&t.context) > MAX_CONTEXT_CHARS {
        return bad("context");
    }
    if !CATEGORIES.contains(&t.category.as_str()) {
        return bad("category");
    }
    if !AUDIENCES.contains(&t.audience.as_str()) {
        return bad("audience");
    }
    if !(MIN_WORDS..=MAX_WORDS).contains(&t.target_words) {
        return bad("target_words");
    }
    if t.sections.len() > MAX_SECTIONS {
        return bad("sections");
    }
    for s in &t.sections {
        if chars(&s.title) == 0 || chars(&s.title) > MAX_TITLE_CHARS {
            return bad("section_title");
        }
        if chars(&s.instruction) == 0 || chars(&s.instruction) > MAX_INSTRUCTION_CHARS {
            return bad("section_instruction");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Austauschformat
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct TemplateFile {
    format: String,
    template: BookTemplate,
}

/// Eine Vorlage als Austauschdatei (JSON). Die ID reist nicht mit: beim
/// Import entsteht immer eine eigene, bearbeitbare Vorlage.
pub fn to_file(t: &BookTemplate) -> Result<String, String> {
    let mut template = t.clone();
    template.id = String::new();
    template.builtin = false;
    serde_json::to_string_pretty(&TemplateFile {
        format: FILE_FORMAT.to_string(),
        template,
    })
    .map_err(|e| e.to_string())
}

/// Austauschdatei lesen und prüfen. `new_id` wird die ID der neuen Vorlage.
pub fn from_file(raw: &str, new_id: String) -> Result<BookTemplate, String> {
    if raw.len() > MAX_FILE_BYTES {
        return Err("template_invalid:file_size".into());
    }
    let file: TemplateFile =
        serde_json::from_str(raw).map_err(|_| "template_invalid:file_format".to_string())?;
    if file.format != FILE_FORMAT {
        return Err("template_invalid:file_format".into());
    }
    let mut t = file.template;
    t.id = new_id;
    t.builtin = false;
    validate(&t)?;
    Ok(t)
}

// ---------------------------------------------------------------------------
// Mitgelieferte Vorlagen
// ---------------------------------------------------------------------------

struct Builtin {
    key: &'static str,
    title: &'static str,
    category: &'static str,
    description: &'static str,
    context: &'static str,
    sections: &'static [(&'static str, &'static str)],
    words: u32,
    form: TemplateForm,
    audience: &'static str,
}

const BUILTINS: &[Builtin] = &[
    Builtin {
        key: "kurzfassung",
        title: "Kurzfassung",
        category: "ueberblick",
        description: "Ein Absatz: worum es geht und was bleibt.",
        context: "Fasse den Inhalt in einem einzigen, dichten Absatz zusammen: Thema, Kernaussage beziehungsweise Handlungsbogen und das wichtigste Ergebnis.",
        sections: &[],
        words: 120,
        form: TemplateForm::Prose,
        audience: "allgemein",
    },
    Builtin {
        key: "eine_seite",
        title: "Eine Seite",
        category: "ueberblick",
        description: "Zusammenfassung auf einer Seite, gut zum Vorlesen.",
        context: "Schreibe eine Zusammenfassung, die auf eine Seite passt. Sie soll das Werk für jemanden erschließen, der es nicht gelesen hat: Thema, Aufbau, wichtigste Gedanken oder Ereignisse und Schluss.",
        sections: &[],
        words: 450,
        form: TemplateForm::Prose,
        audience: "allgemein",
    },
    Builtin {
        key: "vier_seiten",
        title: "Vier Seiten",
        category: "ueberblick",
        description: "Ausführliche Zusammenfassung mit Gliederung.",
        context: "Schreibe eine ausführliche Zusammenfassung von etwa vier Seiten. Folge dem Aufbau des Werks und erhalte Begründungen, Beispiele, Namen und Zahlen, die für das Verständnis nötig sind.",
        sections: &[
            ("Überblick", "Thema, Anliegen und Aufbau des Werks in wenigen Sätzen."),
            ("Inhalt im Verlauf", "Die Teile oder Kapitel der Reihe nach mit ihren wichtigsten Aussagen oder Ereignissen."),
            ("Zentrale Gedanken", "Die tragenden Ideen, Thesen oder Motive und wie sie entwickelt werden."),
            ("Schluss", "Ergebnis, Auflösung oder Fazit des Werks."),
        ],
        words: 1800,
        form: TemplateForm::Structured,
        audience: "allgemein",
    },
    Builtin {
        key: "management_summary",
        title: "Management Summary",
        category: "ueberblick",
        description: "Ergebnis, Bedeutung und Empfehlung für Entscheider.",
        context: "Verdichte den Inhalt für Entscheidungsträger mit wenig Zeit. Beginne mit dem Ergebnis, nicht mit der Herleitung. Konkret, ohne Ausschmückung.",
        sections: &[
            ("Kernaussage", "Die wichtigste Erkenntnis in zwei bis drei Sätzen."),
            ("Bedeutung", "Warum das relevant ist, welche Folgen und Chancen sich ergeben."),
            ("Empfehlungen", "Drei bis fünf konkrete Schlussfolgerungen oder nächste Schritte."),
        ],
        words: 300,
        form: TemplateForm::Structured,
        audience: "management",
    },
    Builtin {
        key: "kernaussagen",
        title: "Kernaussagen",
        category: "ueberblick",
        description: "Die zehn wichtigsten Aussagen als Liste.",
        context: "Nenne die zehn wichtigsten Aussagen, Erkenntnisse oder Wendepunkte. Jeder Punkt ist ein vollständiger Satz mit einer kurzen Erläuterung.",
        sections: &[],
        words: 350,
        form: TemplateForm::List,
        audience: "allgemein",
    },
    Builtin {
        key: "kapitelweise",
        title: "Kapitelweise Zusammenfassung",
        category: "ueberblick",
        description: "Zu jedem Kapitel ein kurzer Absatz.",
        context: "Fasse jedes Kapitel beziehungsweise jeden Abschnitt einzeln zusammen, in der Reihenfolge des Werks. Verwende die Kapiteltitel als Überschriften und schreibe darunter einen kurzen Absatz.",
        sections: &[],
        words: 1500,
        form: TemplateForm::Structured,
        audience: "allgemein",
    },
    Builtin {
        key: "inhaltsangabe",
        title: "Inhaltsangabe",
        category: "ueberblick",
        description: "Handlung im Präsens, für Romane und Erzählungen.",
        context: "Schreibe eine sachliche Inhaltsangabe im Präsens: Einleitung mit Titel, Gattung und Thema, dann die Handlung in ihren wesentlichen Schritten, ohne wörtliche Rede und ohne Wertung.",
        sections: &[],
        words: 600,
        form: TemplateForm::Prose,
        audience: "allgemein",
    },
    Builtin {
        key: "figuren",
        title: "Figurenübersicht",
        category: "ueberblick",
        description: "Wer ist wer, mit Beziehungen und Entwicklung.",
        context: "Stelle die wichtigen Figuren oder Personen vor: Rolle, Eigenschaften, Beziehungen zu anderen und wie sie sich im Verlauf entwickeln. Hauptfiguren zuerst.",
        sections: &[],
        words: 600,
        form: TemplateForm::List,
        audience: "allgemein",
    },
    Builtin {
        key: "glossar",
        title: "Glossar",
        category: "lernen",
        description: "Fachbegriffe mit kurzer Erklärung als Tabelle.",
        context: "Erstelle ein Glossar der Fachbegriffe, Namen und Konzepte, die für das Verständnis wichtig sind. Tabelle mit den Spalten Begriff und Erklärung, alphabetisch sortiert. Erkläre so, wie der Begriff im Werk gebraucht wird.",
        sections: &[],
        words: 900,
        form: TemplateForm::Table,
        audience: "lernende",
    },
    Builtin {
        key: "lernzettel",
        title: "Lernzettel",
        category: "lernen",
        description: "Das Wichtigste zum Wiederholen, knapp gegliedert.",
        context: "Erstelle einen Lernzettel zum Wiederholen. Knapp, gegliedert, mit den Begriffen, Zusammenhängen und Beispielen, die man sich merken sollte.",
        sections: &[
            ("Worum es geht", "Thema und Ziel in zwei Sätzen."),
            ("Das muss ich wissen", "Die wichtigsten Inhalte als knappe Stichpunkte mit Erläuterung."),
            ("Begriffe", "Die zentralen Begriffe mit einer Zeile Erklärung."),
            ("Zusammenhänge", "Wie die Inhalte miteinander zusammenhängen, mit einem Beispiel."),
        ],
        words: 700,
        form: TemplateForm::Structured,
        audience: "lernende",
    },
    Builtin {
        key: "karteikarten",
        title: "Karteikarten",
        category: "lernen",
        description: "Zwanzig Frage-Antwort-Paare zum Lernen.",
        context: "Erstelle zwanzig Karteikarten. Jede Karte hat eine Frage und eine kurze, eindeutige Antwort. Fragen von einfach nach schwer, ohne Doppelungen. Format je Karte: „Frage: …“ und darunter „Antwort: …“.",
        sections: &[],
        words: 800,
        form: TemplateForm::List,
        audience: "lernende",
    },
    Builtin {
        key: "quiz",
        title: "Quiz mit Lösungen",
        category: "lernen",
        description: "Fünfzehn Fragen mit Auswahlantworten und Lösung.",
        context: "Erstelle ein Quiz mit fünfzehn Fragen zum Inhalt. Je Frage drei Antwortmöglichkeiten (a, b, c), genau eine richtig. Am Ende ein Lösungsteil mit kurzer Begründung je Frage.",
        sections: &[
            ("Fragen", "Die fünfzehn Fragen mit den Antwortmöglichkeiten."),
            ("Lösungen", "Die richtige Antwort je Frage mit einem Satz Begründung."),
        ],
        words: 1000,
        form: TemplateForm::Structured,
        audience: "lernende",
    },
    Builtin {
        key: "schluesselstellen",
        title: "Zitate und Schlüsselstellen",
        category: "analyse",
        description: "Die wichtigsten Stellen mit Einordnung.",
        context: "Nenne zehn bis fünfzehn Schlüsselstellen in der Reihenfolge des Werks. Je Stelle: Kapitel, ein kurzes wörtliches Zitat (nur wenn es im Text steht, sonst sinngemäß und so gekennzeichnet) und warum die Stelle wichtig ist.",
        sections: &[],
        words: 900,
        form: TemplateForm::List,
        audience: "allgemein",
    },
    Builtin {
        key: "argumentation",
        title: "Argumentationsanalyse",
        category: "analyse",
        description: "Thesen, Belege und mögliche Einwände.",
        context: "Analysiere die Argumentation des Werks sachlich: Welche Thesen werden vertreten, womit werden sie begründet, wie tragfähig ist das, und welche Einwände liegen nahe?",
        sections: &[
            ("Thesen", "Die zentralen Behauptungen des Werks."),
            ("Begründung und Belege", "Womit die Thesen gestützt werden: Daten, Beispiele, Autoritäten, Logik."),
            ("Bewertung", "Stärken und Schwächen der Argumentation."),
            ("Einwände", "Naheliegende Gegenargumente und offene Fragen."),
        ],
        words: 1000,
        form: TemplateForm::Structured,
        audience: "fachpublikum",
    },
    Builtin {
        key: "rezension",
        title: "Rezension",
        category: "analyse",
        description: "Kritische Würdigung mit Fazit und Empfehlung.",
        context: "Schreibe eine ausgewogene Rezension: kurz worum es geht, was gelingt, was schwächer ist, für wen sich das Werk lohnt. Urteile begründen, nicht behaupten.",
        sections: &[],
        words: 700,
        form: TemplateForm::Prose,
        audience: "allgemein",
    },
    Builtin {
        key: "handlungsempfehlungen",
        title: "Handlungsempfehlungen",
        category: "anwenden",
        description: "Was man konkret umsetzen kann.",
        context: "Leite aus dem Inhalt fünf bis zehn konkrete, umsetzbare Maßnahmen ab. Je Maßnahme: was zu tun ist, warum (Bezug zum Werk) und ein erster Schritt.",
        sections: &[],
        words: 700,
        form: TemplateForm::List,
        audience: "management",
    },
    Builtin {
        key: "zeitleiste",
        title: "Zeitleiste",
        category: "analyse",
        description: "Ereignisse in zeitlicher Reihenfolge als Tabelle.",
        context: "Erstelle eine Zeitleiste der Ereignisse in ihrer zeitlichen (nicht erzählten) Reihenfolge. Tabelle mit den Spalten Zeitpunkt, Ereignis und Bedeutung. Wo der Zeitpunkt unklar ist, relative Angaben verwenden.",
        sections: &[],
        words: 800,
        form: TemplateForm::Table,
        audience: "allgemein",
    },
    Builtin {
        key: "einfache_sprache",
        title: "Einfache Sprache",
        category: "ueberblick",
        description: "Leicht verständlich, kurze Sätze.",
        context: "Fasse den Inhalt in einfacher Sprache zusammen: kurze Sätze, bekannte Wörter, ein Gedanke pro Satz. Schwierige Begriffe, die nötig sind, sofort erklären.",
        sections: &[],
        words: 400,
        form: TemplateForm::Prose,
        audience: "einfache_sprache",
    },
    Builtin {
        key: "hoerfassung",
        title: "Hörfassung in drei Minuten",
        category: "ueberblick",
        description: "Zum Vorlesen geschrieben, etwa drei Minuten.",
        context: "Schreibe eine Zusammenfassung, die vorgelesen etwa drei Minuten dauert. Hörgerecht: kurze Sätze, klare Übergänge, keine Klammern, Abkürzungen ausgeschrieben, Zahlen so, wie man sie spricht.",
        sections: &[],
        words: 430,
        form: TemplateForm::Prose,
        audience: "allgemein",
    },
    Builtin {
        key: "vortragsgeruest",
        title: "Vortragsgerüst",
        category: "anwenden",
        description: "Gliederung mit Folienvorschlägen für einen Vortrag.",
        context: "Entwirf das Gerüst eines Vortrags von etwa zwanzig Minuten über das Werk. Je Folie: Folientitel, drei bis fünf Stichpunkte und ein Satz, was dazu gesagt wird.",
        sections: &[
            ("Einstieg", "Eine Folie, die das Publikum abholt: Frage, Szene oder Zahl aus dem Werk."),
            ("Hauptteil", "Sechs bis acht Folien entlang der wichtigsten Inhalte."),
            ("Abschluss", "Fazit, Kernbotschaft und eine Frage zur Diskussion."),
        ],
        words: 1000,
        form: TemplateForm::Structured,
        audience: "allgemein",
    },
];

/// Die mitgelieferten Vorlagen in fester Reihenfolge.
pub fn builtins() -> Vec<BookTemplate> {
    BUILTINS
        .iter()
        .map(|b| BookTemplate {
            id: format!("{BUILTIN_PREFIX}{}", b.key),
            title: b.title.to_string(),
            category: b.category.to_string(),
            description: b.description.to_string(),
            context: b.context.to_string(),
            sections: b
                .sections
                .iter()
                .map(|(title, instruction)| BookTemplateSection {
                    title: title.to_string(),
                    instruction: instruction.to_string(),
                })
                .collect(),
            target_words: b.words,
            form: b.form,
            audience: b.audience.to_string(),
            builtin: true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_twenty_valid_builtins_with_unique_ids() {
        let all = builtins();
        assert_eq!(all.len(), 20);
        let mut ids: Vec<_> = all.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 20);
        for t in &all {
            assert!(t.builtin && is_builtin_id(&t.id));
            validate(t).unwrap_or_else(|e| panic!("{}: {e}", t.id));
        }
    }

    #[test]
    fn every_category_is_used() {
        let all = builtins();
        for c in CATEGORIES {
            assert!(all.iter().any(|t| t.category == c), "{c}");
        }
    }

    #[test]
    fn validation_rejects_bad_fields_with_stable_codes() {
        let mut t = builtins()[0].clone();
        t.title = " ".into();
        assert_eq!(validate(&t).unwrap_err(), "template_invalid:title");
        let mut t = builtins()[0].clone();
        t.target_words = 10;
        assert_eq!(validate(&t).unwrap_err(), "template_invalid:target_words");
        let mut t = builtins()[0].clone();
        t.category = "x".into();
        assert_eq!(validate(&t).unwrap_err(), "template_invalid:category");
        let mut t = builtins()[2].clone();
        t.sections[0].instruction = String::new();
        assert_eq!(
            validate(&t).unwrap_err(),
            "template_invalid:section_instruction"
        );
    }

    #[test]
    fn export_and_import_round_trip_as_own_template() {
        let original = builtins()[3].clone();
        let raw = to_file(&original).unwrap();
        assert!(raw.contains(FILE_FORMAT));
        let back = from_file(&raw, "01ABC".into()).unwrap();
        assert_eq!(back.id, "01ABC");
        assert!(!back.builtin);
        assert_eq!(back.title, original.title);
        assert_eq!(back.sections, original.sections);
    }

    #[test]
    fn import_rejects_foreign_or_oversized_files() {
        assert_eq!(
            from_file("{\"format\":\"x\"}", "a".into()).unwrap_err(),
            "template_invalid:file_format"
        );
        let big = "x".repeat(MAX_FILE_BYTES + 1);
        assert_eq!(
            from_file(&big, "a".into()).unwrap_err(),
            "template_invalid:file_size"
        );
    }
}
