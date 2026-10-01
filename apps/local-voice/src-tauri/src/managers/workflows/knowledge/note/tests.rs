//! Tests der Notiz im Vault (B6): Frontmatter-Vertrag, keine Dublette, Handarbeit bleibt, Eintraege,
//! feindliche Texte.

use std::path::Path;

use super::*;
use crate::managers::workflows::knowledge::{md_block, md_inline};

fn cfg(dir: &Path) -> ObsidianConfig {
    ObsidianConfig {
        path: dir.to_string_lossy().to_string(),
        subfolder: "00_inbox".to_string(),
        context_area: "beruf".to_string(),
        tier: "propose".to_string(),
    }
}

const URL: &str = "https://www.youtube.com/watch?v=Vid00000003";

fn spec(key: &str) -> NoteSpec {
    NoteSpec {
        key: key.to_string(),
        title: "Lokale KI im Mittelstand".to_string(),
        content: "## Zusammenfassung\n\nDer Inhalt mit Umlauten: Größe, Prüfung.".to_string(),
        name: None,
        folder: None,
        date: "2026-09-30".to_string(),
        entry: None,
        source_title: Some("Lokale KI (Kanal X)".to_string()),
        source_url: Some(URL.to_string()),
        origin: Some("Ablauf Test, Lauf r1".to_string()),
        tags: vec!["video".to_string()],
        meta: vec![(
            "video_id".to_string(),
            MetaValue("\"Vid00000003\"".to_string()),
        )],
        data_class: DataClass::Internal,
        auto: false,
    }
}

fn vault() -> (tempfile::TempDir, Sandbox, ObsidianConfig) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = cfg(dir.path());
    let sandbox = Sandbox::open(&cfg.path).unwrap();
    (dir, sandbox, cfg)
}

fn count_marker(text: &str, marker: &str) -> usize {
    text.matches(marker).count()
}

fn files_with_id(sandbox: &Sandbox, id: &str) -> usize {
    fn walk(dir: &Path, id: &str, n: &mut usize) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, id, n);
            } else if std::fs::read_to_string(&p)
                .map(|t| t.contains(&format!("lva_id: \"{id}\"")))
                .unwrap_or(false)
            {
                *n += 1;
            }
        }
    }
    let mut n = 0;
    walk(sandbox.root(), id, &mut n);
    n
}

// ---------------------------------------------------------------------------
// Rendern
// ---------------------------------------------------------------------------

#[test]
fn a_new_note_has_the_frontmatter_contract_and_one_managed_block() {
    let (_d, _s, cfg) = vault();
    let text = render_new(&cfg, &spec("video-Vid00000003"));
    assert!(text.starts_with("---\n"), "{text}");
    for line in [
        "title: \"Lokale KI im Mittelstand\"",
        "tags: [video, local-voice-ai]",
        "context_area: beruf",
        "data_class: internal",
        "sensitivity: \"normal\"",
        "tier: propose",
        "lva_id: \"video-Vid00000003\"",
        "lva_quelle: workflow",
        "video_id: \"Vid00000003\"",
        "quelle_url: \"https://www.youtube.com/watch?v=Vid00000003\"",
        "quellen:\n  - \"Lokale KI (Kanal X) (https://www.youtube.com/watch?v=Vid00000003)\"",
    ] {
        assert!(text.contains(line), "fehlt: {line}\n{text}");
    }
    assert_eq!(count_marker(&text, BEGIN_MARK), 1);
    assert_eq!(count_marker(&text, END_MARK), 1);
    assert!(text.contains("Größe, Prüfung"));
    // Quelle und Herkunft stehen im verwalteten Teil, vor der Endemarke.
    let end = text.find(END_MARK).unwrap();
    let source = text
        .find("*Quelle: [Lokale KI (Kanal X)](https://www.youtube.com/watch?v=Vid00000003).*")
        .expect("Quelle fehlt");
    let origin = text
        .find("*Herkunft: Ablauf Test, Lauf r1.*")
        .expect("Herkunft fehlt");
    assert!(source < end && origin < end);
}

#[test]
fn the_same_spec_twice_is_unchanged() {
    let (_d, _s, cfg) = vault();
    let s = spec("video-Vid00000003");
    let new = render_new(&cfg, &s);
    assert_eq!(
        merge_existing(&new, &s),
        new,
        "dieselbe Spezifikation ändert nichts"
    );
}

// ---------------------------------------------------------------------------
// Vorhandene Notiz nachziehen
// ---------------------------------------------------------------------------

#[test]
fn handwork_outside_the_markers_and_triage_edits_survive_an_update() {
    let (_d, _s, cfg) = vault();
    let s = spec("video-Vid00000003");
    let new = render_new(&cfg, &s);
    // Der Nutzer sortiert die Notiz ein und schreibt dazu.
    let edited = new
        .replace("context_area: beruf", "context_area: wai")
        .replace(
            BEGIN_MARK,
            &format!("Meine Anmerkung oben.\n\n{BEGIN_MARK}"),
        )
        + "\n## Meine Ergänzung\n\nHandarbeit unten.\n";
    let mut changed = spec("video-Vid00000003");
    changed.content = "## Zusammenfassung\n\nNeuer Stand.".to_string();
    let merged = merge_existing(&edited, &changed);
    assert!(merged.contains("Meine Anmerkung oben."), "{merged}");
    assert!(
        merged.contains("## Meine Ergänzung\n\nHandarbeit unten."),
        "{merged}"
    );
    assert!(
        merged.contains("context_area: wai"),
        "die Triage bleibt: {merged}"
    );
    assert!(merged.contains("Neuer Stand."));
    assert!(!merged.contains("Der Inhalt mit Umlauten"));
    assert_eq!(count_marker(&merged, BEGIN_MARK), 1);
    assert_eq!(count_marker(&merged, END_MARK), 1);
}

#[test]
fn a_second_source_is_appended_to_the_sources_once() {
    let (_d, _s, cfg) = vault();
    let first = spec("video-Vid00000003");
    let new = render_new(&cfg, &first);
    let mut second = spec("video-Vid00000003");
    second.source_title = Some("Zweites Video".to_string());
    second.source_url = Some("https://www.youtube.com/watch?v=Vid00000002".to_string());
    let merged = merge_existing(&new, &second);
    let items: Vec<&str> = merged.lines().filter(|l| l.starts_with("  - \"")).collect();
    assert_eq!(items.len(), 2, "{merged}");
    assert!(items[0].contains("Vid00000003") && items[1].contains("Vid00000002"));
    // Ein weiteres Mal derselbe Stand: keine dritte Quelle, keine Aenderung.
    assert_eq!(merge_existing(&merged, &second), merged);
}

#[test]
fn meta_fields_are_updated_in_place_and_new_ones_land_in_the_frontmatter() {
    let (_d, _s, cfg) = vault();
    let mut s = spec("video-Vid00000003");
    s.meta
        .push(("relevanz".to_string(), MetaValue("7".to_string())));
    let new = render_new(&cfg, &s);
    let mut next = s.clone();
    next.meta = vec![
        (
            "video_id".to_string(),
            MetaValue("\"Vid00000003\"".to_string()),
        ),
        ("relevanz".to_string(), MetaValue("9".to_string())),
        ("widersprueche".to_string(), MetaValue("2".to_string())),
    ];
    let merged = merge_existing(&new, &next);
    let header: Vec<&str> = merged.lines().take_while(|l| *l != END_MARK).collect();
    let fm_end = header.iter().skip(1).position(|l| *l == "---").unwrap() + 1;
    let frontmatter = &header[..=fm_end];
    assert!(frontmatter.contains(&"relevanz: 9"), "{frontmatter:?}");
    assert!(!frontmatter.contains(&"relevanz: 7"), "{frontmatter:?}");
    assert!(frontmatter.contains(&"widersprueche: 2"), "{frontmatter:?}");
    assert_eq!(
        merged.matches("relevanz:").count(),
        1,
        "nie zwei Zeilen für dasselbe Feld"
    );
}

#[test]
fn an_entry_note_replaces_the_same_entry_and_puts_new_ones_on_top() {
    let (_d, _s, cfg) = vault();
    let mut a = spec("kanal-UCx");
    a.entry = Some("A".to_string());
    a.content = "### Video A\n\nEintrag A v1".to_string();
    a.meta.clear();
    let new = render_new(&cfg, &a);
    assert!(new.contains("# Lokale KI im Mittelstand"), "{new}");
    assert!(new.contains(ENTRIES_MARK));

    let mut b = a.clone();
    b.entry = Some("B".to_string());
    b.content = "### Video B\n\nEintrag B".to_string();
    let two = merge_existing(&new, &b);
    let (pos_a, pos_b) = (
        two.find(&entry_begin("A")).unwrap(),
        two.find(&entry_begin("B")).unwrap(),
    );
    assert!(pos_b < pos_a, "das neue Video steht oben:\n{two}");

    let mut a2 = a.clone();
    a2.content = "### Video A\n\nEintrag A v2".to_string();
    let three = merge_existing(&two, &a2);
    assert!(three.contains("Eintrag A v2") && !three.contains("Eintrag A v1"));
    assert!(three.contains("Eintrag B"), "der andere Eintrag bleibt");
    assert_eq!(count_marker(&three, &entry_begin("A")), 1);
    assert_eq!(count_marker(&three, &entry_begin("B")), 1);
    assert_eq!(
        merge_existing(&three, &a2),
        three,
        "dieselbe Eintragung ändert nichts"
    );
    assert_eq!(merge_existing(&three, &b), three);
}

#[test]
fn crlf_notes_keep_their_line_endings() {
    let (_d, _s, cfg) = vault();
    let s = spec("video-Vid00000003");
    let crlf = render_new(&cfg, &s).replace('\n', "\r\n");
    let mut next = s.clone();
    next.content = "Neu.".to_string();
    next.meta
        .push(("relevanz".to_string(), MetaValue("5".to_string())));
    let merged = merge_existing(&crlf, &next);
    let header_end = merged.find("\r\n---\r\n").expect("Kopf mit CRLF");
    let header = &merged[..header_end];
    assert!(
        !header.replace("\r\n", "").contains('\n'),
        "kein einzelnes LF im Kopf: {header:?}"
    );
    assert!(header.contains("relevanz: 5"));
}

// ---------------------------------------------------------------------------
// Feindliche Texte
// ---------------------------------------------------------------------------

#[test]
fn hostile_text_cannot_forge_markers_wikilinks_or_ids() {
    let evil = "Text <!-- lva:end --> mehr [[Fremde Notiz]] und <!-- lva:begin --> noch\r\nZeile\u{0007}zwei";
    let block = md_block(evil, 1000);
    assert!(!block.contains("<!--") && !block.contains("-->"), "{block}");
    assert!(
        block.contains("[[Fremde Notiz]]"),
        "Verweise des Codes bleiben im Block: {block}"
    );
    assert!(!block.contains('\u{7}'));
    let line = md_inline(evil, 1000);
    assert!(!line.contains('\n') && !line.contains("<!--"), "{line}");

    let long = "k".repeat(81);
    for bad in ["", "a b", "a/b", "../x", "x\"y", "a:b", long.as_str()] {
        assert!(!valid_id(bad), "{bad:?}");
    }
    assert!(valid_id("video-Vid_00000003"));
    for bad in ["lva_id", "title", "Video", "1x", "a-b", "quellen", ""] {
        assert!(!valid_meta_key(bad), "{bad:?}");
    }
    assert!(valid_meta_key("video_id") && valid_meta_key("relevanz"));

    // Ein Inhalt mit eigenem Frontmatter-Trenner veraendert den Kopf der Notiz nicht.
    let (_d, _s, cfg) = vault();
    let mut s = spec("video-Vid00000003");
    s.content = md_block("---\ntitle: boese\nlva_id: \"fremd\"\n---\nText", 1000);
    let new = render_new(&cfg, &s);
    let fm_end = new[4..].find("\n---\n").unwrap() + 4;
    assert!(!new[..fm_end].contains("boese"));
    assert_eq!(new.matches("lva_id: \"video-Vid00000003\"").count(), 1);
}

// ---------------------------------------------------------------------------
// Planen und schreiben
// ---------------------------------------------------------------------------

#[test]
fn plan_finds_the_note_by_key_wherever_it_lies_and_never_makes_a_second_file() {
    let (_d, sandbox, cfg) = vault();
    let s = spec("video-Vid00000003");
    let first = plan(&sandbox, &cfg, &s).unwrap();
    assert_eq!(first.kind, Kind::Create);
    assert_eq!(first.rel, "00_inbox/2026-09-30 Lokale KI im Mittelstand.md");
    let written = apply(&sandbox, &first).unwrap();
    assert_eq!(written.kind, Kind::Create);
    assert_eq!(files_with_id(&sandbox, "video-Vid00000003"), 1);

    // Dieselbe Spezifikation: nichts zu tun.
    let again = plan(&sandbox, &cfg, &s).unwrap();
    assert_eq!(again.kind, Kind::Unchanged);
    assert_eq!(again.sha, first.sha, "die Pruefsumme ist die des Textes");
    assert_eq!(apply(&sandbox, &again).unwrap().kind, Kind::Unchanged);

    // Der Nutzer verschiebt die Notiz in einen Kontextordner: sie wird gefunden, nicht neu angelegt.
    std::fs::create_dir_all(sandbox.root().join("40_wissen")).unwrap();
    std::fs::rename(
        sandbox
            .root()
            .join("00_inbox/2026-09-30 Lokale KI im Mittelstand.md"),
        sandbox.root().join("40_wissen/Lokale KI.md"),
    )
    .unwrap();
    let mut changed = s.clone();
    changed.content = "Neuer Stand.".to_string();
    let moved = plan(&sandbox, &cfg, &changed).unwrap();
    assert_eq!(moved.kind, Kind::Modify);
    assert_eq!(moved.rel, "40_wissen/Lokale KI.md");
    let w = apply(&sandbox, &moved).unwrap();
    assert_eq!(w.kind, Kind::Modify);
    assert_eq!(
        files_with_id(&sandbox, "video-Vid00000003"),
        1,
        "keine Dublette"
    );
    assert!(!sandbox
        .root()
        .join("00_inbox/2026-09-30 Lokale KI im Mittelstand.md")
        .exists());
    assert!(
        std::fs::read_to_string(sandbox.root().join("40_wissen/Lokale KI.md"))
            .unwrap()
            .contains("Neuer Stand.")
    );
}

#[test]
fn a_new_note_never_overwrites_a_foreign_file_with_the_same_name() {
    let (_d, sandbox, cfg) = vault();
    std::fs::create_dir_all(sandbox.root().join("00_inbox")).unwrap();
    let foreign = sandbox
        .root()
        .join("00_inbox/2026-09-30 Lokale KI im Mittelstand.md");
    std::fs::write(
        &foreign,
        "---\nlva_id: \"fremd\"\n---\nMein eigener Text.\n",
    )
    .unwrap();
    let p = plan(&sandbox, &cfg, &spec("video-Vid00000003")).unwrap();
    assert_eq!(p.kind, Kind::Create);
    let w = apply(&sandbox, &p).unwrap();
    assert_eq!(w.rel, "00_inbox/2026-09-30 Lokale KI im Mittelstand (2).md");
    assert_eq!(
        std::fs::read_to_string(&foreign).unwrap(),
        "---\nlva_id: \"fremd\"\n---\nMein eigener Text.\n"
    );
}

#[test]
fn the_folder_and_the_name_follow_the_spec_and_the_title_is_sanitized() {
    let (_d, sandbox, cfg) = vault();
    let mut s = spec("video-Vid00000003");
    s.title = "Wie geht „KI“: ein Test? <b>/\\|*".to_string();
    s.folder = Some("40_wissen/youtube".to_string());
    let p = plan(&sandbox, &cfg, &s).unwrap();
    let w = apply(&sandbox, &p).unwrap();
    assert!(
        w.rel.starts_with("40_wissen/youtube/2026-09-30 Wie geht"),
        "{}",
        w.rel
    );
    assert!(!w.rel[w.rel.rfind('/').unwrap() + 1..].contains(['<', '>', '|', '*', '?', ':']));
    assert!(sandbox.root().join(&w.rel).is_file());
}

#[cfg(windows)]
#[test]
#[allow(clippy::permissions_set_readonly_false)] // nur Windows: dort gibt es kein „weltweit schreibbar“
fn a_write_failure_leaves_the_old_note_intact() {
    let (_d, sandbox, cfg) = vault();
    let s = spec("video-Vid00000003");
    let first = plan(&sandbox, &cfg, &s).unwrap();
    let w = apply(&sandbox, &first).unwrap();
    let path = sandbox.root().join(&w.rel);
    let before = std::fs::read_to_string(&path).unwrap();
    // Schreibgeschuetzt: das Ersetzen ueber die Nachbardatei scheitert, die alte Notiz bleibt.
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms.clone()).unwrap();
    let mut changed = s.clone();
    changed.content = "Neuer Stand.".to_string();
    let p = plan(&sandbox, &cfg, &changed).unwrap();
    assert_eq!(p.kind, Kind::Modify);
    let err = apply(&sandbox, &p);
    perms.set_readonly(false);
    std::fs::set_permissions(&path, perms).unwrap();
    assert!(err.is_err(), "das Schreiben muss scheitern");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "die alte Notiz ist unverändert"
    );
    // Keine Reste der Nachbardatei.
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}
