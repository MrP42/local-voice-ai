//! Dokumenttypen neben dem Protokoll (Entwurf 2026-10-13, Etappe 1).
//!
//! Aus einem Transkript entstehen neben dem Protokoll weitere Dokumente fuer andere
//! Leser: Management Summary, Kurzfassung, Statusbericht ... Sie laufen durch
//! DENSELBEN Motor wie das Protokoll (`minutes.rs`: Einzeldurchlauf, wenn alles in den
//! Kontext passt -- bei Cloud-/Abo-Modellen also eine Anfrage --, sonst Bloecke mit
//! Halbieren; Laufsperre, Fortschritt, Stopp, Grundlage und Ausgabesprache). Ein
//! Dokumenttyp aendert nur drei Dinge, wie `ProjectKind::Summary` beim
//! Projekt-Protokoll: die Abschnitte (eigene `TemplateSpec`), Zusatzregeln im
//! System-Prompt (Zielgruppe, Laenge, Tiefe) und eine Eintragsgrenze.
//!
//! Gespeichert wird in `meeting_documents` mit `kind = "doc:<key>"` und
//! `template_id = "doctype:<key>"`; Versionen zaehlen je Typ. Keine Migration.
//!
//! Der Katalog steht im Code wie `notes::templates::builtin_templates`: Schluessel
//! (`key`) duerfen nie umbenannt werden, gespeicherte Dokumente verweisen darauf.
//!
//! Owner-Annahmen (Entwurf Abschnitt 10), hier als Konstanten, damit sie an EINER
//! Stelle aenderbar sind: der Typ `followup_mail` erzeugt nichts selbst, sondern
//! oeffnet den vorhandenen Follow-up-Dialog (`DocEngine::Followup`, Annahme 2).
//!
//! Nichts hier ruft `settings::get_settings` (Test-EXE, siehe `managers/usage.rs`).

use serde::{Deserialize, Serialize};
use specta::Type;

use super::minutes::{MinutesEntry, MinutesSection};
use super::notes::model::{SectionKind, TemplateSection, TemplateSpec};
use super::notes::templates::SPEC_VERSION;
use crate::managers::usage::Purpose;

/// `kind` des Protokolls (unveraendert seit M8).
pub const MINUTES_KIND: &str = "minutes";
/// Praefix der `kind`-Werte der Dokumenttypen.
pub const DOC_KIND_PREFIX: &str = "doc:";
/// Praefix der `template_id` eines erzeugten Dokuments.
pub const TEMPLATE_PREFIX: &str = "doctype:";
/// Schluessel des Protokolls im Katalog (Engine `Minutes`).
pub const PROTOCOL_KEY: &str = "protokoll";
/// Schluessel des Follow-up-Typs (Engine `Followup`, Annahme 2).
pub const FOLLOWUP_KEY: &str = "followup_mail";
/// Provenienz-Operation eines Dokumentlaufs (`[a-z0-9_]`).
pub const OPERATION: &str = "document";
/// Ab diesem Vielfachen des Richtwerts gilt ein Dokument als zu lang (Hinweis, nie Kuerzung).
pub const OVER_LENGTH_FACTOR: f64 = 1.25;

/// Wer ein Dokument erzeugt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum DocEngine {
    /// Das vorhandene Protokoll (Vorlage der Besprechung, `kind = minutes`).
    Minutes,
    /// Ein Dokumenttyp dieses Katalogs, erzeugt im Protokoll-Motor.
    Document,
    /// Der vorhandene Follow-up-Dialog (`followup.rs`, `mail.rs`); kein eigener Lauf.
    Followup,
}

/// Ein Dokumenttyp. Alle Texte sind fest im Code; `spec` haelt die Abschnitte.
#[derive(Clone, Debug)]
pub struct DocType {
    pub key: &'static str,
    pub title: &'static str,
    pub audience: &'static str,
    pub length: &'static str,
    pub depth: &'static str,
    pub engine: DocEngine,
    pub spec: TemplateSpec,
    /// Zusatzregeln im System-Prompt (Englisch wie die uebrigen Regeln).
    pub rules: &'static str,
    /// Hoechstzahl der Eintraege im ganzen Dokument (alle Abschnitte zusammen).
    pub entry_cap: Option<usize>,
    /// Richtwert der Laenge in Woertern; Ueberschreitung wird vermerkt.
    pub max_words: Option<u32>,
    /// Feste Schlusszeile im Dokument (z. B. Verweis aufs Protokoll).
    pub footer: Option<&'static str>,
}

impl DocType {
    /// `kind` in `meeting_documents`.
    pub fn kind(&self) -> String {
        doc_kind(self.key)
    }

    /// `template_id` des gespeicherten Dokuments.
    pub fn template_id(&self) -> String {
        format!("{TEMPLATE_PREFIX}{}", self.key)
    }
}

/// Was die Oberflaeche ueber einen Typ wissen muss (ohne Prompt-Texte).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DocTypeInfo {
    pub key: String,
    pub title: String,
    pub audience: String,
    pub length: String,
    pub depth: String,
    pub engine: DocEngine,
    /// `kind` der Dokumente dieses Typs (`minutes` oder `doc:<key>`); leer bei `Followup`.
    pub kind: String,
    pub sections: Vec<String>,
}

impl From<&DocType> for DocTypeInfo {
    fn from(t: &DocType) -> Self {
        Self {
            key: t.key.to_string(),
            title: t.title.to_string(),
            audience: t.audience.to_string(),
            length: t.length.to_string(),
            depth: t.depth.to_string(),
            engine: t.engine,
            kind: match t.engine {
                DocEngine::Minutes => MINUTES_KIND.to_string(),
                DocEngine::Document => t.kind(),
                DocEngine::Followup => String::new(),
            },
            sections: t.spec.sections.iter().map(|s| s.title.clone()).collect(),
        }
    }
}

/// `kind` eines Schluessels: `minutes` fuer das Protokoll, sonst `doc:<key>`.
pub fn doc_kind(key: &str) -> String {
    if key == PROTOCOL_KEY {
        MINUTES_KIND.to_string()
    } else {
        format!("{DOC_KIND_PREFIX}{key}")
    }
}

/// Schluessel aus einem `kind` (`doc:<key>` oder `minutes`).
pub fn key_of_kind(kind: &str) -> Option<&str> {
    if kind == MINUTES_KIND {
        Some(PROTOCOL_KEY)
    } else {
        kind.strip_prefix(DOC_KIND_PREFIX)
    }
}

/// Der Typ zu einem Schluessel.
pub fn by_key(key: &str) -> Option<DocType> {
    catalog().into_iter().find(|t| t.key == key)
}

/// Fuer die Oberflaeche.
pub fn infos() -> Vec<DocTypeInfo> {
    catalog().iter().map(DocTypeInfo::from).collect()
}

/// Profil eines Laufs im Protokoll-Motor: `None` ist das Protokoll selbst.
#[derive(Clone, Debug, Default)]
pub struct DocProfile {
    pub doc_type: Option<DocType>,
}

impl DocProfile {
    pub fn minutes() -> Self {
        Self { doc_type: None }
    }

    pub fn document(doc_type: DocType) -> Self {
        Self {
            doc_type: Some(doc_type),
        }
    }

    pub fn is_document(&self) -> bool {
        self.doc_type.is_some()
    }

    pub fn kind(&self) -> String {
        self.doc_type
            .as_ref()
            .map(DocType::kind)
            .unwrap_or_else(|| MINUTES_KIND.to_string())
    }

    pub fn purpose(&self) -> Purpose {
        if self.is_document() {
            Purpose::Document
        } else {
            Purpose::Minutes
        }
    }

    /// Wort fuer Fehlertexte und Logs.
    pub fn noun(&self) -> &'static str {
        self.doc_type
            .as_ref()
            .map(|t| t.title)
            .unwrap_or("Protokoll")
    }

    pub fn operation(&self) -> &'static str {
        if self.is_document() {
            OPERATION
        } else {
            "minutes"
        }
    }

    /// Zusatzregeln fuer den System-Prompt (leer beim Protokoll).
    pub fn rules(&self) -> &'static str {
        self.doc_type.as_ref().map(|t| t.rules).unwrap_or("")
    }

    /// Eintragsgrenze: die des Typs, hoechstens `budget_cap` (die Blockgrenze des Motors).
    pub fn cap(&self, budget_cap: Option<usize>) -> Option<usize> {
        match (self.doc_type.as_ref().and_then(|t| t.entry_cap), budget_cap) {
            (Some(own), Some(budget)) => Some(own.min(budget)),
            (Some(own), None) => Some(own),
            (None, budget) => budget,
        }
    }
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

/// Woerter im Inhalt der Abschnitte (ohne Kopf und Titel).
pub fn word_count(sections: &[MinutesSection]) -> u32 {
    sections
        .iter()
        .flat_map(|s| s.entries.iter())
        .map(|e| e.text.split_whitespace().count() as u32)
        .sum()
}

/// Zu lang? `None`, wenn der Typ keinen Richtwert hat.
pub fn over_length(doc_type: &DocType, words: u32) -> bool {
    doc_type
        .max_words
        .is_some_and(|max| f64::from(words) > f64::from(max) * OVER_LENGTH_FACTOR)
}

/// Ein Eintrag als eine Zeile: Zeilenumbrueche aus der Antwort zerreissen sonst die
/// Markdown-Liste (beobachtet mit einem 4B-Modell, 13.10.2026).
pub(crate) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// "Wer"/"Bis" nur, wenn wirklich etwas genannt ist: kleine Modelle schreiben den
/// Platzhalter als Text (`"null"`, `"-"`, `"keine"`) statt JSON-`null`.
fn named(value: Option<&String>) -> Option<String> {
    let value = one_line(value?);
    let lower = value.to_lowercase();
    let placeholder = matches!(
        lower.as_str(),
        "" | "null"
            | "none"
            | "-"
            | "–"
            | "n/a"
            | "na"
            | "keine"
            | "keiner"
            | "offen"
            | "unbekannt"
    );
    (!placeholder).then_some(value)
}

pub(crate) fn entry_line(kind: SectionKind, entry: &MinutesEntry) -> String {
    let mut line = one_line(&entry.text);
    if kind == SectionKind::Tasks {
        let mut extras = Vec::new();
        if let Some(assignee) = named(entry.assignee.as_ref()) {
            extras.push(format!("Wer: {assignee}"));
        }
        if let Some(due) = named(entry.due.as_ref()) {
            extras.push(format!("Bis: {due}"));
        }
        if !extras.is_empty() {
            line.push_str(&format!(" _({})_", extras.join(", ")));
        }
    }
    line
}

/// Kopfdaten eines Dokuments, die nicht aus dem Modell kommen.
pub struct DocHead<'a> {
    pub meeting_title: &'a str,
    pub date_iso: &'a str,
    pub duration: &'a str,
    pub basis_line: Option<&'a str>,
}

/// Das Dokument als Markdown: Titel, Kopfzeile, Abschnitte, feste Schlusszeile,
/// Hinweise (Ueberlaenge, Luecken). Leere Abschnitte fallen weg (anders als im
/// Protokoll: ein Management Summary mit "_keine_" unter "Risiken" liest sich wie ein
/// Befund); ist ALLES leer, steht ein Hinweis da.
pub fn document_to_markdown(
    doc_type: &DocType,
    head: &DocHead<'_>,
    sections: &[MinutesSection],
    gaps: &[String],
) -> String {
    let mut md = format!("# {}: {}\n\n", doc_type.title, head.meeting_title);
    md.push_str(&format!(
        "**Datum:** {} · **Dauer:** {} · **Für:** {}\n",
        head.date_iso, head.duration, doc_type.audience
    ));
    if let Some(line) = head.basis_line {
        // Eigener Absatz: ein einfacher Zeilenumbruch klebt sie an die Kopfzeile.
        md.push_str(&format!("\n{line}\n"));
    }
    let mut any = false;
    for section in sections.iter().filter(|s| !s.entries.is_empty()) {
        any = true;
        md.push_str(&format!("\n## {}\n\n", section.title));
        match section.entries.as_slice() {
            [only] if section.kind == SectionKind::Text => {
                md.push_str(&format!("{}\n", one_line(&only.text)))
            }
            entries => {
                for entry in entries {
                    md.push_str(&format!("- {}\n", entry_line(section.kind, entry)));
                }
            }
        }
    }
    if !any {
        md.push_str("\n_Das Transkript enthält nichts, was in dieses Dokument gehört._\n");
    }
    if let Some(footer) = doc_type.footer {
        md.push_str(&format!("\n_{footer}_\n"));
    }
    let words = word_count(sections);
    if over_length(doc_type, words) {
        if let Some(max) = doc_type.max_words {
            md.push_str(&format!(
                "\n> **Hinweis:** Länger als vorgesehen ({words} Wörter, Richtwert {max}).\n"
            ));
        }
    }
    if !gaps.is_empty() {
        md.push_str(&format!(
            "\n> **Hinweis:** Unvollständig. Die Besprechung bei {} konnte nicht ausgewertet \
             werden und fehlt hier; das Transkript bleibt erhalten.\n",
            gaps.join(", ")
        ));
    }
    md
}

// ---------------------------------------------------------------------------
// Katalog
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

const TASK_RULE: &str = "Wer erledigt was bis wann. Verantwortliche und Termine nur eintragen, wenn sie im Gespräch genannt wurden.";

#[allow(clippy::too_many_arguments)]
fn doc(
    key: &'static str,
    title: &'static str,
    audience: &'static str,
    length: &'static str,
    depth: &'static str,
    rules: &'static str,
    entry_cap: Option<usize>,
    max_words: Option<u32>,
    spec: TemplateSpec,
) -> DocType {
    DocType {
        key,
        title,
        audience,
        length,
        depth,
        engine: DocEngine::Document,
        spec,
        rules,
        entry_cap,
        max_words,
        footer: None,
    }
}

/// Die zehn Dokumenttypen in Anzeigereihenfolge (Entwurf Abschnitt 1).
pub fn catalog() -> Vec<DocType> {
    let empty = || spec("", vec![text("inhalt", "Inhalt", "")]);
    vec![
        DocType {
            key: PROTOCOL_KEY,
            title: "Protokoll",
            audience: "Teilnehmende, Ablage",
            length: "2–6 Seiten",
            depth: "hoch",
            engine: DocEngine::Minutes,
            spec: empty(),
            rules: "",
            entry_cap: None,
            max_words: None,
            footer: None,
        },
        doc(
            "management_summary",
            "Management Summary",
            "Entscheider, nicht anwesend",
            "höchstens 1 Seite",
            "gering",
            "- This is a MANAGEMENT SUMMARY for decision makers who did not attend, not minutes: \
report outcomes, decisions, risks and the recommended next steps, never the course of the \
discussion. No quotes, no detail that does not change a decision.\n\
- Keep the whole document under about 300 words: one or two sentences per text section, \
at most 5 entries per list.\n",
            Some(18),
            Some(300),
            spec(
                "Kurzfassung für Entscheider, die nicht dabei waren: Ergebnis und Wirkung statt Verlauf.",
                vec![
                    text("anlass_ziel", "Anlass und Ziel", "In höchstens zwei Sätzen: warum die Besprechung stattfand und was sie klären sollte."),
                    text("kernergebnisse", "Kernergebnisse", "Drei bis fünf zentrale Ergebnisse, je ein Satz, das Wichtigste zuerst."),
                    text("entscheidungen", "Entscheidungen", "Nur tatsächlich Beschlossenes, je ein Satz. Vorschläge und offene Diskussionen gehören nicht hierher."),
                    text("risiken_handlungsbedarf", "Risiken und Handlungsbedarf", "Genannte Risiken, Probleme und Punkte, bei denen Entscheider handeln oder entscheiden müssen."),
                    tasks("naechste_schritte", "Empfehlung und nächste Schritte", TASK_RULE),
                ],
            ),
        ),
        DocType {
            footer: Some("Details stehen im Protokoll."),
            ..doc(
                "kurzfassung",
                "Zusammenfassung",
                "alle Beteiligten",
                "höchstens ½ Seite",
                "gering",
                "- This is a SHORT SUMMARY for a quick overview, not minutes: only the essential points, \
little depth; readers who want more read the minutes.\n\
- Keep the whole document under about 180 words: at most 6 entries per list, at most 3 next \
steps.\n",
                Some(12),
                Some(180),
                spec(
                    "Schneller Überblick über eine Besprechung: nur das Wesentliche.",
                    vec![
                        text("worum", "Worum ging es", "In ein bis zwei Sätzen: Thema und Anlass."),
                        text("wichtigste_punkte", "Wichtigste Punkte", "Drei bis sechs Stichpunkte mit den wesentlichen Aussagen, ohne Details."),
                        text("ergebnis", "Ergebnis", "Ein Satz: was am Ende feststand."),
                        tasks("naechste_schritte", "Nächste Schritte", "Höchstens drei vereinbarte nächste Schritte. Verantwortliche und Termine nur, wenn genannt."),
                    ],
                ),
            )
        },
        doc(
            "entscheidungen_aufgaben",
            "Entscheidungen und Aufgaben",
            "Umsetzende, Projektsteuerung",
            "nur Listen",
            "mittel",
            "- This is a LIST of decisions, action items and open points only: no prose, no \
summary, no discussion. One sentence per entry.\n",
            None,
            None,
            spec(
                "Arbeitsliste nach einer Besprechung: was beschlossen ist, wer was tut, was offen ist.",
                vec![
                    text("entscheidungen", "Entscheidungen", "Jede tatsächlich getroffene Entscheidung als ein Satz. Keine Vorschläge."),
                    tasks("aufgaben", "Aufgaben", TASK_RULE),
                    text("offene_punkte", "Offene Punkte", "Fragen und Punkte, die ungeklärt blieben, je ein Satz."),
                ],
            ),
        ),
        DocType {
            key: FOLLOWUP_KEY,
            title: "Follow-up-Mail",
            audience: "Teilnehmende, Kunde",
            length: "höchstens 250 Wörter",
            depth: "gering",
            engine: DocEngine::Followup,
            spec: empty(),
            rules: "",
            entry_cap: None,
            max_words: Some(250),
            footer: None,
        },
        doc(
            "statusbericht",
            "Statusbericht",
            "Auftraggeber, Lenkungskreis",
            "1–2 Seiten",
            "mittel",
            "- This is a STATUS REPORT for a sponsor or steering committee, not minutes: state \
where things stand, what changed and what needs a decision. Never invent a traffic-light \
rating or percentages; an overall assessment must be marked as an assessment and rest on \
statements in the transcript.\n\
- Keep it under about 600 words, at most 6 entries per list.\n",
            Some(30),
            Some(600),
            spec(
                "Statusbericht zu einem Vorhaben auf Grundlage der Besprechung.",
                vec![
                    text("stand", "Stand insgesamt", "Kurze Einschätzung des Gesamtstands, als Einschätzung gekennzeichnet, nur aus Aussagen im Gespräch. Keine Ampel und keine Prozentwerte erfinden."),
                    text("erreicht", "Erreicht", "Was seit dem letzten Stand erreicht oder abgeschlossen wurde."),
                    text("meilensteine", "Nächste Meilensteine und Termine", "Genannte Meilensteine, Fristen und Termine mit Datum, in zeitlicher Reihenfolge."),
                    text("risiken_probleme", "Risiken und Probleme", "Genannte Risiken, Probleme und Verzögerungen mit Ursache, soweit genannt."),
                    text("entscheidungsbedarf", "Entscheidungsbedarf", "Punkte, zu denen eine Entscheidung des Auftraggebers oder Lenkungskreises gebraucht wird."),
                    tasks("aufgaben", "Aufgaben", TASK_RULE),
                ],
            ),
        ),
        doc(
            "briefing_abwesende",
            "Briefing für Abwesende",
            "Kolleginnen und Kollegen, die übernehmen",
            "höchstens 1 Seite",
            "mittel",
            "- This is a BRIEFING for a colleague who missed the meeting and has to carry on: tell \
them what they need to know and what is expected of them, addressed directly (\"you\"). No \
course of the discussion.\n\
- Keep it under about 350 words, at most 8 entries per list.\n",
            Some(24),
            Some(350),
            spec(
                "Übergabe an jemanden, der nicht dabei war und weiterarbeiten muss.",
                vec![
                    text("kontext", "Kontext", "Anlass der Besprechung und wer gesprochen hat, nur mit Namen, die im Gespräch genannt wurden."),
                    text("wissen", "Was du wissen musst", "Fünf bis acht Punkte, die man zum Weiterarbeiten kennen muss."),
                    text("aenderungen", "Was sich geändert hat", "Was durch die Besprechung anders ist als vorher: Entscheidungen, neue Fakten, geänderte Pläne."),
                    tasks("erwartet", "Was von dir erwartet wird", TASK_RULE),
                    text("offene_fragen", "Offene Fragen", "Fragen, die noch geklärt werden müssen."),
                ],
            ),
        ),
        doc(
            "faq",
            "Fragen und Antworten",
            "Stakeholder, Wissensablage",
            "nach Bedarf",
            "mittel",
            "- This is a FAQ: every entry is one question that was actually asked or clearly \
discussed in the meeting, followed by the answer given in the meeting, written as \
\"Frage? – Antwort.\" Never answer from general knowledge; a question without an answer in \
the transcript goes to the open questions.\n",
            None,
            None,
            spec(
                "Fragen und Antworten aus einer Besprechung zum Nachschlagen.",
                vec![
                    text("fragen_antworten", "Fragen und Antworten", "Je gestellter Frage ein Eintrag: die Frage kurz, dann die Antwort aus dem Gespräch."),
                    text("offene_fragen", "Offen gebliebene Fragen", "Fragen, auf die es im Gespräch keine Antwort gab."),
                ],
            ),
        ),
        doc(
            "lernzusammenfassung",
            "Lernzusammenfassung",
            "Teilnehmende einer Schulung oder eines Vortrags",
            "1–2 Seiten",
            "mittel",
            "- This is a LEARNING SUMMARY of a talk, training or lecture: explain the content so a \
participant can review it later. Explanations come only from what the speakers said, never \
from general knowledge. Name only sources that were mentioned.\n\
- Keep it under about 700 words.\n",
            Some(35),
            Some(700),
            spec(
                "Lernunterlage aus einer Schulung, einem Seminar oder Vortrag.",
                vec![
                    text("thema_lernziele", "Thema und Lernziele", "Thema und die genannten Lernziele oder Fragestellungen."),
                    text("kernbegriffe", "Kernbegriffe", "Je Begriff ein Eintrag: Begriff, dann die Erklärung, wie sie im Vortrag gegeben wurde."),
                    text("aussagen", "Wichtigste Aussagen", "Die zentralen inhaltlichen Aussagen in sinnvoller Reihenfolge."),
                    text("beispiele", "Beispiele", "Beispiele und Fälle, die im Vortrag genannt wurden."),
                    text("merksaetze", "Merksätze", "Drei bis fünf Take-aways in einem Satz."),
                    text("quellen", "Genannte Quellen", "Bücher, Normen, Links oder Werkzeuge, die ausdrücklich genannt wurden."),
                ],
            ),
        ),
        doc(
            "vortragsgrundlage",
            "Vortragsgrundlage",
            "Vortragende, WAI Presenter",
            "1–2 Seiten",
            "mittel",
            "- This is the BRIEF for a talk to be built from the meeting (input for a presentation \
studio): capture topic, audience, goal, constraints, content building blocks, facts and open \
decisions as they were said. Audience, duration, date and place only when named; never invent \
them. Every number or fact carries its own entry so it can be checked.\n\
- Keep it under about 700 words.\n",
            Some(35),
            Some(700),
            spec(
                "Grundlage für einen Fachvortrag, der aus der Besprechung entstehen soll.",
                vec![
                    text("thema_botschaft", "Thema und Kernbotschaft", "Thema des Vortrags und die Kernbotschaft in einem Satz, soweit erkennbar."),
                    text("zielgruppe", "Zielgruppe", "Für wen der Vortrag ist: Rolle, Vorwissen, Größe, nur wie im Gespräch genannt."),
                    text("ziel", "Ziel", "Was das Publikum danach wissen, glauben oder tun soll."),
                    text("rahmen", "Rahmen", "Dauer, Termin, Ort, Format und Technik, nur wenn genannt."),
                    text("bausteine", "Inhaltliche Bausteine", "Themen und Inhalte, die vorkommen sollen, in einer sinnvollen Reihenfolge als Gliederungsvorschlag."),
                    text("fakten", "Fakten und Zahlen", "Je genannte Zahl, Norm, Frist oder Tatsache ein Eintrag, wörtlich genau, damit sie geprüft werden kann."),
                    text("offene_punkte", "Offene Punkte", "Entscheidungen und Fragen, die vor dem Vortrag geklärt werden müssen."),
                    tasks("organisatorisches", "Organisatorisches", TASK_RULE),
                ],
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::notes::templates::validate_spec;

    #[test]
    fn the_catalog_has_ten_unique_valid_types_with_protocol_first() {
        let all = catalog();
        assert_eq!(all.len(), 10);
        assert_eq!(all[0].key, PROTOCOL_KEY);
        let mut keys: Vec<&str> = all.iter().map(|t| t.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 10, "Schluessel doppelt");
        for t in &all {
            assert!(
                t.key.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{}",
                t.key
            );
            validate_spec(&t.spec).unwrap_or_else(|e| panic!("{}: {e}", t.key));
            if t.engine == DocEngine::Document {
                assert!(!t.rules.is_empty(), "{} ohne Regeln", t.key);
                assert!(t.spec.sections.len() >= 2, "{}", t.key);
            }
        }
        let engines: Vec<DocEngine> = all.iter().map(|t| t.engine).collect();
        assert_eq!(
            engines.iter().filter(|e| **e == DocEngine::Minutes).count(),
            1
        );
        assert_eq!(
            engines
                .iter()
                .filter(|e| **e == DocEngine::Followup)
                .count(),
            1
        );
        assert!(by_key("management_summary").is_some());
        assert!(by_key("kurzfassung").is_some());
    }

    #[test]
    fn kinds_round_trip_and_the_protocol_keeps_its_kind() {
        assert_eq!(doc_kind(PROTOCOL_KEY), "minutes");
        assert_eq!(doc_kind("faq"), "doc:faq");
        assert_eq!(key_of_kind("doc:faq"), Some("faq"));
        assert_eq!(key_of_kind("minutes"), Some(PROTOCOL_KEY));
        assert_eq!(key_of_kind("enhanced_notes"), None);
        let info = infos();
        assert_eq!(info[0].kind, "minutes");
        assert_eq!(info[1].kind, "doc:management_summary");
        assert_eq!(
            info.iter().find(|i| i.key == FOLLOWUP_KEY).unwrap().kind,
            ""
        );
    }

    #[test]
    fn the_profile_caps_entries_with_the_tighter_limit() {
        let p = DocProfile::document(by_key("management_summary").unwrap());
        assert_eq!(p.cap(None), Some(18));
        assert_eq!(p.cap(Some(10)), Some(10));
        assert_eq!(p.cap(Some(40)), Some(18));
        assert_eq!(p.purpose(), Purpose::Document);
        assert_eq!(p.kind(), "doc:management_summary");
        let m = DocProfile::minutes();
        assert_eq!(m.cap(None), None);
        assert_eq!(m.cap(Some(7)), Some(7));
        assert_eq!(m.kind(), "minutes");
        assert_eq!(m.rules(), "");
        assert_eq!(m.operation(), "minutes");
    }

    fn section(id: &str, kind: SectionKind, texts: &[&str]) -> MinutesSection {
        MinutesSection {
            id: id.into(),
            title: id.to_uppercase(),
            kind,
            entries: texts
                .iter()
                .map(|t| MinutesEntry {
                    text: t.to_string(),
                    assignee: (kind == SectionKind::Tasks).then(|| "Anna".to_string()),
                    due: None,
                })
                .collect(),
        }
    }

    #[test]
    fn markdown_has_title_head_drops_empty_sections_and_marks_over_length() {
        let t = by_key("kurzfassung").unwrap();
        let head = DocHead {
            meeting_title: "Jour fixe",
            date_iso: "2026-10-12",
            duration: "30 min",
            basis_line: None,
        };
        let sections = vec![
            section("worum", SectionKind::Text, &["Go-Live-Planung."]),
            section("wichtigste_punkte", SectionKind::Text, &[]),
            section(
                "naechste_schritte",
                SectionKind::Tasks,
                &["Release-Notes schreiben"],
            ),
        ];
        let md = document_to_markdown(&t, &head, &sections, &[]);
        assert!(md.starts_with("# Zusammenfassung: Jour fixe\n"));
        assert!(md.contains("**Für:** alle Beteiligten"));
        assert!(md.contains("## WORUM\n\nGo-Live-Planung.\n"));
        assert!(!md.contains("WICHTIGSTE_PUNKTE"));
        assert!(md.contains("- Release-Notes schreiben _(Wer: Anna)_"));
        assert!(md.contains("_Details stehen im Protokoll._"));
        assert!(!md.contains("Länger als vorgesehen"));

        let long = "wort ".repeat(240);
        let md = document_to_markdown(
            &t,
            &head,
            &[section("worum", SectionKind::Text, &[long.trim()])],
            &["10:00-20:00".into()],
        );
        assert!(md.contains("Länger als vorgesehen (240 Wörter, Richtwert 180)"));
        assert!(md.contains("10:00-20:00"));

        let md = document_to_markdown(&t, &head, &[section("worum", SectionKind::Text, &[])], &[]);
        assert!(md.contains("enthält nichts"));
    }

    /// Beobachtet mit `gemma4-turbo:e4b` am 13.10.2026: "Wer: null" und Eintraege mit
    /// Zeilenumbruechen, die die Liste zerreissen.
    #[test]
    fn placeholder_names_disappear_and_entries_stay_on_one_line() {
        let t = by_key("management_summary").unwrap();
        let head = DocHead {
            meeting_title: "M",
            date_iso: "2026-10-13",
            duration: "5 min",
            basis_line: None,
        };
        let task = |assignee: &str, due: &str| MinutesEntry {
            text: "ADR festlegen\nund prüfen".into(),
            assignee: Some(assignee.into()),
            due: Some(due.into()),
        };
        let sections = vec![MinutesSection {
            id: "naechste_schritte".into(),
            title: "Nächste Schritte".into(),
            kind: SectionKind::Tasks,
            entries: vec![task("null", " - "), task("Anna", "Woche 6")],
        }];
        let md = document_to_markdown(&t, &head, &sections, &[]);
        assert!(md.contains("- ADR festlegen und prüfen\n"), "{md}");
        assert!(md.contains("- ADR festlegen und prüfen _(Wer: Anna, Bis: Woche 6)_\n"));
        assert!(!md.contains("null"));
    }
}
