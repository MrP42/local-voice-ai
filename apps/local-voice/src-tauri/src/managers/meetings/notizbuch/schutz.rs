//! Notizbuch-Text geht nur an lokale Modelle (Spec §3.2; lva#430, Review MUSS 1).
//!
//! Gespeicherte KI-Notizen enthalten Notizbuch-Blöcke als Nutzereinträge (`note_id` beginnt mit
//! `nb:`) und Hinweise „Notizen unvollständig: <Seitentitel>“. Beides ist Notizbuch-Inhalt. Jeder
//! Pfad, der gespeicherte KI-Notizen (oder daraus abgeleitete Aufgaben) an ein Modell gibt, holt
//! sie über [`fuer_modell`]: bei einem nicht lokalen Anbieter fehlen diese Einträge dort.
//! Lokal erkannt wird wie bei den KI-Notizen selbst (`crate::managers::llm::is_local`). Ist der
//! Anbieter unbekannt (Suchindex, Assistent, MCP-Client, Workflow), gilt er als nicht lokal
//! ([`ANBIETER_UNBEKANNT`]).
//!
//! Anzeige und Export lesen das gespeicherte Dokument unverändert; hier wird nur eine Kopie für
//! das Modell gefiltert. Eintrags-IDs bleiben, wie sie sind (Belege `E<k>` gelten weiter).

use std::borrow::Cow;
use std::collections::HashSet;

use super::quelle::{ist_notizbuch_id, UNVOLLSTAENDIG_PRAEFIX, WEITERE_ID, WEITERE_TITEL};
use crate::managers::meetings::notes::model::{
    ActionItem, EnhancedEntry, EnhancedNotes, EnhancedSection, Origin, SectionKind,
};

/// Für Empfänger, deren Modell nicht bekannt ist: wie ein entfernter Anbieter behandeln.
pub const ANBIETER_UNBEKANNT: bool = false;

/// Hinweis „Notizen unvollständig: <Seitentitel>“ aus der Verlustprüfung (trägt einen Titel aus
/// dem Notizbuch).
pub fn ist_hinweis(e: &EnhancedEntry) -> bool {
    e.origin == Origin::Ai && e.note_id.is_none() && e.text.starts_with(UNVOLLSTAENDIG_PRAEFIX)
}

/// Eintrag mit Notizbuch-Inhalt (Block `nb:` oder Hinweis auf eine Seite).
pub fn ist_notizbuch_eintrag(e: &EnhancedEntry) -> bool {
    e.note_id.as_deref().is_some_and(ist_notizbuch_id) || ist_hinweis(e)
}

fn hat_notizbuch(notes: &EnhancedNotes) -> bool {
    notes
        .sections
        .iter()
        .flat_map(|s| s.entries.iter())
        .any(ist_notizbuch_eintrag)
}

fn ohne_notizbuch(notes: &mut EnhancedNotes) {
    for s in notes.sections.iter_mut() {
        s.entries.retain(|e| !ist_notizbuch_eintrag(e));
    }
}

/// Die KI-Notizen, wie ein Modell sie sehen darf: lokal unverändert, sonst ohne
/// Notizbuch-Einträge. Ohne Notizbuch-Einträge wird nichts kopiert.
pub fn fuer_modell(notes: &EnhancedNotes, lokal: bool) -> Cow<'_, EnhancedNotes> {
    if lokal || !hat_notizbuch(notes) {
        return Cow::Borrowed(notes);
    }
    let mut kopie = notes.clone();
    ohne_notizbuch(&mut kopie);
    Cow::Owned(kopie)
}

/// Wie [`fuer_modell`] für einen Besitzwert.
pub fn fuer_modell_besitz(mut notes: EnhancedNotes, lokal: bool) -> EnhancedNotes {
    if !lokal {
        ohne_notizbuch(&mut notes);
    }
    notes
}

/// IDs (`E<k>`) der Notizbuch-Einträge, z. B. um abgeleitete Aufgaben herauszufiltern.
pub fn notizbuch_eintrag_ids(notes: &EnhancedNotes) -> HashSet<String> {
    notes
        .sections
        .iter()
        .flat_map(|s| s.entries.iter())
        .filter(|e| ist_notizbuch_eintrag(e))
        .map(|e| e.id.clone())
        .collect()
}

/// Aufgaben, wie ein Modell sie sehen darf: lokal alle, sonst ohne die aus Notizbuch-Einträgen
/// der KI-Notizen `notes` abgeleiteten.
pub fn aufgaben_fuer_modell(
    items: Vec<ActionItem>,
    notes: Option<&EnhancedNotes>,
    lokal: bool,
) -> Vec<ActionItem> {
    let Some(notes) = notes.filter(|_| !lokal) else {
        return items;
    };
    let nb = notizbuch_eintrag_ids(notes);
    if nb.is_empty() {
        return items;
    }
    items
        .into_iter()
        .filter(|i| !i.entry_id.as_deref().is_some_and(|id| nb.contains(id)))
        .collect()
}

/// Nach einer Anweisung: Hinweise „Notizen unvollständig“ der Vorversion, die das Ergebnis nicht
/// mehr enthält (das Modell sah sie nicht oder ließ sie aus), zurück an ihren Platz bzw. nach
/// „Weitere Notizen“. Gibt `true` zurück, wenn etwas eingefügt wurde (IDs dann neu vergeben).
pub fn hinweise_uebernehmen(vorher: &EnhancedNotes, sections: &mut Vec<EnhancedSection>) -> bool {
    let vorhanden: HashSet<String> = sections
        .iter()
        .flat_map(|s| s.entries.iter())
        .filter(|e| ist_hinweis(e))
        .map(|e| e.text.clone())
        .collect();
    let mut eingefuegt = false;
    for (abschnitt, index, e) in vorher.sections.iter().flat_map(|s| {
        s.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| ist_hinweis(e))
            .map(move |(i, e)| (s, i, e))
    }) {
        if vorhanden.contains(&e.text) {
            continue;
        }
        let ziel = match sections.iter().position(|s| s.id == abschnitt.id) {
            Some(z) => z,
            None => match sections.iter().position(|s| s.id == WEITERE_ID) {
                Some(z) => z,
                None => {
                    sections.push(EnhancedSection {
                        id: WEITERE_ID.to_string(),
                        title: WEITERE_TITEL.to_string(),
                        kind: SectionKind::Text,
                        entries: Vec::new(),
                    });
                    sections.len() - 1
                }
            },
        };
        let at = index.min(sections[ziel].entries.len());
        sections[ziel].entries.insert(at, e.clone());
        eingefuegt = true;
    }
    if eingefuegt {
        let mut n = 0u32;
        for e in sections.iter_mut().flat_map(|s| s.entries.iter_mut()) {
            n += 1;
            e.id = format!("E{n}");
        }
    }
    eingefuegt
}
