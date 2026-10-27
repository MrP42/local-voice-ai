use super::*;
use crate::catalog::PhononVoiceMeta;
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;

// ---- Hilfen ------------------------------------------------------------------------------

/// Eine Phonon-Stimmdatei in der Form der echten (`speaker_wavs` F32 [1,512,125], 256 136 Byte);
/// `seed` macht die Bytes verschieden.
fn voice_bytes(seed: u32) -> Vec<u8> {
    voice_bytes_t(seed, 125)
}

fn voice_bytes_t(seed: u32, t: usize) -> Vec<u8> {
    let n = 512 * t;
    let header = format!(
        r#"{{"speaker_wavs":{{"dtype":"F32","shape":[1,512,{t}],"data_offsets":[0,{}]}},"__metadata__":{{"sig":"ba7a5a45","epoch":"300"}}}}"#,
        n * 4
    );
    let mut h = header.into_bytes();
    while h.len() % 8 != 0 {
        h.push(b' ');
    }
    let mut out = (h.len() as u64).to_le_bytes().to_vec();
    out.extend(h);
    for i in 0..n {
        out.extend(((seed as f32 * 7.0 + i as f32 * 0.001).sin()).to_le_bytes());
    }
    out
}

/// safetensors mit frei gewähltem Kopf und Daten.
fn st(header: &str, data: &[u8]) -> Vec<u8> {
    let mut out = (header.len() as u64).to_le_bytes().to_vec();
    out.extend(header.as_bytes());
    out.extend(data);
    out
}

const SOURCE_JSON: &str = r#"{
  "source": "hf://gradium/phonon-voices@0ee17205f254b5abc43b266c8a0f215565c352f7",
  "license": "apache-2.0",
  "voices": [
    {"filename": "voices/2A2fFf8SkDEJ8nzx.safetensors", "name": "Nele", "voice_id": "2A2fFf8SkDEJ8nzx",
     "language": "de", "locale": "de-de", "gender": "female", "age": "Middle-age",
     "description": "Rounded, open tone."},
    {"filename": "voices/4Mn9VfG2wsLLEzi5.safetensors", "name": "Jette", "voice_id": "4Mn9VfG2wsLLEzi5",
     "language": "de", "locale": "de-de", "gender": "female", "age": "Young",
     "description": "Upbeat.", "zusatz": {"bleibt": true}}
  ]
}"#;

struct Env {
    tmp: TempDir,
    models: Vec<ModelTarget>,
    lock: Mutex<()>,
}

impl Env {
    /// `n` installierte Modelle mit je zwei mitgelieferten Stimmen und `SOURCE.json`.
    fn new(n: usize) -> Self {
        let tmp = TempDir::new().unwrap();
        let mut models = Vec::new();
        for i in 0..n {
            let id = ["phonon-40m-de", "phonon-31m-en"][i].to_string();
            let dir = tmp.path().join("models").join(&id);
            let voices = dir.join("voices");
            std::fs::create_dir_all(&voices).unwrap();
            std::fs::write(voices.join("Nele.safetensors"), voice_bytes(1)).unwrap();
            std::fs::write(voices.join("Jette.safetensors"), voice_bytes(2)).unwrap();
            std::fs::write(voices.join("SOURCE.json"), SOURCE_JSON).unwrap();
            models.push(ModelTarget { id, dir });
        }
        Self {
            tmp,
            models,
            lock: Mutex::new(()),
        }
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().to_path_buf()
    }

    fn store(&self) -> VoiceStore<'_> {
        // `root` muss ueber den Aufruf hinaus leben: der Pfad liegt im TempDir.
        VoiceStore {
            root: self.tmp.path(),
            targets: &self.models,
            lock: &self.lock,
        }
    }

    fn voices_dir(&self, i: usize) -> PathBuf {
        self.models[i].dir.join("voices")
    }

    fn source(&self, i: usize) -> Value {
        serde_json::from_slice(&std::fs::read(self.voices_dir(i).join("SOURCE.json")).unwrap())
            .unwrap()
    }

    fn rows(&self, i: usize) -> Vec<Value> {
        self.source(i)["voices"].as_array().unwrap().clone()
    }

    fn row_named(&self, i: usize, name: &str) -> Option<Value> {
        self.rows(i).into_iter().find(|r| r["name"] == name)
    }

    fn master(&self, id: &str) -> PathBuf {
        self.root()
            .join(STORE_DIR)
            .join(format!("{id}.safetensors"))
    }
}

fn new_voice(name: &str, seed: u32) -> NewVoice {
    NewVoice {
        name: name.to_string(),
        bytes: voice_bytes(seed),
        origin: ORIGIN_IMPORT,
        voice_id: String::new(),
        language: String::new(),
        consent_at: None,
        source_note: None,
    }
}

fn no_partials(dir: &Path) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        assert!(!n.ends_with(".partial"), "Rest: {n} in {}", dir.display());
    }
}

// ---- Namen --------------------------------------------------------------------------------

#[test]
fn namen_werden_geprueft_und_zu_dateinamen() {
    let ok = |raw: &str| parse_name(raw).unwrap();
    assert_eq!(ok("Anna").id, "Anna");
    assert_eq!(ok("  Anna   Lena ").display, "Anna Lena");
    assert_eq!(ok("Anna Lena").id, "Anna-Lena");
    assert_eq!(ok("Müller Größe").id, "Mueller-Groesse");
    assert_eq!(ok("Müller Größe").display, "Müller Größe");
    assert_eq!(ok("José").id, "Jose");
    assert_eq!(ok("Dr. Who (alt)").id, "Dr-Who-alt");
    assert_eq!(ok("a_b-c").id, "a_b-c");
    assert_eq!(ok(&"x".repeat(40)).id.len(), 40);

    for bad in [
        "",
        "   ",
        "..",
        ".versteckt",
        "a/b",
        "a\\b",
        "C:\\x",
        "a:b",
        "n*m",
        "q?",
        "a\"b",
        "<x>",
        "a|b",
        "tab\tname",
        "zeile\nzwei",
        "Ende.",
        "😀",
        "日本語",
        "---",
        &"x".repeat(41),
    ] {
        assert!(
            matches!(parse_name(bad), Err(VoiceError::InvalidName(_))),
            "{bad:?} muss abgelehnt werden"
        );
    }
    // Vom System oder von Phonon belegt.
    for reserved in [
        "CON", "nul", "Com1", "LPT9", "aux", "prn", "default", "Default",
    ] {
        assert!(
            matches!(parse_name(reserved), Err(VoiceError::InvalidName(_))),
            "{reserved}"
        );
    }
}

// ---- safetensors-Prüfung --------------------------------------------------------------------

#[test]
fn eine_echte_form_ist_gueltig_und_kaputtes_nicht() {
    assert_eq!(validate_voice_bytes(&voice_bytes(1)), Ok(()));
    assert_eq!(
        voice_bytes(1).len(),
        256_136,
        "gleiche Groesse wie die echten Stimmen"
    );

    let good_header = |extra: &str| {
        format!(
            r#"{{"speaker_wavs":{{"dtype":"F32","shape":[1,512,8],"data_offsets":[0,16384]}}{extra}}}"#
        )
    };
    let data = vec![0u8; 16384];
    assert_eq!(validate_voice_bytes(&st(&good_header(""), &data)), Ok(()));

    let invalid = |name: &str, bytes: Vec<u8>| {
        assert!(
            matches!(
                validate_voice_bytes(&bytes),
                Err(VoiceError::InvalidFile(_))
            ),
            "{name} muss als ungueltig gelten"
        );
    };
    invalid("leer", Vec::new());
    invalid("zu klein", vec![1; 100]);
    invalid(
        "zufall",
        (0..256_136u32).map(|i| (i * 31 % 251) as u8).collect(),
    );
    invalid("programm", {
        let mut v = b"MZ".to_vec();
        v.resize(300_000, 0x90);
        v
    });
    // Kopflaenge riesig / groesser als die Datei: nie allozieren.
    let mut huge = u64::MAX.to_le_bytes().to_vec();
    huge.resize(20_000, b' ');
    invalid("kopf u64::MAX", huge);
    let mut past = 100_000u64.to_le_bytes().to_vec();
    past.resize(20_000, b' ');
    invalid("kopf hinter dem Ende", past);
    invalid("kopf kein json", st("{kaputt", &data));
    invalid("kopf kein objekt", st("[1,2,3]", &data));
    invalid(
        "ohne speaker_wavs",
        st(
            r#"{"andere":{"dtype":"F32","shape":[1,512,8],"data_offsets":[0,16384]}}"#,
            &data,
        ),
    );
    invalid(
        "falsche form",
        st(
            r#"{"speaker_wavs":{"dtype":"F32","shape":[1,256,16],"data_offsets":[0,16384]}}"#,
            &data,
        ),
    );
    invalid(
        "falscher rang",
        st(
            r#"{"speaker_wavs":{"dtype":"F32","shape":[512,8],"data_offsets":[0,16384]}}"#,
            &data,
        ),
    );
    invalid(
        "ganzzahl statt float",
        st(
            r#"{"speaker_wavs":{"dtype":"I32","shape":[1,512,8],"data_offsets":[0,16384]}}"#,
            &data,
        ),
    );
    invalid(
        "unbekannter dtype",
        st(
            r#"{"speaker_wavs":{"dtype":"X99","shape":[1,512,8],"data_offsets":[0,16384]}}"#,
            &data,
        ),
    );
    invalid(
        "offsets hinter den Daten",
        st(
            r#"{"speaker_wavs":{"dtype":"F32","shape":[1,512,8],"data_offsets":[0,32768]}}"#,
            &data,
        ),
    );
    invalid(
        "groesse passt nicht zur form",
        st(
            r#"{"speaker_wavs":{"dtype":"F32","shape":[1,512,8],"data_offsets":[0,8192]}}"#,
            &data[..8192],
        ),
    );
    invalid("angehaengte Daten", {
        let mut v = st(&good_header(""), &data);
        v.extend(b"MZ-nutzlast-die-nichts-in-einer-stimme-verloren-hat");
        v
    });
    invalid(
        "Loch am Anfang",
        st(
            r#"{"speaker_wavs":{"dtype":"F32","shape":[1,512,7],"data_offsets":[2048,16384]}}"#,
            &data,
        ),
    );
    invalid(
        "form laeuft ueber",
        st(
            r#"{"speaker_wavs":{"dtype":"F32","shape":[1,512,4294967296],"data_offsets":[0,16384]}}"#,
            &data,
        ),
    );
    invalid(
        "metadaten keine strings",
        st(&good_header(r#","__metadata__":{"a":1}"#), &data),
    );
    invalid("zu gross", vec![0; (MAX_VOICE_BYTES + 1) as usize]);
}

// ---- Import ---------------------------------------------------------------------------------

#[test]
fn der_import_legt_datei_und_eintrag_im_modell_ab() {
    let env = Env::new(1);
    let src = env.tmp.path().join("meine-stimme.safetensors");
    let bytes = voice_bytes(42);
    std::fs::write(&src, &bytes).unwrap();

    let v = env
        .store()
        .import_file(&src, "Meine Stimme", false)
        .unwrap();
    assert_eq!(v.id, "Meine-Stimme");
    assert_eq!(v.name, "Meine Stimme");
    assert_eq!(v.origin, ORIGIN_IMPORT);
    assert_eq!(v.license, OWN_LICENSE);
    assert!(!v.verified);
    assert_eq!(v.in_models, vec!["phonon-40m-de".to_string()]);

    // Kopie im Modell, byte-gleich; die Quelle bleibt unberuehrt.
    assert_eq!(
        std::fs::read(env.voices_dir(0).join("Meine-Stimme.safetensors")).unwrap(),
        bytes
    );
    assert_eq!(std::fs::read(&src).unwrap(), bytes);
    assert_eq!(std::fs::read(env.master("Meine-Stimme")).unwrap(), bytes);
    no_partials(&env.voices_dir(0));

    // SOURCE.json: Zeile mit Herkunft, Lizenz, nicht verifiziert.
    let row = env.row_named(0, "Meine Stimme").expect("Zeile fehlt");
    assert_eq!(row["origin"], "import");
    assert_eq!(row["license"], OWN_LICENSE);
    assert_eq!(row["verified"], false);
    assert_eq!(row["filename"], "voices/Meine-Stimme.safetensors");
    // Die Modellseite liest jede Zeile typisiert: Pflichtfelder muessen da sein.
    serde_json::from_value::<PhononVoiceMeta>(row).expect("Zeile muss PhononVoiceMeta entsprechen");

    // Mitgelieferte Zeilen und Kopffelder sind unveraendert, auch unbekannte Zusatzfelder.
    let doc = env.source(0);
    assert_eq!(doc["license"], "apache-2.0");
    assert_eq!(
        doc["source"],
        "hf://gradium/phonon-voices@0ee17205f254b5abc43b266c8a0f215565c352f7"
    );
    let original: Value = serde_json::from_str(SOURCE_JSON).unwrap();
    assert_eq!(doc["voices"][0], original["voices"][0]);
    assert_eq!(doc["voices"][1], original["voices"][1]);
    assert_eq!(doc["voices"].as_array().unwrap().len(), 3);
}

#[test]
fn ohne_source_json_wird_sie_angelegt() {
    let env = Env::new(1);
    std::fs::remove_file(env.voices_dir(0).join("SOURCE.json")).unwrap();
    env.store().add(new_voice("Neu", 5), false).unwrap();
    let doc = env.source(0);
    assert_eq!(doc["voices"].as_array().unwrap().len(), 1);
    assert_eq!(doc["voices"][0]["origin"], "import");
    // Beide Kopffelder gehoeren dazu, sonst liest die Modellseite die Datei nicht.
    assert!(doc["source"].is_string() && doc["license"].is_string());
}

#[test]
fn ohne_voices_ordner_im_modell_wird_er_angelegt() {
    let env = Env::new(1);
    std::fs::remove_dir_all(env.voices_dir(0)).unwrap();
    env.store().add(new_voice("Neu", 5), false).unwrap();
    assert!(env.voices_dir(0).join("Neu.safetensors").is_file());
}

#[test]
fn ohne_modell_liegt_die_stimme_im_eigenen_speicher_und_wird_nachgezogen() {
    let tmp = TempDir::new().unwrap();
    let lock = Mutex::new(());
    let none: Vec<ModelTarget> = Vec::new();
    let store = VoiceStore {
        root: tmp.path(),
        targets: &none,
        lock: &lock,
    };
    let v = store.add(new_voice("Spaeter", 9), false).unwrap();
    assert!(v.in_models.is_empty(), "noch in keinem Modell");
    assert_eq!(store.list().len(), 1);

    // Das Modell wird spaeter installiert: der Abgleich bestueckt es.
    let dir = tmp.path().join("models").join("phonon-31m-en");
    std::fs::create_dir_all(dir.join("voices")).unwrap();
    let later = vec![ModelTarget {
        id: "phonon-31m-en".to_string(),
        dir: dir.clone(),
    }];
    let store = VoiceStore {
        root: tmp.path(),
        targets: &later,
        lock: &lock,
    };
    let report = store.sync();
    assert_eq!(report.copied, 1);
    assert!(dir.join("voices").join("Spaeter.safetensors").is_file());
    assert_eq!(store.list()[0].in_models, vec!["phonon-31m-en".to_string()]);
    // Ein zweiter Abgleich aendert nichts mehr (idempotent, schreibt nicht ohne Not).
    let before = std::fs::read(dir.join("voices").join("SOURCE.json")).unwrap();
    assert_eq!(store.sync(), SyncReport::default());
    assert_eq!(
        std::fs::read(dir.join("voices").join("SOURCE.json")).unwrap(),
        before
    );
}

#[test]
fn jedes_installierte_modell_bekommt_die_stimme() {
    let env = Env::new(2);
    let v = env.store().add(new_voice("Beide", 3), false).unwrap();
    assert_eq!(v.in_models.len(), 2);
    for i in 0..2 {
        assert!(env.voices_dir(i).join("Beide.safetensors").is_file());
        assert!(env.row_named(i, "Beide").is_some());
    }
}

// ---- Überschreiben und Namenskonflikte --------------------------------------------------------

#[test]
fn ueberschreiben_nur_mit_rueckfrage() {
    let env = Env::new(2);
    let first = voice_bytes(1);
    let second = voice_bytes(2);
    env.store()
        .add(
            NewVoice {
                bytes: first.clone(),
                ..new_voice("Anna", 1)
            },
            false,
        )
        .unwrap();

    // Ohne Zustimmung: Fehler, nichts aendert sich.
    let err = env
        .store()
        .add(
            NewVoice {
                bytes: second.clone(),
                ..new_voice("Anna", 2)
            },
            false,
        )
        .unwrap_err();
    assert_eq!(
        err,
        VoiceError::Exists {
            id: "Anna".to_string()
        }
    );
    assert_eq!(std::fs::read(env.master("Anna")).unwrap(), first);
    assert_eq!(
        std::fs::read(env.voices_dir(0).join("Anna.safetensors")).unwrap(),
        first
    );

    // Auch die Namensprobe vorab sagt es (die Oberflaeche fragt dann zurueck).
    let check = env.store().check_name("anna", false);
    assert_eq!(
        check,
        Err(VoiceError::Exists {
            id: "Anna".to_string()
        })
    );
    let check = env.store().check_name("anna", true).unwrap();
    assert!(check.replaces);
    assert_eq!(check.id, "Anna", "der vorhandene Dateiname bleibt");

    // Mit Zustimmung: ersetzt, in allen Modellen, genau eine Zeile.
    let v = env
        .store()
        .add(
            NewVoice {
                bytes: second.clone(),
                ..new_voice("anna", 2)
            },
            true,
        )
        .unwrap();
    assert_eq!(v.id, "Anna");
    assert_eq!(v.name, "anna");
    for i in 0..2 {
        assert_eq!(
            std::fs::read(env.voices_dir(i).join("Anna.safetensors")).unwrap(),
            second
        );
        let n = env
            .rows(i)
            .iter()
            .filter(|r| r["filename"] == "voices/Anna.safetensors")
            .count();
        assert_eq!(n, 1);
    }
    assert_eq!(env.store().list().len(), 1);
    no_partials(&env.voices_dir(0));
    no_partials(&env.root().join(STORE_DIR));
}

#[test]
fn namen_der_modellstimmen_sind_tabu_auch_mit_zustimmung() {
    let env = Env::new(1);
    let nele_before = std::fs::read(env.voices_dir(0).join("Nele.safetensors")).unwrap();
    for name in ["Nele", "nele", "NELE"] {
        for overwrite in [false, true] {
            let err = env.store().add(new_voice(name, 7), overwrite).unwrap_err();
            assert!(
                matches!(err, VoiceError::TakenByModel { .. }),
                "{name}/{overwrite}: {err:?}"
            );
        }
    }
    assert_eq!(
        std::fs::read(env.voices_dir(0).join("Nele.safetensors")).unwrap(),
        nele_before
    );
    assert!(env.store().list().is_empty(), "nichts wurde angelegt");
    assert!(!env.root().join(STORE_DIR).join("Nele.safetensors").exists());
}

// ---- Entfernen --------------------------------------------------------------------------------

#[test]
fn entfernen_nimmt_datei_und_eintrag_aus_allen_modellen() {
    let env = Env::new(2);
    env.store().add(new_voice("Weg", 4), false).unwrap();
    env.store().add(new_voice("Bleibt", 5), false).unwrap();

    env.store().remove("Weg").unwrap();
    for i in 0..2 {
        assert!(!env.voices_dir(i).join("Weg.safetensors").exists());
        assert!(env.row_named(i, "Weg").is_none());
        assert!(env.row_named(i, "Bleibt").is_some());
        assert!(
            env.row_named(i, "Nele").is_some(),
            "mitgelieferte Stimmen bleiben"
        );
    }
    assert!(!env.master("Weg").exists());
    let ids: Vec<String> = env.store().list().into_iter().map(|v| v.id).collect();
    assert_eq!(ids, vec!["Bleibt".to_string()]);
}

#[test]
fn mitgelieferte_stimmen_lassen_sich_nie_hier_entfernen() {
    let env = Env::new(1);
    for id in ["Nele", "nele", "Jette", "gibtsnicht", "../Nele", ""] {
        assert!(
            matches!(env.store().remove(id), Err(VoiceError::NotOwn(_))),
            "{id:?}"
        );
    }
    assert!(env.voices_dir(0).join("Nele.safetensors").is_file());
    assert!(env.row_named(0, "Nele").is_some());
}

// ---- Abgleich / Selbstheilung ---------------------------------------------------------------------

/// Die Modellseite liest und schreibt `SOURCE.json` typisiert und verliert dabei unbekannte
/// Felder. Der Abgleich bringt Herkunft und Lizenz der eigenen Zeilen zurueck.
#[test]
fn der_eintrag_ueberlebt_einen_lauf_der_modellseite() {
    let env = Env::new(1);
    env.store().add(new_voice("Ich", 6), false).unwrap();

    let path = env.voices_dir(0).join("SOURCE.json");
    let mut doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for row in doc["voices"].as_array_mut().unwrap() {
        let meta: PhononVoiceMeta = serde_json::from_value(row.clone()).unwrap();
        *row = serde_json::to_value(meta).unwrap(); // wie `write_voice_source`
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    assert!(env.row_named(0, "Ich").unwrap().get("origin").is_none());

    let report = env.store().sync();
    assert_eq!(report.rows_healed, 1);
    assert!(report.conflicts.is_empty());
    assert_eq!(env.row_named(0, "Ich").unwrap()["origin"], "import");
    assert_eq!(env.store().list()[0].in_models.len(), 1);
}

#[test]
fn ein_geloeschtes_modell_wird_beim_abgleich_neu_bestueckt() {
    let env = Env::new(1);
    env.store().add(new_voice("Treu", 8), false).unwrap();
    // Das Modell wird auf der Modellseite geloescht und neu installiert.
    std::fs::remove_dir_all(env.voices_dir(0)).unwrap();
    std::fs::create_dir_all(env.voices_dir(0)).unwrap();
    std::fs::write(env.voices_dir(0).join("Nele.safetensors"), voice_bytes(1)).unwrap();
    assert!(env.store().list()[0].in_models.is_empty());

    let report = env.store().sync();
    assert_eq!(report.copied, 1);
    assert_eq!(
        std::fs::read(env.voices_dir(0).join("Treu.safetensors")).unwrap(),
        voice_bytes(8)
    );
    assert!(env.row_named(0, "Treu").is_some());
}

#[test]
fn eine_fremde_datei_gleichen_namens_wird_beim_abgleich_nie_ueberschrieben() {
    let env = Env::new(1);
    env.store().add(new_voice("Doppelt", 3), false).unwrap();
    // Ein spaeteres Modell bringt eine eigene Stimme gleichen Namens mit: andere Bytes, Zeile ohne
    // Herkunft.
    let theirs = voice_bytes(99);
    std::fs::write(env.voices_dir(0).join("Doppelt.safetensors"), &theirs).unwrap();
    let path = env.voices_dir(0).join("SOURCE.json");
    let mut doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    doc["voices"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["name"] != "Doppelt");
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();

    let report = env.store().sync();
    assert_eq!(report.conflicts, vec!["phonon-40m-de: Doppelt".to_string()]);
    assert_eq!(
        std::fs::read(env.voices_dir(0).join("Doppelt.safetensors")).unwrap(),
        theirs
    );
    assert!(
        env.row_named(0, "Doppelt").is_none(),
        "keine Zeile fuer fremde Datei"
    );
}

// ---- Nebenläufigkeit ----------------------------------------------------------------------------------

#[test]
fn parallele_importe_verlieren_nichts() {
    let env = Arc::new(Env::new(2));
    let handles: Vec<_> = (0..8u32)
        .map(|i| {
            let env = Arc::clone(&env);
            std::thread::spawn(move || {
                env.store()
                    .add(new_voice(&format!("Stimme {i}"), i + 10), false)
                    .map(|v| v.id)
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap().expect("jeder Import gelingt");
    }
    assert_eq!(env.store().list().len(), 8);
    for m in 0..2 {
        let rows = env.rows(m);
        assert_eq!(rows.len(), 10, "2 mitgelieferte + 8 eigene");
        for i in 0..8 {
            assert!(
                env.row_named(m, &format!("Stimme {i}")).is_some(),
                "Zeile {i} in {m}"
            );
        }
        assert!(env.row_named(m, "Nele").is_some());
        no_partials(&env.voices_dir(m));
    }
}

#[test]
fn gleicher_name_parallel_gewinnt_genau_einer() {
    let env = Arc::new(Env::new(1));
    let handles: Vec<_> = (0..8u32)
        .map(|i| {
            let env = Arc::clone(&env);
            std::thread::spawn(move || env.store().add(new_voice("Streit", i), false))
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(VoiceError::Exists { .. })))
            .count(),
        7
    );
    assert_eq!(env.store().list().len(), 1);
    let n = env.rows(0).iter().filter(|r| r["name"] == "Streit").count();
    assert_eq!(n, 1);
}

// ---- Fehlerwege ----------------------------------------------------------------------------------------

#[test]
fn scheitert_das_schreiben_bleibt_der_alte_stand_und_kein_rest() {
    let env = Env::new(1);
    env.store().add(new_voice("Alt", 1), false).unwrap();
    // Ziel der Ersetzung laesst sich nicht ersetzen (hier: ein Ordner an der Stelle der Datei).
    let master = env.master("Alt");
    std::fs::remove_file(&master).unwrap();
    std::fs::create_dir(&master).unwrap();
    let err = env.store().add(new_voice("Alt", 2), true).unwrap_err();
    assert!(matches!(err, VoiceError::Io(_)), "{err:?}");
    no_partials(&env.root().join(STORE_DIR));
    // Im Modell blieb die alte Stimme unberuehrt.
    assert_eq!(
        std::fs::read(env.voices_dir(0).join("Alt.safetensors")).unwrap(),
        voice_bytes(1)
    );
}

#[test]
fn ein_unbeschreibbares_modell_stoert_die_anderen_nicht() {
    let env = Env::new(2);
    // Im ersten Modell steht an der Stelle von `voices` eine Datei: dort laesst sich nichts ablegen.
    std::fs::remove_dir_all(env.voices_dir(0)).unwrap();
    std::fs::write(env.voices_dir(0), b"keine Verzeichnis").unwrap();

    let v = env.store().add(new_voice("Trotzdem", 3), false).unwrap();
    assert_eq!(v.in_models, vec!["phonon-31m-en".to_string()]);
    assert!(env.voices_dir(1).join("Trotzdem.safetensors").is_file());
    assert!(
        env.master("Trotzdem").is_file(),
        "der eigene Speicher hat sie"
    );
}

#[test]
fn eine_kaputte_source_json_bleibt_unangetastet_die_stimme_ist_trotzdem_da() {
    let env = Env::new(1);
    let path = env.voices_dir(0).join("SOURCE.json");
    std::fs::write(&path, "{{ kaputt").unwrap();
    let v = env.store().add(new_voice("Ohne Zeile", 4), false).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{{ kaputt");
    assert!(env.voices_dir(0).join("Ohne-Zeile.safetensors").is_file());
    assert_eq!(v.in_models.len(), 1);
    no_partials(&env.voices_dir(0));
}

#[test]
fn ein_defekter_index_wird_gesichert_und_aus_den_dateien_neu_aufgebaut() {
    let env = Env::new(1);
    env.store()
        .add(
            NewVoice {
                origin: ORIGIN_GRADIUM,
                voice_id: "abc123".to_string(),
                language: "de".to_string(),
                ..new_voice("Klon", 4)
            },
            false,
        )
        .unwrap();
    std::fs::write(env.root().join(STORE_DIR).join("stimmen.json"), "][ kaputt").unwrap();

    let list = env.store().list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "Klon");
    // Aus der Zeile im Modell zurueckgewonnen.
    assert_eq!(list[0].origin, ORIGIN_GRADIUM);
    assert_eq!(list[0].voice_id, "abc123");
    assert!(env
        .root()
        .join(STORE_DIR)
        .join("stimmen.json.defekt")
        .is_file());
    // Und wieder gueltig: ein weiterer Import funktioniert.
    env.store().add(new_voice("Zwei", 5), false).unwrap();
    assert_eq!(env.store().list().len(), 2);
}

// ---- Datei-Import: Prüfungen vor dem Kopieren -------------------------------------------------------------

#[test]
fn der_dateiimport_prueft_vor_dem_kopieren() {
    let env = Env::new(1);
    let dir = env.tmp.path().join("quelle");
    std::fs::create_dir_all(&dir).unwrap();

    // Falsche Endung, fehlende Datei, Ordner.
    let wav = dir.join("x.wav");
    std::fs::write(&wav, voice_bytes(1)).unwrap();
    for p in [wav, dir.join("fehlt.safetensors"), dir.clone()] {
        assert!(
            matches!(
                env.store().import_file(&p, "Name", false),
                Err(VoiceError::InvalidFile(_))
            ),
            "{}",
            p.display()
        );
    }
    // Zufallsbytes unter richtiger Endung.
    let junk = dir.join("junk.safetensors");
    std::fs::write(
        &junk,
        (0..300_000u32).map(|i| (i % 253) as u8).collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(matches!(
        env.store().import_file(&junk, "Name", false),
        Err(VoiceError::InvalidFile(_))
    ));
    // Abgeschnitten.
    let cut = dir.join("cut.safetensors");
    std::fs::write(&cut, &voice_bytes(1)[..100_000]).unwrap();
    assert!(matches!(
        env.store().import_file(&cut, "Name", false),
        Err(VoiceError::InvalidFile(_))
    ));
    // Riesig (sparse, wird nicht gelesen): Groesse entscheidet vor dem Lesen.
    let big = dir.join("big.safetensors");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(MAX_VOICE_BYTES * 3).unwrap();
    drop(f);
    let err = env.store().import_file(&big, "Name", false).unwrap_err();
    assert!(
        matches!(err, VoiceError::InvalidFile(ref m) if m.contains("groß")),
        "{err:?}"
    );
    // Schlechter Name bei guter Datei: nichts wird kopiert.
    let good = dir.join("gut.safetensors");
    std::fs::write(&good, voice_bytes(1)).unwrap();
    assert!(matches!(
        env.store().import_file(&good, "../Name", false),
        Err(VoiceError::InvalidName(_))
    ));
    assert!(env.store().list().is_empty());
    assert_eq!(
        std::fs::read_dir(env.voices_dir(0)).unwrap().count(),
        3,
        "Modell unveraendert"
    );
}

#[test]
fn die_obergrenze_der_stimmenzahl_gilt_nur_fuer_neue() {
    assert!(check_capacity(0, false).is_ok());
    assert!(check_capacity(MAX_OWN_VOICES - 1, false).is_ok());
    assert_eq!(
        check_capacity(MAX_OWN_VOICES, false),
        Err(VoiceError::TooMany)
    );
    assert!(
        check_capacity(MAX_OWN_VOICES, true).is_ok(),
        "Ersetzen ist keine neue Stimme"
    );
}

#[test]
fn fehlermeldungen_nennen_keine_pfade() {
    let env = Env::new(1);
    let root = env.tmp.path().to_string_lossy().to_string();
    let junk = env.tmp.path().join("kaputt.safetensors");
    std::fs::write(&junk, vec![7u8; 50_000]).unwrap();
    let err = env.store().import_file(&junk, "Name", false).unwrap_err();
    assert!(!err.to_string().contains(&root), "{err}");
    let err = env
        .store()
        .import_file(&env.tmp.path().join("fehlt.safetensors"), "N", false)
        .unwrap_err();
    assert!(!err.to_string().contains(&root), "{err}");
}

#[test]
fn eigene_stimmen_tragen_eine_eigene_unbekannte_nie_verifizierte_lizenz() {
    assert!(origin_is_own(ORIGIN_IMPORT) && origin_is_own(ORIGIN_GRADIUM));
    for other in ["", "catalog", "Import", "gradium"] {
        assert!(!origin_is_own(other), "{other}");
    }
    for origin in [ORIGIN_IMPORT, ORIGIN_GRADIUM] {
        let l = own_voice_license(origin);
        assert_eq!(l.code, OWN_LICENSE);
        assert_eq!(l.weights, OWN_LICENSE);
        assert!(l.non_commercial, "vorsorglich nichtkommerziell");
        assert!(!l.verified && l.verified_on.is_none() && l.source_url.is_empty());
        assert!(l.note.contains("Einwilligung"), "{}", l.note);
    }
    assert!(own_voice_license(ORIGIN_GRADIUM).note.contains("Gradium"));
    assert!(!own_voice_license(ORIGIN_IMPORT).note.contains("Gradium"));
}
