//! Vorlagen fuer KI-Notizen: mitgelieferter Katalog, Pruefung, Import/Export
//! (M1, P1a).
//!
//! Eine Vorlage besteht aus einem Kontext (Zweck der Besprechung) und
//! Abschnitten mit je einer Anweisung. Mitgelieferte Vorlagen haben die feste
//! ID `builtin:<key>` und werden vom Store bei jedem Oeffnen auf den Stand der
//! App gebracht (siehe `MeetingStore::seed_builtin_templates`).

use serde::{Deserialize, Serialize};

use super::model::{SectionKind, TemplateInfo, TemplateSection, TemplateSpec};

/// Praefix der IDs mitgelieferter Vorlagen.
pub const BUILTIN_PREFIX: &str = "builtin:";
/// Standardvorlage, solange der Nutzer keine andere gewaehlt hat.
pub const DEFAULT_TEMPLATE_ID: &str = "builtin:allgemein";
/// P1k: "Automatisch (nach Inhalt)". Kein Vorlagen-Eintrag, sondern die Wahl
/// selbst: sie steht als Wert in `meetings.template_id` (und in der Einstellung
/// `meeting_default_template_id`), und der Motor waehlt beim Erzeugen anhand des
/// Inhalts eine echte Vorlage (`classify`). Nutzer-IDs sind ULIDs und
/// mitgelieferte tragen `builtin:`: keine Kollision.
pub const AUTO_TEMPLATE_ID: &str = "auto";
/// Aktuelle Version des `TemplateSpec`-Formats.
pub const SPEC_VERSION: u32 = 1;
/// Kennung im Austauschformat einer einzelnen Vorlage.
pub const FILE_FORMAT: &str = "lva-meeting-template@1";
/// Empfohlene Dateiendung fuer exportierte Vorlagen.
pub const FILE_EXTENSION: &str = ".lvtemplate.json";
/// Groesste zulaessige Importdatei in Bytes.
pub const MAX_FILE_BYTES: usize = 64 * 1024;

pub const MAX_SECTIONS: usize = 10;
pub const MAX_TITLE_CHARS: usize = 60;
pub const MAX_INSTRUCTION_CHARS: usize = 500;
pub const MAX_CONTEXT_CHARS: usize = 1000;
const MAX_SECTION_ID_CHARS: usize = 32;

/// ID einer mitgelieferten Vorlage aus ihrem Schluessel.
pub fn builtin_id(key: &str) -> String {
    format!("{BUILTIN_PREFIX}{key}")
}

/// `true`, wenn die Wahl "Automatisch (nach Inhalt)" gemeint ist (Gross- und
/// Kleinschreibung und Leerraum egal).
pub fn is_auto_id(id: &str) -> bool {
    id.trim().eq_ignore_ascii_case(AUTO_TEMPLATE_ID)
}

/// `true`, wenn die ID zu einer mitgelieferten (schreibgeschuetzten) Vorlage gehoert.
pub fn is_builtin_id(id: &str) -> bool {
    id.starts_with(BUILTIN_PREFIX)
}

// ---------------------------------------------------------------------------
// Pruefung
// ---------------------------------------------------------------------------

/// Titel einer Vorlage (oder eines Abschnitts): nach dem Trimmen 1-60 Zeichen.
pub fn validate_title(title: &str) -> Result<(), String> {
    let chars = title.trim().chars().count();
    if chars == 0 || chars > MAX_TITLE_CHARS {
        return Err("template_invalid:title".to_string());
    }
    Ok(())
}

fn valid_section_id(id: &str) -> bool {
    let len = id.chars().count();
    (1..=MAX_SECTION_ID_CHARS).contains(&len)
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Prueft eine Vorlage: Version 1, 1-10 Abschnitte, IDs eindeutig und
/// `[a-z0-9_]{1,32}`, Titel 1-60 Zeichen, Anweisung <= 500, Kontext <= 1000,
/// hoechstens ein `Tasks`-Abschnitt. Die Fehlertexte sind stabile Codes
/// (`template_invalid:<grund>[:<abschnitt>]`), die die UI uebersetzen kann.
pub fn validate_spec(spec: &TemplateSpec) -> Result<(), String> {
    if spec.version != SPEC_VERSION {
        return Err("template_invalid:version".to_string());
    }
    if spec.context.chars().count() > MAX_CONTEXT_CHARS {
        return Err("template_invalid:context_too_long".to_string());
    }
    if spec.sections.is_empty() || spec.sections.len() > MAX_SECTIONS {
        return Err("template_invalid:sections_count".to_string());
    }
    let mut seen: Vec<&str> = Vec::with_capacity(spec.sections.len());
    let mut tasks = 0usize;
    for section in &spec.sections {
        if !valid_section_id(&section.id) {
            return Err(format!("template_invalid:section_id:{}", section.id));
        }
        if seen.contains(&section.id.as_str()) {
            return Err(format!(
                "template_invalid:section_id_duplicate:{}",
                section.id
            ));
        }
        seen.push(&section.id);
        let title_chars = section.title.trim().chars().count();
        if title_chars == 0 || title_chars > MAX_TITLE_CHARS {
            return Err(format!("template_invalid:section_title:{}", section.id));
        }
        if section.instruction.chars().count() > MAX_INSTRUCTION_CHARS {
            return Err(format!(
                "template_invalid:section_instruction_too_long:{}",
                section.id
            ));
        }
        if section.kind == SectionKind::Tasks {
            tasks += 1;
        }
    }
    if tasks > 1 {
        return Err("template_invalid:tasks_sections".to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Export / Import
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'static str,
    title: &'a str,
    spec: &'a TemplateSpec,
}

#[derive(Deserialize)]
struct FileIn {
    format: String,
    title: String,
    spec: TemplateSpec,
}

/// Serialisiert eine Vorlage als Austauschdatei
/// (`{"format":"lva-meeting-template@1","title":...,"spec":...}`).
pub fn export_file(info: &TemplateInfo) -> String {
    serde_json::to_string_pretty(&FileOut {
        format: FILE_FORMAT,
        title: &info.title,
        spec: &info.spec,
    })
    // Reine Strings/Zahlen/Enums: Serialisierung kann nicht scheitern.
    .expect("template file serializes")
}

/// Liest eine Austauschdatei. Prueft Groesse (<= 64 KB), Kennung, Titel und
/// die Vorlage selbst; erst danach darf der Aufrufer etwas speichern. Ein
/// fuehrendes BOM (Windows-Editoren) wird toleriert.
pub fn import_file(json: &str) -> Result<(String, TemplateSpec), String> {
    if json.len() > MAX_FILE_BYTES {
        return Err("template_import:too_large".to_string());
    }
    let json = json.strip_prefix('\u{feff}').unwrap_or(json);
    let file: FileIn =
        serde_json::from_str(json).map_err(|_| "template_import:invalid_json".to_string())?;
    if file.format != FILE_FORMAT {
        return Err("template_import:format".to_string());
    }
    validate_title(&file.title)?;
    validate_spec(&file.spec)?;
    Ok((file.title.trim().to_string(), file.spec))
}

// ---------------------------------------------------------------------------
// Mitgelieferter Katalog
// ---------------------------------------------------------------------------

fn text(id: &str, title: &str, instruction: &str) -> TemplateSection {
    TemplateSection {
        id: id.to_string(),
        title: title.to_string(),
        instruction: instruction.to_string(),
        kind: SectionKind::Text,
    }
}

fn tasks(id: &str, title: &str, instruction: &str) -> TemplateSection {
    TemplateSection {
        kind: SectionKind::Tasks,
        ..text(id, title, instruction)
    }
}

fn spec(context: &str, sections: Vec<TemplateSection>) -> TemplateSpec {
    TemplateSpec {
        version: SPEC_VERSION,
        context: context.to_string(),
        sections,
    }
}

/// Die acht mitgelieferten deutschen Vorlagen als `(key, titel, spec)`; die
/// Reihenfolge ist die Anzeigereihenfolge. Jede hat mindestens einen
/// `Tasks`-Abschnitt. Aenderungen an Texten greifen beim naechsten Oeffnen des
/// Stores (Upsert), Schluessel duerfen nie umbenannt werden (Besprechungen
/// verweisen per `builtin:<key>` darauf).
pub fn builtin_templates() -> Vec<(&'static str, &'static str, TemplateSpec)> {
    vec![
        (
            "allgemein",
            "Allgemein",
            spec(
                "Allgemeine Besprechung ohne feste Struktur. Die Notizen sollen Anlass, Ergebnisse und nächste Schritte schnell erfassbar machen.",
                vec![
                    text(
                        "zusammenfassung",
                        "Zusammenfassung",
                        "Fasse Anlass und Ergebnis der Besprechung in 3 bis 5 Sätzen zusammen: worum es ging und was am Ende festgehalten wurde.",
                    ),
                    text(
                        "besprochene_punkte",
                        "Besprochene Punkte",
                        "Ein Stichpunkt je Thema mit der Kernaussage, in der Reihenfolge des Gesprächs. Keine Wiederholungen.",
                    ),
                    text(
                        "entscheidungen",
                        "Entscheidungen",
                        "Nur tatsächlich Beschlossenes, ein Satz je Entscheidung. Vorschläge, Vermutungen und offene Diskussionen gehören nicht hierher.",
                    ),
                    tasks(
                        "aufgaben",
                        "Aufgaben",
                        "Aufgaben, die im Gespräch ausdrücklich vergeben oder zugesagt wurden: wer erledigt was bis wann. Verantwortliche und Termine nur eintragen, wenn sie genannt wurden.",
                    ),
                    text(
                        "offene_fragen",
                        "Offene Fragen",
                        "Fragen und Punkte, die im Gespräch ungeklärt blieben oder später geklärt werden müssen.",
                    ),
                ],
            ),
        ),
        (
            "vertrieb",
            "Kundengespräch / Vertrieb",
            spec(
                "Gespräch mit einem Kunden oder Interessenten. Die Notizen sollen Bedarf, Einwände und die vereinbarten nächsten Schritte für die Nachbereitung festhalten.",
                vec![
                    text(
                        "kunde_ausgangslage",
                        "Kunde & Ausgangslage",
                        "Wer der Kunde ist und in welcher Situation er sich befindet: Unternehmen, Ansprechpartner, Anlass des Gesprächs, bisheriger Stand.",
                    ),
                    text(
                        "bedarf_schmerzpunkte",
                        "Bedarf & Schmerzpunkte",
                        "Was der Kunde braucht und was ihn stört. Wörtliche Aussagen des Kunden bevorzugen und nicht umformulieren, wenn sie prägnant sind.",
                    ),
                    text(
                        "einwaende",
                        "Einwände",
                        "Bedenken, Zweifel und Widerstände des Kunden, jeweils mit dem Punkt, auf den sie sich beziehen, und wie darauf reagiert wurde.",
                    ),
                    text(
                        "budget_zeitrahmen_entscheider",
                        "Budget, Zeitrahmen, Entscheider",
                        "Nur was im Gespräch tatsächlich genannt wurde: Budget, gewünschter Zeitrahmen, beteiligte Entscheider und Entscheidungsweg. Nichts schätzen oder ergänzen.",
                    ),
                    tasks(
                        "naechste_schritte",
                        "Nächste Schritte",
                        "Vereinbarte nächste Schritte: wer tut was bis wann. Verantwortliche und Termine nur eintragen, wenn sie genannt wurden.",
                    ),
                    text(
                        "einschaetzung",
                        "Einschätzung",
                        "Kurze Einschätzung der Lage, die sich aus den Aussagen des Kunden ableiten lässt (Interesse, Dringlichkeit, Hürden). Als Einschätzung kennzeichnen, nichts hinzuerfinden.",
                    ),
                ],
            ),
        ),
        (
            "eins_zu_eins",
            "1:1-Gespräch",
            spec(
                "Regelmäßiges Einzelgespräch zwischen zwei Personen, etwa Führungskraft und Mitarbeiter. Die Notizen sollen Stimmung, Fortschritt, Hindernisse und Vereinbarungen festhalten.",
                vec![
                    text(
                        "stimmung_themen",
                        "Stimmung & Themen der Person",
                        "Wie es der Person geht und welche Themen sie von sich aus angesprochen hat, sinngemäß und respektvoll.",
                    ),
                    text(
                        "fortschritt",
                        "Fortschritt seit letztem Mal",
                        "Was seit dem letzten Gespräch erreicht oder vorangebracht wurde und was nicht.",
                    ),
                    text(
                        "hindernisse",
                        "Hindernisse",
                        "Was die Arbeit bremst oder blockiert, und welche Unterstützung dafür gewünscht oder angeboten wurde.",
                    ),
                    text(
                        "feedback",
                        "Feedback",
                        "Rückmeldungen in beide Richtungen, getrennt aufgeführt: Feedback an die Person und Feedback von der Person.",
                    ),
                    tasks(
                        "vereinbarungen",
                        "Vereinbarungen",
                        "Konkrete Vereinbarungen und Zusagen beider Seiten: wer tut was bis wann. Verantwortliche und Termine nur eintragen, wenn sie genannt wurden.",
                    ),
                ],
            ),
        ),
        (
            "jour_fixe",
            "Jour fixe / Team",
            spec(
                "Regelmäßiges Teamtreffen. Die Notizen sollen den Stand je Person oder Thema, Blocker, Entscheidungen und Aufgaben festhalten.",
                vec![
                    text(
                        "stand",
                        "Stand je Person oder Thema",
                        "Der aktuelle Stand, gegliedert nach Person oder nach Thema, je ein bis zwei Sätze. Zuerst den Namen oder das Thema nennen.",
                    ),
                    text(
                        "blocker",
                        "Blocker",
                        "Was Fortschritt aktuell verhindert, mit dem betroffenen Thema oder der betroffenen Person und dem genannten Grund.",
                    ),
                    text(
                        "entscheidungen",
                        "Entscheidungen",
                        "Nur tatsächlich Beschlossenes, ein Satz je Entscheidung. Vorschläge und offene Diskussionen gehören nicht hierher.",
                    ),
                    tasks(
                        "aufgaben",
                        "Aufgaben",
                        "Aufgaben, die im Treffen vergeben oder zugesagt wurden: wer erledigt was bis wann. Verantwortliche und Termine nur eintragen, wenn sie genannt wurden.",
                    ),
                    text(
                        "themen_naechstes_mal",
                        "Themen fürs nächste Mal",
                        "Themen, die verschoben wurden oder beim nächsten Treffen besprochen werden sollen.",
                    ),
                ],
            ),
        ),
        (
            "kickoff",
            "Projekt-Kickoff",
            spec(
                "Auftakt eines Projekts. Die Notizen sollen Ziel, Umfang, Rollen, Termine und Risiken so festhalten, dass alle Beteiligten denselben Stand haben.",
                vec![
                    text(
                        "ziel_erfolgskriterien",
                        "Ziel & Erfolgskriterien",
                        "Was das Projekt erreichen soll und woran Erfolg gemessen wird. Kriterien nur eintragen, wenn sie genannt wurden.",
                    ),
                    text(
                        "umfang_abgrenzung",
                        "Umfang & Abgrenzung",
                        "Was zum Projekt gehört und was ausdrücklich nicht. Bei unklarer Abgrenzung den Punkt als offen kennzeichnen.",
                    ),
                    text(
                        "rollen_zustaendigkeiten",
                        "Rollen & Zuständigkeiten",
                        "Wer welche Rolle oder Verantwortung im Projekt hat, je Person ein Stichpunkt. Nur Genanntes eintragen.",
                    ),
                    text(
                        "meilensteine_termine",
                        "Meilensteine & Termine",
                        "Genannte Meilensteine, Fristen und Termine mit Datum oder Zeitraum, in zeitlicher Reihenfolge.",
                    ),
                    text(
                        "risiken_annahmen",
                        "Risiken & Annahmen",
                        "Genannte Risiken und die Annahmen, auf denen die Planung beruht, getrennt aufgeführt.",
                    ),
                    tasks(
                        "erste_schritte",
                        "Erste Schritte",
                        "Die ersten vereinbarten Schritte nach dem Kickoff: wer tut was bis wann. Verantwortliche und Termine nur eintragen, wenn sie genannt wurden.",
                    ),
                ],
            ),
        ),
        (
            "interview",
            "Interview / Bewerbung",
            spec(
                "Vorstellungs- oder Interviewgespräch mit einer Person. Die Notizen sollen Werdegang, Antworten, Stärken und Bedenken sachlich und nachvollziehbar festhalten.",
                vec![
                    text(
                        "person_werdegang",
                        "Person & Werdegang",
                        "Nur was die Person über sich und ihren Werdegang gesagt hat: Stationen, Aufgaben, Ausbildung. Nichts ergänzen oder bewerten.",
                    ),
                    text(
                        "fragen_antworten",
                        "Fragen & Antworten",
                        "Je gestellter Frage ein Eintrag: die Frage kurz, die Antwort sinngemäß.",
                    ),
                    text(
                        "staerken",
                        "Stärken",
                        "Stärken, die aus den Aussagen im Gespräch erkennbar wurden, jeweils mit dem Beleg aus dem Gespräch. Keine Vermutungen über Eigenschaften.",
                    ),
                    text(
                        "offene_punkte_bedenken",
                        "Offene Punkte & Bedenken",
                        "Unklare Punkte, Lücken und geäußerte Bedenken, die nachgefragt oder geklärt werden sollten.",
                    ),
                    tasks(
                        "weiteres_vorgehen",
                        "Weiteres Vorgehen",
                        "Vereinbarte nächste Schritte und Fristen für beide Seiten: wer tut was bis wann. Verantwortliche und Termine nur eintragen, wenn sie genannt wurden.",
                    ),
                ],
            ),
        ),
        (
            "workshop",
            "Workshop / Brainstorming",
            spec(
                "Workshop oder Brainstorming zu einer Fragestellung. Die Notizen sollen Ideen gruppiert festhalten, ohne sie zu werten, und die daraus folgenden Entscheidungen und Aufgaben sichern.",
                vec![
                    text(
                        "fragestellung",
                        "Fragestellung",
                        "Die Frage oder das Ziel des Workshops in ein bis zwei Sätzen.",
                    ),
                    text(
                        "ideen",
                        "Ideen",
                        "Alle genannten Ideen, thematisch gruppiert, je Idee ein Stichpunkt. Ohne Wertung und ohne Auslassung von Randideen.",
                    ),
                    text(
                        "bewertung_favoriten",
                        "Bewertung & Favoriten",
                        "Wie die Ideen bewertet wurden und welche als Favoriten galten, mit den genannten Gründen.",
                    ),
                    text(
                        "entscheidungen",
                        "Entscheidungen",
                        "Nur tatsächlich Beschlossenes, ein Satz je Entscheidung.",
                    ),
                    tasks(
                        "aufgaben",
                        "Aufgaben",
                        "Aufgaben, die im Workshop vergeben oder zugesagt wurden: wer erledigt was bis wann. Verantwortliche und Termine nur eintragen, wenn sie genannt wurden.",
                    ),
                    text(
                        "parkplatz",
                        "Parkplatz",
                        "Themen und Ideen, die aufkamen, aber bewusst zurückgestellt wurden oder nicht zur Fragestellung gehörten.",
                    ),
                ],
            ),
        ),
        (
            "lenkungskreis",
            "Lenkungskreis / Entscheidung",
            spec(
                "Sitzung eines Lenkungskreises oder Entscheidungsgremiums. Die Notizen sollen Optionen, Argumente und Beschlüsse so festhalten, dass sie später nachvollziehbar und belegbar sind.",
                vec![
                    text(
                        "entscheidungsvorlage_optionen",
                        "Entscheidungsvorlage & Optionen",
                        "Worüber zu entscheiden war und welche Optionen zur Wahl standen, je Option ein Stichpunkt.",
                    ),
                    text(
                        "diskussion",
                        "Diskussion",
                        "Pro und Contra je Option, wie im Gremium vorgebracht, mit dem Namen der Person, wenn er genannt wurde.",
                    ),
                    text(
                        "beschluesse",
                        "Beschlüsse",
                        "Jeder Beschluss im Wortlaut, soweit er formuliert wurde, samt genannter Gegenstimmen, Enthaltungen und Bedingungen. Nur tatsächlich Beschlossenes.",
                    ),
                    text(
                        "risiken_eskalationen",
                        "Risiken & Eskalationen",
                        "Genannte Risiken und Punkte, die eskaliert oder an eine höhere Ebene gegeben wurden.",
                    ),
                    tasks(
                        "auftraege",
                        "Aufträge",
                        "Aufträge, die das Gremium vergeben hat: wer erledigt was bis wann. Verantwortliche und Termine nur eintragen, wenn sie genannt wurden.",
                    ),
                ],
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_spec() -> TemplateSpec {
        spec(
            "Zweck",
            vec![text("a", "Erster", "Anweisung"), tasks("b", "Zweiter", "")],
        )
    }

    fn info_of(spec: TemplateSpec) -> TemplateInfo {
        TemplateInfo {
            id: "01J0".into(),
            title: "Meine Vorlage".into(),
            builtin: false,
            spec,
            updated_at: 1,
        }
    }

    #[test]
    fn all_builtins_validate() {
        let all = builtin_templates();
        assert_eq!(all.len(), 8, "acht mitgelieferte Vorlagen");

        let mut keys: Vec<&str> = all.iter().map(|(k, _, _)| *k).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 8, "Schluessel eindeutig");

        for (key, title, spec) in &all {
            assert!(
                key.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "Schluessel {key} nur [a-z0-9_]"
            );
            validate_title(title).unwrap_or_else(|e| panic!("{key}: Titel: {e}"));
            validate_spec(spec).unwrap_or_else(|e| panic!("{key}: {e}"));
            let tasks = spec
                .sections
                .iter()
                .filter(|s| s.kind == SectionKind::Tasks)
                .count();
            assert_eq!(tasks, 1, "{key}: genau ein Tasks-Abschnitt");
            assert!(
                !spec.context.trim().is_empty(),
                "{key}: Kontext darf nicht leer sein"
            );
            for section in &spec.sections {
                assert!(
                    !section.instruction.trim().is_empty(),
                    "{key}/{}: Anweisung fehlt",
                    section.id
                );
            }
        }
    }

    #[test]
    fn the_default_template_is_part_of_the_catalog() {
        let ids: Vec<String> = builtin_templates()
            .iter()
            .map(|(key, _, _)| builtin_id(key))
            .collect();
        assert!(ids.contains(&DEFAULT_TEMPLATE_ID.to_string()));
        assert_eq!(ids[0], DEFAULT_TEMPLATE_ID, "Standardvorlage steht vorn");
        assert!(is_builtin_id(&ids[3]));
        assert!(!is_builtin_id("01J0ABCDEF"));
    }

    #[test]
    fn the_auto_choice_is_no_template_id() {
        assert!(is_auto_id("auto"));
        assert!(is_auto_id("  Auto "));
        assert!(!is_auto_id("builtin:allgemein"));
        assert!(!is_auto_id("01J0ABCDEF"));
        assert!(!is_auto_id(""));
        // Kein Katalogschluessel und keine Vorlage darf so heissen.
        assert!(builtin_templates().iter().all(|(key, _, _)| *key != "auto"));
        assert!(!is_builtin_id(AUTO_TEMPLATE_ID));
    }

    #[test]
    fn a_valid_spec_passes() {
        validate_spec(&valid_spec()).unwrap();
    }

    #[test]
    fn invalid_specs_are_rejected_with_a_stable_code() {
        let check = |name: &str, mutate: fn(&mut TemplateSpec), expected: &str| {
            let mut s = valid_spec();
            mutate(&mut s);
            assert_eq!(
                validate_spec(&s).unwrap_err(),
                expected,
                "Fall {name} muss abgelehnt werden"
            );
        };
        check("version", |s| s.version = 2, "template_invalid:version");
        check(
            "context zu lang",
            |s| s.context = "x".repeat(MAX_CONTEXT_CHARS + 1),
            "template_invalid:context_too_long",
        );
        check(
            "keine Abschnitte",
            |s| s.sections.clear(),
            "template_invalid:sections_count",
        );
        check(
            "elf Abschnitte",
            |s| s.sections = (0..11).map(|i| text(&format!("s{i}"), "T", "")).collect(),
            "template_invalid:sections_count",
        );
        check(
            "ID mit Grossbuchstaben",
            |s| s.sections[0].id = "Abc".into(),
            "template_invalid:section_id:Abc",
        );
        check(
            "ID mit Umlaut",
            |s| s.sections[0].id = "größe".into(),
            "template_invalid:section_id:größe",
        );
        check(
            "leere ID",
            |s| s.sections[0].id = String::new(),
            "template_invalid:section_id:",
        );
        check(
            "ID zu lang",
            |s| s.sections[0].id = "a".repeat(33),
            &format!("template_invalid:section_id:{}", "a".repeat(33)),
        );
        check(
            "doppelte ID",
            |s| s.sections[1].id = "a".into(),
            "template_invalid:section_id_duplicate:a",
        );
        check(
            "leerer Titel",
            |s| s.sections[0].title = "   ".into(),
            "template_invalid:section_title:a",
        );
        check(
            "Titel zu lang",
            |s| s.sections[0].title = "ä".repeat(MAX_TITLE_CHARS + 1),
            "template_invalid:section_title:a",
        );
        check(
            "Anweisung zu lang",
            |s| s.sections[0].instruction = "x".repeat(MAX_INSTRUCTION_CHARS + 1),
            "template_invalid:section_instruction_too_long:a",
        );
        check(
            "zwei Tasks-Abschnitte",
            |s| s.sections[0].kind = SectionKind::Tasks,
            "template_invalid:tasks_sections",
        );
    }

    #[test]
    fn limits_count_characters_not_bytes() {
        let mut s = valid_spec();
        // 60 Umlaute = 120 Bytes, aber genau 60 Zeichen: erlaubt.
        s.sections[0].title = "ä".repeat(MAX_TITLE_CHARS);
        s.sections[0].instruction = "ö".repeat(MAX_INSTRUCTION_CHARS);
        s.context = "ü".repeat(MAX_CONTEXT_CHARS);
        validate_spec(&s).unwrap();
        // Genau 10 Abschnitte ist die Obergrenze und erlaubt.
        s.sections = (0..MAX_SECTIONS)
            .map(|i| text(&format!("s{i}"), "T", ""))
            .collect();
        validate_spec(&s).unwrap();
    }

    #[test]
    fn a_template_without_tasks_section_is_valid() {
        let s = spec("", vec![text("a", "A", "")]);
        validate_spec(&s).unwrap();
    }

    #[test]
    fn titles_are_validated_after_trimming() {
        validate_title("  Kundentermin  ").unwrap();
        assert!(validate_title("   ").is_err());
        assert!(validate_title("").is_err());
        assert!(validate_title(&"x".repeat(MAX_TITLE_CHARS + 1)).is_err());
        validate_title(&"x".repeat(MAX_TITLE_CHARS)).unwrap();
    }

    #[test]
    fn export_then_import_round_trips_every_builtin() {
        for (key, title, spec) in builtin_templates() {
            let info = TemplateInfo {
                id: builtin_id(key),
                title: title.to_string(),
                builtin: true,
                spec: spec.clone(),
                updated_at: 1,
            };
            let file = export_file(&info);
            assert!(file.len() < MAX_FILE_BYTES, "{key}: Datei passt ins Limit");
            let (t, s) = import_file(&file).unwrap_or_else(|e| panic!("{key}: {e}"));
            assert_eq!(t, title);
            assert_eq!(
                s, spec,
                "{key}: Rundreise verlustfrei (Umlaute, Sonderzeichen)"
            );
        }
    }

    #[test]
    fn the_export_file_has_the_documented_shape() {
        let file = export_file(&info_of(valid_spec()));
        let v: serde_json::Value = serde_json::from_str(&file).unwrap();
        assert_eq!(v["format"], FILE_FORMAT);
        assert_eq!(v["title"], "Meine Vorlage");
        assert_eq!(v["spec"]["version"], 1);
        assert_eq!(v["spec"]["sections"][1]["kind"], "tasks");
        assert!(v.get("id").is_none(), "die ID gehoert nicht in die Datei");
    }

    #[test]
    fn import_tolerates_a_bom_and_trims_the_title() {
        let mut v: serde_json::Value =
            serde_json::from_str(&export_file(&info_of(valid_spec()))).unwrap();
        v["title"] = "  Mit Rand  ".into();
        let file = format!("\u{feff}{}", serde_json::to_string(&v).unwrap());
        let (title, _) = import_file(&file).unwrap();
        assert_eq!(title, "Mit Rand");
    }

    #[test]
    fn import_rejects_bad_files_without_panicking() {
        let good = export_file(&info_of(valid_spec()));

        assert_eq!(
            import_file(&" ".repeat(MAX_FILE_BYTES + 1)).unwrap_err(),
            "template_import:too_large"
        );
        assert_eq!(
            import_file("das ist kein json").unwrap_err(),
            "template_import:invalid_json"
        );
        assert_eq!(import_file("").unwrap_err(), "template_import:invalid_json");
        assert_eq!(
            import_file("{\"format\":\"x\"}").unwrap_err(),
            "template_import:invalid_json",
            "fehlende Felder"
        );
        assert_eq!(
            import_file(&good.replace(FILE_FORMAT, "lva-meeting-template@2")).unwrap_err(),
            "template_import:format"
        );
        assert_eq!(
            import_file(&good.replace("Meine Vorlage", "   ")).unwrap_err(),
            "template_invalid:title"
        );
        // Gueltiges JSON, ungueltige Vorlage: die Pruefung greift auch beim Import.
        assert_eq!(
            import_file(&good.replace("\"version\": 1", "\"version\": 7")).unwrap_err(),
            "template_invalid:version"
        );
    }

    #[test]
    fn import_of_a_file_exactly_at_the_limit_is_measured_in_bytes() {
        // 64 KB Leerraum nach einem gueltigen Dokument: noch erlaubt.
        let good = export_file(&info_of(valid_spec()));
        let padded = format!("{good}{}", " ".repeat(MAX_FILE_BYTES - good.len()));
        assert_eq!(padded.len(), MAX_FILE_BYTES);
        import_file(&padded).unwrap();
        assert_eq!(
            import_file(&format!("{padded} ")).unwrap_err(),
            "template_import:too_large"
        );
    }
}
