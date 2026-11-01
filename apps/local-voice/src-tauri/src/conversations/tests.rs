use super::store::*;

fn entry(role: &str, text: &str) -> NewEntry {
    NewEntry {
        id: None,
        role: role.to_string(),
        mode: if role == "user" { "spoken" } else { "typed" }.to_string(),
        text: text.to_string(),
        status: "complete".to_string(),
        created_at: None,
        author_id: Some(
            if role == "user" {
                OWNER_ID
            } else {
                VOICEBOT_ID
            }
            .to_string(),
        ),
        triggered_by: None,
        model: None,
        attachments: Vec::new(),
        variants: Vec::new(),
        active_variant: None,
        sources: Vec::new(),
        untrusted: None,
        reply_to: None,
        ext: None,
    }
}

fn store() -> ConversationStore {
    ConversationStore::open_in_memory().unwrap()
}

/// Wartet, bis sich die Millisekunde ändert, damit `updated_at` unterscheidbar ist.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(5));
}

#[test]
fn create_follows_the_contract_defaults() {
    let s = store();
    let c = s.create(None).unwrap();
    assert_eq!(c.title, DEFAULT_TITLE);
    assert_eq!(c.kind, "voice");
    assert_eq!(c.schema_version, SCHEMA_VERSION);
    assert_eq!(c.scope.data_class, "INTERNAL");
    assert!(!c.scope.special_category);
    assert!(c.id.starts_with("conv_"));
    assert_eq!(c.id.len(), "conv_".len() + 26);
    assert_eq!(c.trace_id.len(), 32);
    assert!(c.trace_id.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert!(c.created_at.ends_with('Z'));
    assert_eq!(c.created_at, c.updated_at);
    assert!(!c.pinned && !c.archived);
    assert!(c.context.is_empty());
    let kinds: Vec<_> = c.participants.iter().map(|p| p.kind.as_str()).collect();
    assert_eq!(kinds, ["person", "voicebot"]);
    assert_eq!(c.participants[0].role, "owner");
}

#[test]
fn append_and_get_round_trip_in_order() {
    let s = store();
    let c = s.create(Some("Planung".to_string())).unwrap();
    let mut answer = entry("assistant", "Gern.");
    answer.model = Some("Claude Sonnet".to_string());
    answer.triggered_by = Some(OWNER_ID.to_string());
    answer.ext = Some(ConversationExt {
        local_voice_ai: Some(VoiceExt {
            prompt_tokens: Some(120),
            completion_tokens: Some(30),
            usage_event_ids: vec![7, 8],
            provider_id: Some("claude_cli".to_string()),
            local: Some(false),
        }),
    });
    answer.attachments = vec![ConversationAttachment {
        id: "att-1".to_string(),
        name: "notiz.txt".to_string(),
        mime: "text/plain".to_string(),
        size_bytes: Some(12),
        reference: None,
    }];
    let first = s.append_entry(&c.id, entry("user", "Hallo")).unwrap();
    let second = s.append_entry(&c.id, answer.clone()).unwrap();
    assert!(first.id.starts_with("entry_"));
    assert_eq!(first.conversation_id, c.id);

    let detail = s.get(&c.id).unwrap();
    assert_eq!(detail.conversation.title, "Planung");
    assert_eq!(detail.entries.len(), 2);
    assert_eq!(detail.entries[0], first);
    assert_eq!(detail.entries[1], second);
    assert_eq!(detail.entries[1].ext, answer.ext);
    assert_eq!(detail.entries[1].attachments, answer.attachments);
    assert_eq!(detail.entries[1].model.as_deref(), Some("Claude Sonnet"));
}

#[test]
fn append_sets_updated_at() {
    let s = store();
    let c = s.create(None).unwrap();
    tick();
    s.append_entry(&c.id, entry("user", "Hallo")).unwrap();
    let after = s.get(&c.id).unwrap().conversation;
    assert!(after.updated_at > c.updated_at);
    assert_eq!(after.created_at, c.created_at);
}

#[test]
fn list_sorts_pinned_first_then_newest() {
    let s = store();
    let old = s.create(Some("alt".to_string())).unwrap();
    tick();
    let mid = s.create(Some("mitte".to_string())).unwrap();
    tick();
    let new = s.create(Some("neu".to_string())).unwrap();
    tick();
    s.update(&old.id, None, Some(true), None).unwrap();

    let ids: Vec<_> = s.list(false).unwrap().into_iter().map(|c| c.id).collect();
    assert_eq!(ids, [old.id.clone(), new.id.clone(), mid.id.clone()]);

    tick();
    s.append_entry(&mid.id, entry("user", "Frisch")).unwrap();
    let ids: Vec<_> = s.list(false).unwrap().into_iter().map(|c| c.id).collect();
    assert_eq!(ids, [old.id, mid.id, new.id]);
}

#[test]
fn list_reports_count_and_last_text() {
    let s = store();
    let c = s.create(None).unwrap();
    assert_eq!(s.list(false).unwrap()[0].entry_count, 0);
    assert_eq!(s.list(false).unwrap()[0].last_text, "");
    s.append_entry(&c.id, entry("user", "Hallo")).unwrap();
    let long = format!("Zeile eins\n\n  Zeile   zwei {}", "x".repeat(300));
    s.append_entry(&c.id, entry("assistant", &long)).unwrap();
    let row = &s.list(false).unwrap()[0];
    assert_eq!(row.entry_count, 2);
    assert!(row.last_text.starts_with("Zeile eins Zeile zwei x"));
    assert!(!row.last_text.contains('\n'));
    assert_eq!(row.last_text.chars().count(), 120);
}

#[test]
fn rename_pin_and_archive_keep_other_fields() {
    let s = store();
    let c = s.create(Some("alt".to_string())).unwrap();
    let renamed = s
        .update(&c.id, Some("  Neuer Titel  ".to_string()), None, None)
        .unwrap();
    assert_eq!(renamed.title, "Neuer Titel");
    assert!(!renamed.pinned && !renamed.archived);
    assert_eq!(renamed.updated_at, c.updated_at);

    let pinned = s.update(&c.id, None, Some(true), None).unwrap();
    assert!(pinned.pinned);
    assert_eq!(pinned.title, "Neuer Titel");
    let both = s.update(&c.id, None, Some(false), Some(true)).unwrap();
    assert!(!both.pinned && both.archived);
}

#[test]
fn archive_filter_hides_archived_unless_asked() {
    let s = store();
    let keep = s.create(Some("bleibt".to_string())).unwrap();
    let gone = s.create(Some("archiviert".to_string())).unwrap();
    s.update(&gone.id, None, None, Some(true)).unwrap();

    let visible = s.list(false).unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, keep.id);

    let all = s.list(true).unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.iter().any(|c| c.id == gone.id && c.archived));
}

#[test]
fn delete_cascades_to_entries() {
    let s = store();
    let a = s.create(None).unwrap();
    let b = s.create(None).unwrap();
    s.append_entry(&a.id, entry("user", "eins")).unwrap();
    s.append_entry(&a.id, entry("assistant", "zwei")).unwrap();
    s.append_entry(&b.id, entry("user", "drei")).unwrap();

    s.delete(&a.id).unwrap();
    assert!(s.get(&a.id).is_err());
    assert_eq!(s.get(&b.id).unwrap().entries.len(), 1);
    assert_eq!(s.list(true).unwrap().len(), 1);
    assert!(s.delete(&a.id).is_err());
}

#[test]
fn delete_leaves_no_orphan_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conversations.db");
    let id = {
        let s = ConversationStore::open(&path).unwrap();
        let c = s.create(None).unwrap();
        s.append_entry(&c.id, entry("user", "x")).unwrap();
        s.delete(&c.id).unwrap();
        c.id
    };
    let raw = rusqlite::Connection::open(&path).unwrap();
    let left: i64 = raw
        .query_row(
            "SELECT COUNT(*) FROM conversation_entries WHERE conversation_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0);
}

#[test]
fn append_to_unknown_conversation_fails() {
    let s = store();
    let ghost = format!("conv_{}", "0".repeat(26));
    assert!(s.append_entry(&ghost, entry("user", "x")).is_err());
}

#[test]
fn migration_is_idempotent_and_keeps_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conversations.db");
    let id = {
        let s = ConversationStore::open(&path).unwrap();
        let c = s.create(Some("bleibt".to_string())).unwrap();
        s.append_entry(&c.id, entry("user", "Hallo")).unwrap();
        c.id
    };
    for _ in 0..2 {
        let s = ConversationStore::open(&path).unwrap();
        let detail = s.get(&id).unwrap();
        assert_eq!(detail.conversation.title, "bleibt");
        assert_eq!(detail.entries.len(), 1);
    }
}

#[test]
fn rejects_invalid_titles() {
    let s = store();
    assert!(s.create(Some("x".repeat(MAX_TITLE_CHARS + 1))).is_err());
    assert!(s.create(Some("x".repeat(MAX_TITLE_CHARS))).is_ok());
    let c = s.create(None).unwrap();
    assert!(s
        .update(&c.id, Some("   ".to_string()), None, None)
        .is_err());
    assert!(s
        .update(&c.id, Some("ä".repeat(MAX_TITLE_CHARS + 1)), None, None)
        .is_err());
    assert_eq!(s.get(&c.id).unwrap().conversation.title, DEFAULT_TITLE);
}

#[test]
fn rejects_invalid_entries_and_ids() {
    let s = store();
    let c = s.create(None).unwrap();

    let too_long = entry("user", &"a".repeat(MAX_TEXT_CHARS + 1));
    assert!(s.append_entry(&c.id, too_long).is_err());
    assert!(s
        .append_entry(&c.id, entry("user", &"a".repeat(MAX_TEXT_CHARS)))
        .is_ok());

    let mut bad_role = entry("user", "x");
    bad_role.role = "robot".to_string();
    assert!(s.append_entry(&c.id, bad_role).is_err());

    let mut bad_mode = entry("user", "x");
    bad_mode.mode = "telepathy".to_string();
    assert!(s.append_entry(&c.id, bad_mode).is_err());

    let mut bad_status = entry("user", "x");
    bad_status.status = "streaming".to_string();
    assert!(s.append_entry(&c.id, bad_status).is_err());

    let mut bad_id = entry("user", "x");
    bad_id.id = Some("kein-id".to_string());
    assert!(s.append_entry(&c.id, bad_id).is_err());

    let mut bad_time = entry("user", "x");
    bad_time.created_at = Some("gestern".to_string());
    assert!(s.append_entry(&c.id, bad_time).is_err());

    assert!(s.get("kein-id").is_err());
    assert!(s.append_entry("kein-id", entry("user", "x")).is_err());
    assert!(s.update("kein-id", None, None, None).is_err());
    assert!(s.delete("kein-id").is_err());
}

#[test]
fn created_at_is_normalised_to_utc_and_orders_entries() {
    let s = store();
    let c = s.create(None).unwrap();
    let mut later = entry("assistant", "spät");
    later.created_at = Some("2020-10-31T12:00:05+01:00".to_string());
    let mut earlier = entry("user", "früh");
    earlier.created_at = Some("2020-10-31T11:00:01Z".to_string());
    s.append_entry(&c.id, later).unwrap();
    let stored = s.append_entry(&c.id, earlier).unwrap();
    assert_eq!(stored.created_at, "2020-10-31T11:00:01.000Z");
    let texts: Vec<_> = s
        .get(&c.id)
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert_eq!(texts, ["früh", "spät"]);
}

#[test]
fn supplied_entry_id_is_kept_and_must_be_unique() {
    let s = store();
    let c = s.create(None).unwrap();
    let mut first = entry("user", "eins");
    first.id = Some("entry_01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string());
    let stored = s.append_entry(&c.id, first.clone()).unwrap();
    assert_eq!(stored.id, "entry_01ARZ3NDEKTSV4RRFFQ69G5FAV");
    assert!(s.append_entry(&c.id, first).is_err());
    assert_eq!(s.get(&c.id).unwrap().entries.len(), 1);
}

#[test]
fn ext_serializes_under_its_own_key_without_usage_in_the_contract_fields() {
    let s = store();
    let c = s.create(None).unwrap();
    let mut answer = entry("assistant", "Hallo");
    answer.ext = Some(ConversationExt {
        local_voice_ai: Some(VoiceExt {
            prompt_tokens: Some(1),
            ..VoiceExt::default()
        }),
    });
    let stored = s.append_entry(&c.id, answer).unwrap();
    let json = serde_json::to_value(&stored).unwrap();
    assert_eq!(json["ext"]["local-voice-ai"]["prompt_tokens"], 1);
    assert!(json.get("usage").is_none());
    assert!(json.get("prompt_tokens").is_none());
}

#[test]
fn update_entry_changes_text_and_status_in_place() {
    let s = store();
    let c = s.create(None).unwrap();
    let stored = s
        .append_entry(&c.id, entry("assistant", "Volle Antwort"))
        .unwrap();
    s.append_entry(&c.id, entry("user", "danach")).unwrap();
    let before = s.get(&c.id).unwrap().conversation.updated_at;

    let changed = s
        .update_entry(&stored.id, "Volle".to_string(), "interrupted".to_string())
        .unwrap();
    assert_eq!(changed.id, stored.id);
    assert_eq!(changed.text, "Volle");
    assert_eq!(changed.status, "interrupted");
    assert_eq!(changed.created_at, stored.created_at);

    let detail = s.get(&c.id).unwrap();
    assert_eq!(detail.entries.len(), 2);
    assert_eq!(detail.entries[0].text, "Volle");
    assert_eq!(detail.entries[0].status, "interrupted");
    assert_eq!(detail.conversation.updated_at, before);
}

#[test]
fn update_entry_rejects_bad_input() {
    let s = store();
    let c = s.create(None).unwrap();
    let e = s.append_entry(&c.id, entry("assistant", "x")).unwrap();
    assert!(s
        .update_entry(&e.id, "y".into(), "streaming".into())
        .is_err());
    assert!(s
        .update_entry(&e.id, "a".repeat(MAX_TEXT_CHARS + 1), "complete".into())
        .is_err());
    assert!(s
        .update_entry("kein-id", "y".into(), "complete".into())
        .is_err());
    let ghost = format!("entry_{}", "0".repeat(26));
    assert!(s
        .update_entry(&ghost, "y".into(), "complete".into())
        .is_err());
    assert_eq!(s.get(&c.id).unwrap().entries[0].text, "x");
}

#[test]
fn limits_for_ext_sources_references_and_future_timestamps() {
    let s = store();
    let c = s.create(None).unwrap();

    let mut big_ext = entry("assistant", "x");
    big_ext.ext = Some(ConversationExt {
        local_voice_ai: Some(VoiceExt {
            provider_id: Some("p".repeat(20_000)),
            ..VoiceExt::default()
        }),
    });
    assert!(s.append_entry(&c.id, big_ext).is_err());

    let mut many = entry("assistant", "x");
    many.sources = vec!["a".to_string(); 51];
    assert!(s.append_entry(&c.id, many).is_err());
    let mut long = entry("assistant", "x");
    long.sources = vec!["a".repeat(2001)];
    assert!(s.append_entry(&c.id, long).is_err());
    let mut ok = entry("assistant", "x");
    ok.sources = vec!["a".repeat(2000); 50];
    assert!(s.append_entry(&c.id, ok).is_ok());

    let mut reference = entry("user", "x");
    reference.attachments = vec![ConversationAttachment {
        id: "a".to_string(),
        name: "n.txt".to_string(),
        mime: "text/plain".to_string(),
        size_bytes: None,
        reference: Some("r".repeat(1001)),
    }];
    assert!(s.append_entry(&c.id, reference).is_err());

    let mut future = entry("user", "x");
    future.created_at = Some("2999-01-01T00:00:00Z".to_string());
    let stored = s.append_entry(&c.id, future).unwrap();
    assert!(stored.created_at < "2100".to_string());
    assert!(stored.created_at > "2020".to_string());
}

#[test]
fn list_is_limited_to_500_rows() {
    let s = store();
    for _ in 0..502 {
        s.create(None).unwrap();
    }
    assert_eq!(s.list(true).unwrap().len(), 500);
}

#[test]
fn recent_entries_are_newest_first_and_skip_archived() {
    let s = store();
    let open = s.create(Some("Offen".to_string())).unwrap();
    let archived = s.create(Some("Archiviert".to_string())).unwrap();
    s.append_entry(&open.id, entry("user", "erste Frage")).unwrap();
    tick();
    s.append_entry(&archived.id, entry("user", "versteckt")).unwrap();
    tick();
    s.append_entry(&open.id, entry("assistant", "zweite Antwort")).unwrap();
    s.update(&archived.id, None, None, Some(true)).unwrap();

    let hits = s.recent_entries(10).unwrap();
    let texts: Vec<&str> = hits.iter().map(|h| h.text.as_str()).collect();
    assert_eq!(texts, vec!["zweite Antwort", "erste Frage"]);
    assert_eq!(hits[0].title, "Offen");
    assert_eq!(hits[0].role, "assistant");
    assert_eq!(hits[0].conversation_id, open.id);
    assert_eq!(s.recent_entries(1).unwrap().len(), 1);
}
