use super::*;
use crate::managers::tts::phonon_voices::{sample_voice_bytes, ModelTarget, STORE_DIR};
use crate::managers::tts::providers::test_http::{FakeServer, Reply, Req};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const KEY: &str = "gsk-geheim-abcdef123456";

type KeyFn = Box<dyn Fn() -> Result<Option<Secret>, String> + Send + Sync>;

struct Fx {
    tmp: TempDir,
    models: Vec<ModelTarget>,
    lock: Mutex<()>,
    recording: Mutex<Option<Recording>>,
    key: KeyFn,
    key_calls: Arc<AtomicUsize>,
    fish: PathBuf,
}

impl Fx {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("models").join("phonon-40m-de");
        std::fs::create_dir_all(dir.join("voices")).unwrap();
        std::fs::write(
            dir.join("voices").join("Nele.safetensors"),
            sample_voice_bytes(1),
        )
        .unwrap();
        let fish = tmp.path().join("fish");
        std::fs::create_dir_all(&fish).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        Self {
            models: vec![ModelTarget {
                id: "phonon-40m-de".to_string(),
                dir,
            }],
            lock: Mutex::new(()),
            recording: Mutex::new(None),
            key: Box::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(Some(Secret::new(KEY).unwrap()))
            }),
            key_calls: calls,
            fish,
            tmp,
        }
    }

    fn with_key(mut self, key: KeyFn) -> Self {
        self.key = key;
        self
    }

    fn env(&self, cfg: gradium::Config) -> CloneEnv<'_> {
        CloneEnv {
            store: VoiceStore {
                root: self.tmp.path(),
                targets: &self.models,
                lock: &self.lock,
            },
            fish_dir: &self.fish,
            gradium: cfg,
            key: &*self.key,
            recording: &self.recording,
        }
    }

    fn voices(&self) -> PathBuf {
        self.models[0].dir.join("voices")
    }

    /// Nichts hat sich im Modell oder im eigenen Speicher geaendert.
    fn assert_untouched(&self) {
        let names: Vec<String> = std::fs::read_dir(self.voices())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["Nele.safetensors".to_string()], "{names:?}");
        assert!(
            !self.tmp.path().join(STORE_DIR).exists(),
            "eigener Speicher blieb leer"
        );
    }

    fn write_wav(&self, name: &str, seconds: f32, rate: u32, amp: f32) -> String {
        let p = self.tmp.path().join(name);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&p, spec).unwrap();
        for i in 0..(seconds * rate as f32) as usize {
            let v = amp * (i as f32 * 0.07).sin();
            w.write_sample((v * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
        p.to_string_lossy().to_string()
    }
}

fn request(source: CloneSource) -> CloneRequest {
    CloneRequest {
        source,
        name: "Meine Stimme".to_string(),
        language: "de".to_string(),
        start_s: 0.0,
        consent: true,
        overwrite: false,
    }
}

fn noop(_: Phase) {}

const CREATE: &str = "/api/voices/";
const POLL: &str = "/api/voices/embeddings/vox_9/default";
const BLOB: &str = "/api/voices/embeddings/vox_9/default/blob?as_safetensors=1";

fn gradium_ok(blob: Vec<u8>) -> impl Fn(&Req) -> Reply + Send + Sync + 'static {
    move |req| match (req.method.as_str(), req.path.as_str()) {
        ("POST", CREATE) => Reply::json(200, json!({"uid": "vox_9"})),
        ("GET", POLL) => Reply::json(200, json!({"found": true})),
        ("GET", BLOB) => Reply::audio(blob.clone()),
        _ => Reply::text(404, ""),
    }
}

fn silent_server() -> impl Fn(&Req) -> Reply + Send + Sync + 'static {
    |_| panic!("an dieser Stelle darf kein Netzzugriff stattfinden")
}

// ---- Sperren vor dem Netz ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ohne_haekchen_geschieht_nichts_nicht_einmal_das_lesen_des_schluessels() {
    let fx = Fx::new();
    let fake = FakeServer::start(gradium_ok(sample_voice_bytes(5))).await;
    let slot = JobSlot::new();
    // Auch mit einer nicht vorhandenen Datei: die Einwilligung wird ZUERST geprueft.
    let mut req = request(CloneSource::File {
        path: "C:\\gibt\\es\\nicht.wav".to_string(),
    });
    req.consent = false;
    let err = create_voice(&fx.env(gradium::Config::for_fake(&fake)), &slot, req, &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::ConsentRequired);
    assert!(fake.requests().is_empty(), "kein einziger Netzzugriff");
    assert_eq!(
        fx.key_calls.load(Ordering::SeqCst),
        0,
        "Schluessel nicht gelesen"
    );
    assert!(!slot.is_running());
    fx.assert_untouched();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ohne_schluessel_oder_mit_defektem_speicher_kein_netzzugriff() {
    let wav = |fx: &Fx| fx.write_wav("q.wav", 12.0, 16_000, 0.5);
    let fake = FakeServer::start(silent_server()).await;
    let slot = JobSlot::new();

    let fx = Fx::new().with_key(Box::new(|| Ok(None)));
    let req = request(CloneSource::File { path: wav(&fx) });
    let err = create_voice(&fx.env(gradium::Config::for_fake(&fake)), &slot, req, &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::NoKey);
    assert!(err.message.contains("Schlüssel"), "{err}");

    let fx = Fx::new().with_key(Box::new(|| Err("nicht lesbar".to_string())));
    let req = request(CloneSource::File { path: wav(&fx) });
    let err = create_voice(&fx.env(gradium::Config::for_fake(&fake)), &slot, req, &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::KeyStore);

    assert!(fake.requests().is_empty());
    assert!(
        !slot.is_running(),
        "der Platz wird auch nach einem Fehler frei"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ungueltige_eingaben_scheitern_vor_dem_netz() {
    let fx = Fx::new();
    let fake = FakeServer::start(silent_server()).await;
    let slot = JobSlot::new();
    let good = fx.write_wav("gut.wav", 12.0, 16_000, 0.5);
    let tiny = fx.write_wav("kurz.wav", 1.5, 16_000, 0.5);
    let silent = fx.write_wav("still.wav", 12.0, 16_000, 0.0);
    let junk_mp3 = fx.tmp.path().join("kaputt.mp3");
    std::fs::write(&junk_mp3, b"das ist kein mp3, nur text").unwrap();
    let exe = fx.tmp.path().join("x.exe");
    std::fs::write(&exe, b"MZ").unwrap();

    let run = |source: CloneSource, edit: &dyn Fn(&mut CloneRequest)| {
        let mut req = request(source);
        edit(&mut req);
        let env = fx.env(gradium::Config::for_fake(&fake));
        let slot = &slot;
        async move { create_voice(&env, slot, req, &noop).await.unwrap_err() }
    };
    let file = |p: &str| CloneSource::File {
        path: p.to_string(),
    };
    use PhononVoiceErrorKind as K;

    assert_eq!(
        run(file(&good), &|r| r.language = "xx".into()).await.kind,
        K::InvalidLanguage
    );
    assert_eq!(
        run(file(&good), &|r| r.name = "../x".into()).await.kind,
        K::InvalidName
    );
    assert_eq!(
        run(file(&good), &|r| r.name = "Nele".into()).await.kind,
        K::TakenByModel
    );
    assert_eq!(
        run(file(&good), &|r| r.start_s = f32::NAN).await.kind,
        K::BadSource
    );
    assert_eq!(
        run(file(&good), &|r| r.start_s = 11.0).await.kind,
        K::BadSource
    );
    assert_eq!(run(file(&tiny), &|_| {}).await.kind, K::BadSource);
    assert_eq!(run(file(&silent), &|_| {}).await.kind, K::BadSource);
    assert_eq!(
        run(file("C:\\gibt\\es\\nicht.wav"), &|_| {}).await.kind,
        K::BadSource
    );
    assert_eq!(
        run(file(&fx.tmp.path().to_string_lossy()), &|_| {})
            .await
            .kind,
        K::BadSource
    );
    assert_eq!(
        run(file(&exe.to_string_lossy()), &|_| {}).await.kind,
        K::BadSource
    );
    // Nicht dekodierbar (mit oder ohne ffmpeg): eine Meldung, kein Netz, keine Reste.
    let err = run(file(&junk_mp3.to_string_lossy()), &|_| {}).await;
    assert_eq!(err.kind, K::BadSource);
    assert!(
        !err.message.contains(&*fx.tmp.path().to_string_lossy()),
        "{err}"
    );
    // Mikrofon ohne Aufnahme, unbekannte Fish-Stimme.
    assert_eq!(
        run(CloneSource::Recording, &|_| {}).await.kind,
        K::BadSource
    );
    assert_eq!(
        run(
            CloneSource::Fish {
                voice_id: "gibtsnicht".into()
            },
            &|_| {}
        )
        .await
        .kind,
        K::BadSource
    );
    assert_eq!(
        run(
            CloneSource::Fish {
                voice_id: "../x".into()
            },
            &|_| {}
        )
        .await
        .kind,
        K::BadSource
    );

    assert!(
        fake.requests().is_empty(),
        "keine dieser Eingaben darf das Netz erreichen"
    );
    assert_eq!(
        fx.key_calls.load(Ordering::SeqCst),
        0,
        "Schluessel erst nach allen Pruefungen"
    );
    fx.assert_untouched();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ein_vorhandener_name_braucht_die_zustimmung_zum_ersetzen_vor_dem_netz() {
    let fx = Fx::new();
    let fake = FakeServer::start(gradium_ok(sample_voice_bytes(5))).await;
    let slot = JobSlot::new();
    let src = CloneSource::File {
        path: fx.write_wav("q.wav", 12.0, 16_000, 0.5),
    };
    create_voice(
        &fx.env(gradium::Config::for_fake(&fake)),
        &slot,
        request(src.clone()),
        &noop,
    )
    .await
    .expect("erste Erstellung");
    let calls_before = fake.requests().len();

    let err = create_voice(
        &fx.env(gradium::Config::for_fake(&fake)),
        &slot,
        request(src.clone()),
        &noop,
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::Exists);
    assert_eq!(
        fake.requests().len(),
        calls_before,
        "ein vergebener Name kostet keinen Aufruf"
    );

    let mut again = request(src);
    again.overwrite = true;
    create_voice(
        &fx.env(gradium::Config::for_fake(&fake)),
        &slot,
        again,
        &noop,
    )
    .await
    .expect("mit Zustimmung");
    assert_eq!(fake.requests().len(), calls_before * 2);
}

// ---- Erfolg ----------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aus_einer_datei_wird_eine_stimme_nur_das_zehnsekundenfenster_geht_hinaus() {
    let fx = Fx::new();
    let blob = sample_voice_bytes(77);
    let fake = FakeServer::start(gradium_ok(blob.clone())).await;
    let slot = JobSlot::new();
    // 20 s Datei mit eigenem Namen; Start bei 3 s.
    let path = fx.write_wav("Mein Privatgespraech 2026.wav", 20.0, 16_000, 0.5);
    let mut req = request(CloneSource::File { path });
    req.start_s = 3.0;
    let phases = Mutex::new(Vec::new());
    let voice = create_voice(
        &fx.env(gradium::Config::for_fake(&fake)),
        &slot,
        req,
        &|p| phases.lock().unwrap().push(p),
    )
    .await
    .expect("Erstellung gelingt");

    assert_eq!(voice.origin, "gradium-clone");
    assert_eq!(voice.voice_id, "vox_9");
    assert_eq!(voice.language, "de");
    assert_eq!(voice.name, "Meine Stimme");
    assert!(
        voice.consent_at.is_some(),
        "die bestaetigte Einwilligung wird festgehalten"
    );
    assert_eq!(voice.source_note.as_deref(), Some("Audiodatei"));
    assert_eq!(voice.license, "eigene/unbekannt");
    assert!(!voice.verified);
    assert_eq!(voice.in_models, vec!["phonon-40m-de".to_string()]);
    assert_eq!(
        *phases.lock().unwrap(),
        vec![Phase::Uploading, Phase::Waiting, Phase::Downloading]
    );

    // Die Datei im Modell ist die von Gradium, die Zeile traegt Herkunft und Kennung.
    assert_eq!(
        std::fs::read(fx.voices().join("Meine-Stimme.safetensors")).unwrap(),
        blob
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fx.voices().join("SOURCE.json")).unwrap()).unwrap();
    let row = &doc["voices"][0];
    assert_eq!(row["origin"], "gradium-clone");
    assert_eq!(row["voice_id"], "vox_9");
    assert_eq!(row["language"], "de");
    assert_eq!(row["license"], "eigene/unbekannt");
    assert_eq!(row["verified"], false);
    assert!(row["consent_at"].is_string());
    // Weder Pfad noch Dateiname stehen irgendwo.
    let source = std::fs::read_to_string(fx.voices().join("SOURCE.json")).unwrap();
    assert!(!source.contains("Privatgespraech"), "{source}");

    // Hinaus ging nur das Fenster: 10 s Mono-WAV mit 16 kHz, nicht die 20 s der Datei.
    let post = fake
        .requests()
        .into_iter()
        .find(|r| r.method == "POST")
        .unwrap();
    let text = String::from_utf8_lossy(&post.body).to_string();
    assert!(text.contains("filename=\"voice.wav\""));
    assert!(!text.contains("Privatgespraech"));
    assert!(
        (320_000..324_000).contains(&post.body.len()),
        "10 s * 16 kHz * 2 Byte = 320 044 Byte plus Rahmen, war {}",
        post.body.len()
    );
    // Der Schluessel ging nur im Kopf mit.
    assert!(!text.contains(KEY));
    assert_eq!(post.headers.get("x-api-key").map(String::as_str), Some(KEY));
    assert!(!slot.is_running());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eine_fish_stimme_wird_uebernommen_mit_jeweils_eigener_einwilligung() {
    let fx = Fx::new();
    // Referenz im Fish-Format: references/<id>/sample.wav + sample.lab
    let dir = fx.fish.join("references").join("erzaehler");
    std::fs::create_dir_all(&dir).unwrap();
    let wav = fx.write_wav("ref.wav", 14.0, 16_000, 0.5);
    std::fs::copy(&wav, dir.join("sample.wav")).unwrap();
    std::fs::write(dir.join("sample.lab"), "Das ist die Referenz.").unwrap();
    // Eine zweite ohne Referenzaufnahme (nur .wav ohne .lab ist keine Stimme).
    let broken = fx.fish.join("references").join("kaputt");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::copy(&wav, broken.join("sample.wav")).unwrap();

    let fake = FakeServer::start(gradium_ok(sample_voice_bytes(8))).await;
    let slot = JobSlot::new();
    let mut req = request(CloneSource::Fish {
        voice_id: "erzaehler".into(),
    });
    req.name = "Erzähler Phonon".into();

    // Ohne Haekchen: auch fuer eine Fish-Stimme nichts.
    let mut no = req.clone();
    no.consent = false;
    let err = create_voice(&fx.env(gradium::Config::for_fake(&fake)), &slot, no, &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::ConsentRequired);
    assert!(fake.requests().is_empty());

    let voice = create_voice(&fx.env(gradium::Config::for_fake(&fake)), &slot, req, &noop)
        .await
        .unwrap();
    assert_eq!(voice.id, "Erzaehler-Phonon");
    assert_eq!(voice.name, "Erzähler Phonon");
    assert_eq!(voice.source_note.as_deref(), Some("Fish-Stimme: erzaehler"));
    // Die Fish-Stimme selbst bleibt unveraendert.
    assert!(dir.join("sample.wav").is_file() && dir.join("sample.lab").is_file());

    let err = create_voice(
        &fx.env(gradium::Config::for_fake(&fake)),
        &slot,
        request(CloneSource::Fish {
            voice_id: "kaputt".into(),
        }),
        &noop,
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::BadSource);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn die_mikrofonaufnahme_wird_genutzt_und_danach_verworfen() {
    let fx = Fx::new();
    let fake = FakeServer::start(gradium_ok(sample_voice_bytes(9))).await;
    let slot = JobSlot::new();
    let samples: Vec<f32> = (0..16_000 * 8)
        .map(|i| 0.4 * (i as f32 * 0.05).sin())
        .collect();
    *fx.recording.lock().unwrap() = Some(Recording {
        samples,
        rate: 16_000,
    });

    create_voice(
        &fx.env(gradium::Config::for_fake(&fake)),
        &slot,
        request(CloneSource::Recording),
        &noop,
    )
    .await
    .expect("Aufnahme genuegt");
    assert!(
        fx.recording.lock().unwrap().is_none(),
        "die Aufnahme wird nach dem Erfolg verworfen"
    );
    let post = fake
        .requests()
        .into_iter()
        .find(|r| r.method == "POST")
        .unwrap();
    assert!(
        (256_000..260_000).contains(&post.body.len()),
        "8 s waren da: {}",
        post.body.len()
    );
}

// ---- Fehler von Gradium ------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fehler_von_gradium_aendern_lokal_nichts() {
    use PhononVoiceErrorKind as K;
    let cases: Vec<(u16, K)> = vec![
        (401, K::Unauthorized),
        (422, K::BadAudio),
        (429, K::QuotaOrRate),
        (503, K::Server),
    ];
    for (status, kind) in cases {
        let fx = Fx::new();
        let fake = FakeServer::start(move |_| {
            Reply::json(
                status,
                json!({"detail": format!("Fehler mit {KEY} im Text")}),
            )
        })
        .await;
        let slot = JobSlot::new();
        let src = CloneSource::File {
            path: fx.write_wav("q.wav", 12.0, 16_000, 0.5),
        };
        let err = create_voice(
            &fx.env(gradium::Config::for_fake(&fake)),
            &slot,
            request(src),
            &noop,
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind, kind, "{status}");
        assert!(
            !err.message.contains(KEY),
            "Schluessel in der Meldung: {err}"
        );
        assert!(!slot.is_running());
        fx.assert_untouched();
    }
    // Nicht erreichbar.
    let fx = Fx::new();
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let cfg = gradium::Config {
        base: reqwest::Url::parse(&format!("http://127.0.0.1:{closed}/api")).unwrap(),
        origin: crate::managers::model::download::Origin::plain_http("127.0.0.1", closed),
        request_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(10),
        max_polls: 3,
        https_only: false,
    };
    let src = CloneSource::File {
        path: fx.write_wav("q.wav", 12.0, 16_000, 0.5),
    };
    let err = create_voice(&fx.env(cfg), &JobSlot::new(), request(src), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::Network);
    fx.assert_untouched();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eine_ungueltige_stimmdatei_von_gradium_wird_nicht_gespeichert() {
    let fx = Fx::new();
    let fake = FakeServer::start(gradium_ok(b"das ist keine stimme".to_vec())).await;
    let src = CloneSource::File {
        path: fx.write_wav("q.wav", 12.0, 16_000, 0.5),
    };
    let err = create_voice(
        &fx.env(gradium::Config::for_fake(&fake)),
        &JobSlot::new(),
        request(src),
        &noop,
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::BadResponse);
    assert!(
        err.message.contains("vox_9"),
        "die Stimme im Konto wird genannt: {err}"
    );
    fx.assert_untouched();
}

// ---- Auftragssperre, Abbruch ------------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ein_zweiter_auftrag_waehrend_des_ersten_ist_busy() {
    let fx = Fx::new();
    let blob = sample_voice_bytes(3);
    let fake = FakeServer::start(move |req| {
        if req.method == "POST" {
            Reply::delayed(
                Duration::from_millis(600),
                Reply::json(200, json!({"uid": "vox_9"})),
            )
        } else {
            gradium_ok(blob.clone())(req)
        }
    })
    .await;
    let slot = JobSlot::new();
    let src = CloneSource::File {
        path: fx.write_wav("q.wav", 12.0, 16_000, 0.5),
    };
    let mut other = request(src.clone());
    other.name = "Zweite".into();
    let env = fx.env(gradium::Config::for_fake(&fake));
    let a = create_voice(&env, &slot, request(src), &noop);
    let b = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        create_voice(&env, &slot, other, &noop).await
    };
    let (ra, rb) = tokio::join!(a, b);
    assert!(ra.is_ok(), "{ra:?}");
    assert_eq!(rb.unwrap_err().kind, PhononVoiceErrorKind::Busy);
    assert_eq!(
        fake.requests()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
    assert!(!slot.is_running(), "danach ist der Platz wieder frei");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abbruch_speichert_nichts_und_gibt_den_platz_frei() {
    let fx = Fx::new();
    let fake = FakeServer::start(|req| match req.method.as_str() {
        "POST" => Reply::json(200, json!({"uid": "vox_9"})),
        _ => Reply::json(200, json!({"found": false})),
    })
    .await;
    let mut cfg = gradium::Config::for_fake(&fake);
    cfg.max_polls = 10_000;
    cfg.poll_interval = Duration::from_millis(20);
    let slot = JobSlot::new();
    let src = CloneSource::File {
        path: fx.write_wav("q.wav", 12.0, 16_000, 0.5),
    };
    let env = fx.env(cfg);
    let job = create_voice(&env, &slot, request(src), &noop);
    let stopper = async {
        tokio::time::sleep(Duration::from_millis(250)).await;
        slot.cancel();
    };
    let t = Instant::now();
    let (res, ()) = tokio::join!(job, stopper);
    assert_eq!(res.unwrap_err().kind, PhononVoiceErrorKind::Cancelled);
    assert!(t.elapsed() < Duration::from_secs(5));
    assert!(!slot.is_running());
    fx.assert_untouched();
}

#[test]
fn der_platz_ist_atomar_und_wird_auch_nach_einem_panic_frei() {
    let slot = JobSlot::new();
    let first = slot.claim().expect("frei");
    assert!(slot.is_running());
    assert!(slot.claim().is_none(), "ein zweiter Zugriff scheitert");
    slot.cancel();
    assert!(
        first.cancel.load(Ordering::SeqCst),
        "Abbruch setzt die Marke des laufenden Auftrags"
    );
    drop(first);
    assert!(!slot.is_running());
    // Ohne laufenden Auftrag tut Abbrechen nichts.
    slot.cancel();
    let second = slot.claim().expect("wieder frei");
    assert!(
        !second.cancel.load(Ordering::SeqCst),
        "frische Marke je Auftrag"
    );
    drop(second);

    // Panic im Auftrag.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _g = slot.claim().unwrap();
        panic!("Auftrag stuerzt ab");
    }));
    assert!(result.is_err());
    assert!(
        !slot.is_running(),
        "der Wächter gibt den Platz beim Aufraeumen frei"
    );
    assert!(slot.claim().is_some());
}

#[test]
fn nebenlaeufige_zugriffe_auf_den_platz_ergeben_genau_einen_besitzer() {
    let slot = Arc::new(JobSlot::new());
    let winners = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let (slot, winners, barrier) = (slot.clone(), winners.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                if let Some(g) = slot.claim() {
                    winners.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(100));
                    drop(g);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(winners.load(Ordering::SeqCst), 1);
}

/// Der Abbruch erreicht auch das Dekodieren (ffmpeg): ein schon gesetzter Abbruch startet gar keinen
/// Prozess und meldet `cancelled`, nicht „nicht dekodierbar“.
#[test]
fn der_abbruch_erreicht_auch_das_dekodieren() {
    let fx = Fx::new();
    let junk = fx.tmp.path().join("hoerbuch.mp3");
    std::fs::write(&junk, b"kein echtes mp3").unwrap();
    let env = fx.env(gradium::Config::production());
    let source = CloneSource::File {
        path: junk.to_string_lossy().to_string(),
    };
    let cancelled = AtomicBool::new(true);
    let err = resolve_clip(&env, &source, 0.0, &cancelled).unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::Cancelled);
    // Ohne Abbruch ist es eine unbrauchbare Quelle (mit oder ohne ffmpeg).
    let err = resolve_clip(&env, &source, 0.0, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::BadSource);
}

// ---- Fehlerabbildung ------------------------------------------------------------------------------------------------------------

#[test]
fn fehler_werden_auf_die_klassen_der_oberflaeche_abgebildet() {
    use PhononVoiceErrorKind as K;
    let v = |e: VoiceError| PhononVoiceError::from(e).kind;
    assert_eq!(v(VoiceError::InvalidName("x".into())), K::InvalidName);
    assert_eq!(v(VoiceError::Exists { id: "a".into() }), K::Exists);
    assert_eq!(
        v(VoiceError::TakenByModel { id: "a".into() }),
        K::TakenByModel
    );
    assert_eq!(v(VoiceError::InvalidFile("x".into())), K::InvalidFile);
    assert_eq!(v(VoiceError::NotOwn("a".into())), K::NotOwn);
    assert_eq!(v(VoiceError::TooMany), K::TooMany);
    assert_eq!(v(VoiceError::Io("x".into())), K::Io);
    let g = |kind| {
        PhononVoiceError::from(GradiumError {
            kind,
            message: "m".into(),
        })
        .kind
    };
    assert_eq!(g(GradiumErrorKind::Unauthorized), K::Unauthorized);
    assert_eq!(g(GradiumErrorKind::QuotaOrRate), K::QuotaOrRate);
    assert_eq!(g(GradiumErrorKind::BadAudio), K::BadAudio);
    assert_eq!(g(GradiumErrorKind::Server), K::Server);
    assert_eq!(g(GradiumErrorKind::Timeout), K::Timeout);
    assert_eq!(g(GradiumErrorKind::Network), K::Network);
    assert_eq!(g(GradiumErrorKind::BadResponse), K::BadResponse);
    assert_eq!(g(GradiumErrorKind::Cancelled), K::Cancelled);
    // Und als JSON fuer die Oberflaeche: snake_case.
    let json = serde_json::to_string(&PhononVoiceError::new(K::ConsentRequired, "m")).unwrap();
    assert!(json.contains("\"consent_required\""), "{json}");
}

// ---- Schluesselspeicher -------------------------------------------------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn der_schluessel_liegt_verschluesselt_und_nur_im_geheimnisspeicher() {
    let dir = TempDir::new().unwrap();
    assert!(!key_status(dir.path()).present);

    save_key(dir.path(), &format!("  {KEY}  ")).unwrap();
    let file = dir.path().join("int-tts-gradium-apikey.bin");
    assert!(file.is_file());
    let raw = std::fs::read(&file).unwrap();
    assert!(
        !raw.windows(KEY.len()).any(|w| w == KEY.as_bytes()),
        "Klartext in der Datei"
    );
    let status = key_status(dir.path());
    assert!(status.present && status.supported && status.problem.is_none());
    assert_eq!(
        load_key(dir.path()).unwrap().unwrap().expose_for_test(),
        KEY
    );
    // Keine andere Datei im Ordner enthaelt ihn.
    for e in std::fs::read_dir(dir.path()).unwrap().flatten() {
        let bytes = std::fs::read(e.path()).unwrap();
        assert!(!bytes.windows(KEY.len()).any(|w| w == KEY.as_bytes()));
    }

    // Aendern, dann loeschen.
    save_key(dir.path(), "gsk-anderer-schluessel-9999").unwrap();
    assert_eq!(
        load_key(dir.path()).unwrap().unwrap().expose_for_test(),
        "gsk-anderer-schluessel-9999"
    );
    delete_key(dir.path());
    assert!(!file.exists());
    assert!(!key_status(dir.path()).present);
    assert!(load_key(dir.path()).unwrap().is_none());
}

#[test]
fn ungueltige_schluessel_werden_abgelehnt_ohne_etwas_zu_schreiben() {
    let dir = TempDir::new().unwrap();
    let long = "k".repeat(600);
    for bad in [
        "",
        "   ",
        "kurz",
        "mit leerzeichen drin",
        "zeile\nzwei-xxxxxxxx",
        "umlaut-ä-xxxxxxxx",
        long.as_str(),
    ] {
        let err = save_key(dir.path(), bad).unwrap_err();
        assert_eq!(err.kind, PhononVoiceErrorKind::KeyStore, "{bad:?}");
        assert!(!err.message.contains("mit leerzeichen drin"), "{err}");
    }
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "nichts wurde geschrieben"
    );
}

#[cfg(not(windows))]
#[test]
fn ohne_dpapi_wird_der_schluessel_nicht_gespeichert() {
    let dir = TempDir::new().unwrap();
    let err = save_key(dir.path(), KEY).unwrap_err();
    assert_eq!(err.kind, PhononVoiceErrorKind::KeyStore);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "nie Klartext als Ersatz"
    );
    assert!(!key_status(dir.path()).supported);
}

// ---- Nur ueber die Oberflaeche -----------------------------------------------------------------------------------------------------------

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Die Netzaufrufe zu Gradium sind ausschliesslich UI-Aktionen: Agentenbruecke, CLI, MCP, Start
/// und Gesundheitspruefungen kennen weder das Modul noch den Befehl. Ein neuer Verwender faellt
/// hier auf.
#[test]
fn gradium_ist_nur_ueber_die_befehle_der_oberflaeche_erreichbar() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() > 100, "Quellbaum nicht gefunden");
    let rel = |p: &Path| {
        p.strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
    };
    // Wer das Modul oder den Ablauf ansprechen darf.
    let users = [
        "managers/tts/gradium.rs",
        "managers/tts/gradium/tests.rs",
        "managers/tts/phonon_clone.rs",
        "managers/tts/phonon_clone/tests.rs",
        "commands/phonon_voices.rs",
    ];
    // Die Befehle selbst stehen nur in der Registrierung.
    let registration = ["lib.rs"];
    let module_patterns = ["tts::gradium", "gradium::", "phonon_clone::"];
    let command_patterns = [
        "phonon_voice_clone",
        "phonon_gradium_key",
        "phonon_voice_record",
    ];
    for f in &files {
        let name = rel(f);
        let text = std::fs::read_to_string(f).unwrap_or_default();
        if !users.contains(&name.as_str()) {
            for pat in module_patterns {
                assert!(
                    !text.contains(pat),
                    "{name} darf {pat:?} nicht verwenden (nur UI-Aktion)"
                );
            }
        }
        if !users.contains(&name.as_str()) && !registration.contains(&name.as_str()) {
            for pat in command_patterns {
                assert!(
                    !text.contains(pat),
                    "{name} darf {pat:?} nicht verwenden (nur UI-Aktion)"
                );
            }
        }
    }
    // Die Hüllen und der Ablauf fassen die Einstellungen nie an: der Schluessel steht dort nie.
    for name in [
        "commands/phonon_voices.rs",
        "managers/tts/phonon_clone.rs",
        "managers/tts/gradium.rs",
    ] {
        let text = std::fs::read_to_string(src.join(name)).unwrap();
        for pat in ["get_settings", "write_settings", "settings::"] {
            assert!(!text.contains(pat), "{name} darf {pat:?} nicht anfassen");
        }
    }
}
