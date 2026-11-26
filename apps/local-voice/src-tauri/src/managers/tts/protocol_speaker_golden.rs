//! Gemeinsame Rust-/Swift-Vektoren (lva#954). UPDATE_GOLDEN=1 schreibt das Soll aus
//! den produktiven Funktionen neu; normale Läufe lesen ausschließlich.
use super::*;
use serde_json::{json, Value};

#[test]
fn speaker_script_golden() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apple/tests/speaker-script-golden.json");
    let mut golden: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let speakers = vec![
        KnownSpeaker { id: "anna".into(), names: vec!["Anna".into()] },
        KnownSpeaker { id: "father".into(), names: vec!["German Father".into()] },
        KnownSpeaker { id: "narrator".into(), names: vec!["Erzähler".into()] },
    ];
    let update = std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1");
    for row in golden["cases"].as_array_mut().unwrap() {
        let text = row["text"].as_str().unwrap();
        // NSRange zählt UTF-16, protocol.rs zählt UTF-8-Bytes. Nur die Darstellung
        // der Offsets wird umgerechnet; Scanner und Zerlegung bleiben das Original.
        let utf16 = |byte: usize| text[..byte].encode_utf16().count();
        let actual = json!({
            "tags": tag_spans(text).iter().map(|r| [utf16(r.start), utf16(r.end)]).collect::<Vec<_>>(),
            "markers": scan_marker_candidates(text).iter().map(|m| json!({
                "start": utf16(m.start), "end": utf16(m.end), "name": m.name, "style": m.style
            })).collect::<Vec<_>>(),
            "segments": split_speaker_segments(text, &speakers).iter().map(|s| json!({
                "voice": s.voice, "style": s.style, "text": s.text
            })).collect::<Vec<_>>()
        });
        if update { row["expected"] = actual; }
        else { assert_eq!(row["expected"], actual, "Golden-Fall weicht ab"); }
    }
    for row in golden["names"].as_array_mut().unwrap() {
        let actual = json!(fold_speaker_name(row["input"].as_str().unwrap()));
        if update { row["folded"] = actual; }
        else { assert_eq!(row["folded"], actual); }
    }
    for row in golden["pauses"].as_array_mut().unwrap() {
        let actual = json!(pause_millis(row["input"].as_str().unwrap()));
        if update { row["millis"] = actual; }
        else { assert_eq!(row["millis"], actual); }
    }
    if update {
        let mut bytes = serde_json::to_vec_pretty(&golden).unwrap();
        bytes.push(b'\n');
        std::fs::write(path, bytes).unwrap();
    }
}
