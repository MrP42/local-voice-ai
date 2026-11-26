use super::*;

#[test]
fn vier_zustaende_und_unlesbare_sperren_sind_belegt() {
    assert_eq!(erster_grund([Ok(false); 4]), None);
    for (i, grund) in GRUENDE.iter().enumerate() {
        for wert in [Ok(true), Err(())] {
            let mut zustand = [Ok(false); 4];
            zustand[i] = wert;
            assert_eq!(erster_grund(zustand), Some(*grund));
        }
    }
}

#[test]
fn installation_blockiert_alle_startpfade_bis_abort_oder_frist() {
    let jetzt = Instant::now();
    let mut gate = Gate::default();
    gate.begin("1.2.3".into(), None, jetzt).unwrap();
    for grund in GRUENDE {
        assert_eq!(gate.start(grund, jetzt), Err("update_laeuft"));
    }
    assert_eq!(gate.start(Grund::Diktat, jetzt + FRIST - Duration::from_secs(1)), Err("update_laeuft"));
    assert!(gate.start(Grund::Diktat, jetzt + FRIST).is_ok());
    gate.installation = None;
    gate.begin("1.2.3".into(), None, jetzt).unwrap_err();
    gate.starts.clear();
    gate.begin("1.2.3".into(), None, jetzt).unwrap();
    gate.abort();
    assert!(gate.start(Grund::Vorlesen, jetzt).is_ok());
}

#[test]
fn startreservierung_und_belegte_manager_verhindern_installation() {
    let jetzt = Instant::now();
    for grund in GRUENDE {
        let mut gate = Gate::default();
        assert_eq!(gate.begin("1.2.3".into(), Some(grund), jetzt), Err(UpdateGateFehler::Belegt(grund)));
        let id = gate.start(grund, jetzt).unwrap();
        assert_eq!(gate.begin("1.2.3".into(), None, jetzt), Err(UpdateGateFehler::Belegt(grund)));
        gate.starts.remove(&id);
        assert!(gate.begin("1.2.3".into(), None, jetzt).is_ok());
        assert!(gate.begin("1.2.3".into(), None, jetzt).is_err());
    }
}

#[test]
fn vergifteter_mutex_wird_nicht_als_frei_gelesen() {
    let mutex = Mutex::new(false);
    let _ = std::panic::catch_unwind(|| {
        let _guard = mutex.lock().unwrap();
        panic!("synthetisch");
    });
    assert_eq!(erster_grund([mutex.lock().map(|v| *v).map_err(|_| ()), Ok(false), Ok(false), Ok(false)]), Some(Grund::Diktat));
}

#[test]
fn bestaetigte_version_und_installationsbeginn_schuetzen_die_sperre() {
    let jetzt = Instant::now();
    let mut gate = Gate::default();
    gate.begin("1.2.3".into(), None, jetzt).unwrap();
    assert!(gate.install("1.2.4", jetzt).is_err());
    gate.install("1.2.3", jetzt).unwrap();
    assert_eq!(gate.start(Grund::Anruf, jetzt + FRIST), Err("update_laeuft"));
    gate.abort();
    assert!(gate.start(Grund::Anruf, jetzt + FRIST).is_ok());
    gate.starts.clear();
    gate.begin("1.2.3".into(), None, jetzt).unwrap();
    assert!(gate.install("1.2.3", jetzt + FRIST).is_err());
}

#[test]
fn nur_ein_versionsanstieg_meldet_aktualisiert() {
    assert!(aktualisiert(Some("1.2.3"), "1.2.4"));
    for vorher in [None, Some("1.2.4"), Some("1.2.5"), Some("ungueltig")] {
        assert!(!aktualisiert(vorher, "1.2.4"));
    }
}

#[test]
fn alle_produktiven_startpfade_reservieren_vor_der_vorbereitung() {
    let pfade = [
        (include_str!("managers/audio.rs"), "pub fn try_start_recording_at(", "let mut state"),
        (include_str!("managers/meetings/recorder.rs"), "pub fn start_for_event(", "consent_gate("),
        (include_str!("managers/meetings/recorder.rs"), "pub fn start_playback_capture(", "crate::richtlinie::erlauben("),
        (include_str!("managers/tts/mod.rs"), "pub async fn speak_text_at(", "let max_chars"),
        (include_str!("managers/tts/mod.rs"), "pub async fn speak_text_from(", "let prepared"),
        (include_str!("managers/tts/mod.rs"), "async fn run_speak_session(", "let _ = self.app.emit("),
        (include_str!("managers/tts/mod.rs"), "pub fn reading_play(", "let (key, title"),
        (include_str!("voice_call/mod.rs"), "pub async fn voice_call_start(", "let started"),
    ];
    for (quelle, start, vorbereitung) in pfade {
        let rumpf = quelle.split_once(start).unwrap().1;
        let davor = rumpf.split_once(vorbereitung).unwrap().0;
        assert!(davor.contains("crate::update_gate::pruefe_start("), "{start}");
        // Gegenprobe: Entfernen der Reservierung muss denselben Vertrag verletzen.
        assert!(!davor.replace("crate::update_gate::pruefe_start(", "entfernt(").contains("crate::update_gate::pruefe_start("));
    }
}
