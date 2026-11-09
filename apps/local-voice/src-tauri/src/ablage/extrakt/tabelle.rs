//! XLSX/ODS über `calamine`: je Blatt Zeilengruppen mit Anker `blatt=<name>&zeilen=<a>-<b>`; jede Zeile
//! als `Spalte: Wert; …` (Spaltennamen aus der ersten belegten Zeile). Vorher prüft [`super::zipsicher`]
//! die ZIP-Grenzen; Blätter über [`MAX_ZELLEN`] werden abgewiesen.

use super::zipsicher;
use super::{Abschnitt, ExtraktFehler};
use calamine::{open_workbook_from_rs, Data, Ods, Range, Reader, Xlsx};
use std::io::Cursor;

pub const MAX_ZELLEN: usize = 5_000_000;
const ZEILEN_JE_ABSCHNITT: usize = 20;

/// Blattname im Anker: `%`, `&`, `=` und Steuerzeichen werden prozentkodiert.
fn anker_name(n: &str) -> String {
    let mut aus = String::new();
    for c in n.chars() {
        if matches!(c, '%' | '&' | '=') || c.is_control() {
            let mut b = [0u8; 4];
            for x in c.encode_utf8(&mut b).bytes() {
                aus.push_str(&format!("%{x:02X}"));
            }
        } else {
            aus.push(c);
        }
    }
    aus
}

fn spaltenbuchstabe(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push((b'A' + (i % 26) as u8) as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.iter().rev().collect()
}

fn zelle(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::Error(_) => String::new(),
        andere => andere.to_string().trim().to_string(),
    }
}

fn blatt(name: &str, r: &Range<Data>, aus: &mut Vec<Abschnitt>) -> Result<(), ExtraktFehler> {
    let (h, b) = r.get_size();
    if h.saturating_mul(b) > MAX_ZELLEN {
        return Err(ExtraktFehler::neu("zu_gross"));
    }
    let start = r.start().map(|(z, _)| z as usize).unwrap_or(0);
    let start_spalte = r.start().map(|(_, s)| s as usize).unwrap_or(0);
    let mut kopf: Option<Vec<String>> = None;
    let mut gruppe: Vec<String> = Vec::new();
    let mut von = 0usize;
    let mut bis = 0usize;
    let anker = anker_name(name);
    let mut abschliessen = |gruppe: &mut Vec<String>, von: usize, bis: usize| {
        if !gruppe.is_empty() {
            aus.push(Abschnitt {
                anker: format!("blatt={anker}&zeilen={von}-{bis}"),
                pfad: vec![name.to_string()],
                text: gruppe.join("\n"),
            });
            gruppe.clear();
        }
    };
    for (i, zeile) in r.rows().enumerate() {
        let nr = start + i + 1;
        let werte: Vec<String> = zeile.iter().map(zelle).collect();
        if werte.iter().all(String::is_empty) {
            continue;
        }
        let Some(k) = kopf.as_ref() else {
            // Erste belegte Zeile: Spaltennamen (leere Namen werden zu Spaltenbuchstaben).
            let k: Vec<String> = werte
                .iter()
                .enumerate()
                .map(|(j, w)| {
                    if w.is_empty() {
                        format!("Spalte {}", spaltenbuchstabe(start_spalte + j))
                    } else {
                        w.clone()
                    }
                })
                .collect();
            // Die Kopfzeile selbst ist auch Text (Suche nach Spaltennamen).
            gruppe.push(
                werte
                    .iter()
                    .filter(|w| !w.is_empty())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; "),
            );
            von = nr;
            bis = nr;
            kopf = Some(k);
            continue;
        };
        let text: Vec<String> = werte
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.is_empty())
            .map(|(j, w)| format!("{}: {w}", k.get(j).map(String::as_str).unwrap_or("?")))
            .collect();
        if gruppe.is_empty() {
            von = nr;
        }
        gruppe.push(text.join("; "));
        bis = nr;
        if gruppe.len() >= ZEILEN_JE_ABSCHNITT {
            abschliessen(&mut gruppe, von, bis);
        }
    }
    abschliessen(&mut gruppe, von, bis);
    Ok(())
}

pub fn tabelle(bytes: &[u8]) -> Result<Vec<Abschnitt>, ExtraktFehler> {
    let z = zipsicher::ZipLeser::oeffnen(bytes)?;
    let namen = z.namen();
    drop(z);
    let kaputt = |_| ExtraktFehler::neu("tabelle");
    let mut aus = Vec::new();
    if namen.iter().any(|n| n == "xl/workbook.xml") {
        let mut wb: Xlsx<_> = open_workbook_from_rs(Cursor::new(bytes)).map_err(kaputt)?;
        for n in wb.sheet_names() {
            let r = wb
                .worksheet_range(&n)
                .map_err(|_| ExtraktFehler::neu("tabelle"))?;
            blatt(&n, &r, &mut aus)?;
        }
    } else if namen.iter().any(|n| n == "content.xml") {
        let mut wb: Ods<_> =
            open_workbook_from_rs(Cursor::new(bytes)).map_err(|_| ExtraktFehler::neu("tabelle"))?;
        for n in wb.sheet_names() {
            let r = wb
                .worksheet_range(&n)
                .map_err(|_| ExtraktFehler::neu("tabelle"))?;
            blatt(&n, &r, &mut aus)?;
        }
    } else {
        return Err(ExtraktFehler::neu("format"));
    }
    Ok(aus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spalten_und_anker() {
        assert_eq!(spaltenbuchstabe(0), "A");
        assert_eq!(spaltenbuchstabe(25), "Z");
        assert_eq!(spaltenbuchstabe(26), "AA");
        assert_eq!(anker_name("Q1&Q2=50%"), "Q1%26Q2%3D50%25");
    }
}
