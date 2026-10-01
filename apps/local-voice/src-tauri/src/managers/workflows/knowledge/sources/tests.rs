//! Tests der Suche im Vault (B6): Woerter, Rangfolge, Ausschluss, Grenzen.

use super::*;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn vault_with(notes: &[(&str, &str)]) -> (tempfile::TempDir, Sandbox) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, text) in notes {
        write(dir.path(), rel, text);
    }
    let sandbox = Sandbox::open(&dir.path().to_string_lossy()).unwrap();
    (dir, sandbox)
}

fn never() -> bool {
    false
}

#[test]
fn tokens_fold_umlauts_and_drop_fillers_and_tiny_words() {
    assert_eq!(
        tokens("Größe der Ölpumpe, 98 Prozent & KI! Und 7."),
        vec!["groesse", "oelpumpe", "98", "prozent"]
    );
    assert!(tokens("der die das und oder").is_empty());
    assert_eq!(hash("abc"), hash("abc"));
    assert_ne!(hash("abc"), hash("abd"));
}

#[test]
fn the_vault_search_ranks_by_coverage_and_returns_a_snippet() {
    let (_d, sandbox) = vault_with(&[
        (
            "50_wissen/Lokale Modelle.md",
            "---\ntitle: \"Lokale Modelle im Mittelstand\"\n---\n# Lokale Modelle\n\nLokale Sprachmodelle laufen auch ohne GPU auf Notebooks mit 8 GB Arbeitsspeicher.\n\nEin anderer Absatz über Kaffee.\n",
        ),
        ("50_wissen/Kaffee.md", "Kaffee und Tee im Büro. Keine KI hier.\n"),
        (
            "00_inbox/Teilweise.md",
            "Sprachmodelle sind groß. Mehr steht hier nicht.\n",
        ),
    ]);
    let index = VaultIndex::build(&sandbox, &[], &VaultLimits::default(), &never);
    assert_eq!(index.files_scanned, 3);
    assert!(!index.incomplete);
    let hits = index.search(
        "Lokale Sprachmodelle laufen ohne GPU auf Notebooks mit 8 GB Arbeitsspeicher.",
        5,
    );
    assert_eq!(hits.len(), 1, "{hits:?}");
    let h = &hits[0];
    assert_eq!(h.source, "vault");
    assert_eq!(
        h.title, "Lokale Modelle im Mittelstand",
        "Titel aus dem Frontmatter"
    );
    assert_eq!(h.path, "50_wissen/Lokale Modelle.md");
    assert!(
        h.snippet.contains("ohne GPU auf Notebooks"),
        "{}",
        h.snippet
    );
    assert!(!h.snippet.contains("Kaffee"));
    assert!(h.score > 0.8, "{}", h.score);
    // Ohne gemeinsame Woerter kein Treffer.
    assert!(index
        .search("Das Quartalsergebnis liegt über Plan.", 5)
        .is_empty());
}

#[test]
fn notes_with_an_excluded_frontmatter_line_do_not_count_and_are_reported() {
    let (_d, sandbox) = vault_with(&[
        (
            "00_inbox/Dieses Video.md",
            "---\ntitle: \"Dieses Video\"\nlva_id: \"video-Vid00000003\"\n---\nLokale Sprachmodelle laufen ohne GPU.\n",
        ),
        (
            "00_inbox/Anderes.md",
            "---\ntitle: \"Anderes\"\nvideo_id: \"Vid00000009\"\n---\nLokale Sprachmodelle laufen ohne GPU.\n",
        ),
    ]);
    let exclude = vec![
        "lva_id: \"video-Vid00000003\"".to_string(),
        "video_id: \"Vid00000003\"".to_string(),
    ];
    let index = VaultIndex::build(&sandbox, &exclude, &VaultLimits::default(), &never);
    assert_eq!(index.excluded, vec!["00_inbox/Dieses Video.md"]);
    let hits = index.search("Lokale Sprachmodelle laufen ohne GPU", 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Anderes");
}

#[test]
fn the_vault_scan_is_bounded_and_says_so() {
    let notes: Vec<(String, String)> = (0..6)
        .map(|i| {
            (
                format!("n/{i}.md"),
                format!("Lokale Sprachmodelle Nummer {i} laufen schnell.\n"),
            )
        })
        .collect();
    let refs: Vec<(&str, &str)> = notes
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_d, sandbox) = vault_with(&refs);

    // Zu wenige Dateien erlaubt.
    let limits = VaultLimits {
        max_files: 2,
        ..VaultLimits::default()
    };
    let index = VaultIndex::build(&sandbox, &[], &limits, &never);
    assert_eq!(index.files_scanned, 2);
    assert!(
        index.incomplete,
        "eine Grenze erreicht heisst: nur teilweise durchsucht"
    );

    // Zu wenig Bytes insgesamt.
    let limits = VaultLimits {
        max_total_bytes: 60,
        ..VaultLimits::default()
    };
    let index = VaultIndex::build(&sandbox, &[], &limits, &never);
    assert!(index.incomplete && index.files_scanned < 6);

    // Abgebrochen: sofort fertig, unvollstaendig.
    let index = VaultIndex::build(&sandbox, &[], &VaultLimits::default(), &|| true);
    assert!(index.incomplete && index.files_scanned == 0);

    // Eine Datei wird nur bis zur Grenze gelesen: ein Wort hinter dem Ende zaehlt nicht.
    let (_d2, sandbox2) = vault_with(&[(
        "lang.md",
        &format!("{}\nHinterwort", "Fuelltext ".repeat(40)),
    )]);
    let limits = VaultLimits {
        max_bytes_per_file: 100,
        ..VaultLimits::default()
    };
    let index = VaultIndex::build(&sandbox2, &[], &limits, &never);
    assert!(
        index.search("Hinterwort", 3).is_empty(),
        "hinter der Grenze gelesen"
    );
}

#[test]
fn hidden_folders_and_other_file_types_are_skipped() {
    let (_d, sandbox) = vault_with(&[
        (
            ".obsidian/plugins/x.md",
            "Lokale Sprachmodelle laufen ohne GPU.\n",
        ),
        (".trash/alt.md", "Lokale Sprachmodelle laufen ohne GPU.\n"),
        (
            "node_modules/p/readme.md",
            "Lokale Sprachmodelle laufen ohne GPU.\n",
        ),
        ("bild.png", "Lokale Sprachmodelle laufen ohne GPU.\n"),
        ("echt.md", "Lokale Sprachmodelle laufen ohne GPU.\n"),
    ]);
    let index = VaultIndex::build(&sandbox, &[], &VaultLimits::default(), &never);
    assert_eq!(index.files_scanned, 1);
    assert_eq!(
        index
            .search("Lokale Sprachmodelle laufen ohne GPU", 5)
            .len(),
        1
    );
}

#[test]
fn the_exclusion_matches_by_video_id_and_by_path_in_either_notation() {
    let ev = |path: &str, title: &str, snippet: &str| Evidence {
        source: "wissen",
        title: title.to_string(),
        path: path.to_string(),
        snippet: snippet.to_string(),
        score: 0.5,
    };
    let ex = Exclude {
        video_id: Some("Vid00000003".to_string()),
        rels: vec!["00_inbox/Dieses Video.md".to_string()],
    };
    assert!(ex.hits(&ev("x/y.md", "t", "Quelle: watch?v=Vid00000003")));
    assert!(
        ex.hits(&ev("00_inbox\\Dieses Video.md", "t", "")),
        "Windows-Schreibweise"
    );
    assert!(
        ex.hits(&ev("vault/00_inbox/Dieses Video.md", "t", "")),
        "mit Vorsatz"
    );
    assert!(!ex.hits(&ev("00_inbox/Anderes.md", "t", "nichts")));
    assert!(!Exclude::default().hits(&ev("a.md", "b", "c")));
}

#[test]
fn a_wissen_source_needs_a_config_and_a_key() {
    use crate::managers::integrations::model::{Integration, Kind};
    let make = |config: &str| Integration {
        id: "wissen-1".to_string(),
        kind: Kind::Wissen,
        label: "Wissen".to_string(),
        enabled: true,
        direction: crate::managers::integrations::model::Direction::Read,
        config_json: config.to_string(),
        account_hint: None,
        data_class: None,
        last_ok_at: None,
        last_error: None,
        created_at: 0,
        updated_at: 0,
    };
    let good = r#"{"endpoint":"http://127.0.0.1:9/mcp"}"#;
    assert!(matches!(
        WissenSource::open(&make(good), None, HttpOpts::default()),
        Err(SearchError::Permanent(_))
    ));
    assert!(matches!(
        WissenSource::open(
            &make(good),
            Some(Zeroizing::new(String::new())),
            HttpOpts::default()
        ),
        Err(SearchError::Permanent(_))
    ));
    assert!(matches!(
        WissenSource::open(
            &make("{}"),
            Some(Zeroizing::new("k".into())),
            HttpOpts::default()
        ),
        Err(SearchError::Permanent(_))
    ));
    assert!(WissenSource::open(
        &make(good),
        Some(Zeroizing::new("k".into())),
        HttpOpts::default()
    )
    .is_ok());
}
