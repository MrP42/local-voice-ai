//! XLSX/ODS über `calamine`: je Blatt Zeilengruppen mit Anker `blatt=<name>&zeilen=<a>-<b>`; jede Zeile
//! als `Spalte: Wert; …` (Spaltennamen aus der ersten belegten Zeile).
//!
//! calamine bekommt nie die Rohbytes: alle Teile werden über [`super::zipsicher::ZipLeser`] begrenzt
//! entpackt (tatsächlich gelesene Bytes zählen, nicht die Angaben im Verzeichnis), jede XML wird ohne
//! DOCTYPE/Entitäten vorgeprüft ([`super::XmlLeser`]), und die Fläche jedes Blatts wird vorab aus der
//! XML bestimmt (XLSX: Zellbezüge belegter Zellen; ODS: `number-rows-repeated` /
//! `number-columns-repeated`). Erst danach liest calamine ein neu gepacktes, unkomprimiertes Archiv aus
//! genau diesen geprüften Teilen. Blätter über [`MAX_ZELLEN`] werden vor dem Aufbau des Range abgewiesen.

use super::zipsicher::ZipLeser;
use super::{attr, Abschnitt, ExtraktFehler, XmlLeser, X};
use calamine::{open_workbook_from_rs, Data, Ods, Range, Reader, Xlsx};
use std::io::{Cursor, Write};

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

fn zu_gross() -> ExtraktFehler {
    ExtraktFehler::neu("zu_gross")
}

/// Lokaler Name ohne Präfix (`x:c` → `c`).
fn lokal(n: &str) -> &str {
    n.rsplit(':').next().unwrap_or(n)
}

/// `B12` → (Zeile 11, Spalte 1), nullbasiert; `None` bei ungültigem Bezug.
pub(crate) fn zellbezug(r: &str) -> Option<(u64, u64)> {
    let b = r.as_bytes();
    let n = b.iter().take_while(|c| c.is_ascii_alphabetic()).count();
    if n == 0 || n > 3 || n == b.len() {
        return None;
    }
    let mut spalte = 0u64;
    for c in &b[..n] {
        spalte = spalte * 26 + u64::from(c.to_ascii_uppercase() - b'A' + 1);
    }
    let zeile: u64 = r[n..].parse().ok()?;
    if zeile == 0 {
        return None;
    }
    Some((zeile - 1, spalte - 1))
}

/// Begrenzende Fläche (Zeilen × Spalten) belegter Zellen in einem Blatt.
#[derive(Default)]
struct Flaeche {
    min_z: Option<u64>,
    max_z: u64,
    min_s: u64,
    max_s: u64,
}

impl Flaeche {
    fn zelle(&mut self, z: u64, s: u64) {
        match self.min_z {
            None => {
                self.min_z = Some(z);
                self.max_z = z;
                self.min_s = s;
                self.max_s = s;
            }
            Some(m) => {
                self.min_z = Some(m.min(z));
                self.max_z = self.max_z.max(z);
                self.min_s = self.min_s.min(s);
                self.max_s = self.max_s.max(s);
            }
        }
    }

    fn pruefen(&self) -> Result<(), ExtraktFehler> {
        let Some(min_z) = self.min_z else {
            return Ok(());
        };
        let h = self.max_z - min_z + 1;
        let b = self.max_s - self.min_s + 1;
        if h.saturating_mul(b) > MAX_ZELLEN as u64 {
            return Err(zu_gross());
        }
        Ok(())
    }
}

/// XLSX-Blatt: Fläche aus den Bezügen der Zellen mit Wert (`v`, `is`, `f`), wie calamine sie anlegt.
fn xlsx_blatt_pruefen(xml: &[u8]) -> Result<(), ExtraktFehler> {
    let mut r = XmlLeser::neu(xml);
    let mut f = Flaeche::default();
    let (mut zeile, mut spalte) = (0u64, 0u64);
    let mut erste_zeile = true;
    let mut zelle: Option<(u64, u64)> = None;
    let mut belegt = false;
    loop {
        match r.naechstes()? {
            X::Start(n, a) | X::Leer(n, a) if lokal(&n) == "row" => {
                zeile = match attr(&a, "r").and_then(|v| v.parse::<u64>().ok()) {
                    Some(z) if z > 0 => z - 1,
                    _ if erste_zeile => 0,
                    _ => zeile + 1,
                };
                erste_zeile = false;
                spalte = 0;
                zelle = None;
            }
            X::Start(n, a) if lokal(&n) == "c" => {
                let pos = match attr(&a, "r").and_then(zellbezug) {
                    Some(p) => p,
                    None => (zeile, if zelle.is_some() { spalte + 1 } else { spalte }),
                };
                spalte = pos.1;
                zelle = Some(pos);
                belegt = false;
            }
            X::Leer(n, a) if lokal(&n) == "c" => {
                if let Some(p) = attr(&a, "r").and_then(zellbezug) {
                    spalte = p.1;
                }
                zelle = Some((zeile, spalte));
            }
            X::Start(n, _) | X::Leer(n, _)
                if zelle.is_some() && matches!(lokal(&n), "v" | "is" | "f") =>
            {
                belegt = true;
            }
            X::Ende(n) if lokal(&n) == "c" => {
                if let (Some((z, s)), true) = (zelle, belegt) {
                    f.zelle(z, s);
                    f.pruefen()?;
                }
                belegt = false;
            }
            X::Eof => break,
            _ => {}
        }
    }
    f.pruefen()
}

fn wiederholt(a: &[(String, String)], name: &str) -> Result<u64, ExtraktFehler> {
    match attr(a, name) {
        None => Ok(1),
        Some(v) => v
            .trim()
            .parse::<u64>()
            .map(|n| n.max(1))
            .map_err(|_| ExtraktFehler::neu("tabelle")),
    }
}

/// ODS `content.xml`: je Tabelle Zeilen- und Spaltenzahl mit Wiederholungen. Belegt ist eine Zelle mit
/// Werttyp, Formel oder Absatz; leere Wiederholungen am Ende zählen nicht (calamine schneidet sie ab),
/// leere Wiederholungen zwischen belegten Zellen/Zeilen dagegen schon (sie liegen in der Fläche).
fn ods_pruefen(xml: &[u8]) -> Result<(), ExtraktFehler> {
    struct Lage {
        f: Flaeche,
        zeile: u64,
        spalte: u64,
        zeile_wdh: u64,
    }
    impl Lage {
        fn zelle_fertig(&mut self, wdh: u64, belegt: bool) -> Result<(), ExtraktFehler> {
            if belegt {
                self.f
                    .zelle(self.zeile, self.spalte.saturating_add(wdh - 1));
                self.f
                    .zelle(self.zeile.saturating_add(self.zeile_wdh - 1), self.spalte);
                self.f.pruefen()?;
            }
            self.spalte = self.spalte.saturating_add(wdh);
            Ok(())
        }
    }
    let mut r = XmlLeser::neu(xml);
    let mut l = Lage {
        f: Flaeche::default(),
        zeile: 0,
        spalte: 0,
        zeile_wdh: 1,
    };
    let mut in_tabelle = false;
    // Offene Zelle: (Wiederholungen, belegt, Tiefe darunter).
    let mut zelle: Option<(u64, bool, u32)> = None;
    loop {
        let ev = r.naechstes()?;
        if let Some((wdh, belegt, tiefe)) = zelle.as_mut() {
            match &ev {
                X::Start(n, _) => {
                    *belegt |= lokal(n) == "p";
                    *tiefe += 1;
                }
                X::Leer(n, _) => *belegt |= lokal(n) == "p",
                X::Ende(_) if *tiefe > 0 => *tiefe -= 1,
                X::Ende(_) => {
                    let (w, b) = (*wdh, *belegt);
                    zelle = None;
                    l.zelle_fertig(w, b)?;
                }
                X::Eof => break,
                X::Text(_) => {}
            }
            continue;
        }
        match ev {
            X::Start(n, _) if lokal(&n) == "table" => {
                l.f.pruefen()?;
                l.f = Flaeche::default();
                l.zeile = 0;
                in_tabelle = true;
            }
            X::Ende(n) if lokal(&n) == "table" => {
                l.f.pruefen()?;
                in_tabelle = false;
            }
            X::Start(n, a) if in_tabelle && lokal(&n) == "table-row" => {
                l.zeile_wdh = wiederholt(&a, "table:number-rows-repeated")?;
                l.spalte = 0;
            }
            X::Leer(n, a) if in_tabelle && lokal(&n) == "table-row" => {
                l.zeile = l
                    .zeile
                    .saturating_add(wiederholt(&a, "table:number-rows-repeated")?);
            }
            X::Ende(n) if in_tabelle && lokal(&n) == "table-row" => {
                l.zeile = l.zeile.saturating_add(l.zeile_wdh);
                l.zeile_wdh = 1;
            }
            X::Start(n, a)
                if in_tabelle && matches!(lokal(&n), "table-cell" | "covered-table-cell") =>
            {
                let w = wiederholt(&a, "table:number-columns-repeated")?;
                let belegt =
                    attr(&a, "office:value-type").is_some() || attr(&a, "table:formula").is_some();
                zelle = Some((w, belegt, 0));
            }
            X::Leer(n, a)
                if in_tabelle && matches!(lokal(&n), "table-cell" | "covered-table-cell") =>
            {
                let w = wiederholt(&a, "table:number-columns-repeated")?;
                let belegt =
                    attr(&a, "office:value-type").is_some() || attr(&a, "table:formula").is_some();
                l.zelle_fertig(w, belegt)?;
            }
            X::Eof => break,
            _ => {}
        }
    }
    l.f.pruefen()
}

/// Prüft eine XML-Datei des Archivs vollständig (DOCTYPE, Entitäten, Wohlgeformtheit soweit gelesen).
fn xml_pruefen(xml: &[u8]) -> Result<(), ExtraktFehler> {
    let mut r = XmlLeser::neu(xml);
    while !matches!(r.naechstes()?, X::Eof) {}
    Ok(())
}

fn ist_xml(n: &str) -> bool {
    let k = n.to_ascii_lowercase();
    k.ends_with(".xml") || k.ends_with(".rels")
}

/// Liest alle Teile begrenzt, prüft sie und packt sie unkomprimiert neu (Eingabe für calamine).
fn geprueft_neu_packen(bytes: &[u8], xlsx: bool) -> Result<Vec<u8>, ExtraktFehler> {
    let mut z = ZipLeser::oeffnen(bytes)?;
    let mut aus = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opt = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(false);
    let kaputt = |_| ExtraktFehler::neu("tabelle");
    for n in z.namen() {
        if n.ends_with('/') {
            continue;
        }
        let Some(b) = z.lesen(&n)? else {
            continue;
        };
        if ist_xml(&n) {
            let k = n.to_ascii_lowercase();
            if xlsx && k.starts_with("xl/worksheets/") && k.ends_with(".xml") {
                xlsx_blatt_pruefen(&b)?;
            } else if !xlsx && k == "content.xml" {
                ods_pruefen(&b)?;
            } else {
                xml_pruefen(&b)?;
            }
        }
        aus.start_file(n.as_str(), opt).map_err(kaputt)?;
        aus.write_all(&b)
            .map_err(|_| ExtraktFehler::neu("tabelle"))?;
    }
    Ok(aus.finish().map_err(kaputt)?.into_inner())
}

pub fn tabelle(bytes: &[u8]) -> Result<Vec<Abschnitt>, ExtraktFehler> {
    let namen = ZipLeser::oeffnen(bytes)?.namen();
    let xlsx = namen.iter().any(|n| n == "xl/workbook.xml");
    if !xlsx && !namen.iter().any(|n| n == "content.xml") {
        return Err(ExtraktFehler::neu("format"));
    }
    let geprueft = geprueft_neu_packen(bytes, xlsx)?;
    let mut aus = Vec::new();
    if xlsx {
        let mut wb: Xlsx<_> = open_workbook_from_rs(Cursor::new(geprueft.as_slice()))
            .map_err(|_| ExtraktFehler::neu("tabelle"))?;
        for n in wb.sheet_names() {
            let r = wb
                .worksheet_range(&n)
                .map_err(|_| ExtraktFehler::neu("tabelle"))?;
            blatt(&n, &r, &mut aus)?;
        }
    } else {
        let mut wb: Ods<_> = open_workbook_from_rs(Cursor::new(geprueft.as_slice()))
            .map_err(|_| ExtraktFehler::neu("tabelle"))?;
        for n in wb.sheet_names() {
            let r = wb
                .worksheet_range(&n)
                .map_err(|_| ExtraktFehler::neu("tabelle"))?;
            blatt(&n, &r, &mut aus)?;
        }
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
