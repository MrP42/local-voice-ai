//! M6-P6c: Follow-up-Mail zu einer Besprechung.
//!
//! Das Recipe „Follow-up-E-Mail an …“ (M4) liefert eine Antwort mit `Betreff:`
//! in der ersten Zeile und dem Mailtext danach. Dieses Modul macht daraus einen
//! bearbeitbaren Entwurf (`MailDraft`) und die drei Ausgänge: `mailto:`-Adresse,
//! .eml-Datei (MIME multipart/alternative, UTF-8, `X-Unsent: 1`) und die
//! Empfängerliste aus den Teilnehmenden.
//!
//! Reine Funktionen ohne App-Zustand: kein Zugriff auf Einstellungen, kein
//! Netz. Die .eml ist bei gleicher Eingabe Byte für Byte gleich (Datum,
//! Message-ID und Boundary kommen von außen), damit Golden-Tests möglich sind.

use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use mail_builder::headers::{address::Address, content_type::ContentType, date::Date, raw::Raw};
use mail_builder::mime::MimePart;
use mail_builder::MessageBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::export::markdown_to_text;
use super::store::MeetingStore;

/// Ab dieser Länge (Zeichen der fertigen Adresse) übergibt `mailto:` nur noch
/// Empfänger und Betreff: Windows kappt Adressen für `ShellExecute` bei etwa
/// 2 000 Zeichen, und manche Mailprogramme schon früher.
pub const MAILTO_MAX_LEN: usize = 1800;

/// Längster Betreff, den der Entwurf übernimmt (ein Modell könnte Absätze liefern).
const MAX_SUBJECT_CHARS: usize = 200;

/// Ein bearbeitbarer Mailentwurf. `body_html` wird beim Kopieren und Speichern
/// aus `body_text` neu gebaut (`finalize`), damit Änderungen im Dialog nie
/// hinter einer alten HTML-Fassung zurückbleiben.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct MailDraft {
    pub to: Vec<String>,
    pub subject: String,
    pub body_text: String,
    pub body_html: String,
}

// ---------------------------------------------------------------------------
// Entwurf aus der Antwort
// ---------------------------------------------------------------------------

fn citation_marks() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // `[1]`, `[1, 2]`, `[1;2]` samt dem Leerzeichen davor; zwei Marken
        // hintereinander (`[1][2]`) fängt der zweite Treffer.
        Regex::new(r"[ \t]*\[\d+(?:[ \t]*[,;][ \t]*\d+)*\]").expect("Zitatmarken-Muster")
    })
}

fn subject_line() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // `Betreff: …`, auch fett (`**Betreff:** …`) oder als Überschrift.
        Regex::new(r"(?i)^[\s#>*_]*(?:betreff|subject)[\s*_]*:[\s*_]*(.*)$")
            .expect("Betreff-Muster")
    })
}

/// Zitatmarken `[n]` entfernen (in einer Mail sind sie sinnlos).
pub fn strip_citation_marks(text: &str) -> String {
    citation_marks().replace_all(text, "").into_owned()
}

/// Betreff einer Zeile säubern: eine Zeile, keine Auszeichnung, begrenzt.
fn clean_subject(raw: &str) -> String {
    let flat: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = flat.trim().trim_matches('*').trim();
    trimmed
        .chars()
        .take(MAX_SUBJECT_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Ist das eine brauchbare Adresse? Bewusst streng: ein Zeichen, das in einer
/// Kopfzeile etwas anderes bedeuten könnte (Leerraum, Steuerzeichen,
/// `<>",;`), macht die Adresse unbrauchbar statt sie zu „reparieren“.
pub fn valid_address(addr: &str) -> bool {
    let a = addr.trim();
    if a.is_empty() || a.len() > 254 {
        return false;
    }
    if a.chars()
        .any(|c| c.is_whitespace() || c.is_control() || "<>\",;()[]\\".contains(c))
    {
        return false;
    }
    let mut parts = a.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

/// Nur brauchbare, einmalige Adressen (Groß-/Kleinschreibung egal), in Reihenfolge.
fn clean_recipients(to: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    to.iter()
        .map(|a| a.trim().to_string())
        .filter(|a| valid_address(a) && seen.insert(a.to_lowercase()))
        .collect()
}

/// Einstellung „Meine E-Mail-Adressen“: gebrauchte Form (klein, brauchbar,
/// ohne Duplikate); Unbrauchbares fällt still weg.
pub fn normalize_self_emails(list: &[String]) -> Vec<String> {
    clean_recipients(list)
        .into_iter()
        .map(|a| a.to_lowercase())
        .collect()
}

/// Entwurf aus der Chat-Antwort: Zitatmarken weg, `Betreff:` abgetrennt
/// (fehlt er: „Nachbereitung: <Titel>“), Text und HTML gebaut.
pub fn draft_from_answer(answer: &str, meeting_title: &str, to: Vec<String>) -> MailDraft {
    let cleaned = strip_citation_marks(&answer.replace("\r\n", "\n"));
    let mut subject = String::new();
    let mut rest: Vec<&str> = Vec::new();
    let mut seen_first = false;
    for line in cleaned.lines() {
        if !seen_first {
            if line.trim().is_empty() {
                continue;
            }
            seen_first = true;
            if let Some(caps) = subject_line().captures(line) {
                subject = clean_subject(caps.get(1).map_or("", |m| m.as_str()));
                continue;
            }
        }
        rest.push(line);
    }
    if subject.is_empty() {
        subject = clean_subject(&format!("Nachbereitung: {}", meeting_title.trim()));
    }
    let body_text = markdown_to_text(&rest.join("\n")).replace("\r\n", "\n");
    let body_text = body_text.trim().to_string();
    MailDraft {
        to: clean_recipients(&to),
        subject,
        body_html: html_from_text(&body_text),
        body_text,
    }
}

/// Vor Kopieren, `mailto:` und Speichern: Empfänger säubern, Betreff auf eine
/// Zeile bringen und die HTML-Fassung aus dem (evtl. bearbeiteten) Text bauen.
pub fn finalize(draft: &MailDraft) -> MailDraft {
    let body_text = draft.body_text.replace("\r\n", "\n").trim().to_string();
    MailDraft {
        to: clean_recipients(&draft.to),
        subject: clean_subject(&draft.subject),
        body_html: html_from_text(&body_text),
        body_text,
    }
}

// ---------------------------------------------------------------------------
// HTML
// ---------------------------------------------------------------------------

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn bullet_text(line: &str) -> Option<&str> {
    let t = line.trim_start();
    ["\u{2022} ", "- ", "* "]
        .iter()
        .find_map(|p| t.strip_prefix(p))
}

/// Klartext als HTML: Leerzeile = Absatz, Zeilenumbruch = `<br>`, Zeilen mit
/// „• “ (oder „- “) = Aufzählung. Alles maskiert.
pub fn html_from_text(text: &str) -> String {
    fn flush_para(out: &mut String, para: &mut Vec<String>) {
        if !para.is_empty() {
            out.push_str("<p>");
            out.push_str(&para.join("<br>\n"));
            out.push_str("</p>\n");
            para.clear();
        }
    }
    fn flush_list(out: &mut String, items: &mut Vec<String>) {
        if !items.is_empty() {
            out.push_str("<ul>\n");
            for item in items.drain(..) {
                out.push_str("<li>");
                out.push_str(&item);
                out.push_str("</li>\n");
            }
            out.push_str("</ul>\n");
        }
    }
    let mut out = String::new();
    let mut para: Vec<String> = Vec::new();
    let mut items: Vec<String> = Vec::new();
    for line in text.replace("\r\n", "\n").lines() {
        if line.trim().is_empty() {
            flush_para(&mut out, &mut para);
            flush_list(&mut out, &mut items);
        } else if let Some(item) = bullet_text(line) {
            flush_para(&mut out, &mut para);
            items.push(escape_html(item.trim()));
        } else {
            flush_list(&mut out, &mut items);
            para.push(escape_html(line.trim_end()));
        }
    }
    flush_para(&mut out, &mut para);
    flush_list(&mut out, &mut items);
    out.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// mailto:
// ---------------------------------------------------------------------------

/// Prozent-Kodierung nach RFC 3986: nur `A-Za-z0-9-._~` (und `extra`) bleiben.
fn percent_encode(text: &str, extra: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) || extra.as_bytes().contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `mailto:`-Adresse. Passt sie nicht in `max_len` Zeichen, enthält sie nur
/// Empfänger und Betreff; der zweite Wert ist dann `true` (der Text muss in
/// die Zwischenablage, der Nutzer fügt ihn ein). Umbrüche werden `%0D%0A`.
pub fn mailto_url(d: &MailDraft, max_len: usize) -> (String, bool) {
    let to = clean_recipients(&d.to)
        .iter()
        .map(|a| percent_encode(a, "@+"))
        .collect::<Vec<_>>()
        .join(",");
    let subject = percent_encode(&clean_subject(&d.subject), "");
    let head = format!("mailto:{to}?subject={subject}");
    let body = d
        .body_text
        .replace("\r\n", "\n")
        .trim()
        .replace('\n', "\r\n");
    if body.is_empty() {
        return (head, false);
    }
    let full = format!("{head}&body={}", percent_encode(&body, ""));
    if full.len() <= max_len {
        (full, false)
    } else {
        (head, true)
    }
}

// ---------------------------------------------------------------------------
// .eml
// ---------------------------------------------------------------------------

/// Entwurf als .eml: `multipart/alternative` (Text, HTML), UTF-8, `X-Unsent: 1`
/// (Outlook klassisch öffnet die Datei als Entwurf zum Senden). Datum,
/// Message-ID und Boundary hängen nur von `date` und `boundary_seed` ab.
pub fn write_eml(d: &MailDraft, date: DateTime<Utc>, boundary_seed: u64) -> Vec<u8> {
    let d = finalize(d);
    let boundary = format!("=_lva_{boundary_seed:016x}");
    let html = format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"></head><body>\n{}\n</body></html>",
        d.body_html
    );
    let body = MimePart::new(
        ContentType::new("multipart/alternative").attribute("boundary", boundary),
        vec![
            MimePart::new("text/plain", d.body_text.clone()),
            MimePart::new("text/html", html),
        ],
    );
    let mut message = MessageBuilder::new()
        .message_id(format!("lva-{boundary_seed:016x}@local-voice-ai.invalid"))
        .date(Date::new(date.timestamp()))
        .subject(d.subject.clone())
        .header("X-Unsent", Raw::new("1"))
        .body(body);
    if !d.to.is_empty() {
        let list: Vec<Address<'_>> =
            d.to.iter()
                .map(|a| Address::new_address(None::<&str>, a.clone()))
                .collect();
        message = message.to(Address::new_list(list));
    }
    let mut out = Vec::new();
    message.serialize(&mut out);
    out
}

// ---------------------------------------------------------------------------
// Empfänger aus den Teilnehmenden
// ---------------------------------------------------------------------------

/// Teilnehmende einer Besprechung ohne die eigene Person.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recipients {
    /// Adressen der Teilnehmenden mit brauchbarer E-Mail, ohne Duplikate.
    pub emails: Vec<String>,
    /// Anzeigenamen aller Teilnehmenden ohne die eigene Person (auch ohne Adresse).
    pub names: Vec<String>,
}

/// Teilnehmende der Besprechung aus `meeting_participants`. „Ich“ sind
/// Personen mit `is_self` und jede Adresse aus `self_emails` (Einstellung
/// „Meine E-Mail-Adressen“). Ohne Teilnehmende (P5d noch nicht gelaufen, kein
/// Kalendertermin) ist das Ergebnis leer, kein Fehler.
pub fn participant_recipients(
    store: &MeetingStore,
    meeting_id: &str,
    self_emails: &[String],
) -> anyhow::Result<Recipients> {
    let own: std::collections::HashSet<String> = self_emails
        .iter()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .collect();
    let conn = store.get_connection()?;
    let mut stmt = conn.prepare(
        "SELECT h.name, COALESCE(h.email_norm, h.email)
         FROM meeting_participants p JOIN humans h ON h.id = p.human_id
         WHERE p.meeting_id = ?1 AND h.deleted_at IS NULL AND h.merged_into IS NULL
           AND h.is_self = 0
         ORDER BY p.created_at, h.name",
    )?;
    let rows = stmt.query_map([meeting_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let mut out = Recipients::default();
    for row in rows {
        let (name, email) = row?;
        let email = email
            .map(|e| e.trim().to_string())
            .filter(|e| !e.is_empty());
        if email
            .as_ref()
            .is_some_and(|e| own.contains(&e.to_lowercase()))
        {
            continue;
        }
        let name = name.trim().to_string();
        if !name.is_empty() && !out.names.contains(&name) {
            out.names.push(name);
        }
        if let Some(e) = email {
            if valid_address(&e) && !out.emails.iter().any(|x| x.eq_ignore_ascii_case(&e)) {
                out.emails.push(e);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::store::MeetingSource;
    use chrono::TimeZone;

    fn draft(subject: &str, body: &str, to: &[&str]) -> MailDraft {
        MailDraft {
            to: to.iter().map(|s| s.to_string()).collect(),
            subject: subject.into(),
            body_text: body.into(),
            body_html: String::new(),
        }
    }

    #[test]
    fn citation_marks_are_removed_with_their_leading_space() {
        let d = draft_from_answer(
            "Betreff: Angebot\nDer Preis ist zu hoch [1]. Wir prüfen es [2, 3][4].",
            "T",
            vec![],
        );
        assert_eq!(d.body_text, "Der Preis ist zu hoch. Wir prüfen es.");
        assert!(!d.body_html.contains('['), "{}", d.body_html);
    }

    #[test]
    fn subject_line_is_split_off_the_body() {
        let d = draft_from_answer(
            "Betreff: Nächste Schritte zum Angebot\n\nHallo Anna,\n\nvielen Dank.",
            "Kundentermin",
            vec![],
        );
        assert_eq!(d.subject, "Nächste Schritte zum Angebot");
        assert_eq!(d.body_text, "Hallo Anna,\n\nvielen Dank.");
        assert!(!d.body_text.contains("Betreff"));
    }

    #[test]
    fn bold_or_english_subject_prefix_is_recognised() {
        let a = draft_from_answer("**Betreff:** Follow-up\nText", "T", vec![]);
        assert_eq!(a.subject, "Follow-up");
        let b = draft_from_answer("Subject: Follow-up\nText", "T", vec![]);
        assert_eq!(b.subject, "Follow-up");
        assert_eq!(b.body_text, "Text");
    }

    #[test]
    fn missing_subject_falls_back_to_the_meeting_title() {
        let d = draft_from_answer("Hallo Anna,\n\nvielen Dank.", "Kundentermin Meyer", vec![]);
        assert_eq!(d.subject, "Nachbereitung: Kundentermin Meyer");
        assert_eq!(d.body_text, "Hallo Anna,\n\nvielen Dank.");
        // Leerer Betreff zählt wie fehlender.
        let e = draft_from_answer("Betreff:   \nText", "Jour fixe", vec![]);
        assert_eq!(e.subject, "Nachbereitung: Jour fixe");
        assert_eq!(e.body_text, "Text");
    }

    #[test]
    fn only_the_first_line_can_be_the_subject() {
        let d = draft_from_answer("Hallo,\nBetreff: kommt später", "T", vec![]);
        assert_eq!(d.subject, "Nachbereitung: T");
        assert!(d.body_text.contains("Betreff: kommt später"));
    }

    #[test]
    fn bullets_become_a_list_and_text_is_escaped_in_html() {
        let d = draft_from_answer(
            "Betreff: X\nHallo <Anna> & Co,\n\n- Preis prüfen\n- Termin klären",
            "T",
            vec![],
        );
        assert!(
            d.body_text.contains("\u{2022} Preis prüfen"),
            "{}",
            d.body_text
        );
        assert!(d.body_html.contains("<ul>"), "{}", d.body_html);
        assert!(d.body_html.contains("<li>Preis prüfen</li>"));
        assert!(d.body_html.contains("Hallo &lt;Anna&gt; &amp; Co,"));
    }

    #[test]
    fn recipients_are_cleaned_and_deduplicated() {
        let d = draft_from_answer(
            "Text",
            "T",
            vec![
                "anna@firma.de".into(),
                "ANNA@firma.de".into(),
                "kaputt".into(),
                "a b@x.de".into(),
                "evil@x.de>\r\nBcc: z@z.de".into(),
            ],
        );
        assert_eq!(d.to, vec!["anna@firma.de".to_string()]);
    }

    #[test]
    fn mailto_encodes_umlauts_and_ampersands() {
        let d = draft(
            "Käse & Brot",
            "Größe: 5 & mehr\nZeile 2",
            &["anna@firma.de"],
        );
        let (url, clipped) = mailto_url(&d, MAILTO_MAX_LEN);
        assert!(!clipped);
        assert_eq!(
            url,
            "mailto:anna@firma.de?subject=K%C3%A4se%20%26%20Brot\
             &body=Gr%C3%B6%C3%9Fe%3A%205%20%26%20mehr%0D%0AZeile%202"
        );
        assert_eq!(url.matches('&').count(), 1, "nur das Trennzeichen bleibt");
    }

    #[test]
    fn long_mailto_keeps_only_recipient_and_subject() {
        let long = "Wort ".repeat(600);
        let d = draft("Betreff", &long, &["a@x.de", "b@y.de"]);
        let (url, clipped) = mailto_url(&d, MAILTO_MAX_LEN);
        assert!(clipped, "Text gehört dann in die Zwischenablage");
        assert_eq!(url, "mailto:a@x.de,b@y.de?subject=Betreff");
        assert!(url.len() <= MAILTO_MAX_LEN);
        // Genau an der Grenze passt es noch.
        let short = draft("B", "ab", &["a@x.de"]);
        let (full, _) = mailto_url(&short, usize::MAX);
        let (at_limit, c1) = mailto_url(&short, full.len());
        let (over, c2) = mailto_url(&short, full.len() - 1);
        assert_eq!((at_limit.as_str(), c1), (full.as_str(), false));
        assert!(c2 && !over.contains("body="));
    }

    #[test]
    fn empty_body_needs_no_clipboard() {
        let (url, clipped) = mailto_url(&draft("S", "  ", &[]), MAILTO_MAX_LEN);
        assert_eq!(url, "mailto:?subject=S");
        assert!(!clipped);
    }

    fn fixed_date() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 29, 10, 30, 0).unwrap()
    }

    #[test]
    fn eml_golden() {
        let d = MailDraft {
            to: vec!["anna@firma.de".into()],
            subject: "Nachbereitung: Angebot".into(),
            body_text: "Hallo Anna,\n\nvielen Dank.".into(),
            body_html: String::new(),
        };
        let eml = String::from_utf8(write_eml(&d, fixed_date(), 0x1234)).unwrap();
        let expected = concat!(
            "Message-ID: <lva-0000000000001234@local-voice-ai.invalid>\r\n",
            "Date: Tue, 29 Sep 2026 10:30:00 +0000\r\n",
            "Subject: Nachbereitung: Angebot\r\n",
            "X-Unsent: 1\r\n",
            "To: <anna@firma.de>\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/alternative; boundary=\"=_lva_0000000000001234\"\r\n",
            "\r\n",
            "\r\n",
            "--=_lva_0000000000001234\r\n",
            "Content-Type: text/plain; charset=\"utf-8\"\r\n",
            "Content-Transfer-Encoding: 7bit\r\n",
            "\r\n",
            "Hallo Anna,\r\n",
            "\r\n",
            "vielen Dank.\r\n",
            "--=_lva_0000000000001234\r\n",
            "Content-Type: text/html; charset=\"utf-8\"\r\n",
            "Content-Transfer-Encoding: 7bit\r\n",
            "\r\n",
            "<!DOCTYPE html>\r\n",
            "<html><head><meta charset=\"utf-8\"></head><body>\r\n",
            "<p>Hallo Anna,</p>\r\n",
            "<p>vielen Dank.</p>\r\n",
            "</body></html>\r\n",
            "--=_lva_0000000000001234--\r\n",
        );
        assert_eq!(eml, expected);
    }

    #[test]
    fn eml_marks_the_message_as_unsent_and_is_deterministic() {
        let d = draft("Angebot", "Text", &["a@x.de"]);
        let a = write_eml(&d, fixed_date(), 7);
        let b = write_eml(&d, fixed_date(), 7);
        assert_eq!(a, b, "gleiche Eingabe, gleiche Bytes");
        assert!(String::from_utf8(a)
            .unwrap()
            .contains("\r\nX-Unsent: 1\r\n"));
        let c = write_eml(&d, fixed_date(), 8);
        assert_ne!(b, c, "andere Boundary");
    }

    #[test]
    fn eml_subject_with_umlauts_is_rfc2047_encoded() {
        let d = draft("Nächste Schritte – Größe", "Text", &[]);
        let eml = String::from_utf8(write_eml(&d, fixed_date(), 1)).unwrap();
        let line = eml
            .lines()
            .find(|l| l.starts_with("Subject:"))
            .expect("Subject-Zeile");
        assert!(line.contains("=?utf-8?"), "{line}");
        assert!(line.is_ascii(), "Kopfzeilen sind reines ASCII: {line}");
        assert!(!eml.contains("\r\nTo:"), "ohne Empfänger keine To-Zeile");
    }

    #[test]
    fn eml_body_with_umlauts_is_transfer_encoded_utf8() {
        let d = draft("S", "Grüße aus Köln – bis bald", &["a@x.de"]);
        let eml = String::from_utf8(write_eml(&d, fixed_date(), 1)).unwrap();
        assert!(eml.contains("charset=\"utf-8\""));
        assert!(
            eml.contains("Content-Transfer-Encoding: quoted-printable")
                || eml.contains("Content-Transfer-Encoding: base64"),
            "{eml}"
        );
    }

    #[test]
    fn eml_drops_a_header_injection_in_recipients_and_subject() {
        let d = draft(
            "Hallo\r\nBcc: geheim@x.de",
            "Text",
            &["a@x.de\r\nBcc: geheim@x.de"],
        );
        let eml = String::from_utf8(write_eml(&d, fixed_date(), 1)).unwrap();
        assert!(!eml.contains("\r\nBcc:"), "{eml}");
    }

    #[test]
    fn own_addresses_are_normalised_for_the_setting() {
        let list = vec![
            " Ich@Wolff.de ".to_string(),
            "ich@wolff.de".to_string(),
            "kein-mail".to_string(),
        ];
        assert_eq!(
            normalize_self_emails(&list),
            vec!["ich@wolff.de".to_string()]
        );
    }

    #[test]
    fn edited_text_rebuilds_the_html() {
        let mut d = draft_from_answer("Betreff: S\nAlt", "T", vec![]);
        d.body_text = "Neu & besser".into();
        let f = finalize(&d);
        assert_eq!(f.body_html, "<p>Neu &amp; besser</p>");
    }

    fn store() -> MeetingStore {
        let dir = tempfile::tempdir().unwrap();
        let s = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        std::mem::forget(dir);
        s
    }

    fn add_person(
        s: &MeetingStore,
        meeting: &str,
        id: &str,
        name: &str,
        email: Option<&str>,
        is_self: bool,
    ) {
        let conn = s.get_connection().unwrap();
        conn.execute(
            "INSERT INTO humans (id, name, email_norm, is_self, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, 1)",
            rusqlite::params![id, name, email, is_self as i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meeting_participants (meeting_id, human_id, role, source, created_at)
             VALUES (?1, ?2, 'attendee', 'calendar', 1)",
            rusqlite::params![meeting, id],
        )
        .unwrap();
    }

    #[test]
    fn recipients_come_from_participants_without_myself() {
        let s = store();
        let m = s
            .create_meeting("Jour fixe", MeetingSource::Live, Some(1))
            .unwrap();
        add_person(&s, &m.id, "H1", "Anna Berg", Some("anna@firma.de"), false);
        add_person(&s, &m.id, "H2", "Ich", Some("ich@wolff.de"), true);
        add_person(&s, &m.id, "H3", "Ben Ohne", None, false);
        add_person(
            &s,
            &m.id,
            "H4",
            "Ich Privat",
            Some("privat@wolff.de"),
            false,
        );
        let r = participant_recipients(&s, &m.id, &["Privat@Wolff.de".to_string()]).unwrap();
        assert_eq!(r.emails, vec!["anna@firma.de".to_string()]);
        assert_eq!(
            r.names,
            vec!["Anna Berg".to_string(), "Ben Ohne".to_string()]
        );
    }

    #[test]
    fn a_meeting_without_participants_has_no_recipients() {
        let s = store();
        let m = s
            .create_meeting("Allein", MeetingSource::Live, Some(1))
            .unwrap();
        assert_eq!(
            participant_recipients(&s, &m.id, &[]).unwrap(),
            Recipients::default()
        );
    }
}
