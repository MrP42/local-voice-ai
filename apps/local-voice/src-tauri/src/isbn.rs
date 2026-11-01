//! Buchdaten per ISBN aus Open Library (lva#149).
//!
//! Nur auf Knopfdruck: der Abruf geht ins Netz (openlibrary.org) und schickt
//! dabei allein die ISBN — keinen Buchtext, keinen Seitentitel. Das Ergebnis
//! ist ein Vorschlag; übernommen wird erst nach Bestätigung im Dialog, denn es
//! kann die falsche Ausgabe sein. Kein Abruf bei Amazon/Audible.

use serde::{Deserialize, Serialize};

const OPEN_LIBRARY: &str = "https://openlibrary.org/api/books";
const OFFLINE: &str =
    "Open Library ist gerade nicht erreichbar. Prüfe die Internetverbindung und versuche es noch einmal.";

/// Was Open Library zu einer ISBN weiß, so weit es die App braucht.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, PartialEq, Default)]
pub struct IsbnBook {
    /// Normalisierte ISBN (nur Ziffern bzw. `X`).
    pub isbn: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub authors: Vec<String>,
    pub publishers: Vec<String>,
    pub publish_date: Option<String>,
    pub pages: Option<u32>,
    /// Link auf die Open-Library-Seite (zum Nachsehen der Ausgabe).
    pub url: Option<String>,
}

/// ISBN-10 oder ISBN-13 aus einer Eingabe wie „978-3-16-148410-0“ oder
/// „ISBN 3-16-148410-X“ — mit Prüfziffer, sonst `None`.
pub fn normalize_isbn(input: &str) -> Option<String> {
    let s = input.trim();
    // „ISBN“, „isbn“, „ISBN-13:“ … vorn abschneiden.
    let s = match s.get(..4) {
        Some(p) if p.eq_ignore_ascii_case("isbn") => {
            let rest = &s[4..];
            rest.strip_prefix("-13")
                .or_else(|| rest.strip_prefix("-10"))
                .unwrap_or(rest)
        }
        _ => s,
    };
    let s = s.trim_start_matches([':', ' ', '-', '\u{a0}'].as_ref());
    let chars: Vec<char> = s
        .chars()
        .filter(|c| !matches!(c, '-' | ' ' | '\u{2010}' | '\u{2011}'))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let digit = |c: char| c.to_digit(10);
    match chars.len() {
        10 => {
            let mut sum = 0;
            for (i, &c) in chars.iter().enumerate() {
                let v = match (i, c) {
                    (9, 'X') => 10,
                    _ => digit(c)?,
                };
                sum += v * (10 - i as u32);
            }
            (sum % 11 == 0).then(|| chars.iter().collect())
        }
        13 => {
            let mut sum = 0;
            for (i, &c) in chars.iter().enumerate() {
                sum += digit(c)? * if i % 2 == 0 { 1 } else { 3 };
            }
            (sum % 10 == 0
                && (chars.starts_with(&['9', '7', '8']) || chars.starts_with(&['9', '7', '9'])))
            .then(|| chars.iter().collect())
        }
        _ => None,
    }
}

/// Die Antwort von `api/books?jscmd=data` lesen. `None`: kein Treffer.
pub fn parse_open_library(isbn: &str, body: &str) -> Result<Option<IsbnBook>, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("Antwort von Open Library unlesbar: {e}"))?;
    let Some(entry) = value.get(format!("ISBN:{isbn}")) else {
        return Ok(None);
    };
    let text = |v: &serde_json::Value| {
        v.as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let names = |key: &str| -> Vec<String> {
        entry
            .get(key)
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|a| a.get("name").and_then(text))
                    .collect()
            })
            .unwrap_or_default()
    };
    let Some(title) = entry.get("title").and_then(text) else {
        return Ok(None);
    };
    Ok(Some(IsbnBook {
        isbn: isbn.to_string(),
        title,
        subtitle: entry.get("subtitle").and_then(text),
        authors: names("authors"),
        publishers: names("publishers"),
        publish_date: entry.get("publish_date").and_then(text),
        pages: entry
            .get("number_of_pages")
            .and_then(|v| v.as_u64())
            .and_then(|n| u32::try_from(n).ok()),
        url: entry
            .get("url")
            .and_then(text)
            .filter(|u| u.starts_with("https://openlibrary.org/")),
    }))
}

/// Buchdaten zu einer ISBN abrufen. `Ok(None)`: Open Library kennt sie nicht.
#[tauri::command]
#[specta::specta]
pub async fn isbn_lookup(isbn: String) -> Result<Option<IsbnBook>, String> {
    let isbn = normalize_isbn(&isbn).ok_or_else(|| {
        "Keine gültige ISBN (10 oder 13 Stellen mit passender Prüfziffer).".to_string()
    })?;
    let client = reqwest::Client::builder()
        .user_agent("LocalVoiceAI (Vorlesen; ISBN-Abfrage)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(OPEN_LIBRARY)
        .query(&[
            ("bibkeys", format!("ISBN:{isbn}")),
            ("format", "json".to_string()),
            ("jscmd", "data".to_string()),
        ])
        .send()
        .await
        .map_err(|e| {
            // Technische Ursache ins Log, verstaendlicher Satz in den Dialog.
            log::warn!("ISBN-Abfrage: Open Library nicht erreichbar: {e}");
            OFFLINE.to_string()
        })?;
    if !response.status().is_success() {
        return Err(format!("Open Library antwortete mit {}", response.status()));
    }
    let body = response.text().await.map_err(|e| e.to_string())?;
    parse_open_library(&isbn, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isbn_checksums() {
        assert_eq!(
            normalize_isbn("978-3-16-148410-0").as_deref(),
            Some("9783161484100")
        );
        assert_eq!(
            normalize_isbn("ISBN 3-16-148410-X").as_deref(),
            Some("316148410X")
        );
        assert_eq!(
            normalize_isbn("isbn: 0-306-40615-2").as_deref(),
            Some("0306406152")
        );
        assert_eq!(
            normalize_isbn("ISBN-13: 978-3-16-148410-0").as_deref(),
            Some("9783161484100")
        );
        assert_eq!(
            normalize_isbn("Isbn-10: 3-16-148410-X").as_deref(),
            Some("316148410X")
        );
        assert_eq!(normalize_isbn("978-3-16-148410-1"), None);
        assert_eq!(normalize_isbn("3-16-148410-9"), None);
        assert_eq!(normalize_isbn("12345"), None);
        assert_eq!(normalize_isbn("ABCDEFGHIJ"), None);
        // 13 Stellen mit passender Prüfziffer, aber kein Buchpräfix (EAN).
        assert_eq!(normalize_isbn("4006381333931"), None);
    }

    #[test]
    fn parses_open_library_data() {
        let body = r#"{"ISBN:9783161484100": {
            "title": "Der Leuchtturm",
            "subtitle": "Roman",
            "authors": [{"name": "Ada Beispiel", "url": "https://openlibrary.org/authors/OL1A"}, {"name": " "}],
            "publishers": [{"name": "Beispielverlag"}],
            "publish_date": "2020",
            "number_of_pages": 312,
            "url": "https://openlibrary.org/books/OL1M/Der_Leuchtturm"
        }}"#;
        let book = parse_open_library("9783161484100", body).unwrap().unwrap();
        assert_eq!(book.title, "Der Leuchtturm");
        assert_eq!(book.subtitle.as_deref(), Some("Roman"));
        assert_eq!(book.authors, vec!["Ada Beispiel"]);
        assert_eq!(book.publishers, vec!["Beispielverlag"]);
        assert_eq!(book.pages, Some(312));
        assert!(book.url.unwrap().starts_with("https://openlibrary.org/"));
    }

    #[test]
    fn no_hit_and_foreign_links() {
        assert_eq!(parse_open_library("9783161484100", "{}").unwrap(), None);
        let body = r#"{"ISBN:9783161484100": {"title": "X", "url": "https://example.com/x"}}"#;
        assert_eq!(
            parse_open_library("9783161484100", body)
                .unwrap()
                .unwrap()
                .url,
            None
        );
        assert!(parse_open_library("9783161484100", "<html>").is_err());
    }
}
