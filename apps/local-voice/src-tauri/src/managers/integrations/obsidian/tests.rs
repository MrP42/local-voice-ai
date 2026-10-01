use super::*;

const GOLDEN: &str = include_str!("golden_note.md");

fn cfg(root: &Path) -> ObsidianConfig {
    ObsidianConfig {
        path: root.to_str().unwrap().to_string(),
        subfolder: DEFAULT_SUBFOLDER.to_string(),
        context_area: DEFAULT_AREA.to_string(),
        tier: DEFAULT_TIER.to_string(),
    }
}

fn input() -> NoteInput {
    NoteInput {
        meeting_id: "01J8ZTESTMEETING0000000001".to_string(),
        title: "Wochenmeeting „Planung“ Q4".to_string(),
        date_label: "30.09.2026 14:30".to_string(),
        date_iso: "2026-09-30".to_string(),
        source: "besprechung".to_string(),
        source_url: None,
        body_md: "# Wochenmeeting „Planung“ Q4\n\n**Datum:** 30.09.2026 14:30\n\n## KI-Notizen\n\n- Budget bleibt\n- \"Zitat\" mit Anführungszeichen\n".to_string(),
    }
}

fn vault() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    std::fs::create_dir_all(root.join("00_inbox")).unwrap();
    std::fs::create_dir_all(root.join("10_contexts").join("beruf")).unwrap();
    (dir, root)
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel))
        .unwrap()
        .replace("\r\n", "\n")
}

#[test]
fn golden_note_matches_byte_for_byte() {
    let note = render_note(&cfg(Path::new("C:/V")), &input());
    assert_eq!(note, GOLDEN.replace("\r\n", "\n"));
}

#[test]
fn the_frontmatter_carries_the_ai_os_contract() {
    let note = render_note(&cfg(Path::new("C:/V")), &input());
    assert!(note.starts_with(
        "---
"
    ));
    for field in [
        "title: \"",
        "tags: [",
        "context_area: beruf
",
        "data_class: confidential
",
        "sensitivity: \"normal\"
",
        "tier: propose
",
    ] {
        assert!(
            note.contains(&format!(
                "
{field}"
            )),
            "{field} fehlt"
        );
    }
}

#[test]
fn data_class_follows_the_source() {
    assert_eq!(data_class_for("besprechung"), "confidential");
    assert_eq!(data_class_for("import"), "confidential");
    assert_eq!(data_class_for("youtube"), "internal");
    let mut n = input();
    n.source = "youtube".to_string();
    n.source_url = Some("https://www.youtube.com/watch?v=abc".to_string());
    let note = render_note(&cfg(Path::new("C:/V")), &n);
    assert!(note.contains("data_class: internal\n"));
    assert!(note.contains("tags: [video, youtube, local-voice-ai]"));
    assert!(note.contains("quelle_url: \"https://www.youtube.com/watch?v=abc\"\n"));
    assert!(note.contains("Video „Wochenmeeting"));
}

#[test]
fn a_hostile_title_cannot_break_out_of_the_frontmatter() {
    let mut n = input();
    n.title = "Evil\"\ntier: auto\n---\n# x".to_string();
    let note = render_note(&cfg(Path::new("C:/V")), &n);
    // Genau ein Frontmatter-Block, `tier` nur einmal und mit dem Wert der Einstellung.
    assert_eq!(note.matches("\ntier:").count(), 1);
    assert!(note.contains("\ntier: propose\n"));
    assert!(note.lines().next() == Some("---"));
    assert_eq!(note.lines().filter(|l| *l == "---").count(), 3, "{note}");
}

#[test]
fn a_javascript_or_odd_source_url_is_not_written() {
    let mut n = input();
    n.source = "youtube".to_string();
    n.source_url = Some("javascript:alert(1)".to_string());
    assert!(!render_note(&cfg(Path::new("C:/V")), &n).contains("quelle_url"));
    n.source_url = Some("https://x.de/a b".to_string());
    assert!(!render_note(&cfg(Path::new("C:/V")), &n).contains("quelle_url"));
}

#[test]
fn a_new_note_lands_in_the_inbox_with_a_dated_name() {
    let (_d, root) = vault();
    let r = save_note(&cfg(&root), &input()).unwrap();
    assert_eq!(r.kind, SaveKind::Created);
    assert_eq!(r.rel, "00_inbox/2026-09-30 Wochenmeeting „Planung“ Q4.md");
    assert_eq!(read(&root, &r.rel), GOLDEN.replace("\r\n", "\n"));
}

#[test]
fn saving_twice_creates_no_duplicate() {
    let (_d, root) = vault();
    let first = save_note(&cfg(&root), &input()).unwrap();
    let second = save_note(&cfg(&root), &input()).unwrap();
    assert_eq!(first.kind, SaveKind::Created);
    assert_eq!(second.kind, SaveKind::Unchanged);
    assert_eq!(second.rel, first.rel);
    let count = std::fs::read_dir(root.join("00_inbox")).unwrap().count();
    assert_eq!(count, 1);
}

#[test]
fn a_changed_meeting_updates_the_block_and_keeps_manual_edits() {
    let (_d, root) = vault();
    let first = save_note(&cfg(&root), &input()).unwrap();
    // Nutzer ergaenzt in Obsidian und aendert Bereich und Autonomie nach der Triage.
    let path = root.join(&first.rel);
    let edited = read(&root, &first.rel)
        .replace("context_area: beruf", "context_area: kunden")
        .replace("tier: propose", "tier: logged")
        + "\n## Meine Gedanken\n\nRueckruf am Montag.\n";
    std::fs::write(&path, edited).unwrap();

    let mut n = input();
    n.date_iso = "2026-10-02".to_string();
    n.body_md = "# Wochenmeeting „Planung“ Q4\n\n## KI-Notizen\n\n- Budget steigt\n".to_string();
    let second = save_note(&cfg(&root), &n).unwrap();
    assert_eq!(second.kind, SaveKind::Updated);
    assert_eq!(second.rel, first.rel);
    let text = read(&root, &second.rel);
    assert!(text.contains("- Budget steigt"));
    assert!(!text.contains("- Budget bleibt"));
    assert!(text.contains("updated: \"2026-10-02\""));
    assert!(
        text.contains("context_area: kunden"),
        "Bereich der Triage blieb nicht"
    );
    assert!(text.contains("tier: logged"));
    assert!(text.contains("Rueckruf am Montag."));
    assert_eq!(text.matches(BEGIN_MARK).count(), 1);
    assert_eq!(text.matches("lva_id:").count(), 1);
}

#[test]
fn a_note_moved_out_of_the_inbox_is_still_found() {
    let (_d, root) = vault();
    let first = save_note(&cfg(&root), &input()).unwrap();
    let moved = root.join("10_contexts").join("beruf").join("Planung Q4.md");
    std::fs::rename(root.join(&first.rel), &moved).unwrap();
    let mut n = input();
    n.body_md = "# Neu\n\nnur das\n".to_string();
    let second = save_note(&cfg(&root), &n).unwrap();
    assert_eq!(second.kind, SaveKind::Updated);
    assert_eq!(second.rel, "10_contexts/beruf/Planung Q4.md");
    assert_eq!(std::fs::read_dir(root.join("00_inbox")).unwrap().count(), 0);
}

#[test]
fn missing_markers_append_a_new_block_instead_of_overwriting() {
    let existing = "---\ntitle: \"X\"\nupdated: \"2026-01-01\"\nlva_id: \"meeting-1\"\n---\n\nMein Text ohne Marken.\n";
    let mut n = input();
    n.meeting_id = "1".to_string();
    let merged = merge_existing(existing, &n).unwrap();
    assert!(merged.contains("Mein Text ohne Marken."));
    assert!(merged.contains(BEGIN_MARK) && merged.contains(END_MARK));
    assert!(merged.contains("updated: \"2026-09-30\""));
}

#[test]
fn only_the_frontmatter_id_counts_not_text_in_the_body() {
    let (_d, root) = vault();
    // Eine fremde Notiz, die die Kennung nur im Text zitiert, ist keine Fundstelle.
    std::fs::write(
        root.join("00_inbox").join("zitat.md"),
        "---\ntitle: \"Zitat\"\n---\n\nlva_id: \"meeting-01J8ZTESTMEETING0000000001\"\n",
    )
    .unwrap();
    let sb = Sandbox::open(root.to_str().unwrap()).unwrap();
    assert_eq!(
        find_by_id(&sb, "meeting-01J8ZTESTMEETING0000000001").unwrap(),
        None
    );
    let r = save_note(&cfg(&root), &input()).unwrap();
    assert_eq!(r.kind, SaveKind::Created);
}

#[test]
fn the_scan_skips_obsidian_internals_and_links() {
    let (d, root) = vault();
    let outside = d.path().join("anderer-vault");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("n.md"), "---\nlva_id: \"meeting-X\"\n---\n").unwrap();
    std::fs::create_dir_all(root.join(".obsidian")).unwrap();
    std::fs::write(
        root.join(".obsidian").join("n.md"),
        "---\nlva_id: \"meeting-X\"\n---\n",
    )
    .unwrap();
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(root.join("verknuepft"))
            .arg(&outside)
            .output();
    }
    #[cfg(unix)]
    {
        let _ = std::os::unix::fs::symlink(&outside, root.join("verknuepft"));
    }
    let sb = Sandbox::open(root.to_str().unwrap()).unwrap();
    assert_eq!(find_by_id(&sb, "meeting-X").unwrap(), None);
}

#[test]
fn a_vault_path_that_does_not_exist_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let c = cfg(&dir.path().join("gibt-es-nicht"));
    let err = save_note(&c, &input()).unwrap_err();
    assert_eq!(err.code(), "vault_path_not_found");
    assert!(err.to_string().contains("Vault"));
}

#[test]
fn a_hostile_subfolder_cannot_leave_the_vault() {
    let (d, root) = vault();
    let mut c = cfg(&root);
    c.subfolder = "../außen".to_string();
    let err = save_note(&c, &input()).unwrap_err();
    assert_eq!(err.code(), "folder_path_escape");
    assert!(!d.path().join("außen").exists());
}

#[test]
fn config_fields_are_validated() {
    let (_d, root) = vault();
    let mut c = cfg(&root);
    c.context_area = "geheim".to_string();
    assert!(c.validate_fields().is_err());
    let mut c = cfg(&root);
    c.tier = "alles-erlaubt".to_string();
    assert!(c.validate_fields().is_err());
    let parsed = ObsidianConfig::from_config_json(&format!(
        "{{\"path\":{}}}",
        serde_json::to_string(root.to_str().unwrap()).unwrap()
    ))
    .unwrap();
    assert_eq!(parsed.subfolder, DEFAULT_SUBFOLDER);
    assert_eq!(parsed.context_area, DEFAULT_AREA);
    assert_eq!(parsed.tier, DEFAULT_TIER);
}

#[test]
fn the_connection_test_leaves_no_probe_file_behind() {
    let (_d, root) = vault();
    test(&cfg(&root)).unwrap();
    assert_eq!(std::fs::read_dir(root.join("00_inbox")).unwrap().count(), 0);
    let dir = tempfile::tempdir().unwrap();
    assert!(test(&cfg(&dir.path().join("weg"))).is_err());
}
