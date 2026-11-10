//! D5a-4 (lva#441, Epic #395): Paritätsvektoren des lexikalischen Index für das iPhone.
//!
//! Alle Werte entstehen aus den echten Desktop-Wegen, nicht aus Literalen: ein synthetischer Korpus (nur
//! `example.*`-Adressen, erfundene Namen) wird über `Ablage::importieren`, die Extraktoren (`extrakt::abschnitte`, wie
//! sie die Textwarteschlange aufruft; ohne den Unterprozess, der nur die Isolation liefert und auf diesem macOS nicht
//! startet: `extrakt.start`), `Ablage::text_ablegen` und `index::abgleichen_mit` in einen echten `index.db` geschrieben. Gesucht wird mit
//! `index::suchen_mit` und `index::ki_suchen`; zusätzlich fragt der Test den Index selbst mit derselben SQL-Abfrage wie
//! `wai-retrieval::rank` (FTS5, `bm25(chunks_fts)`) und liest Stücke, Stücklängen (`chunks_fts_docsize`) und Rohrang.
//! `content_tokens` und die Zerlegung des Tokenizers (`unicode61 remove_diacritics 2`) kommen aus `wai-retrieval` bzw. aus
//! dem SQLite des Index. Swift (`VoiceCore/Ablage/AblageIndex*.swift`) liest die Datei
//! `docs/superpowers/plans/ablage-index-vektoren.json` und muss dieselben Stücke, Wörter, Rohränge und Treffer liefern.
//!
//! `json_passt_zum_code` vergleicht die Datei mit dem, was der Code heute berechnet (Gleitkommazahlen mit relativer
//! Toleranz 1e-9, wegen `log` je Plattform); mit `LVA_VEKTOREN_SCHREIBEN=1` schreibt derselbe Test sie neu:
//! `LVA_VEKTOREN_SCHREIBEN=1 cargo test --lib ablage::tests_index_paritaet::json_passt_zum_code`.
//!
//! lva#463: Abschnitt `normalisierung` (Faltung ß → ss usw. vor Index und Anfrage) und `gesperrt` (Rohrang bei gemischtem
//! Bestand: Papierkorb zählt nicht in N, n und avgdl) in derselben Datei.
//!
//! Die FTS5-Tabellen (Token-Zeichen, Faltung) stammen aus dem gebündelten SQLite (Cargo.lock) und sind damit unabhängig
//! von der Rust-Version; Eigenschaften der Rust-Standardbibliothek (`is_alphanumeric`, `to_lowercase`) stehen nur als
//! Proben in der Datei, die Swift mit den Unicode-Eigenschaften des Systems nachbildet.

use super::extrakt::{self, Textfassung};
use super::freigabe::KiStufe;
use super::index;
use super::index::{KiProfil, KiWeg};
use super::semantik::AblageEinbettung;
use super::speicher::{Ablage, AblageArt, AblageImportStand};
use super::zusammenfuehren::Text;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;

const JSON_PFAD: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../docs/superpowers/plans/ablage-index-vektoren.json"
);
const WAI_RETRIEVAL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/vendor/wai-retrieval/src/lib.rs"
));
const KANDIDATEN: usize = 60;
const STELLEN_JE_DATEI: usize = 3;
const SQL_ZEILEN: usize = 60 * 50;
const KONTROLLE: &str = "Zebrafink";

// ---------------------------------------------------------------------------
// Korpus (synthetisch)
// ---------------------------------------------------------------------------

const HAUSORDNUNG: &str = "Hausordnung der Beispiel-Genossenschaft\n\
Gültig ab dem 1. Januar 2026 für alle Mieter und Gäste.\n\
\n\
Ruhezeiten: Die Nachtruhe gilt täglich von 22 Uhr bis 6 Uhr. Auch mittags zwischen 13 und 15 Uhr ist auf die Nachbarn Rücksicht zu nehmen. Musik und Heimwerken sind in dieser Zeit zu unterlassen.\n\
\n\
Müll und Wertstoffe: Müll gehört in die Tonnen im Hof, Wertstoffe in die gelben Säcke. Sperrmüll darf nicht im Treppenhaus abgestellt werden. Fragen an hausmeister@example.org\n\
\n\
Fahrräder und Kinderwagen: Fahrräder werden im Fahrradkeller abgestellt, nicht im Treppenhaus. Kinderwagen dürfen im Flur stehen, solange der Fluchtweg frei bleibt.\n";

const KUENDIGUNG: &str = "# Kündigung des Vertrags\n\
\n\
Sehr geehrte Damen und Herren,\n\
\n\
hiermit kündige ich den Vertrag zum nächstmöglichen Termin. Bitte bestätigen Sie die Kündigung schriftlich.\n\
\n\
Mit freundlichen Grüßen\n\
Max Mustermann (max@example.com)\n";

const HANDBUCH: &str = "<html><body><h1>Handbuch Beispielgerät</h1>\
<p>Dieses Handbuch beschreibt Installation und Fehlerbehebung des Geräts.</p>\
<h2>Installation</h2><p>Stecken Sie das Netzteil ein und drücken Sie die Taste für drei Sekunden. Die Anzeige leuchtet grün.</p>\
<h2>Troubleshooting</h2><p>To reset the device, hold the power button for ten seconds. If the display stays red, contact support at support@example.com.</p>\
<p>Das Gerät lässt sich über die Taste an der Rückseite auf die Werkseinstellungen zurücksetzen.</p></body></html>";

const ANGEBOT_EML: &str = "From: Anna Beispiel <anna@example.org>\r\nTo: bernd@example.com\r\nSubject: Angebot Fähre\r\n\
Date: Mon, 2 Nov 2026 10:00:00 +0100\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\
Content-Transfer-Encoding: 8bit\r\n\r\n\
Hallo Bernd,\r\nanbei das Angebot für die Fähre nach Rügen. Die Überfahrt dauert zwei Stunden. Preis pro Person 24 Euro.\r\nGruß Anna\r\n";

const PROTOKOLL: &str = "Protokoll der Sitzung vom 3. November 2026 (Beispiel-Genossenschaft)\n\
\n\
Das Budget für 2027 wurde diskutiert. Das Budget sieht Ausgaben für Heizung, Fassade und Spielplatz vor; das Budget für die Fassade wurde um zehn Prozent gekürzt, weil das Budget der Rücklage nicht ausreicht. Über das Budget des Spielplatzes wird im Frühjahr erneut abgestimmt, sobald das Budget der Stadt feststeht.\n\
\n\
Beschlüsse: Der Vorstand wird beauftragt, drei Angebote für die Fassadensanierung einzuholen. Die Angebote sind bis Ende Januar vorzulegen. Der Vorstand berichtet in der nächsten Sitzung über das Ergebnis und schlägt eine Reihenfolge der Maßnahmen vor.\n\
\n\
Verschiedenes: Die Kündigungsfrist für den Wartungsvertrag der Aufzüge beträgt sechs Monate; eine Kündigung muss rechtzeitig vorbereitet werden.\n";

const CAFE: &str = "Café de la Gare\n\
Crème brûlée, Œuf cocotte und Zoë's Quiche.\n\
\n\
Unsere Adresse: Straße der Pariser Kommune 5. Inhaber: Müller und Mueller GbR.\n\
\n\
Gäste aus Łódź, Gdańsk, Ærø und Ørsted-Allee: naïve Résumé-Mappe mit ﬁnal-Preis. İstanbul-Gebäck ab 3 €.\n";

const REZEPTE: &str = "# Pancakes\n\
\n\
Mix the flour, milk and eggs. Fry in butter until golden. Serve with maple syrup.\n\
\n\
# Soup\n\
\n\
Simmer onions and carrots for twenty minutes. Season with salt and pepper.\n";

const INVOICE: &str = "Invoice 2026-114\n\
Payment terms: net 30 days. Please transfer the amount to the account of Example Ltd. Late payment fees apply after the due date. Contact: billing@example.com.\n";

const GRIECHISCH: &str = "Привет мир! Ёлка и Ёж.\n\
\n\
Όμηρος και Σίσυφος. ΌΣΟΣ ο ΣΟΦΟΣ. Ελλάδα.\n";

const STOPWOERTER: &str = "Der die das und oder ist sind wie was wer.\n\
\n\
Nur das Wort Zugvogel steht hier wirklich, dazu Zugvogel und noch ein Zugvogel.\n";

const NEBENKOSTEN: &str = "Nebenkostenabrechnung 2025: Heizung, Wasser, Müllabfuhr und Hausmeister.\n\
Die Abrechnung erfolgt jährlich, Nachzahlungen sind innerhalb von vier Wochen fällig. Die Kündigungsfrist für Wartungsverträge beträgt ein Monat.\n";

fn zip_bauen(eintraege: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opt = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (n, b) in eintraege {
            z.start_file(*n, opt).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn docx(absaetze: &[(Option<&str>, &str)]) -> Vec<u8> {
    let mut body = String::new();
    for (stil, t) in absaetze {
        body.push_str("<w:p>");
        if let Some(s) = stil {
            body.push_str(&format!("<w:pPr><w:pStyle w:val=\"{s}\"/></w:pPr>"));
        }
        body.push_str(&format!(
            "<w:r><w:t xml:space=\"preserve\">{t}</w:t></w:r></w:p>"
        ));
    }
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
         <w:body>{body}</w:body></w:document>"
    );
    zip_bauen(&[
        ("[Content_Types].xml", b"<?xml version=\"1.0\"?><Types/>"),
        ("word/document.xml", xml.as_bytes()),
    ])
}

fn mietvertrag() -> Vec<u8> {
    docx(&[
        (Some("Heading1"), "Mietvertrag Beispielstraße 12"),
        (None, "Zwischen der Beispiel-Genossenschaft und dem Mieter Max Mustermann wird folgender Vertrag geschlossen."),
        (Some("Heading2"), "§ 3 Miete und Nebenkosten"),
        (None, "Die Kaltmiete beträgt 650 Euro im Monat. Die Nebenkosten werden jährlich abgerechnet."),
        (Some("Heading2"), "§ 7 Kündigung"),
        (None, "Die Kündigungsfrist beträgt für den Mieter drei Monate. Eine Kündigung muss schriftlich erfolgen."),
        (Some("Heading2"), "§ 8 Schönheitsreparaturen"),
        (None, "Der Mieter übernimmt keine Schönheitsreparaturen. Über Ausnahmen entscheidet die Vermieterin schriftlich."),
    ])
}

fn korpus() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("Hausordnung.txt", HAUSORDNUNG.as_bytes().to_vec()),
        ("Mietvertrag.docx", mietvertrag()),
        ("Kuendigung.md", KUENDIGUNG.as_bytes().to_vec()),
        ("Handbuch.html", HANDBUCH.as_bytes().to_vec()),
        ("Angebot.eml", ANGEBOT_EML.as_bytes().to_vec()),
        ("Protokoll.txt", PROTOKOLL.as_bytes().to_vec()),
        ("Cafe.txt", CAFE.as_bytes().to_vec()),
        ("Rezepte.md", REZEPTE.as_bytes().to_vec()),
        ("Invoice.txt", INVOICE.as_bytes().to_vec()),
        ("Griechisch.txt", GRIECHISCH.as_bytes().to_vec()),
        ("Stopwoerter.txt", STOPWOERTER.as_bytes().to_vec()),
        ("Nebenkosten.txt", NEBENKOSTEN.as_bytes().to_vec()),
    ]
}

/// Musterfragen (Spec §9 D5, Abnahme 5): Stoppwörter, Diakritika, `ß`, Duplikate (Kappung je Datei), nur Stoppwörter,
/// griechisches Sigma, Satzzeichen, Zahlen, über 200 Zeichen.
fn fragen() -> Vec<String> {
    let lang = format!(
        "{} Zugvogel",
        "Ruhezeiten im Haus und Nachtruhe für alle Mieter der Genossenschaft bitte beachten "
            .repeat(3)
    );
    assert!(
        lang.chars().count() > 200,
        "die lange Frage muss über 200 Zeichen haben"
    );
    vec![
        "Wann gelten die Ruhezeiten im Haus?".into(),
        "Kündigungsfrist".into(),
        "KUNDIGUNG".into(),
        "Kündigungsfrist Vertrag Laufzeit".into(),
        "Straße".into(),
        "Strasse".into(),
        "creme brulee".into(),
        "Zoe Chloe cafe".into(),
        "Mueller Müller".into(),
        "reset the device power button".into(),
        "payment terms invoice".into(),
        "Müll Tonnen Säcke Treppenhaus".into(),
        "Budget Budget Budget".into(),
        "the and of der die das".into(),
        "Angebot Fähre Überfahrt".into(),
        "ЁЛКА привет".into(),
        "όσος σισυφος".into(),
        "Ruhe-Zeiten l'étage".into(),
        "Nachtruhe 22 Uhr".into(),
        "Zugvogel".into(),
        lang,
        "Fassade Angebote Vorstand Kündigung".into(),
        "STRASSE".into(),
        "Pariser Strasse".into(),
        "Beispielstrasse".into(),
        "Lodz Gdansk".into(),
        "Aero Orsted".into(),
    ]
}

/// Proben für `content_tokens` (Rust) und die FTS5-Zerlegung.
fn proben() -> Vec<String> {
    let mut v: Vec<String> = [
        "Wann gelten die Ruhezeiten im Haus?",
        "der die das den dem des ein eine einer einen und oder ist sind wie was wer wo wann im in am an auf zu zum zur",
        "von vom mit bei fuer für es sie er wir ihr nicht auch wird werden hat haben nach als ab",
        "Der Hund und die Katze sind nicht im Haus",
        "the and of a to in is it",
        "Ärger Öl Übung ÄÖÜ äöü ß ẞ Straße STRASSE",
        "Œuvre Æther Ørsted Łódź Gdańsk İstanbul ı ŉ ǅ ǆ ﬁnal ﬀ",
        "naïve Résumé café crème brûlée Zoë Chloé",
        "Привет мир Ёлка ёж ЁЛКА",
        "Όμηρος Σίσυφος ΌΣΟΣ ΣΑΣ ὈΔΥΣΣΕΎΣ ς σ Σ",
        "x² x₂ co₂ ① ② ½ Ⅷ",
        "a_b a-b a'b a.b 3.14 1,000 e-mail user@example.org",
        "tab\tnew\nline  mehrere   Leerzeichen",
        "e\u{301} a\u{308} o\u{308} u\u{308}",
        "हिन्दी हिंदी ไทย ภาษาไทย",
        "日本語のテキスト 中文 한국어",
        "😀 emoji 🙂 ok",
        "ǰ ſ K K å Å",
        "",
        "   ",
        "---",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // Jedes Stoppwort einzeln und in Großschreibung.
    for w in stop_liste() {
        v.push(w.clone());
        v.push(w.to_uppercase());
    }
    v
}

/// Die Stoppwörter aus dem Quelltext von `wai-retrieval` (die Konstante ist nicht öffentlich).
fn stop_liste() -> Vec<String> {
    let start = WAI_RETRIEVAL
        .find("const STOP: &[&str] = &[")
        .expect("STOP fehlt");
    let rest = &WAI_RETRIEVAL[start..];
    let ende = rest.find("];").expect("STOP nicht beendet");
    let block = &rest[..ende];
    let mut aus = Vec::new();
    let mut teile = block.split('"');
    teile.next();
    while let Some(w) = teile.next() {
        aus.push(w.to_string());
        teile.next();
    }
    aus
}

/// Eingaben für `index::stuecke` mit den Werten der App und kleinen Werten: Leerraum aller Art (geschütztes Leerzeichen, Geviert,
/// Zeilenumbrüche), Mehrbyte-Zeichen, Wörter ohne Trenner, leere Eingaben.
fn stuecke_proben() -> Value {
    let texte: Vec<String> = vec![
        "kurz".into(),
        String::new(),
        "  \n\t ".into(),
        "Wort ".repeat(30),
        "äöü ßé€ ".repeat(20),
        "ä".repeat(70),
        "Zeile eins\nZeile zwei\r\nZeile drei\u{a0}mit geschütztem Leerzeichen\u{2003}und Geviert. ".repeat(2),
        "😀 emoji 🙂 ".repeat(12),
        "a b c d e f g h i j k l m n o p q r s t u v w x y z ".repeat(2),
        "   führender Leerraum und langes_Wort_ohne_Trenner_das_nicht_in_ein_kleines_Stueck_passt_und_noch_laenger_wird nachher mehr Text folgt".into(),
        format!("{}Ende", "x ".repeat(40)),
    ];
    let paare = [(220usize, 40usize), (30, 10), (7, 2)];
    let mut aus: Vec<Value> = Vec::new();
    for t in &texte {
        for (m, u) in paare {
            aus.push(json!({"text": t, "max": m, "ueber": u, "stuecke": index::stuecke(t, m, u)}));
        }
    }
    Value::Array(aus)
}

// ---------------------------------------------------------------------------
// FTS5 (SQLite des Index): Zerlegung und Tabellen
// ---------------------------------------------------------------------------

/// Zerlegt Texte wie der Tokenizer des Index (`unicode61 remove_diacritics 2`): je Text die Wörter in Reihenfolge.
fn fts5_zerlegen(c: &Connection, texte: &[String]) -> Vec<Vec<String>> {
    c.execute_batch("DELETE FROM t").unwrap();
    {
        let mut ins = c
            .prepare("INSERT INTO t(rowid, text) VALUES(?1, ?2)")
            .unwrap();
        for (i, t) in texte.iter().enumerate() {
            ins.execute(params![i as i64 + 1, t]).unwrap();
        }
    }
    let mut q = c.prepare("SELECT doc, \"offset\", term FROM v").unwrap();
    let mut zeilen: Vec<(i64, i64, String)> = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    zeilen.sort();
    let mut aus = vec![Vec::new(); texte.len()];
    for (doc, _, term) in zeilen {
        aus[doc as usize - 1].push(term);
    }
    aus
}

fn fts5_verbindung() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(text, tokenize='unicode61 remove_diacritics 2');
         CREATE VIRTUAL TABLE v USING fts5vocab(t, 'instance');",
    )
    .unwrap();
    c
}

fn hex(n: u32) -> String {
    format!("{n:x}")
}

/// `a-b,c-d,e` (hex, aufsteigend) für sortierte Code-Punkte.
fn bereiche(punkte: &[u32]) -> String {
    let mut teile: Vec<String> = Vec::new();
    let mut i = 0;
    while i < punkte.len() {
        let mut j = i;
        while j + 1 < punkte.len() && punkte[j + 1] == punkte[j] + 1 {
            j += 1;
        }
        teile.push(if i == j {
            hex(punkte[i])
        } else {
            format!("{}-{}", hex(punkte[i]), hex(punkte[j]))
        });
        i = j + 1;
    }
    teile.join(",")
}

/// Läufe `start:anzahl:schritt:delta` (alle Zahlen dezimal) für sortierte `(code, ziel)`-Paare mit `ziel != code`.
fn laeufe(paare: &[(u32, u32)]) -> String {
    let mut teile: Vec<String> = Vec::new();
    let mut i = 0;
    while i < paare.len() {
        let (c, z) = paare[i];
        let delta = z as i64 - c as i64;
        let mut anzahl = 1usize;
        let mut schritt = 1u32;
        if i + 1 < paare.len() {
            let (c2, z2) = paare[i + 1];
            let s = c2 - c;
            if z2 as i64 - c2 as i64 == delta && s <= 2 {
                schritt = s;
                anzahl = 2;
                while i + anzahl < paare.len() {
                    let (cn, zn) = paare[i + anzahl];
                    if cn - paare[i + anzahl - 1].0 == schritt && zn as i64 - cn as i64 == delta {
                        anzahl += 1;
                    } else {
                        break;
                    }
                }
            }
        }
        teile.push(format!("{c}:{anzahl}:{schritt}:{delta}"));
        i += anzahl;
    }
    teile.join(",")
}

/// Verhalten des Tokenizers für jeden Code-Punkt (ohne Ersatzzeichen und NUL), aus dem SQLite des Index:
/// Zeichen, die ein Wort beginnen und fortsetzen (`token_zeichen`), Zeichen, die nur ein Wort fortsetzen (Diakritika,
/// `folge_zeichen`), Faltung (`falten`, Läufe) und Zeichen, die verschwinden (`entfernen`).
fn fts5_tabellen() -> Value {
    let c = fts5_verbindung();
    const BATCH: u32 = 16384;
    let mut token: Vec<u32> = Vec::new();
    let mut folge: Vec<u32> = Vec::new();
    let mut falten: Vec<(u32, u32)> = Vec::new();
    let mut entfernen: Vec<u32> = Vec::new();
    let mut start = 1u32;
    while start <= 0x10FFFF {
        let ende = (start + BATCH).min(0x110000);
        let codes: Vec<(u32, char)> = (start..ende)
            .filter_map(|n| char::from_u32(n).map(|ch| (n, ch)))
            .collect();
        let mut text = String::new();
        for (_, ch) in &codes {
            // A: Zeichen am Wortanfang ("Xq"), B: Zeichen mitten im Wort ("qXq"), getrennt durch die Marke "mmm".
            text.push_str(&format!(" mmm {ch}q mmm q{ch}q"));
        }
        let zerlegt = fts5_zerlegen(&c, &[text]);
        let mut segmente: Vec<Vec<String>> = Vec::new();
        for t in &zerlegt[0] {
            if t == "mmm" {
                segmente.push(Vec::new());
            } else {
                segmente
                    .last_mut()
                    .expect("Wort vor der Marke")
                    .push(t.clone());
            }
        }
        assert_eq!(segmente.len(), codes.len() * 2);
        for (i, (n, ch)) in codes.iter().enumerate() {
            let a = &segmente[2 * i];
            let b = &segmente[2 * i + 1];
            let beginnt = !(a.len() == 1 && a[0] == "q");
            let in_wort = b.len() == 1;
            if beginnt {
                assert!(a.len() == 1 && a[0].ends_with('q'), "U+{n:X}: {a:?}");
                assert_eq!(b.len(), 1, "U+{n:X}: {b:?}");
                token.push(*n);
            } else if in_wort {
                folge.push(*n);
            } else {
                assert_eq!(b, &["q".to_string(), "q".to_string()], "U+{n:X}: {b:?}");
                continue;
            }
            let mitte = &b[0][1..b[0].len() - 1];
            let ziel: Vec<char> = mitte.chars().collect();
            match ziel.len() {
                0 => entfernen.push(*n),
                1 => {
                    if ziel[0] != *ch {
                        falten.push((*n, ziel[0] as u32));
                    }
                }
                _ => panic!("U+{n:X} faltet zu mehreren Zeichen: {mitte:?}"),
            }
        }
        start = ende;
    }
    json!({
        "hinweis": "Verhalten von FTS5 unicode61 remove_diacritics 2 je Code-Punkt (SQLite des Index). token_zeichen: beginnen und setzen ein Wort fort (Hex-Bereiche). folge_zeichen: setzen nur ein Wort fort (Diakritika). falten: Läufe start:anzahl:schritt:delta (dezimal) vom Code-Punkt zum gefalteten. entfernen: Zeichen, die im Wort verschwinden (Hex-Bereiche). Alle anderen Code-Punkte trennen Wörter; nicht aufgeführte Zeichen falten auf sich selbst.",
        "token_zeichen": bereiche(&token),
        "folge_zeichen": bereiche(&folge),
        "falten": laeufe(&falten),
        "entfernen": bereiche(&entfernen),
    })
}

// ---------------------------------------------------------------------------
// Index des Desktops lesen
// ---------------------------------------------------------------------------

struct Env {
    d: tempfile::TempDir,
    a: Ablage,
}

fn env() -> Env {
    let d = tempfile::tempdir().unwrap();
    let a = Ablage::neu(d.path().join("ablage"));
    assert_eq!(a.wartung(), None);
    Env { d, a }
}

impl Env {
    fn standard(&self) -> String {
        self.a.ansicht().unwrap().ordner[0].id.clone()
    }

    fn ablegen(&self, name: &str, bytes: &[u8]) -> String {
        let q = self.d.path().join("q").join(name);
        fs::create_dir_all(q.parent().unwrap()).unwrap();
        fs::write(&q, bytes).unwrap();
        let z = self.a.importieren(&q, &self.standard(), None).unwrap();
        assert_eq!(z[0].stand, AblageImportStand::Abgelegt, "{:?}", z[0]);
        z[0].dat.clone().unwrap()
    }

    /// Wie die Textwarteschlange, nur im Prozess: für jede Fassung ohne Text die Abschnitte extrahieren, die Textfassung
    /// ablegen und eintragen (`text_ablegen`), danach den Index abgleichen.
    fn text_und_index(&self) {
        for auf in self.a.text_auftraege().unwrap() {
            let bytes = fs::read(&auf.pfad).unwrap();
            let format = extrakt::format_aus_typ(&auf.typ).expect("Format mit Textfassung");
            let (id, version) = extrakt::extraktor(format);
            let tf = Textfassung {
                marken_version: extrakt::MARKEN_VERSION,
                textrev: auf.textrev.clone(),
                original_sha256: auf.sha256.clone(),
                extraktor: id.to_string(),
                extraktor_version: version.to_string(),
                abschnitte: extrakt::abschnitte(&bytes, format).expect("Extraktion"),
            };
            assert!(!tf.abschnitte.is_empty());
            let text = Text {
                textrev: tf.textrev.clone(),
                blob: String::new(),
                size: 0,
                extraktor: tf.extraktor.clone(),
                extraktor_version: tf.extraktor_version.clone(),
                marken_version: tf.marken_version,
                stand: "ok".to_string(),
                code: None,
                versuche: 0,
                versuche_zeit: 0,
            };
            let json = serde_json::to_vec(&tf).unwrap();
            self.a
                .text_ablegen(&auf.dat, &auf.fas, Some(&json), text)
                .unwrap();
        }
        index::abgleichen_mit(&self.a, &AblageEinbettung::Keine).unwrap();
    }

    fn db(&self) -> Connection {
        Connection::open(self.a.dir().join(index::INDEX)).unwrap()
    }

    fn namen(&self) -> BTreeMap<String, String> {
        self.a
            .ansicht()
            .unwrap()
            .dateien
            .into_iter()
            .map(|d| (d.id, d.name))
            .collect()
    }
}

struct Stueck {
    dat: String,
    name: String,
    seite: i64,
    abschnitt: i64,
    anker: String,
    pfad: Vec<String>,
    text: String,
    id: i64,
    laenge: i64,
}

/// SQLite-Varint (so legt FTS5 `chunks_fts_docsize.sz` ab).
fn varint(b: &[u8]) -> i64 {
    let mut n: i64 = 0;
    for (i, x) in b.iter().enumerate() {
        if i == 8 {
            return (n << 8) | *x as i64;
        }
        n = (n << 7) | (*x & 0x7f) as i64;
        if *x & 0x80 == 0 {
            break;
        }
    }
    n
}

fn stuecke_lesen(e: &Env) -> Vec<Stueck> {
    let c = e.db();
    let namen = e.namen();
    let mut laengen: BTreeMap<i64, i64> = BTreeMap::new();
    {
        let mut q = c.prepare("SELECT id, sz FROM chunks_fts_docsize").unwrap();
        let z = q
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
            .unwrap();
        for r in z {
            let (id, sz) = r.unwrap();
            laengen.insert(id, varint(&sz));
        }
    }
    let mut q = c
        .prepare(
            "SELECT s.dat, s.seite, s.abschnitt, s.anker, s.pfad, c.id, c.text
             FROM ablage_stueck s JOIN chunks c ON c.source_id = s.dat AND c.page = s.seite
             ORDER BY s.dat, s.seite, c.start_off",
        )
        .unwrap();
    let z = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .unwrap();
    let mut aus: Vec<Stueck> = Vec::new();
    for r in z {
        let (dat, seite, abschnitt, anker, pfad, id, text) = r.unwrap();
        assert!(
            aus.last()
                .map_or(true, |l| !(l.dat == dat && l.seite == seite)),
            "genau ein Stück je Seite erwartet"
        );
        aus.push(Stueck {
            name: namen[&dat].clone(),
            dat,
            seite,
            abschnitt,
            anker,
            pfad: serde_json::from_str(&pfad).unwrap(),
            text,
            id,
            laenge: laengen[&id],
        });
    }
    aus
}

/// Zeilen der lexikalischen Rangliste von `wai-retrieval::rank` (mit Rohrang), beste zuerst, für den Leser der Suche.
fn bm25_zeilen(a: &Ablage, anfrage: &str) -> Vec<(i64, String, i64, f64)> {
    index::lexikalisch_zeilen(a, anfrage, index::LESER_SUCHE, SQL_ZEILEN)
        .unwrap()
        .into_iter()
        .map(|z| (z.chunk_id, z.source_id, z.page as i64, z.bm25))
        .collect()
}

/// Gegenprobe: `bm25(chunks_fts)` von FTS5 (Zählung über ALLE Stücke der Tabelle) für dieselben gefalteten Wörter, mit dem
/// Rechtefilter des Lesers der Suche.
fn fts5_bm25_zeilen(c: &Connection, anfrage: &str) -> Vec<(i64, String, i64, f64)> {
    let tokens = wai_retrieval::content_tokens(anfrage);
    if tokens.is_empty() {
        return vec![];
    }
    let m = tokens
        .iter()
        .map(|t| {
            format!(
                "\"{}\"",
                wai_retrieval::fts_normalisieren(t).replace('"', "")
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    let sql = format!(
        "SELECT c.id, c.source_id, c.page, bm25(chunks_fts) FROM chunks_fts f JOIN chunks c ON c.id = f.rowid \
         JOIN docs d ON d.source_id = c.source_id WHERE chunks_fts MATCH ?3 AND d.tenant = ?1 AND \
         (d.acl_mode = 'tenant' OR EXISTS (SELECT 1 FROM acl a WHERE a.source_id = d.source_id AND a.principal = ?2)) \
         ORDER BY bm25(chunks_fts), c.id LIMIT {SQL_ZEILEN}"
    );
    let mut q = c.prepare(&sql).unwrap();
    q.query_map(params![index::MANDANT, index::LESER_SUCHE, m], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

/// Kappung je Datei und Kandidatenzahl wie in `rank`.
fn kappen(zeilen: &[(i64, String, i64, f64)]) -> Vec<&(i64, String, i64, f64)> {
    let mut je: BTreeMap<&str, usize> = BTreeMap::new();
    let mut aus = Vec::new();
    for z in zeilen {
        let n = je.entry(z.1.as_str()).or_default();
        if *n >= STELLEN_JE_DATEI {
            continue;
        }
        *n += 1;
        if aus.len() >= KANDIDATEN {
            break;
        }
        aus.push(z);
    }
    aus
}

// ---------------------------------------------------------------------------
// Vektoren
// ---------------------------------------------------------------------------

fn alle() -> Value {
    let e = env();
    let mut dats: BTreeMap<String, String> = BTreeMap::new();
    for (name, bytes) in korpus() {
        let dat = e.ablegen(name, &bytes);
        dats.insert(name.to_string(), dat);
    }
    e.text_und_index();

    // Textfassungen und Stücke.
    let quellen = e.a.index_quellen().unwrap();
    let stuecke = stuecke_lesen(&e);
    let mut korpus_json: Vec<Value> = Vec::new();
    let mut gesamt_stuecke = 0usize;
    let mut gesamt_tokens = 0i64;
    for (name, dat) in &dats {
        let q = quellen.iter().find(|q| &q.dat == dat).expect("Quelle");
        let rev = q.textrev.clone().expect("Textfassung vorhanden");
        let tf: Textfassung =
            serde_json::from_slice(&fs::read(e.a.text_pfad(dat, &rev)).unwrap()).unwrap();
        // Die Stücke des Index sind genau die Stücke der echten Funktion `stuecke`.
        let mut erwartet: Vec<(usize, String)> = Vec::new();
        for (i, ab) in tf.abschnitte.iter().enumerate() {
            for s in index::stuecke(&ab.text, index::STUECK, index::UEBERLAPPUNG) {
                erwartet.push((i, s));
            }
        }
        let im_index: Vec<&Stueck> = stuecke.iter().filter(|s| &s.dat == dat).collect();
        assert_eq!(
            erwartet.len(),
            im_index.len(),
            "{name}: Zahl der Stücke weicht ab"
        );
        for (x, y) in erwartet.iter().zip(&im_index) {
            assert_eq!(x.1, y.text, "{name}: Stück weicht ab");
            assert_eq!(x.0 as i64, y.abschnitt);
        }
        gesamt_stuecke += im_index.len();
        gesamt_tokens += im_index.iter().map(|s| s.laenge).sum::<i64>();
        let typ = name.rsplit('.').next().unwrap();
        korpus_json.push(json!({
            "name": name,
            "typ": typ,
            "abschnitte": tf.abschnitte.iter().map(|a| json!({"anker": a.anker, "pfad": a.pfad, "text": a.text})).collect::<Vec<_>>(),
            "stuecke": im_index.iter().map(|s| json!({
                "seite": s.seite, "abschnitt": s.abschnitt, "anker": s.anker, "pfad": s.pfad, "text": s.text, "laenge": s.laenge,
            })).collect::<Vec<_>>(),
        }));
    }
    let zeilen_gesamt: i64 = e
        .db()
        .query_row("SELECT COUNT(*) FROM chunks_fts_docsize", [], |r| r.get(0))
        .unwrap();
    assert_eq!(zeilen_gesamt as usize, gesamt_stuecke);

    // Fragen.
    let namen = e.namen();
    let leser_name = |dat: &str| namen[dat].clone();
    let seite_von = |dat: &str, text: &str| -> i64 {
        stuecke
            .iter()
            .find(|s| s.dat == dat && s.text == text)
            .map(|s| s.seite)
            .expect("Stück")
    };
    let mut fragen_json: Vec<Value> = Vec::new();
    for frage in fragen() {
        let abgeschnitten: String = frage.chars().take(200).collect();
        let tokens = wai_retrieval::content_tokens(&abgeschnitten);
        let zeilen = bm25_zeilen(&e.a, &abgeschnitten);
        // Gleichstand im Rohrang würde von der (zufälligen) Reihenfolge der Kennungen abhängen: nicht erlaubt.
        for w in zeilen.windows(2) {
            let (a, b) = (w[0].3, w[1].3);
            assert!(
                (a - b).abs() > 1e-9 * a.abs().max(1.0),
                "Gleichstand im Rohrang bei {frage:?}: Korpus ändern"
            );
        }
        let gekappt = kappen(&zeilen);
        // Die eigene Abfrage entspricht dem echten Weg.
        let ki = index::ki_suchen(
            &e.a,
            &abgeschnitten,
            &AblageEinbettung::Keine,
            index::LESER_SUCHE,
            KANDIDATEN,
        )
        .unwrap();
        assert_eq!(ki.stellen.len(), gekappt.len(), "{frage:?}");
        for (s, z) in ki.stellen.iter().zip(&gekappt) {
            assert_eq!(s.dat, z.1, "{frage:?}");
            assert_eq!(seite_von(&s.dat, &s.text), z.2, "{frage:?}");
        }
        let treffer = index::suchen_mit(&e.a, &frage, &AblageEinbettung::Keine).unwrap();
        let stellen_json: Vec<Value> = ki
            .stellen
            .iter()
            .take(10)
            .map(|s| {
                json!({
                    "name": leser_name(&s.dat),
                    "seite": seite_von(&s.dat, &s.text),
                    "anker": s.anker,
                    "rang": s.rang,
                })
            })
            .collect();
        let bm25_json: Vec<Value> = zeilen
            .iter()
            .take(12)
            .map(|z| json!({"name": leser_name(&z.1), "seite": z.2, "wert": z.3}))
            .collect();
        let dateien_json: Vec<Value> = treffer
            .iter()
            .take(5)
            .map(|t| {
                json!({
                    "name": t.name,
                    "typ": t.typ,
                    "rang": t.rang,
                    "stellen": t.stellen.iter().map(|s| json!({"anker": s.anker, "pfad": s.pfad, "auszug": s.auszug})).collect::<Vec<_>>(),
                })
            })
            .collect();
        fragen_json.push(json!({
            "frage": frage,
            "tokens": tokens,
            "treffer_gesamt": treffer.len(),
            "dateien": dateien_json,
            "stellen": stellen_json,
            "bm25": bm25_json,
            "bm25_anzahl": zeilen.len(),
        }));
    }

    // Papierkorb: nach den Fragen (der Eintrag bliebe sonst in den Zählern des Index), mit Kontrolltext.
    let dat_alt = e.ablegen(
        "Altlasten.txt",
        format!("{KONTROLLE} im Papierkorb").as_bytes(),
    );
    e.text_und_index();
    let vorher = index::suchen_mit(&e.a, KONTROLLE, &AblageEinbettung::Keine).unwrap();
    assert!(vorher.iter().any(|t| t.dat == dat_alt));
    let nur_stopwoerter = index::suchen_mit(&e.a, "der die das", &AblageEinbettung::Keine).unwrap();
    assert!(nur_stopwoerter.is_empty());
    e.a.in_papierkorb(AblageArt::Datei, &dat_alt).unwrap();
    index::abgleichen_mit(&e.a, &AblageEinbettung::Keine).unwrap();
    let nachher = index::suchen_mit(&e.a, KONTROLLE, &AblageEinbettung::Keine).unwrap();
    assert!(
        nachher.iter().all(|t| t.dat != dat_alt),
        "Papierkorb: 0 Treffer"
    );

    // lva#463: Gemischter Bestand. Dateien im Papierkorb (für den Leser der Suche gesperrt) tragen viele Wörter der Fragen;
    // ihre Stücke zählen weder in N noch in n noch in avgdl: der Rohrang bleibt der des Bestands ohne sie.
    let gesperrt_dateien = gesperrte_dateien();
    for (name, text) in &gesperrt_dateien {
        let dat = e.ablegen(name, text.as_bytes());
        e.text_und_index();
        e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    }
    index::abgleichen_mit(&e.a, &AblageEinbettung::Keine).unwrap();
    let mut gesperrt_fragen: Vec<Value> = Vec::new();
    for (f, j) in fragen().iter().zip(&fragen_json) {
        let abgeschnitten: String = f.chars().take(200).collect();
        let zeilen = bm25_zeilen(&e.a, &abgeschnitten);
        let namen = e.namen();
        let ist: Vec<Value> = zeilen
            .iter()
            .take(12)
            .map(|z| json!({"name": namen[&z.1].clone(), "seite": z.2, "wert": z.3}))
            .collect();
        let mut abw = Vec::new();
        abweichungen(&j["bm25"], &Value::Array(ist.clone()), "$", &mut abw);
        assert!(
            abw.is_empty(),
            "gemischter Bestand ändert den Rohrang bei {f:?}: {abw:?}"
        );
        assert_eq!(zeilen.len() as u64, j["bm25_anzahl"].as_u64().unwrap());
        gesperrt_fragen.push(json!({"frage": f, "bm25_anzahl": zeilen.len(), "bm25": ist}));
    }

    // Proben.
    let proben = proben();
    let c5 = fts5_verbindung();
    let zerlegt = fts5_zerlegen(&c5, &proben);
    let proben_json: Vec<Value> = proben
        .iter()
        .zip(&zerlegt)
        .map(|(p, z)| json!({"text": p, "inhalt": wai_retrieval::content_tokens(p), "fts5": z}))
        .collect();

    json!({
        "hinweis": "Paritätsvektoren des lexikalischen Index der Dokumentenablage (D5a-4). Erzeugt von ablage::tests_index_paritaet::json_passt_zum_code aus den echten Desktop-Wegen (Import, Textwarteschlange, index.db, FTS5). Neu schreiben nur mit LVA_VEKTOREN_SCHREIBEN=1 (cargo test --lib ablage::tests_index_paritaet::json_passt_zum_code); ohne die Variable prüft der Test nur. Nicht von Hand ändern. Alle Inhalte sind synthetisch (example.*).",
        "konstanten": {
            "stueck": index::STUECK,
            "ueberlappung": index::UEBERLAPPUNG,
            "kandidaten": KANDIDATEN,
            "stellen_je_datei": STELLEN_JE_DATEI,
            "anfrage_max": 200,
            "sql_zeilen": SQL_ZEILEN,
            "bm25_k1": 1.2,
            "bm25_b": 0.75,
        },
        "stop": stop_liste(),
        "normalisierung": normalisierung_json(&c5),
        "gesperrt": json!({
            "hinweis": "Gemischter Bestand (lva#463): zusätzlich zum Korpus liegen diese Dateien im Papierkorb des Desktops (für den Leser der Suche gesperrt). Der Desktop zählt sie nicht in N, n (IDF) und avgdl; der Rohrang `bm25` je Frage ist deshalb derselbe wie ohne sie und gleich dem Rohrang von `fragen`. Der iPhone-Index enthält sie nie (Index-Tor).",
            "dateien": gesperrt_dateien.iter().map(|(n, t)| json!({"name": n, "text": t, "grund": "papierkorb"})).collect::<Vec<_>>(),
            "fragen": gesperrt_fragen,
        }),
        "proben": proben_json,
        "stuecke_proben": stuecke_proben(),
        "fts5": fts5_tabellen(),
        "korpus": korpus_json,
        "statistik": {"stuecke": gesamt_stuecke, "tokens": gesamt_tokens},
        "fragen": fragen_json,
        "papierkorb": {
            "frage": KONTROLLE,
            "treffer_vorher": vorher.len(),
            "treffer_nachher": nachher.len(),
        },
    })
}

/// Vergleicht zwei JSON-Werte; Gleitkommazahlen mit relativer Toleranz 1e-9. Liefert die ersten Abweichungen.
fn abweichungen(soll: &Value, ist: &Value, pfad: &str, aus: &mut Vec<String>) {
    if aus.len() >= 8 {
        return;
    }
    match (soll, ist) {
        (Value::Number(a), Value::Number(b)) if a.is_f64() || b.is_f64() => {
            let (x, y) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            if (x - y).abs() > 1e-9 * x.abs().max(y.abs()).max(1e-300) {
                aus.push(format!("{pfad}: {x} != {y}"));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                aus.push(format!("{pfad}: Länge {} != {}", a.len(), b.len()));
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                abweichungen(x, y, &format!("{pfad}[{i}]"), aus);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            for k in a
                .keys()
                .chain(b.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                match (a.get(k), b.get(k)) {
                    (Some(x), Some(y)) => abweichungen(x, y, &format!("{pfad}.{k}"), aus),
                    _ => aus.push(format!("{pfad}.{k}: fehlt auf einer Seite")),
                }
            }
        }
        _ => {
            if soll != ist {
                aus.push(format!("{pfad}: {soll} != {ist}"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// lva#463: Faltung (ß → ss usw.) und IDF nur über lesbare Stücke
// ---------------------------------------------------------------------------

/// Texte für die Normalisierung (Eingabe → gefaltete Form → Wörter des FTS5-Tokenizers).
fn normalisierung_proben() -> Vec<String> {
    [
        "Straße",
        "STRASSE",
        "Strasse",
        "GROẞE",
        "Maß Fuß-Ball weiß Süßigkeit",
        "ßß ẞ",
        "Æther Ærø Œuvre cœur",
        "Ørsted Øresund Łódź Łukasz",
        "Đorđe Þór Ðóra Ħal",
        "ıstanbul İstanbul Ĳssel ĳs",
        "naïve Résumé café",
        "Müller Mueller",
        "ſ ǅ ﬁnal ŉ ŋ ŧ ŀ ĸ",
        "x²  1,000",
        "",
        "ASCII only 123",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn normalisierung_json(c5: &Connection) -> Value {
    let proben = normalisierung_proben();
    let norm: Vec<String> = proben
        .iter()
        .map(|p| wai_retrieval::fts_normalisieren(p))
        .collect();
    let zerlegt = fts5_zerlegen(c5, &norm);
    json!({
        "hinweis": "lva#463: Vor dem Indexieren UND bei der Anfrage ersetzt der Desktop die Zeichen aus `zeichen` (ß/ẞ → ss, æ → ae, …); danach zerlegt `unicode61 remove_diacritics 2`. Der Anfragetext durchläuft zuerst `content_tokens` (Kleinschreibung, Stoppwörter), dann die Faltung, dann den Tokenizer. Gespeicherter Text und Zitat bleiben unverändert. `proben`: Eingabe → `normalisiert` (Ergebnis der Faltung) → `fts5` (Wörter des Tokenizers dazu, in Reihenfolge).",
        "zeichen": wai_retrieval::FTS_FALTUNG
            .iter()
            .map(|(von, nach)| json!({"von": von.to_string(), "nach": nach, "codepunkt": *von as u32}))
            .collect::<Vec<_>>(),
        "proben": proben
            .iter()
            .zip(norm.iter().zip(&zerlegt))
            .map(|(p, (n, z))| json!({"text": p, "normalisiert": n, "fts5": z}))
            .collect::<Vec<_>>(),
    })
}

/// Dateien, die der Test in den Papierkorb legt: viele Wörter der Fragen, damit eine Zählung über sie den Rohrang
/// sichtbar verschieben würde (synthetisch).
fn gesperrte_dateien() -> Vec<(&'static str, String)> {
    vec![
        (
            "Gesperrt-Vertrag.txt",
            "Kündigung Kündigungsfrist Vertrag Laufzeit Straße Strasse Budget Fassade Angebot Vorstand. "
                .repeat(8),
        ),
        (
            "Gesperrt-Haus.txt",
            "Ruhezeiten Haus Müll Tonnen Säcke Treppenhaus Nachtruhe Zugvogel Mieter Genossenschaft. "
                .repeat(8),
        ),
        (
            "Gesperrt-Cafe.txt",
            "Café crème brûlée Zoë Müller Mueller Lodz Gdansk Ærø Ørsted Straße Pariser Kommune. "
                .repeat(8),
        ),
    ]
}

fn korpus_env() -> (Env, BTreeMap<String, String>) {
    let e = env();
    let mut dats: BTreeMap<String, String> = BTreeMap::new();
    for (name, bytes) in korpus() {
        let dat = e.ablegen(name, &bytes);
        dats.insert(name.to_string(), dat);
    }
    e.text_und_index();
    (e, dats)
}

fn nahe(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1e-300)
}

/// Der Tokenizer (SQLite des Index) hinterlässt bei diesen Zeichen Nicht-ASCII; nach der Faltung bleibt nichts mehr übrig.
#[test]
fn faltung_ergaenzt_nur_was_fts5_nicht_faltet() {
    let c = fts5_verbindung();
    // Gemessen: im Latein-1-Block und in Latein erweitert-A (U+00C0..U+017F) lässt `remove_diacritics 2` genau diese
    // Buchstaben auf Nicht-ASCII stehen (Kleinschreibung zählt nicht als Faltung).
    let mut proben: Vec<char> = Vec::new();
    for n in 0xC0u32..=0x17F {
        let Some(ch) = char::from_u32(n) else {
            continue;
        };
        if ch == '×' || ch == '÷' {
            continue;
        }
        proben.push(ch);
    }
    let text: Vec<String> = proben.iter().map(|ch| format!("q{ch}q")).collect();
    let zerlegt = fts5_zerlegen(&c, &text);
    let mut ungefaltet: Vec<char> = Vec::new();
    for (ch, z) in proben.iter().zip(&zerlegt) {
        // Ein einzelnes Wort aus Buchstaben des Grundalphabets: gefaltet. Sonst bleibt Nicht-ASCII übrig.
        if z.len() == 1 && z[0].is_ascii() {
            continue;
        }
        ungefaltet.push(*ch);
    }
    let erfasst: Vec<char> = wai_retrieval::FTS_FALTUNG.iter().map(|(c, _)| *c).collect();
    // Bewusst nicht aufgenommen: Zeichen ohne gebräuchliche ASCII-Schreibung im Deutschen/Europa (ŀ Katalanisch mit Mittelpunkt,
    // ŉ Afrikaans, ĸ Grönländisch, ŋ/ŧ Samisch) und Großbuchstaben, die der Tokenizer schon klein faltet.
    let bewusst: Vec<char> = "ŀĿŉĸŋŊŧŦ".chars().collect();
    for ch in &ungefaltet {
        let klein_vorhanden = erfasst.contains(ch) || bewusst.contains(ch);
        assert!(
            klein_vorhanden,
            "U+{:04X} {ch} wird nicht gefaltet und fehlt in FTS_FALTUNG",
            *ch as u32
        );
    }
    // Und jedes aufgenommene Zeichen wird tatsächlich nicht gefaltet (sonst überflüssig) und nach der Faltung ASCII.
    for (ch, nach) in wai_retrieval::FTS_FALTUNG {
        assert!(
            ungefaltet.contains(ch) || *ch == 'ẞ',
            "{ch} wird schon von FTS5 gefaltet"
        );
        let roh = fts5_zerlegen(&c, &[format!("q{ch}q")]);
        assert!(!roh[0][0].is_ascii(), "{ch}: FTS5 faltet selbst: {roh:?}");
        let gefaltet = fts5_zerlegen(
            &c,
            &[format!(
                "q{}q",
                wai_retrieval::fts_normalisieren(&ch.to_string())
            )],
        );
        assert_eq!(gefaltet[0].len(), 1);
        assert!(gefaltet[0][0].is_ascii(), "{ch} → {nach}: {gefaltet:?}");
        assert_eq!(gefaltet[0][0], format!("q{nach}q"));
    }
}

#[test]
fn normalisieren_beispiele() {
    use wai_retrieval::fts_normalisieren as n;
    assert_eq!(n("Straße"), "Strasse");
    assert_eq!(n("GROẞE"), "GROssE");
    assert_eq!(n("Æther Œuvre Ørsted Łódź"), "aether oeuvre orsted lódź");
    assert_eq!(n("Þór Đorđe ıstanbul Ĳssel"), "thór dorde istanbul ijssel");
    assert_eq!(n("Müller café 123"), "Müller café 123");
}

/// Eigenes BM25 gleicht `bm25(chunks_fts)` von FTS5 (ohne gesperrte Stücke im Bestand), für alle Musterfragen und für
/// Anfragewörter, die der Tokenizer in mehrere Wörter zerlegt (Schriften mit Vokalzeichen).
#[test]
fn eigenes_bm25_gleicht_fts5() {
    let (e, _) = korpus_env();
    e.ablegen(
        "Schriften.txt",
        "हिन्दी हिंदी ไทย ภาษาไทย Hindi lernen. हिन्दी ist eine Sprache, ภาษาไทย auch.".as_bytes(),
    );
    e.ablegen(
        "Wiederholung.txt",
        "Wald Wald Wald Fuchs Wald im Wald; Fuchs und Hase im Wald.".as_bytes(),
    );
    e.text_und_index();
    let c = e.db();
    let mut fragen = fragen();
    fragen.extend(
        [
            "हिन्दी",
            "हिंदी ภาษาไทย",
            "Wald Wald Fuchs",
            "Wald",
            "Hase Wald Müller",
        ]
        .map(String::from),
    );
    let mut gesehen = 0usize;
    for f in &fragen {
        let a: String = f.chars().take(200).collect();
        let fts = fts5_bm25_zeilen(&c, &a);
        let eigen = bm25_zeilen(&e.a, &a);
        assert_eq!(fts.len(), eigen.len(), "{f:?}: Zahl der Zeilen");
        for (x, y) in fts.iter().zip(&eigen) {
            assert_eq!((x.0, &x.1, x.2), (y.0, &y.1, y.2), "{f:?}: Reihenfolge");
            assert!(nahe(x.3, y.3), "{f:?}: {} != {}", x.3, y.3);
        }
        gesehen += fts.len();
    }
    assert!(gesehen > 50, "die Probe soll Treffer liefern: {gesehen}");
}

/// ß findet ss und umgekehrt, am echten Weg (Index, Suche, KI-Suche); das Zitat bleibt der Originaltext.
#[test]
fn strasse_findet_strasse_und_umgekehrt() {
    let e = env();
    e.ablegen(
        "Eins.txt",
        "Die Straße vor dem Haus ist gesperrt.".as_bytes(),
    );
    e.ablegen(
        "Zwei.txt",
        "Hauptstrasse 5, GROSSE Baustelle, Fußweg gesperrt.".as_bytes(),
    );
    e.ablegen("Drei.txt", "Der Hund bellt im Garten.".as_bytes());
    e.text_und_index();
    let namen = e.namen();
    let treffer = |frage: &str| -> Vec<String> {
        let mut v: Vec<String> = index::suchen_mit(&e.a, frage, &AblageEinbettung::Keine)
            .unwrap()
            .iter()
            .map(|t| namen[&t.dat].clone())
            .collect();
        v.sort();
        v
    };
    for frage in ["Strasse", "STRASSE", "Straße", "straße", "STRAẞE"] {
        assert_eq!(treffer(frage), vec!["Eins.txt"], "{frage}");
    }
    for frage in [
        "Hauptstrasse",
        "Hauptstraße",
        "große",
        "grosse",
        "Fussweg",
        "Fußweg",
    ] {
        assert_eq!(treffer(frage), vec!["Zwei.txt"], "{frage}");
    }
    assert!(treffer("Straßen").is_empty());
    // Der Auszug ist der Originaltext, nicht die gefaltete Form.
    let t = index::suchen_mit(&e.a, "Strasse", &AblageEinbettung::Keine).unwrap();
    assert!(
        t[0].stellen[0].auszug.contains("Straße"),
        "{:?}",
        t[0].stellen[0].auszug
    );
    assert!(!t[0].stellen[0].auszug.contains("Strasse"));
    // KI-Suche (derselbe Rangweg).
    let ki = index::ki_suchen(
        &e.a,
        "Strasse",
        &AblageEinbettung::Keine,
        index::LESER_SUCHE,
        KANDIDATEN,
    )
    .unwrap();
    assert_eq!(ki.stellen.len(), 1);
    assert!(ki.stellen[0].text.contains("Straße"));
}

/// Zählt Einbettungsaufrufe (Spion): die Umstellung eines Altbestands darf nie einbetten.
struct Spion(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl wai_retrieval::Embedder for Spion {
    fn id(&self) -> String {
        "spion".into()
    }

    async fn embed(&self, _text: &str) -> wai_retrieval::Result<Vec<f32>> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(wai_retrieval::RetrievalError::Embedding("spion".into()))
    }

    async fn embed_batch(&self, texts: &[String]) -> wai_retrieval::Result<Vec<Vec<f32>>> {
        self.0
            .fetch_add(texts.len().max(1), std::sync::atomic::Ordering::SeqCst);
        Err(wai_retrieval::RetrievalError::Embedding("spion".into()))
    }
}

/// Bestehender Index (ungefalteter FTS-Text, keine Marke `fts_fmt`) wird beim Öffnen einmalig umgestellt: ohne
/// Einbetten (Spion), ohne den Stempel anzufassen (der Abgleich schreibt nichts), danach Treffer; ausdrücklicher
/// Neuaufbau funktioniert weiter. (Eine Transaktion, die Marke steht erst am Ende: Abbruch lässt den alten Stand.)
#[test]
fn alter_index_wird_ohne_einbetten_umgestellt() {
    let e = env();
    e.ablegen(
        "Eins.txt",
        "Die Straße vor dem Haus ist gesperrt.".as_bytes(),
    );
    e.ablegen("Zwei.txt", "Der Hund bellt im Garten.".as_bytes());
    e.text_und_index();
    let suche = |f: &str| {
        index::suchen_mit(&e.a, f, &AblageEinbettung::Keine)
            .unwrap()
            .len()
    };
    assert_eq!(suche("Strasse"), 1);
    let marke = |c: &Connection| -> Option<String> {
        c.query_row("SELECT value FROM meta WHERE key = 'fts_fmt'", [], |r| {
            r.get(0)
        })
        .ok()
    };
    assert_eq!(marke(&e.db()).as_deref(), Some("2"));

    // Zustand der Version davor: FTS-Text ungefaltet, keine Marke.
    let altstand = |e: &Env| {
        let c = e.db();
        let zeilen: Vec<(i64, String)> = {
            let mut q = c.prepare("SELECT id, text FROM chunks").unwrap();
            q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        for (id, text) in &zeilen {
            c.execute("DELETE FROM chunks_fts WHERE rowid = ?1", params![id])
                .unwrap();
            c.execute(
                "INSERT INTO chunks_fts(rowid, text) VALUES(?1, ?2)",
                params![id, text],
            )
            .unwrap();
        }
        c.execute("DELETE FROM meta WHERE key = 'fts_fmt'", [])
            .unwrap();
        c.execute("DELETE FROM qcache", []).unwrap();
        c.execute("DELETE FROM lexstat", []).unwrap();
    };
    altstand(&e);
    assert!(marke(&e.db()).is_none());

    // Öffnen mit dem Spion stellt um, ohne einen Einbettungsaufruf.
    let aufrufe = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let pfad = e.a.dir().join(index::INDEX);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let r = wai_retrieval::Retriever::open(&pfad, Spion(aufrufe.clone()))
            .await
            .unwrap();
        r.close().await;
    });
    assert_eq!(aufrufe.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(marke(&e.db()).as_deref(), Some("2"));
    assert_eq!(suche("Strasse"), 1);
    assert_eq!(suche("Straße"), 1);
    let b = index::abgleichen_mit(&e.a, &AblageEinbettung::Keine).unwrap();
    assert_eq!(
        b.geschrieben, 0,
        "der Stempel bleibt: nichts wird neu eingebettet"
    );

    // Ausdrücklicher Neuaufbau (index.db weg).
    let b = index::neu_aufbauen_mit(&e.a, &AblageEinbettung::Keine).unwrap();
    assert_eq!(b.geschrieben, 2);
    assert_eq!(suche("Strasse"), 1);
}

/// Stückzahl und Wörter der lesbaren Stücke werden je (Leser, Generation) einmal gezählt, unabhängig von `k`
/// (wachsender Kandidatenpool der KI-Suche), und mit jeder Änderung des Index oder der Freigabe neu.
#[test]
fn zaehlung_je_leser_und_generation_einmal() {
    let e = env();
    let w = e.standard();
    let frei = e.a.ordner_anlegen(Some(&w), "Frei").unwrap();
    let zu = e.a.ordner_anlegen(Some(&w), "Zu").unwrap();
    e.a.ki_setzen(&frei, Some(KiStufe::An)).unwrap();
    e.a.ki_setzen(&zu, Some(KiStufe::Gesperrt)).unwrap();
    e.ablegen_in(
        "Frei-1.txt",
        "Der Kuckuck ruft im Wald. Ein Fuchs im Wald.",
        &frei,
    );
    e.ablegen_in("Frei-2.txt", "Im Garten blüht der Flieder.", &frei);
    e.ablegen_in("Zu-1.txt", "Wald Wald Wald Eule Fuchs Wald.", &zu);
    e.text_und_index();
    let leser = index::ki_leser(KiWeg::Global, KiProfil::Lokal);
    let zaehlungen = |e: &Env| -> i64 {
        e.db()
            .query_row(
                "SELECT value FROM meta WHERE key = 'lexstat_zaehlungen'",
                [],
                |r| r.get::<_, String>(0),
            )
            .map(|v| v.parse().unwrap())
            .unwrap_or(0)
    };
    let ki = |e: &Env, f: &str, k: usize| {
        index::ki_suchen(&e.a, f, &AblageEinbettung::Keine, &leser, k).unwrap()
    };
    assert_eq!(zaehlungen(&e), 0);
    let erste = ki(&e, "Wald", 20);
    assert_eq!(erste.stellen.len(), 1);
    assert_eq!(zaehlungen(&e), 1);
    // Wachsender Pool und andere Wörter: nicht neu gezählt.
    for k in [40, 160, 640] {
        ki(&e, "Wald", k);
    }
    ki(&e, "Fuchs Garten", 20);
    assert_eq!(zaehlungen(&e), 1, "je (Leser, Generation) einmal");
    // Anderer Leser: eigene Zählung (er sieht mehr).
    let ui = index::lexikalisch_zeilen(&e.a, "Wald", index::LESER_SUCHE, 100).unwrap();
    assert_eq!(ui.len(), 2);
    assert_eq!(zaehlungen(&e), 2);

    // Neue Datei: neue Generation, neu gezählt, Ränge ändern sich.
    let vorher = index::lexikalisch_zeilen(&e.a, "Wald", &leser, 100).unwrap()[0].bm25;
    assert_eq!(zaehlungen(&e), 2);
    e.ablegen_in("Frei-3.txt", "Wald und Wiese.", &frei);
    e.text_und_index();
    let danach = index::lexikalisch_zeilen(&e.a, "Wald", &leser, 100).unwrap();
    assert_eq!(zaehlungen(&e), 3);
    assert!(!nahe(vorher, danach.iter().find(|z| z.page == 1).unwrap().bm25) || danach.len() > 1);

    // Freigabeänderung: der gesperrte Ordner wird freigegeben; er zählt ab jetzt mit.
    let namen = e.namen();
    assert!(danach
        .iter()
        .all(|z| namen[&z.source_id].starts_with("Frei")));
    e.a.ki_setzen(&zu, Some(KiStufe::An)).unwrap();
    e.text_und_index();
    let frei_gegeben = index::lexikalisch_zeilen(&e.a, "Wald", &leser, 100).unwrap();
    assert_eq!(zaehlungen(&e), 4);
    let namen = e.namen();
    assert!(frei_gegeben
        .iter()
        .any(|z| namen[&z.source_id].starts_with("Zu")));
}

/// Eine Datei im Papierkorb beeinflusst den Rohrang der übrigen nicht (UI-Leser), auch nicht nach dem Zurückholen
/// umgekehrt: erst dann zählt sie wieder.
#[test]
fn papierkorb_zaehlt_nicht_in_die_idf() {
    let (e, _) = korpus_env();
    let roh = |f: &str| -> Vec<(String, i64, f64)> {
        let namen = e.namen();
        bm25_zeilen(&e.a, f)
            .iter()
            .map(|z| (namen[&z.1].clone(), z.2, z.3))
            .collect()
    };
    let fragen = [
        "Kündigung",
        "Kündigungsfrist Vertrag",
        "Straße",
        "Ruhezeiten Haus",
    ];
    let vorher: Vec<_> = fragen.iter().map(|f| roh(f)).collect();
    let mut korb = Vec::new();
    for (name, text) in gesperrte_dateien() {
        let dat = e.ablegen(name, text.as_bytes());
        e.text_und_index();
        e.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
        korb.push(dat);
    }
    index::abgleichen_mit(&e.a, &AblageEinbettung::Keine).unwrap();
    for (f, v) in fragen.iter().zip(&vorher) {
        let nachher = roh(f);
        assert_eq!(v.len(), nachher.len(), "{f}");
        for (x, y) in v.iter().zip(&nachher) {
            assert_eq!((&x.0, x.1), (&y.0, y.1), "{f}");
            assert!(nahe(x.2, y.2), "{f}: {} != {}", x.2, y.2);
        }
    }
    // Kontrolle: FTS5 selbst zählt über die ganze Tabelle; mit ihm verschöbe der Papierkorb den Rohrang.
    let fts = fts5_bm25_zeilen(&e.db(), "Kündigungsfrist Vertrag");
    let eigen = bm25_zeilen(&e.a, "Kündigungsfrist Vertrag");
    assert_eq!(fts.len(), eigen.len());
    assert!(
        fts.iter().zip(&eigen).any(|(x, y)| !nahe(x.3, y.3)),
        "die gesperrten Dateien müssten bm25(chunks_fts) verschieben"
    );
    // Zurückgeholt zählen sie wieder: der Rohrang ändert sich.
    for d in &korb {
        e.a.wiederherstellen(AblageArt::Datei, d).unwrap();
    }
    index::abgleichen_mit(&e.a, &AblageEinbettung::Keine).unwrap();
    let danach = roh("Kündigungsfrist Vertrag");
    assert!(danach.len() > vorher[1].len());
}

impl Env {
    fn ablegen_in(&self, name: &str, text: &str, ordner: &str) -> String {
        let q = self
            .d
            .path()
            .join("q")
            .join(ulid::Ulid::new().to_string())
            .join(name);
        fs::create_dir_all(q.parent().unwrap()).unwrap();
        fs::write(&q, text).unwrap();
        let z = self.a.importieren(&q, ordner, None).unwrap();
        assert_eq!(z[0].stand, AblageImportStand::Abgelegt, "{:?}", z[0]);
        z[0].dat.clone().unwrap()
    }
}

/// KI-Leser: Dateien eines für die KI gesperrten Ordners zählen weder in N noch in n noch in avgdl; der Rohrang der
/// freigegebenen Stücke ist mit und ohne sie derselbe (Keine Statistik gesperrter Inhalte fließt in Ränge ein).
#[test]
fn ki_leser_zaehlt_gesperrte_dateien_nicht() {
    let frei_texte = [
        (
            "Frei-1.txt",
            "Der Kuckuck ruft im Wald. Ein Fuchs schleicht durch den Wald.",
        ),
        (
            "Frei-2.txt",
            "Im Garten blüht der Flieder; die Amsel singt.",
        ),
        (
            "Frei-3.txt",
            "Wald und Wiese, Bach und Brücke, Fuchs und Hase.",
        ),
    ];
    let gesperrt_texte = [
        (
            "Zu-1.txt",
            "Wald Wald Wald Wald Fuchs Fuchs Kuckuck Eule Wald.",
        ),
        ("Zu-2.txt", "Wald im Nebel, Wald im Regen, Eule im Wald."),
        ("Zu-3.txt", "Eule Eule Eule Eule Eule Wald Fuchs."),
        ("Zu-4.txt", "Eule im Turm. Eule und Fuchs. Wald."),
    ];
    let leser = index::ki_leser(KiWeg::Global, KiProfil::Lokal);
    let roh = |mit_gesperrten: bool| -> Vec<(String, i64, f64)> {
        let e = env();
        let w = e.standard();
        let frei = e.a.ordner_anlegen(Some(&w), "Frei").unwrap();
        let zu = e.a.ordner_anlegen(Some(&w), "Zu").unwrap();
        e.a.ki_setzen(&frei, Some(KiStufe::An)).unwrap();
        e.a.ki_setzen(&zu, Some(KiStufe::Gesperrt)).unwrap();
        for (n, t) in frei_texte {
            e.ablegen_in(n, t, &frei);
        }
        if mit_gesperrten {
            for (n, t) in gesperrt_texte {
                e.ablegen_in(n, t, &zu);
            }
        }
        e.text_und_index();
        let namen = e.namen();
        let mut aus = Vec::new();
        for f in ["Wald", "Wald Eule", "Fuchs Kuckuck Wald"] {
            let z = index::lexikalisch_zeilen(&e.a, f, &leser, 100).unwrap();
            assert!(
                z.iter().all(|z| namen[&z.source_id].starts_with("Frei")),
                "{f}: nur Freigegebenes"
            );
            aus.extend(z.iter().map(|z| {
                (
                    format!("{f}|{}", namen[&z.source_id]),
                    z.page as i64,
                    z.bm25,
                )
            }));
        }
        // Der UI-Leser sieht alle (Kontrolle, dass die gesperrten Dateien wirklich im Index liegen).
        if mit_gesperrten {
            let ui = index::lexikalisch_zeilen(&e.a, "Eule", index::LESER_SUCHE, 100).unwrap();
            assert!(ui.iter().any(|z| namen[&z.source_id].starts_with("Zu")));
            let ki = index::lexikalisch_zeilen(&e.a, "Eule", &leser, 100).unwrap();
            assert!(ki.is_empty());
        }
        aus
    };
    let ohne = roh(false);
    let mit = roh(true);
    assert!(!ohne.is_empty());
    assert_eq!(ohne.len(), mit.len());
    for (x, y) in ohne.iter().zip(&mit) {
        assert_eq!((&x.0, x.1), (&y.0, y.1));
        assert!(nahe(x.2, y.2), "{}: {} != {}", x.0, x.2, y.2);
    }
}

/// `docs/superpowers/plans/ablage-index-vektoren.json` enthält genau das, was der Code heute berechnet.
#[test]
fn json_passt_zum_code() {
    let ist = alle();
    if std::env::var("LVA_VEKTOREN_SCHREIBEN").as_deref() == Ok("1") {
        fs::write(
            JSON_PFAD,
            serde_json::to_string_pretty(&ist).unwrap() + "\n",
        )
        .unwrap();
    }
    let soll: Value = serde_json::from_str(
        &fs::read_to_string(JSON_PFAD).expect("ablage-index-vektoren.json fehlt"),
    )
    .expect("ablage-index-vektoren.json ist kein JSON");
    let mut a = Vec::new();
    abweichungen(&soll, &ist, "$", &mut a);
    assert!(
        a.is_empty(),
        "ablage-index-vektoren.json weicht vom Code ab (neu schreiben mit LVA_VEKTOREN_SCHREIBEN=1): {a:#?}"
    );
}

/// Die Stoppwörter, wie sie `content_tokens` wirklich entfernt.
#[test]
fn stopwoerter_werden_entfernt() {
    let stop = stop_liste();
    assert!(stop.len() > 40);
    for w in &stop {
        assert!(wai_retrieval::content_tokens(w).is_empty(), "{w}");
        assert!(
            wai_retrieval::content_tokens(&w.to_uppercase()).is_empty(),
            "{w}"
        );
    }
    assert_eq!(
        wai_retrieval::content_tokens("Der Zugvogel"),
        vec!["zugvogel".to_string()]
    );
}

/// Der Korpus enthält keine Datei, die der Test nicht kennt (Namen sind eindeutig, ASCII).
#[test]
fn korpusnamen_sind_eindeutig() {
    let mut n: Vec<&str> = korpus().iter().map(|k| k.0).collect();
    n.sort();
    n.dedup();
    assert_eq!(n.len(), korpus().len());
    assert!(korpus().iter().all(|k| k.0.is_ascii()));
    let _ = extrakt::MARKEN_VERSION;
}
