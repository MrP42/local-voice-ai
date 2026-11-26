use super::*;

#[test]
fn kurze_saetze_bleiben_einzeln_und_offsets_sind_utf16() {
    let text = "  Hi🙂. Ja!\n\nEnde";
    let units = units(text).unwrap();
    assert_eq!(units.len(), 3);
    // Gegenprobe mit dem bisherigen Vorlese-Schnitt: er verbindet kurze Sätze.
    assert_ne!(crate::managers::tts::protocol::split_sentences(text).len(), units.len());
    assert_eq!((units[0].start, units[0].end), (2, 7));
    assert_eq!((units[1].start, units[1].end), (8, 11));
    assert_eq!(&text[units[2].bytes.clone()], "Ende");
    assert_eq!(window(&units, 7, 1.0).unwrap().start, 1);
    assert_eq!(window(&units, 6, 1.0).unwrap().start, 0);
}

#[test]
fn lange_saetze_werden_nicht_in_halbe_saetze_zerlegt() {
    let text = format!("{} fertig. Kurz. Noch kurz.", "Wort ".repeat(90));
    let list = units(&text).unwrap();
    assert_eq!(list.len(), 3);
    assert!(list[0].end > 400);
    assert_eq!(&text[list[1].bytes.clone()], "Kurz.");
}

#[test]
fn satzzeichen_zitate_kuerzel_und_cjk() {
    let text = "Dr. Test sagt: „Hallo!“ Weiter. 你好。再见！";
    let list = units(text).unwrap();
    let texts: Vec<_> = list.iter().map(|u| &text[u.bytes.clone()]).collect();
    assert_eq!(texts, ["Dr. Test sagt: „Hallo!“", "Weiter.", "你好。", "再见！"]);
}

#[test]
fn vorlauf_hat_mindestzahl_zeit_und_harte_obergrenze() {
    let short = units(&"Kurz. ".repeat(100)).unwrap();
    assert_eq!(window(&short, 0, 1.0).unwrap(), 0..40);
    let long = units(&format!("{} Ende. ", "Wort ".repeat(30)).repeat(20)).unwrap();
    assert_eq!(window(&long, 0, 1.0).unwrap(), 0..6);
    assert_eq!(window(&long, long.last().unwrap().end, 1.0).unwrap().len(), 0);
    assert!(window(&short, 0, f64::NAN).is_err());
    assert!(window(&short, 0, 0.0).is_err());
}

#[test]
fn epoche_verwirft_spaete_ergebnisse_und_abbruch_auch_spaete_fehler() {
    let mut slot = Slot::default();
    let first = slot.enter(&request(1)).unwrap();
    assert!(slot.current(first));
    let second = slot.enter(&request(2)).unwrap();
    assert!(!slot.current(first));
    assert!(slot.current(second));
    assert_eq!(slot.enter(&request(1)), Err("reader_live:stale_epoch"));
    assert_eq!(slot.enter(&request(2)), Err("reader_live:stale_epoch"));
    assert!(!slot.cancel("anderer-leser", 3));
    assert!(slot.current(second));
    assert!(slot.cancel("synthetisch", 3));
    assert!(!slot.current(second));
    assert_eq!(slot.enter(&request(2)), Err("reader_live:stale_epoch"));
}

#[test]
fn abgeloeste_sitzung_kann_neue_sitzung_nicht_abbrechen() {
    let state = LiveState::default();
    state.begin(&request(10)).unwrap();
    let mut next = request(1);
    next.session_id = "zweiter-leser".into();
    let (generation, mut rx) = state.begin(&next).unwrap();
    for epoch in [9, 11] {
        assert!(matches!(state.begin(&request(epoch)), Err("reader_live:stale_epoch")));
        assert!(!state.cancel("synthetisch", epoch));
        assert!(state.inner.lock().unwrap().current(generation));
        assert!(!rx.has_changed().unwrap());
    }
    next.epoch = 2;
    assert!(state.begin(&next).is_ok());
    assert!(rx.has_changed().unwrap());
    rx.borrow_and_update();
}

#[test]
fn sitzungshistorie_bleibt_begrenzt() {
    let mut slot = Slot::default();
    for i in 0..=130 {
        let mut req = request(1);
        req.session_id = format!("leser-{i}");
        slot.enter(&req).unwrap();
    }
    assert_eq!(slot.retired.len(), 128);
    let mut req = request(2);
    req.session_id = "leser-129".into();
    assert_eq!(slot.enter(&req), Err("reader_live:stale_epoch"));
    assert_eq!(slot.active.as_ref().unwrap().id, "leser-130");
}

#[test]
fn cache_bytebudget_verdrangt_den_am_laengsten_nicht_genutzten_satz() {
    let mut slot = Slot::default();
    slot.enter(&request(1)).unwrap();
    let session = slot.active.as_mut().unwrap();
    for i in 0..512 {
        session.put(i, "x".repeat(8192));
    }
    assert!(session.get(0).is_some());
    session.put(512, "x".repeat(8192));
    assert_eq!(session.cache.len(), 512);
    assert!(session.get(1).is_none());
    assert!(session.get(0).is_some());
    session.put(0, "klein".into());
    session.put(513, "x".repeat(8187));
    assert_eq!(session.cache.len(), 513);
    session.bind_route("anderes-modell".into());
    assert!(session.cache.is_empty());
    assert_eq!(session.cache_bytes, 0);
    assert!(session.lru.is_empty());
}

#[tokio::test]
async fn zu_grosse_antwort_wird_weder_ausgegeben_noch_gespeichert() {
    for (source, size, accepted) in [
        ("Test.".to_string(), 20, true), ("Test.".to_string(), 21, false),
        ("Test.".to_string(), 22, false),
        ("a".repeat(3000), 8192, true), ("a".repeat(3000), 8193, false),
    ] {
        let state = LiveState::default();
        let mut req = request(1);
        req.text = source.into();
        let list = units(&req.text).unwrap();
        let (generation, rx) = state.begin(&req).unwrap();
        let events = Mutex::new(Vec::new());
        // Mehrbytezeichen: das Limit gilt in Bytes, nicht Zeichen.
        let answer = "ä".repeat(size / 2) + &"x".repeat(size % 2);
        let result = run_window(&state, generation, rx, &req, &list, 0..1,
            |_| std::future::ready(Ok(answer.clone())),
            |event| { events.lock().unwrap().push(event); Ok(()) }).await;
        assert_eq!(result, if accepted { Ok(1) } else { Err("reader_live:unusable_reply") });
        assert_eq!(events.lock().unwrap().len(), usize::from(accepted));
        assert_eq!(state.inner.lock().unwrap().active.as_ref().unwrap().cache.len(), usize::from(accepted));
    }
}

#[test]
fn cache_ist_begrenzt_und_an_quelle_und_sprache_gebunden() {
    let mut slot = Slot::default();
    slot.enter(&request(1)).unwrap();
    let session = slot.active.as_mut().unwrap();
    for i in 0..=CACHE_LIMIT as u32 {
        session.put(i, "synthetisch".into());
    }
    assert_eq!(session.cache.len(), CACHE_LIMIT);
    assert!(!session.cache.contains_key(&0));
    slot.enter(&request(2)).unwrap();
    assert_eq!(slot.active.as_ref().unwrap().cache.len(), CACHE_LIMIT);
    let mut changed = request(3);
    changed.text.push('!');
    slot.enter(&changed).unwrap();
    assert!(slot.active.as_ref().unwrap().cache.is_empty());
    slot.active.as_mut().unwrap().put(0, "Test".into());
    changed.epoch = 4;
    changed.target_lang = "fr".into();
    slot.enter(&changed).unwrap();
    assert!(slot.active.as_ref().unwrap().cache.is_empty());
    let session = slot.active.as_mut().unwrap();
    session.bind_route("lokal:eins".into());
    session.put(0, "Test".into());
    session.bind_route("lokal:eins".into());
    assert_eq!(session.cache.len(), 1);
    session.bind_route("lokal:zwei".into());
    assert!(session.cache.is_empty());
}

#[test]
fn leere_und_abgeschnittene_antworten_werden_nicht_ausgegeben() {
    assert_eq!(usable_reply(Some(" Test ".into()), false), Ok("Test".into()));
    assert!(usable_reply(Some(" \n".into()), false).is_err());
    assert!(usable_reply(None, false).is_err());
    assert!(usable_reply(Some("Halber Satz".into()), true).is_err());
}

#[test]
fn nur_http_loopback_und_passendes_laufendes_modell() {
    for url in ["https://cloud.example/v1", "cli://claude", "file:///tmp/test"] {
        assert!(local_target(&provider("custom", url), "test", None).is_none());
    }
    assert!(local_target(&provider("custom", "http://localhost:11434/v1"), "test", None).is_some());
    let built_in = provider("local", crate::managers::llm::LOCAL_PLACEHOLDER_URL);
    assert!(local_target(&built_in, "test", None).is_none());
    assert!(local_target(&built_in, "test", Some(("anderes", 8080))).is_none());
    assert!(local_target(&built_in, "test", Some(("test", 8080))).unwrap().ist_lokal());
    // Die Anbieterkennung allein darf eine entfernte Adresse nicht lokal machen.
    assert!(local_target(&provider("local", "https://cloud.example/v1"), "test", None).is_none());
}

#[test]
fn cloud_standard_faellt_nur_auf_lokal_zurueck_und_nie_umgekehrt() {
    let mut settings = crate::settings::get_default_settings();
    settings.llm_models.clear();
    settings.llm_connections.clear();
    settings.post_process_providers = vec![
        provider("openai", "https://cloud.example/v1"),
        provider("custom", "http://localhost:8080/v1"),
    ];
    for (id, url) in [("openai", "https://cloud.example/v1"), ("custom", "http://localhost:8080/v1")] {
        settings.llm_connections.push(serde_json::from_value(serde_json::json!({
            "id": id, "kind": id, "label": id, "base_url": url
        })).unwrap());
        settings.llm_models.push(serde_json::from_value(serde_json::json!({
            "id": id, "connection_id": id, "remote_id": "test", "label": id
        })).unwrap());
    }
    settings.llm_active_model_id = Some("openai".into());
    assert_eq!(route(&settings, None).unwrap().0.id, "custom");
    settings.llm_models[1].enabled = false;
    assert!(matches!(route(&settings, None), Err("reader_live:no_local_model")));
    settings.llm_models[1].enabled = true;
    settings.llm_connections[1].enabled = false;
    assert!(route(&settings, None).is_err());
}

#[tokio::test]
async fn ganzuebersetzung_hat_vorrang_vor_modellwahl_und_ereignissen() {
    let result = prefer_existing(Some("Vollständig.".into()), || async {
        panic!("Weder Modellwahl noch Satzereignis bei vorhandener Ganzübersetzung")
    }).await.unwrap();
    assert!(matches!(result, LiveResult::Existing { translation } if translation == "Vollständig."));
    let result = prefer_existing(None, || async { Ok(LiveResult::Window { sentences: 2 }) }).await.unwrap();
    assert!(matches!(result, LiveResult::Window { sentences: 2 }));
}

#[tokio::test]
async fn ein_ereignis_je_satz_cache_und_abbruch_ohne_transport() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let state = LiveState::default();
    let req = request(1);
    let list = units(&req.text).unwrap();
    let (generation, rx) = state.begin(&req).unwrap();
    let calls = AtomicUsize::new(0);
    let events = Mutex::new(Vec::new());
    let send = |_: String| {
        calls.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok("Translation.".into()))
    };
    let emit = |event: ReaderLiveSentence| {
        events.lock().unwrap().push(event);
        Ok(())
    };
    assert_eq!(run_window(&state, generation, rx, &req, &list, 0..list.len(), send, &emit).await, Ok(2));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let stored = events.lock().unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].source_start, 0);
    assert_eq!(stored[1].source_start, 12);
    drop(stored);
    let next = request(2);
    let (generation, rx) = state.begin(&next).unwrap();
    let no_send = |_: String| async { panic!("Cache darf nicht senden") };
    assert_eq!(run_window(&state, generation, rx, &next, &list, 0..list.len(), no_send, &emit).await, Ok(2));
    assert!(state.cancel("synthetisch", 3));
    let no_send = |_: String| async { panic!("Alte Epoche darf nicht senden") };
    let rx = state.changed.subscribe();
    assert_eq!(run_window(&state, generation, rx, &next, &list, 0..list.len(), no_send, &emit).await, Err("reader_live:cancelled"));
}

#[tokio::test]
async fn wechsel_waehrend_antwort_verwirft_text_und_fehler() {
    for answer in [Ok("Spaet".into()), Err("reader_live:translation_failed")] {
        let state = LiveState::default();
        let req = request(1);
        let list = units(&req.text).unwrap();
        let (generation, rx) = state.begin(&req).unwrap();
        let send = |_: String| {
            state.begin(&request(2)).unwrap();
            std::future::ready(answer.clone())
        };
        let emit = |_: ReaderLiveSentence| panic!("Kein Ereignis aus alter Epoche");
        assert_eq!(run_window(&state, generation, rx, &req, &list, 0..list.len(), send, emit).await, Err("reader_live:cancelled"));
        assert!(state.inner.lock().unwrap().active.as_ref().unwrap().cache.is_empty());
    }
}

#[tokio::test]
async fn abbruch_laesst_laufendes_future_fallen_und_gibt_arbeiter_frei() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Wache<'a>(&'a AtomicBool);
    impl Drop for Wache<'_> {
        fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); }
    }
    let state = LiveState::default();
    let req = request(1);
    let list = units(&req.text).unwrap();
    let (generation, rx) = state.begin(&req).unwrap();
    let dropped = AtomicBool::new(false);
    let send = |_: String| async {
        let _guard = Wache(&dropped);
        std::future::pending::<Result<String, &'static str>>().await
    };
    let work = run_window(&state, generation, rx, &req, &list, 0..1, send, |_| {
        panic!("Abgebrochener Satz darf nicht erscheinen")
    });
    let cancel = async {
        tokio::task::yield_now().await;
        assert!(state.cancel("synthetisch", 2));
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(work, cancel)
    })
    .await
    .expect("Abbruch muss ohne Antwort des Modells fertig werden");
    assert_eq!(result, Err("reader_live:cancelled"));
    assert!(dropped.load(Ordering::SeqCst));
    assert!(state.serial.try_lock().is_ok());
}

fn provider(id: &str, url: &str) -> crate::settings::PostProcessProvider {
    crate::settings::PostProcessProvider {
        id: id.into(), label: id.into(), base_url: url.into(),
        allow_base_url_edit: false, models_endpoint: None, supports_structured_output: false,
    }
}

fn request(epoch: u32) -> LiveRequest {
    LiveRequest {
        session_id: "synthetisch".into(),
        page_id: "page_test".into(),
        text: "Hallo Welt. Zweiter Satz.".into(),
        target_lang: "en".into(),
        source_offset: 0,
        epoch,
        rate: 1.0,
    }
}
