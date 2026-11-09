//! Zweite Notizquelle für KI-Notizen und Protokoll (Spec §5.3, §5.4; lva#396 N4a Schritt 2).
//!
//! Zugeordnete Seiten (`bestaetigt`, `auto`) werden **vollständig** gelesen – ohne die Grenzen der
//! Werkzeuge (20 Treffer, 32 000 Zeichen) – und als Notizblöcke neben den Notizblock der Aufnahme
//! gestellt. Sie werden **nicht** in `meeting_notes` kopiert: Kennungen der Form
//! `nb:<seite_id>#<bid>` verweisen auf das Notizbuch ([`NoteSource::Notizbuch`]); geändert wird
//! dort, die Besprechung sieht beim nächsten Lauf den neuen Stand.
//!
//! Gelesen wird nur über die Freigabe (Bereich „Notizbuch“ und Abschnitte, Spec §3.2): eine
//! nicht mehr freigegebene Seite fehlt still, ohne Hinweis (M1).
//!
//! [`verlust_sichern`] prüft nach `assemble` deterministisch, dass jeder Nutzerblock beider
//! Quellen im Ergebnis steht: einem Eintrag zugeordnet oder im Abschnitt „Weitere Notizen“.
//! Fehlende Blöcke werden angehängt, nie verworfen. Nicht vollständig lesbare Seiten stehen als
//! „Notizen unvollständig: <Titel>“ im Ergebnis.

use std::collections::{BTreeMap, HashSet};

use wai_notizbuch_zugang::{BlockArt, Freigabe, Seite, Zugang};

use super::links::NoteLink;
use super::verlust::{fehlende_bloecke, pruefbloecke_aus_seite};
use super::zeit::{abbilden, blockzeit, Guete, Zeitachse};
use crate::managers::meetings::notes::model::{
    EnhanceStats, EnhancedEntry, EnhancedSection, EntryFlags, NoteBlock, NoteBlockKind, Origin,
    SectionKind,
};

/// Präfix der Kennung eines Notizbuch-Blocks in `NoteBlock.id` / `EnhancedEntry.note_id`.
pub const PRAEFIX: &str = "nb:";
/// Abschnitt für Notizen, die das Modell keinem Eintrag zugeordnet hat (§5.4).
pub const WEITERE_ID: &str = "weitere_notizen";
pub const WEITERE_TITEL: &str = "Weitere Notizen";
/// Über dieser Textmenge (Bytes) gilt eine Seite als nicht vollständig lesbar (§5.4 M7).
pub const MAX_SEITENTEXT: usize = 1024 * 1024;

/// Herkunft eines Nutzerblocks in KI-Notizen und Protokoll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteSource {
    /// Notizblock der Aufnahme (`meeting_notes`).
    Notizblock { block_id: String },
    /// Notizbuch-Seite; `stand` ist der Freigabestand beim Lesen.
    Notizbuch {
        seite_id: String,
        block_id: String,
        stand: String,
    },
}

/// Eine gelesene, zugeordnete Seite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeleseneSeite {
    pub seite_id: String,
    pub titel: String,
    pub stand: String,
    /// Nicht vollständig lesbar (Blockgrenze des Zugangs, über 1 MB Text, Leserfehler).
    pub unvollstaendig: bool,
    /// Blöcke als Notizblöcke (`id` = `nb:<kennung>`), leere Blöcke entfallen.
    pub bloecke: Vec<NoteBlock>,
}

/// Alle Notizbuch-Notizen einer Besprechung.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotizbuchNotizen {
    pub seiten: Vec<GeleseneSeite>,
}

impl NotizbuchNotizen {
    pub fn ist_leer(&self) -> bool {
        self.seiten.is_empty()
    }

    /// Alle Blöcke in Seitenreihenfolge.
    pub fn note_blocks(&self) -> Vec<NoteBlock> {
        self.seiten
            .iter()
            .flat_map(|s| s.bloecke.iter().cloned())
            .collect()
    }

    /// Herkunft einer `note_id` (Notizbuch-Block mit Stand, sonst Notizblock).
    pub fn quelle(&self, note_id: &str) -> NoteSource {
        let Some(kennung) = note_id.strip_prefix(PRAEFIX) else {
            return NoteSource::Notizblock {
                block_id: note_id.to_string(),
            };
        };
        let (seite_id, block_id) = kennung.split_once('#').unwrap_or((kennung, ""));
        let stand = self
            .seiten
            .iter()
            .find(|s| s.seite_id == seite_id)
            .map(|s| s.stand.clone())
            .unwrap_or_default();
        NoteSource::Notizbuch {
            seite_id: seite_id.to_string(),
            block_id: block_id.to_string(),
            stand,
        }
    }

    /// Kurzbericht für Dokument-Metadaten (nur Kennungen, Zähler, Stand; kein Inhalt).
    pub fn metadaten(&self, im_prompt: bool, ergaenzt: usize) -> serde_json::Value {
        serde_json::json!({
            "im_prompt": im_prompt,
            "ergaenzt": ergaenzt,
            "seiten": self.seiten.iter().map(|s| serde_json::json!({
                "seite_id": s.seite_id,
                "stand": s.stand,
                "bloecke": s.bloecke.len(),
                "unvollstaendig": s.unvollstaendig,
            })).collect::<Vec<_>>(),
        })
    }
}

/// `nb:`-Kennung einer Notiz?
pub fn ist_notizbuch_id(note_id: &str) -> bool {
    note_id.starts_with(PRAEFIX)
}

fn art_zu_kind(art: BlockArt, kategorie: Option<&str>) -> NoteBlockKind {
    if art == BlockArt::Aufgabe || kategorie.is_some_and(|k| k.eq_ignore_ascii_case("aufgabe")) {
        return NoteBlockKind::Todo;
    }
    match art {
        BlockArt::Ueberschrift => NoteBlockKind::Heading,
        BlockArt::Aufzaehlung => NoteBlockKind::Bullet,
        _ => NoteBlockKind::Paragraph,
    }
}

/// Seite → Notizblöcke. `at_ms` nur bei Güte `genau` (keine Scheingenauigkeit im Prompt, §5.1);
/// ganz außerhalb der Aufnahme → `None`. Liefert zusätzlich, ob die Textgrenze griff.
pub fn bloecke_aus_seite(seite: &Seite, achse: &Zeitachse) -> (Vec<NoteBlock>, bool) {
    // Kennungen exakt wie in der Verlustprüfung (`pruefbloecke_aus_seite`), damit beide Seiten
    // dieselben Blöcke meinen (auch bei doppelter `bid`).
    let pruef = pruefbloecke_aus_seite(seite);
    let nicht_leer = seite.bloecke.iter().filter(|b| !b.text.trim().is_empty());
    let mut out = Vec::with_capacity(pruef.len());
    let mut bytes = 0usize;
    let mut gekappt = false;
    for (b, p) in nicht_leer.zip(pruef.iter()) {
        bytes = bytes.saturating_add(b.text.len());
        if bytes > MAX_SEITENTEXT {
            gekappt = true;
            break;
        }
        let at_ms = blockzeit(b, &seite.kopf)
            .filter(|z| z.guete == Guete::Genau)
            .and_then(|z| abbilden(&z, achse))
            .map(|a| a.von_ms.max(0) as u64);
        let text = if b.art == BlockArt::Zeichnung {
            format!("[Zeichnung auf Seite {}]", seite.kopf.titel)
        } else {
            b.text.clone()
        };
        out.push(NoteBlock {
            id: format!("{PRAEFIX}{}", p.kennung),
            kind: art_zu_kind(b.art, b.kategorie.as_deref()),
            text,
            at_ms,
            checked: b.erledigt == Some(true),
        });
    }
    (out, gekappt)
}

/// Liest die zugeordneten Seiten (`bestaetigt`, `auto`) vollständig über die Freigabe.
/// Nicht sichtbare Seiten fehlen still (M1); ein Leserfehler ergibt eine leere, als
/// unvollständig markierte Seite ohne Titel aus dem Notizbuch.
pub fn lesen(
    zugang: &Zugang,
    freigabe: &Freigabe,
    links: &[NoteLink],
    achse: &Zeitachse,
) -> NotizbuchNotizen {
    let mut seiten = Vec::new();
    for l in links.iter().filter(|l| l.art.ist_zugeordnet()) {
        match zugang.seite_mit_stand(&l.seite_id, freigabe) {
            Ok(m) => {
                let Some(seite) = m.wert else {
                    continue;
                };
                let (bloecke, gekappt) = bloecke_aus_seite(&seite, achse);
                seiten.push(GeleseneSeite {
                    seite_id: seite.kopf.id.clone(),
                    titel: seite.kopf.titel.clone(),
                    stand: m.stand,
                    unvollstaendig: seite.unvollstaendig || gekappt,
                    bloecke,
                });
            }
            Err(_) => seiten.push(GeleseneSeite {
                seite_id: l.seite_id.clone(),
                titel: String::new(),
                stand: String::new(),
                unvollstaendig: true,
                bloecke: Vec::new(),
            }),
        }
    }
    NotizbuchNotizen { seiten }
}

/// Notizbuch-Notizen einer Besprechung für einen Lauf; leer ohne Freigabe, ohne Zuordnung oder
/// ohne Beginn der Aufnahme. Scheitert nie (die Besprechung geht ohne Notizbuch weiter).
pub fn fuer_besprechung(
    store: &crate::managers::meetings::store::MeetingStore,
    settings: &crate::settings::AppSettings,
    meeting_id: &str,
) -> NotizbuchNotizen {
    let links = match store.note_links(meeting_id) {
        Ok(l) if l.iter().any(|l| l.art.ist_zugeordnet()) => l,
        _ => return NotizbuchNotizen::default(),
    };
    let Some((zugang, freigabe)) = super::dienst::sicht_aus_einstellungen(settings) else {
        return NotizbuchNotizen::default();
    };
    let Ok(Some(m)) = store.get_meeting(meeting_id) else {
        return NotizbuchNotizen::default();
    };
    let achse = Zeitachse::neu(
        m.started_at.unwrap_or(0),
        m.duration_ms.map(|d| d as i64).unwrap_or(0),
    );
    lesen(&zugang, &freigabe, &links, &achse)
}

fn eintrag(origin: Origin, text: String, note_id: Option<String>) -> EnhancedEntry {
    EnhancedEntry {
        id: String::new(),
        origin,
        text,
        note_id,
        source_segment_ids: Vec::new(),
        source_slide_ids: Vec::new(),
        assignee: None,
        due: None,
        flags: EntryFlags {
            placed_by_fallback: origin == Origin::User,
            ..EntryFlags::default()
        },
    }
}

/// Ergebnis der Verlustprüfung.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verlustbericht {
    /// Blöcke (Kennungen), die im Ergebnis fehlten und angehängt wurden.
    pub ergaenzt: Vec<String>,
    /// Seiten mit „Notizen unvollständig“.
    pub unvollstaendig: Vec<String>,
}

fn weitere_index(sections: &mut Vec<EnhancedSection>) -> usize {
    if let Some(i) = sections.iter().position(|s| s.id == WEITERE_ID) {
        return i;
    }
    sections.push(EnhancedSection {
        id: WEITERE_ID.to_string(),
        title: WEITERE_TITEL.to_string(),
        kind: SectionKind::Text,
        entries: Vec::new(),
    });
    sections.len() - 1
}

/// Verlustprüfung nach `assemble` für beide Quellen (§5.4):
///
/// 1. Notizbuch-Blöcke, die `assemble` nur ersatzweise platziert hat (`placed_by_fallback`),
///    wandern in „Weitere Notizen“.
/// 2. Jeder nicht leere Block aus `notizblock` und `notizbuch`, der weder einem Eintrag
///    zugeordnet ist noch in „Weitere Notizen“ steht, wird dort angehängt (Text byte-genau).
/// 3. Nicht vollständig lesbare Seiten: Eintrag „Notizen unvollständig: <Titel>“.
/// 4. IDs `E1..En` neu, Statistik nachgeführt.
///
/// Der Abschnitt „Weitere Notizen“ entsteht nur, wenn er gebraucht wird.
pub fn verlust_sichern(
    sections: &mut Vec<EnhancedSection>,
    stats: &mut EnhanceStats,
    notizblock: &[NoteBlock],
    notizbuch: &NotizbuchNotizen,
) -> Verlustbericht {
    let mut bericht = Verlustbericht::default();

    // Aufgaben bleiben Aufgaben (§5.4): ersatzweise platzierte Aufgaben-Blöcke stehen schon im
    // Aufgaben-Abschnitt, fehlende kommen dorthin, wenn die Vorlage einen hat.
    let aufgaben: HashSet<&str> = notizbuch
        .seiten
        .iter()
        .flat_map(|s| s.bloecke.iter())
        .filter(|b| b.kind == NoteBlockKind::Todo)
        .map(|b| b.id.as_str())
        .collect();
    let aufgaben_abschnitt = sections.iter().position(|s| s.kind == SectionKind::Tasks);

    // 1. Ersatzweise platzierte Notizbuch-Blöcke (außer Aufgaben) nach „Weitere Notizen“.
    let mut verschoben: Vec<EnhancedEntry> = Vec::new();
    for s in sections.iter_mut().filter(|s| s.id != WEITERE_ID) {
        let (weg, bleibt): (Vec<_>, Vec<_>) =
            std::mem::take(&mut s.entries).into_iter().partition(|e| {
                e.origin == Origin::User
                    && e.flags.placed_by_fallback
                    && e.note_id.as_deref().is_some_and(|id| {
                        ist_notizbuch_id(id)
                            && !(aufgaben.contains(id) && aufgaben_abschnitt.is_some())
                    })
            });
        s.entries = bleibt;
        verschoben.extend(weg);
    }
    if !verschoben.is_empty() {
        let w = weitere_index(sections);
        sections[w].entries.extend(verschoben);
    }

    // 2. Fehlende Blöcke beider Quellen.
    let alle: Vec<&NoteBlock> = notizblock
        .iter()
        .chain(notizbuch.seiten.iter().flat_map(|s| s.bloecke.iter()))
        .filter(|b| !b.text.trim().is_empty())
        .collect();
    let erwartet: Vec<String> = alle.iter().map(|b| b.id.clone()).collect();
    let mut referenzen: Vec<String> = Vec::new();
    let mut weitere: Vec<String> = Vec::new();
    for s in sections.iter() {
        let ziel = if s.id == WEITERE_ID {
            &mut weitere
        } else {
            &mut referenzen
        };
        ziel.extend(
            s.entries
                .iter()
                .filter(|e| e.origin == Origin::User)
                .filter_map(|e| e.note_id.clone()),
        );
    }
    let fehlend = fehlende_bloecke(&erwartet, &referenzen, &weitere);
    if !fehlend.is_empty() {
        let nach_id: BTreeMap<&str, &NoteBlock> =
            alle.iter().map(|b| (b.id.as_str(), *b)).collect();
        for k in &fehlend {
            if let Some(b) = nach_id.get(k.as_str()) {
                let ziel = match aufgaben_abschnitt {
                    Some(a) if b.kind == NoteBlockKind::Todo => a,
                    _ => weitere_index(sections),
                };
                sections[ziel].entries.push(eintrag(
                    Origin::User,
                    b.text.clone(),
                    Some(b.id.clone()),
                ));
            }
        }
        stats.user_notes_by_fallback += fehlend.len() as u32;
        bericht.ergaenzt = fehlend;
    }

    // 3. „Notizen unvollständig“ (Hinweis, kein Nutzertext; keine Quelle nötig).
    let unvollstaendig: Vec<&GeleseneSeite> = notizbuch
        .seiten
        .iter()
        .filter(|s| s.unvollstaendig)
        .collect();
    if !unvollstaendig.is_empty() {
        let w = weitere_index(sections);
        let schon: HashSet<String> = sections[w].entries.iter().map(|e| e.text.clone()).collect();
        for s in unvollstaendig {
            let titel = if s.titel.trim().is_empty() {
                "zugeordnete Notizbuch-Seite (nicht lesbar)".to_string()
            } else {
                s.titel.clone()
            };
            let text = format!("Notizen unvollständig: {titel}");
            if !schon.contains(&text) {
                sections[w].entries.push(eintrag(Origin::Ai, text, None));
            }
            bericht.unvollstaendig.push(s.seite_id.clone());
        }
    }

    // 4. IDs und Statistik (Nutzernotizen beider Quellen).
    let mut n = 0u32;
    for s in sections.iter_mut() {
        for e in s.entries.iter_mut() {
            n += 1;
            e.id = format!("E{n}");
        }
    }
    stats.user_notes_total = stats.user_notes_total.max(erwartet.len() as u32);
    bericht
}
