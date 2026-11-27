//! Buchregeln ohne Modell; alle Bereiche beziehen sich auf die unveraenderte Basis.

use std::collections::{HashMap, HashSet};
use crate::aufbereitung::{Aenderung, AenderungArt, AenderungStatus, DokumentArt};
use super::sprache::vertrag::{zeilen_text, zeilen_zerlegen};

super::re!(SEITENMARKE, r"\{\{S\.\s*\d+\}\}");

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize, specta::Type, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
#[specta(rename = "AufbereitungQuelle")]
pub enum Quelle { Buch, Seite, Transkript }

pub fn pruefe_tidy(text: &str, angegeben: Option<Quelle>) -> Result<(), String> {
    if angegeben == Some(Quelle::Buch) || quelle(text, None) == Quelle::Buch {
        return Err("book_requires_proposals".into());
    }
    Ok(())
}

pub fn quelle(text: &str, art: Option<DokumentArt>) -> Quelle {
    match art {
        Some(DokumentArt::Buch) => Quelle::Buch,
        Some(DokumentArt::Gespraech) => Quelle::Transkript,
        Some(DokumentArt::Seite) => Quelle::Seite,
        None if SEITENMARKE.is_match(text) => Quelle::Buch,
        None if text.lines().take(100).any(|l| {
            let l = l.trim_start_matches('[');
            let b = l.as_bytes();
            b.len() >= 8 && b[0..2].iter().all(u8::is_ascii_digit)
                && b[2] == b':' && b[3..5].iter().all(u8::is_ascii_digit)
                && b[5] == b':' && b[6..8].iter().all(u8::is_ascii_digit)
        }) => Quelle::Transkript,
        None => Quelle::Seite,
    }
}

/// Markdown-Kapitel aus der Importstruktur; Vorspann bleibt ein eigenes Kapitel.
pub fn kapitel(text: &str) -> Vec<[usize; 2]> {
    let mut starts = vec![0];
    for z in zeilen_zerlegen(text) {
        let l = zeilen_text(text, &z).trim();
        if (l.starts_with("# ") || l.starts_with("## ")) && z.bereich[0] > 0 {
            if text[..z.bereich[0]].trim().is_empty() { continue; }
            starts.push(z.bereich[0]);
        }
    }
    let mut result = Vec::new();
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(text.len());
        if !text[start..end].trim().is_empty() { result.push([start, end]); }
    }
    result
}

fn normalisiert(text: &str) -> String {
    text.chars().filter(|c| !c.is_numeric()).collect::<String>()
        .split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Formatvorschlaege bleiben erhalten, soweit sie keinen Buchtreffer beruehren.
pub fn regeln_mit_format(text: &str, opt: &super::format::FormatOptionen) -> Vec<Aenderung> {
    let mut buch = regeln(text);
    let format = super::aenderungen::regel_aenderungen(text, opt).into_iter().filter(|a| {
        let n = buch.partition_point(|b| b.bereich[0] <= a.bereich[1]);
        n == 0 || buch[n - 1].bereich[1] < a.bereich[0]
    }).collect::<Vec<_>>();
    buch.extend(format);
    buch.sort_by_key(|a| a.bereich);
    buch
}

pub fn regeln(text: &str) -> Vec<Aenderung> {
    let zeilen = zeilen_zerlegen(text);
    let marken: Vec<_> = SEITENMARKE.find_iter(text).collect();
    let woerter: HashSet<_> = text.split(|c: char| !c.is_alphabetic())
        .filter(|s| !s.is_empty()).map(str::to_lowercase).collect();
    let mut vorkommen: HashMap<String, HashSet<usize>> = HashMap::new();
    let seite = |pos| marken.partition_point(|m| m.start() <= pos);
    let markenzeilen: Vec<_> = zeilen.iter().enumerate().filter_map(|(i, z)|
        SEITENMARKE.is_match(zeilen_text(text, z)).then_some(i)).collect();
    // Nur kurze Einzelzeilen kommen als Lauftitel infrage. Ganze Absaetze bleiben stehen.
    for (i, z) in zeilen.iter().enumerate() {
        let l = zeilen_text(text, z).trim();
        let p = markenzeilen.partition_point(|&m| m <= i);
        let am_rand = p > 0 && i - markenzeilen[p - 1] <= 3
            || markenzeilen.get(p).is_some_and(|&m| m - i <= 3);
        let wasserzeichen = l.eq_ignore_ascii_case("OceanofPDF.com");
        if (am_rand || wasserzeichen) && l.len() <= 160
            && (wasserzeichen || !l.ends_with(['.', '!', '?', ':', '-'])) && !l.starts_with('#')
            && !SEITENMARKE.is_match(l) {
            let key = normalisiert(l);
            if key.chars().filter(|c| c.is_alphabetic()).count() >= 4 {
                vorkommen.entry(key).or_default().insert(seite(z.bereich[0]));
            }
        }
    }
    let schutz = crate::aufbereitung::geschuetzte_bereiche(text);
    let mut schutz = schutz;
    schutz.sort_unstable();
    let mut max = 0;
    let enden: Vec<_> = schutz.iter().map(|s| { max = max.max(s.1); max }).collect();
    let mut result = Vec::new();
    let mut push = |von: usize, bis: usize, neu: &str, regel: &str| {
        let n = schutz.partition_point(|s| s.0 < bis);
        if n > 0 && enden[n - 1] > von { return; }
        let id = format!("buch-{regel}-{}", result.len());
        result.push(Aenderung { id: id.clone(), art: AenderungArt::Regel(regel.into()),
            bereich: [von, bis], alt: text[von..bis].into(), neu: neu.into(),
            grund_code: regel.into(), zaehler: Some(1), gruppe: Some(id),
            status: AenderungStatus::Offen });
    };
    let mut entfernt = vec![false; zeilen.len()];
    let nichtleer: Vec<_> = zeilen.iter().enumerate().filter_map(|(i, z)|
        (!zeilen_text(text, z).trim().is_empty()).then_some(i)).collect();
    for (i, z) in zeilen.iter().enumerate() {
        let l = zeilen_text(text, z).trim();
        if l.is_empty() || SEITENMARKE.is_match(l) { continue; }
        let wiederholt = marken.len() >= 5 && vorkommen.get(&normalisiert(l))
            .is_some_and(|p| p.len() * 5 >= marken.len());
        let position = nichtleer.partition_point(|&j| j < i);
        let neben_marke = [position.checked_sub(1), Some(position + 1)].into_iter().flatten()
            .filter_map(|j| nichtleer.get(j)).filter_map(|&j| zeilen.get(j))
            .any(|n| SEITENMARKE.is_match(zeilen_text(text, n)));
        let nummer = l.chars().all(|c| c.is_ascii_digit()) && l.len() <= 6 && neben_marke;
        if wiederholt || nummer {
            let ende = zeilen.get(i + 1).map_or(z.bereich[1], |n| n.bereich[0]);
            let regel = if nummer { "buch_seitenzahl" }
                else if l.eq_ignore_ascii_case("OceanofPDF.com") { "buch_wasserzeichen" }
                else { "buch_kopfzeile" };
            push(z.bereich[0], ende, "", regel);
            entfernt[i] = true;
        }
    }
    for (i, paar) in zeilen.windows(2).enumerate() {
        if entfernt[i] || entfernt[i + 1] { continue; }
        let a = zeilen_text(text, &paar[0]);
        let b = zeilen_text(text, &paar[1]);
        if a.trim().is_empty() || b.trim().is_empty() || a.starts_with('#') || b.starts_with('#')
            || SEITENMARKE.is_match(a) || SEITENMARKE.is_match(b)
            || super::struktur::LIST_LINE_RE.is_match(b.trim_start())
            || super::struktur::SPEAKER_LINE_RE.is_match(b.trim_start()) { continue; }
        let links = a.trim_end();
        let rechts = b.trim_start();
        let von = paar[0].bereich[0] + links.len();
        let bis = paar[1].bereich[0] + b.len() - rechts.len();
        if let Some(stamm) = links.strip_suffix('-') {
            let wort = stamm.rsplit(|c: char| !c.is_alphabetic()).next().unwrap_or("");
            let ende = rechts.split(|c: char| !c.is_alphabetic()).next().unwrap_or("");
            if !wort.is_empty() && !ende.is_empty()
                && woerter.contains(&format!("{wort}{ende}").to_lowercase()) {
                push(von - 1, bis, "", "buch_silbentrennung");
            } else if !wort.is_empty() && !ende.is_empty() {
                // Unbelegte Zusammensetzungen behalten den Bindestrich.
                let abstand = if matches!(ende, "und" | "oder" | "bzw" | "sowie" | "bis" | "als") { " " } else { "" };
                push(von, bis, abstand, "buch_umbruch");
            }
        } else if links.chars().next_back().is_some_and(|c| c.is_alphanumeric() || matches!(c, '.' | ',' | ';' | '!' | '?'))
            && rechts.chars().next().is_some_and(char::is_alphabetic) {
            push(von, bis, " ", "buch_umbruch");
        }
    }
    result.sort_by_key(|a| a.bereich);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anwenden(text: &str, a: &[Aenderung]) -> String {
        crate::aufbereitung::pruefe_aenderungen(text, a).unwrap();
        let mut neu = text.to_string();
        for a in a.iter().rev() { neu.replace_range(a.bereich[0]..a.bereich[1], &a.neu); }
        neu
    }

    #[test]
    fn synthetische_300_seiten_mit_kontrollabsaetzen() {
        let text: String = (1..=300).map(|n| format!(
            "{{{{S. {n}}}}}\n{n}\nSynthetic Book {n}\nOceanofPDF.com\n\nKontrollabsatz bleibt exakt.\n\nEin Beispielwort und Bei-\nspielwort.\n\nEin langer Satz mit\nhartem Umbruch.\n\nUnbe-\nkannt.\n\n")).collect();
        let a = regeln(&text);
        for (regel, anzahl) in [("buch_seitenzahl", 300), ("buch_kopfzeile", 300), ("buch_wasserzeichen", 300),
            ("buch_silbentrennung", 300), ("buch_umbruch", 600)] {
            assert_eq!(a.iter().filter(|a| a.grund_code == regel).count(), anzahl);
        }
        let neu = anwenden(&text, &a);
        assert_eq!(SEITENMARKE.find_iter(&neu).count(), 300);
        assert_eq!(neu.matches("Kontrollabsatz bleibt exakt.").count(), 300);
        assert_eq!(neu.matches("Unbe-kannt.").count(), 300);
        assert!(regeln("Kontrollabsatz bleibt exakt.\n\nEinmaliger Titel").is_empty());
    }

    #[test]
    fn quellen_und_kapitel_aus_importstruktur() {
        assert_eq!(quelle("{{S. 1}} Text", None), Quelle::Buch);
        assert_eq!(quelle("[00:12:34] Test", None), Quelle::Transkript);
        assert_eq!(quelle("Text", None), Quelle::Seite);
        assert_eq!(quelle("Text", Some(DokumentArt::Buch)), Quelle::Buch);
        let text = "Vorspann\n\n## Eins\nText\n\n## Zwei\nText";
        assert_eq!(kapitel(text).len(), 3);
        assert_eq!(kapitel("Text ohne Struktur"), vec![[0, 18]]);
    }

    #[test]
    fn kopfzeilen_zaehlen_seiten_statt_wiederholungen_auf_einer_seite() {
        // Gegenprobe: Zwei Treffer derselben Seite duerfen die 20-Prozent-Schwelle nicht erfuellen.
        for zweite_seite in [false, true] {
            let text: String = (1..=10).map(|n| {
                let kopf = if n == 1 { "Randtitel\nRandtitel".into() }
                    else if n == 2 && zweite_seite { "Randtitel".into() }
                    else { format!("Titel {}", char::from(b'A' + n as u8)) };
                format!("{{{{S. {n}}}}}\n{kopf}\n\nEin Kontrollabsatz.\n\n")
            }).collect();
            let a = regeln(&text);
            assert_eq!(a.iter().filter(|a| a.grund_code == "buch_kopfzeile").count(), if zweite_seite { 3 } else { 0 });
        }
    }

    #[test]
    fn ganztext_umschreiben_fuer_buecher_gesperrt() {
        assert_eq!(pruefe_tidy("{{S. 1}} Text", Some(Quelle::Seite)), Err("book_requires_proposals".into()));
        assert!(pruefe_tidy("Buch ohne Marken", Some(Quelle::Buch)).is_err());
        assert!(pruefe_tidy("Ein Seitentext.", Some(Quelle::Seite)).is_ok());
    }

    #[test]
    #[ignore = "Manueller Benchmark: cargo test buch::tests::anderthalb -- --ignored"]
    fn anderthalb_millionen_zeichen() {
        let seite = "{{S. 1}}\n\nEin synthetischer Absatz bleibt stehen.\n\n";
        let text = seite.repeat(1_500_000 / seite.chars().count() + 1);
        assert!(text.chars().count() >= 1_500_000);
        let start = std::time::Instant::now();
        let _ = regeln(&text);
        assert!(start.elapsed().as_secs_f64() < 5.0);
    }
}
