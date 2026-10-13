//! Gestalteter Export eines Dokuments (Protokoll oder Dokumenttyp) mit Briefkopf
//! (Entwurf 2026-10-13, Abschnitt 3; Etappe 2).
//!
//! Eine Quelle: der gespeicherte Markdown-Text (Handkorrekturen gelten mit). Er wird mit
//! dem Parser von `export.rs` gelesen und hier zu einem kleinen Dokumentmodell
//! gruppiert ([`Item`]: Ueberschrift, Absatz, Aufzaehlung, Tabelle, Hinweis), aus dem
//! jedes Format rendert:
//! - `.md` / `.txt`: wie bisher (`export::write_document`),
//! - `.docx`: [`docx`] -- Kopf mit Logo und Firma, Fuss mit "Seite x von y", echte
//!   Tabellen, Farben des Briefkopfs, Schrift Segoe UI (Medienregel des Design-Systems),
//! - `.pdf`: [`pdf`] -- `krilla`, gleiche Ausgabe auf Windows und macOS, Schriften
//!   eingebettet (IBM Plex Sans, Archivo; OFL-1.1), PDF/A-2b.
//!
//! Geschrieben wird ueber `<ziel>.part` und Umbenennen: ein Fehler laesst ein vorhandenes
//! Ziel unangetastet. Kein Zugriff auf Einstellungen; Briefkopf und Logo kommen als
//! Argument.

use std::path::Path;

use super::export::{
    is_table_separator, parse_blocks, parse_spans, table_cells, task_marker, Block, ExportFormat,
    Span,
};
use crate::managers::letterhead::{Letterhead, LogoData};

pub mod docx;
pub mod pdf;
pub mod pptx;

/// Ein Baustein des Dokuments.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph(Vec<Span>),
    Bullet {
        spans: Vec<Span>,
        /// `- [ ]` / `- [x]`: Aufgabe offen/erledigt.
        done: Option<bool>,
    },
    Table {
        header: Option<Vec<Vec<Span>>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    /// `> ...` (Hinweise des Generators: unvollstaendig, Ueberlaenge).
    Note(Vec<Span>),
}

/// Markdown -> Bausteine. Tabellen (aufeinanderfolgende `|`-Zeilen) und Hinweise (`> `)
/// werden zusammengefasst; Leerzeilen trennen nur.
pub fn parse(markdown: &str) -> Vec<Item> {
    let blocks = parse_blocks(markdown);
    let mut items = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        match &blocks[i] {
            Block::Heading(level, text) => items.push(Item::Heading {
                level: *level,
                spans: parse_spans(text),
            }),
            Block::Bullet(text) => match task_marker(text) {
                Some((done, rest)) => items.push(Item::Bullet {
                    spans: parse_spans(rest),
                    done: Some(done),
                }),
                None => items.push(Item::Bullet {
                    spans: parse_spans(text),
                    done: None,
                }),
            },
            Block::Paragraph(text) if text.starts_with('|') => {
                let mut lines: Vec<&str> = Vec::new();
                while let Some(Block::Paragraph(row)) = blocks.get(i) {
                    if !row.starts_with('|') {
                        break;
                    }
                    lines.push(row);
                    i += 1;
                }
                items.push(table(&lines));
                continue;
            }
            Block::Paragraph(text) if text.starts_with('>') => {
                let rest = text.trim_start_matches('>').trim();
                items.push(Item::Note(parse_spans(rest)));
            }
            Block::Paragraph(text) => items.push(Item::Paragraph(parse_spans(text))),
            Block::Blank => {}
        }
        i += 1;
    }
    items
}

fn table(lines: &[&str]) -> Item {
    let has_header = lines.len() >= 2 && is_table_separator(lines[1]);
    let cells =
        |row: &str| -> Vec<Vec<Span>> { table_cells(row).iter().map(|c| parse_spans(c)).collect() };
    let mut rows = Vec::new();
    let mut header = None;
    for (n, line) in lines.iter().enumerate() {
        if is_table_separator(line) {
            continue;
        }
        if has_header && n == 0 {
            header = Some(cells(line));
        } else {
            rows.push(cells(line));
        }
    }
    Item::Table { header, rows }
}

/// Reiner Text der Abschnitte (fuer Titel, Tests).
pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// Titel des Dokuments: die erste Ueberschrift der Ebene 1.
pub fn title_of(items: &[Item]) -> Option<String> {
    items.iter().find_map(|item| match item {
        Item::Heading { level: 1, spans } => Some(plain(spans)),
        _ => None,
    })
}

/// Alles, was ein gestalteter Export braucht.
pub struct Design<'a> {
    pub letterhead: &'a Letterhead,
    pub logo: Option<&'a LogoData>,
}

/// Schreibt `markdown` in das Format der Endung: `md`, `txt`, `docx`, `pdf`, `pptx`
/// (die drei letzten mit Briefkopf). Andere Endungen: `export_unsupported_format`.
pub fn write(path: &Path, markdown: &str, design: &Design<'_>) -> Result<(), String> {
    // PowerPoint kennt der Besprechungs-Export nicht; deshalb hier vor `ExportFormat`.
    let pptx = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pptx"));
    if pptx {
        let bytes = pptx::render(&parse(markdown), design)?;
        return write_part(path, &bytes);
    }
    match ExportFormat::from_path(path) {
        ExportFormat::Markdown | ExportFormat::PlainText => {
            super::export::write_document(path, markdown)
        }
        ExportFormat::Docx => {
            let bytes = docx::render(&parse(markdown), design)?;
            write_part(path, &bytes)
        }
        ExportFormat::Pdf => {
            let bytes = pdf::render(&parse(markdown), design)?;
            write_part(path, &bytes)
        }
        _ => Err("export_unsupported_format".to_string()),
    }
}

/// Ein gestaltetes Format als Bytes (`docx`, `pdf`, `pptx`), z. B. fuer ein Presenter-Paket.
pub fn render_bytes(ext: &str, markdown: &str, design: &Design<'_>) -> Result<Vec<u8>, String> {
    let items = parse(markdown);
    match ext {
        "docx" => docx::render(&items, design),
        "pdf" => pdf::render(&items, design),
        "pptx" => pptx::render(&items, design),
        _ => Err("export_unsupported_format".to_string()),
    }
}

/// Schreibt ueber `<ziel>.part` und Umbenennen.
fn write_part(path: &Path, bytes: &[u8]) -> Result<(), String> {
    crate::managers::letterhead::write_atomic(path, bytes)
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Protokoll: Jour fixe\n\n**Datum:** 2026-10-13 · **Dauer:** 30 min\n\n## Aufgaben\n\n- Angebot schicken _(Wer: Anna)_\n- [x] Raum buchen\n\n## Sprecher & Redeanteile\n\n| Sprecher | Redezeit | Anteil |\n|---|---|---|\n| Ich | 10:00 | 60,0 % |\n| Gegenstelle | 06:40 | 40,0 % |\n\n> **Hinweis:** Das Protokoll ist unvollständig.\n";

    #[test]
    fn markdown_becomes_headings_lists_tables_and_notes() {
        let items = parse(SAMPLE);
        assert_eq!(title_of(&items).as_deref(), Some("Protokoll: Jour fixe"));
        assert!(matches!(&items[1], Item::Paragraph(spans) if spans[0].bold));
        assert!(items.contains(&Item::Bullet {
            spans: parse_spans("Raum buchen"),
            done: Some(true)
        }));
        let table = items
            .iter()
            .find_map(|i| match i {
                Item::Table { header, rows } => Some((header.clone(), rows.clone())),
                _ => None,
            })
            .unwrap();
        assert_eq!(plain(&table.0.unwrap()[2]), "Anteil");
        assert_eq!(table.1.len(), 2);
        assert_eq!(plain(&table.1[1][0]), "Gegenstelle");
        assert!(
            matches!(items.last(), Some(Item::Note(spans)) if plain(spans).starts_with("Hinweis:"))
        );
    }

    #[test]
    fn unknown_formats_are_refused_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let lh = crate::managers::letterhead::builtins().remove(0);
        let design = Design {
            letterhead: &lh,
            logo: None,
        };
        let target = dir.path().join("a.srt");
        assert_eq!(
            write(&target, SAMPLE, &design).unwrap_err(),
            "export_unsupported_format"
        );
        assert!(!target.exists());
    }
}

/// Handprobe ohne Modell: rendert `LVA_SAMPLE_MD` mit jedem mitgelieferten Briefkopf und
/// einem eigenen (Logo rechts, eigene Akzentfarbe) nach `LVA_SAMPLE_OUT_DIR`.
#[cfg(test)]
#[test]
#[ignore = "Beispieldateien fuer die Sichtpruefung, nur von Hand"]
fn probe_letterhead_samples() {
    use crate::managers::letterhead::{builtins, LogoPosition, Store};
    let md = std::fs::read_to_string(std::env::var("LVA_SAMPLE_MD").unwrap()).unwrap();
    let out = std::path::PathBuf::from(std::env::var("LVA_SAMPLE_OUT_DIR").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let store = Store::new(out.join("briefkoepfe"));
    let mut own = builtins().remove(0);
    own.id = "eigene-firma".into();
    own.name = "Muster GmbH".into();
    own.builtin = false;
    own.firma.name = "Muster GmbH".into();
    own.firma.zeilen = vec!["Musterstraße 1 · 12345 Musterstadt".into()];
    own.farben.akzent = "#0E7C86".into();
    own.farben.akzent_text = "#FFFFFF".into();
    own.farben.titel = "#0E7C86".into();
    own.fusszeile.zeilen = vec!["Muster GmbH · vertraulich".into()];
    store.save(&own).unwrap();
    let logo_src = out.join("muster-logo.png");
    std::fs::write(
        &logo_src,
        include_bytes!("../../../assets/documents/logo-ingenieurbuero-w.png"),
    )
    .unwrap();
    let mut own = store.set_logo("eigene-firma", Some(&logo_src)).unwrap();
    if let Some(logo) = own.logo.as_mut() {
        logo.position = LogoPosition::Rechts;
        logo.breite_mm = 16.0;
    }
    store.save(&own).unwrap();
    for lh in store.list() {
        let logo = store.logo(&lh).unwrap();
        let design = Design {
            letterhead: &lh,
            logo: logo.as_ref(),
        };
        for ext in ["docx", "pdf", "pptx"] {
            write(&out.join(format!("{}.{ext}", lh.id)), &md, &design).unwrap();
        }
    }
}

/// Handprobe ohne Modell: jede `*.md` aus `LVA_SAMPLE_MD_DIR` als `.pptx` (und `.pdf`) im
/// Briefkopf `LVA_SAMPLE_LETTERHEAD` (Vorgabe `wai`) nach `LVA_SAMPLE_OUT_DIR`.
#[cfg(test)]
#[test]
#[ignore = "Beispieldateien fuer die Sichtpruefung, nur von Hand"]
fn probe_documents_as_slides() {
    use crate::managers::letterhead::Store;
    let src = std::path::PathBuf::from(std::env::var("LVA_SAMPLE_MD_DIR").unwrap());
    let out = std::path::PathBuf::from(std::env::var("LVA_SAMPLE_OUT_DIR").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let store = Store::new(out.join("briefkoepfe"));
    let lh = store
        .get(&std::env::var("LVA_SAMPLE_LETTERHEAD").unwrap_or_else(|_| "wai".into()))
        .unwrap();
    let logo = store.logo(&lh).unwrap();
    let design = Design {
        letterhead: &lh,
        logo: logo.as_ref(),
    };
    for entry in std::fs::read_dir(&src).unwrap().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let md = std::fs::read_to_string(&path).unwrap();
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        write(&out.join(format!("{stem}.pptx")), &md, &design).unwrap();
    }
}
