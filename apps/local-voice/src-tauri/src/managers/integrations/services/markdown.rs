//! Kleine Markdown-Umsetzer fuer Protokolle und Notizen: Ueberschriften (#, ##, ###),
//! Aufzaehlungen (- / * / 1.) und Absaetze. Mehr braucht ein Protokoll nicht; alles andere
//! bleibt Text. Kein HTML wird durchgereicht (Confluence: alles maskiert).

use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(u8, String),
    Bullet(String),
    Numbered(String),
    Paragraph(String),
}

/// Zerlegt Markdown zeilenweise in Bloecke. Leerzeilen trennen Absaetze; aufeinanderfolgende
/// Textzeilen werden ein Absatz.
pub fn blocks(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let flush = |para: &mut Vec<String>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Paragraph(para.join(" ")));
            para.clear();
        }
    };
    for raw in md.lines() {
        let line = raw.trim();
        if line.is_empty() {
            flush(&mut para, &mut out);
            continue;
        }
        let heading = line
            .strip_prefix("### ")
            .map(|t| (3, t))
            .or_else(|| line.strip_prefix("## ").map(|t| (2, t)))
            .or_else(|| line.strip_prefix("# ").map(|t| (1, t)));
        if let Some((level, text)) = heading {
            flush(&mut para, &mut out);
            out.push(Block::Heading(level, strip_inline(text)));
        } else if let Some(text) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush(&mut para, &mut out);
            out.push(Block::Bullet(strip_inline(text)));
        } else if let Some(text) = numbered(line) {
            flush(&mut para, &mut out);
            out.push(Block::Numbered(strip_inline(text)));
        } else {
            para.push(strip_inline(line));
        }
    }
    flush(&mut para, &mut out);
    out
}

fn numbered(line: &str) -> Option<&str> {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    line[digits..].strip_prefix(". ")
}

/// Fett/Kursiv-Zeichen entfernen (die Dienste haben eigene Formatierung).
fn strip_inline(s: &str) -> String {
    s.replace("**", "").replace("__", "").trim().to_string()
}

/// Text in Stuecke von hoechstens `max` Zeichen (an Zeichengrenzen).
pub fn chunks(s: &str, max: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars.chunks(max).map(|c| c.iter().collect()).collect()
}

/// Notion-Bloecke (je Rich-Text hoechstens 2000 Zeichen).
pub fn notion_blocks(md: &str) -> Vec<Value> {
    let rich = |t: &str| -> Value {
        Value::Array(
            chunks(t, 2000)
                .into_iter()
                .map(|c| json!({"type": "text", "text": {"content": c}}))
                .collect(),
        )
    };
    blocks(md)
        .into_iter()
        .map(|b| match b {
            Block::Heading(l, t) => {
                let kind = format!("heading_{}", l.min(3));
                let mut block = serde_json::Map::new();
                block.insert("object".into(), json!("block"));
                block.insert("type".into(), json!(kind));
                block.insert(kind, json!({"rich_text": rich(&t)}));
                Value::Object(block)
            }
            Block::Bullet(t) => json!({"object": "block", "type": "bulleted_list_item",
                "bulleted_list_item": {"rich_text": rich(&t)}}),
            Block::Numbered(t) => json!({"object": "block", "type": "numbered_list_item",
                "numbered_list_item": {"rich_text": rich(&t)}}),
            Block::Paragraph(t) => json!({"object": "block", "type": "paragraph",
                "paragraph": {"rich_text": rich(&t)}}),
        })
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Confluence-Storage-Format (XHTML), alles Inhaltliche maskiert.
pub fn confluence_storage(md: &str) -> String {
    let mut out = String::new();
    let mut list: Option<&str> = None;
    let close = |out: &mut String, list: &mut Option<&str>| {
        if let Some(tag) = list.take() {
            out.push_str(&format!("</{tag}>"));
        }
    };
    for b in blocks(md) {
        let want = match &b {
            Block::Numbered(_) => "ol",
            _ => "ul",
        };
        match b {
            Block::Bullet(t) | Block::Numbered(t) => {
                if list != Some(want) {
                    close(&mut out, &mut list);
                    out.push_str(&format!("<{want}>"));
                    list = Some(want);
                }
                out.push_str(&format!("<li>{}</li>", escape(&t)));
            }
            Block::Heading(l, t) => {
                close(&mut out, &mut list);
                out.push_str(&format!("<h{l}>{}</h{l}>", escape(&t)));
            }
            Block::Paragraph(t) => {
                close(&mut out, &mut list);
                out.push_str(&format!("<p>{}</p>", escape(&t)));
            }
        }
    }
    close(&mut out, &mut list);
    out
}

/// Jira-Beschreibung im Atlassian Document Format (Absaetze und Aufzaehlungen).
pub fn jira_adf(md: &str) -> Value {
    let text = |t: &str| json!([{"type": "text", "text": t}]);
    let mut content: Vec<Value> = Vec::new();
    let mut bullets: Vec<Value> = Vec::new();
    let flush = |content: &mut Vec<Value>, bullets: &mut Vec<Value>| {
        if !bullets.is_empty() {
            content.push(json!({"type": "bulletList", "content": std::mem::take(bullets)}));
        }
    };
    for b in blocks(md) {
        match b {
            Block::Bullet(t) | Block::Numbered(t) => bullets.push(json!({
                "type": "listItem",
                "content": [{"type": "paragraph", "content": text(&t)}]
            })),
            Block::Heading(l, t) => {
                flush(&mut content, &mut bullets);
                content
                    .push(json!({"type": "heading", "attrs": {"level": l}, "content": text(&t)}));
            }
            Block::Paragraph(t) => {
                flush(&mut content, &mut bullets);
                content.push(json!({"type": "paragraph", "content": text(&t)}));
            }
        }
    }
    flush(&mut content, &mut bullets);
    if content.is_empty() {
        content.push(json!({"type": "paragraph", "content": []}));
    }
    json!({"type": "doc", "version": 1, "content": content})
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD: &str = "# Protokoll\n\nRelease **verschoben**.\nWeiter am Freitag.\n\n## Aufgaben\n- Tests reparieren\n- Mail an Kunde\n\n1. Erstens\n2. Zweitens\n";

    #[test]
    fn markdown_becomes_blocks() {
        assert_eq!(
            blocks(MD),
            vec![
                Block::Heading(1, "Protokoll".into()),
                Block::Paragraph("Release verschoben. Weiter am Freitag.".into()),
                Block::Heading(2, "Aufgaben".into()),
                Block::Bullet("Tests reparieren".into()),
                Block::Bullet("Mail an Kunde".into()),
                Block::Numbered("Erstens".into()),
                Block::Numbered("Zweitens".into()),
            ]
        );
    }

    #[test]
    fn notion_blocks_follow_the_api_shape_and_split_long_text() {
        let b = notion_blocks(MD);
        assert_eq!(b[0]["type"], "heading_1");
        assert_eq!(
            b[0]["heading_1"]["rich_text"][0]["text"]["content"],
            "Protokoll"
        );
        assert_eq!(b[3]["type"], "bulleted_list_item");
        let long = "x".repeat(4500);
        let p = notion_blocks(&long);
        assert_eq!(p[0]["paragraph"]["rich_text"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn confluence_storage_escapes_and_lists() {
        let s = confluence_storage("## A & B\n- <script>\n1. eins\n");
        assert_eq!(
            s,
            "<h2>A &amp; B</h2><ul><li>&lt;script&gt;</li></ul><ol><li>eins</li></ol>"
        );
    }

    #[test]
    fn jira_adf_is_a_valid_document() {
        let d = jira_adf("Text\n- a\n- b");
        assert_eq!(d["type"], "doc");
        assert_eq!(d["version"], 1);
        assert_eq!(d["content"][0]["type"], "paragraph");
        assert_eq!(d["content"][1]["type"], "bulletList");
        assert_eq!(d["content"][1]["content"].as_array().unwrap().len(), 2);
        assert_eq!(jira_adf("")["content"][0]["type"], "paragraph");
    }
}
