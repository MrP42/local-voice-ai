use super::*;
use crate::managers::tts::providers::test_http::{FakeServer, Reply, Req};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const KEY: &str = "sk-geheim-1234";

fn client(cfg: Config) -> Client {
    Client::new(cfg, Secret::new(KEY).unwrap()).unwrap()
}

fn wav() -> Vec<u8> {
    b"RIFF\x24\x00\x00\x00WAVEfmt FAKE-WAV-PAYLOAD".to_vec()
}

fn job() -> CloneJob {
    CloneJob {
        name: "Meine Stimme".to_string(),
        language: "de".to_string(),
        wav: wav(),
    }
}

fn flag() -> CancelFlag {
    Arc::new(AtomicBool::new(false))
}

fn noop(_: Phase) {}

/// Keine Meldung darf Schluessel, Adresse oder Rohtext der Verbindung tragen.
fn assert_clean(e: &GradiumError) {
    for bad in [KEY, "127.0.0.1", "http://", "https://", "x-api-key"] {
        assert!(!e.message.contains(bad), "{bad:?} in {:?}", e.message);
    }
    assert!(!e.message.trim().is_empty());
}

const CREATE: &str = "/api/voices/";
const POLL: &str = "/api/voices/embeddings/vox_123/default";
const BLOB: &str = "/api/voices/embeddings/vox_123/default/blob?as_safetensors=1";

/// Gradium wie beschrieben: anlegen, `found` erst nach `found_after` Abfragen, dann der Blob.
fn happy(found_after: usize, blob: Vec<u8>) -> impl Fn(&Req) -> Reply + Send + Sync + 'static {
    let polls = Arc::new(AtomicUsize::new(0));
    move |req| match (req.method.as_str(), req.path.as_str()) {
        ("POST", CREATE) => Reply::json(200, json!({"uid": "vox_123", "error": null})),
        ("GET", POLL) => {
            let n = polls.fetch_add(1, Ordering::SeqCst) + 1;
            Reply::json(200, json!({"found": n > found_after}))
        }
        ("GET", BLOB) => Reply::audio(blob.clone()),
        _ => Reply::text(404, "unbekannt"),
    }
}

// ---- Erfolg ------------------------------------------------------------------------------

#[tokio::test]
async fn anlegen_pollen_und_blob_holen() {
    let blob = b"SAFETENSORS-BYTES".to_vec();
    let fake = FakeServer::start(happy(2, blob.clone())).await;
    let phases = Mutex::new(Vec::new());
    let got = client(Config::for_fake(&fake))
        .clone_voice(&job(), &flag(), &|p| phases.lock().unwrap().push(p))
        .await
        .expect("Klonen gelingt");
    assert_eq!(got.voice_id, "vox_123");
    assert_eq!(got.safetensors, blob);
    assert_eq!(
        *phases.lock().unwrap(),
        vec![Phase::Uploading, Phase::Waiting, Phase::Downloading]
    );

    let reqs = fake.requests();
    let order: Vec<String> = reqs
        .iter()
        .map(|r| format!("{} {}", r.method, r.path))
        .collect();
    assert_eq!(
        order,
        vec![
            format!("POST {CREATE}"),
            format!("GET {POLL}"),
            format!("GET {POLL}"),
            format!("GET {POLL}"),
            format!("GET {BLOB}"),
        ],
        "zwei Mal nicht fertig, dann fertig, dann der Blob"
    );
    // Jede Anfrage traegt den Schluessel im Kopf x-api-key (und sonst nirgends).
    for r in &reqs {
        assert_eq!(r.headers.get("x-api-key").map(String::as_str), Some(KEY));
        assert!(!r.path.contains(KEY));
        assert!(!r.headers.contains_key("authorization"));
    }
}

#[tokio::test]
async fn der_upload_enthaelt_nur_das_ausgeschnittene_stueck_und_neutrale_namen() {
    let fake = FakeServer::start(happy(0, b"X".to_vec())).await;
    client(Config::for_fake(&fake))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap();
    let post = fake
        .requests()
        .into_iter()
        .find(|r| r.method == "POST")
        .unwrap();
    let ct = post.headers.get("content-type").unwrap();
    let boundary = ct.strip_prefix("multipart/form-data; boundary=").expect(ct);
    let text = String::from_utf8_lossy(&post.body).to_string();
    assert!(text.contains("name=\"name\"\r\n\r\nMeine Stimme\r\n"));
    assert!(text.contains("name=\"language\"\r\n\r\nde\r\n"));
    assert!(text.contains("name=\"start_s\"\r\n\r\n0.0\r\n"));
    assert!(text.contains(
        "name=\"audio_file\"; filename=\"voice.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFF"
    ));
    assert!(post
        .body
        .ends_with(format!("\r\n--{boundary}--\r\n").as_bytes()));
    // Die Nutzlast ist unveraendert dabei.
    assert!(post
        .body
        .windows(wav().len())
        .any(|w| w == wav().as_slice()));
}

#[test]
fn multipart_hat_die_erwartete_gestalt() {
    let body = build_multipart(
        "BOUNDARY",
        &[("name", "A B"), ("language", "de")],
        "voice.wav",
        "audio/wav",
        b"\x00\x01DATA",
    );
    let expect = [
        &b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nA B\r\n"[..],
        b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\nde\r\n",
        b"--BOUNDARY\r\nContent-Disposition: form-data; name=\"audio_file\"; filename=\"voice.wav\"\r\nContent-Type: audio/wav\r\n\r\n",
        b"\x00\x01DATA",
        b"\r\n--BOUNDARY--\r\n",
    ]
    .concat();
    assert_eq!(body, expect);
}

#[test]
fn voice_kennungen_sind_nur_unschaedliche_zeichen() {
    for ok in ["vox_123", "2A2fFf8SkDEJ8nzx", "a-b_c", &"x".repeat(64)] {
        assert!(valid_voice_id(ok), "{ok}");
    }
    for bad in [
        "",
        "../x",
        "a/b",
        "a?b",
        "a#b",
        "a b",
        "a%2fb",
        "ä",
        "a\nb",
        &"x".repeat(65),
        "..",
    ] {
        assert!(!valid_voice_id(bad), "{bad:?}");
    }
}

// ---- Schluessel nur an den Ursprung ---------------------------------------------------------------

#[tokio::test]
async fn weiterleitung_zu_einem_anderen_ursprung_bekommt_den_schluessel_nie() {
    // B ist ein anderer Ursprung (anderer Port), z. B. ein CDN.
    let other = FakeServer::start(|_| Reply::audio(b"VOM-CDN".to_vec())).await;
    let other_root = other.root.clone();
    let api = FakeServer::start(move |req| match (req.method.as_str(), req.path.as_str()) {
        ("POST", CREATE) => Reply::json(200, json!({"uid": "vox_123"})),
        ("GET", POLL) => Reply::json(200, json!({"found": true})),
        ("GET", BLOB) => Reply::redirect(&format!("{other_root}/signiert/abc")),
        _ => Reply::text(404, ""),
    })
    .await;
    let got = client(Config::for_fake(&api))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .expect("das CDN liefert den Blob");
    assert_eq!(got.safetensors, b"VOM-CDN");
    let at_other = other.requests();
    assert_eq!(at_other.len(), 1);
    assert_eq!(at_other[0].path, "/signiert/abc");
    assert!(
        !at_other[0].headers.contains_key("x-api-key")
            && !at_other[0].headers.contains_key("authorization"),
        "der Schluessel darf den Ursprung nie verlassen: {:?}",
        at_other[0].headers
    );
    assert!(api
        .requests()
        .iter()
        .all(|r| r.headers.get("x-api-key").map(String::as_str) == Some(KEY)));
}

#[tokio::test]
async fn weiterleitung_innerhalb_des_ursprungs_behaelt_den_schluessel() {
    let api = FakeServer::start(|req| match (req.method.as_str(), req.path.as_str()) {
        ("POST", CREATE) => Reply::json(200, json!({"uid": "vox_123"})),
        ("GET", POLL) => Reply::json(200, json!({"found": true})),
        ("GET", BLOB) => Reply::redirect("/api/blob-neu"),
        ("GET", "/api/blob-neu") => Reply::audio(b"NEU".to_vec()),
        _ => Reply::text(404, ""),
    })
    .await;
    let got = client(Config::for_fake(&api))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap();
    assert_eq!(got.safetensors, b"NEU");
    let last = api.requests().pop().unwrap();
    assert_eq!(last.path, "/api/blob-neu");
    assert_eq!(last.headers.get("x-api-key").map(String::as_str), Some(KEY));
}

#[tokio::test]
async fn ein_post_wird_nie_weitergeleitet() {
    let other = FakeServer::start(|_| Reply::text(200, "nie")).await;
    let other_root = other.root.clone();
    let api = FakeServer::start(move |_| Reply::redirect(&format!("{other_root}/x"))).await;
    let err = client(Config::for_fake(&api))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::BadResponse);
    assert_clean(&err);
    assert!(
        other.requests().is_empty(),
        "Aufnahme und Schluessel gehen nirgends hin"
    );
}

#[test]
fn die_produktion_kennt_genau_einen_ursprung_und_nur_https() {
    let cfg = Config::production();
    assert_eq!(cfg.base.as_str(), "https://api.gradium.ai/api");
    assert_eq!(cfg.origin, Origin::gradium());
    assert!(cfg.https_only);
    // Keine Verwechslung ueber Unterdomain, Port oder Schema.
    for other in [
        "https://api.gradium.ai.evil.example/api",
        "https://evil.example/api.gradium.ai",
        "https://gradium.ai/api",
        "https://api.gradium.ai:8443/api",
        "http://api.gradium.ai/api",
    ] {
        let url = Url::parse(other).unwrap();
        assert_ne!(Origin::of(&url), Some(Origin::gradium()), "{other}");
    }
    let ok = Url::parse("https://api.gradium.ai/api/voices/").unwrap();
    assert_eq!(Origin::of(&ok), Some(Origin::gradium()));
}

#[test]
fn der_schluessel_steht_in_keinem_debug() {
    let c = client(Config::production());
    let shown = format!("{c:?} {:?}", Secret::new(KEY).unwrap());
    assert!(!shown.contains(KEY), "{shown}");
}

// ---- Fehlerbilder ---------------------------------------------------------------------------------------

#[tokio::test]
async fn abgelehnter_schluessel_ist_unauthorized_an_jeder_stelle() {
    // Beim Anlegen.
    let fake = FakeServer::start(|_| {
        Reply::json(
            401,
            json!({"detail": format!("Ungueltiger Schluessel {KEY}")}),
        )
    })
    .await;
    let err = client(Config::for_fake(&fake))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::Unauthorized);
    assert!(err.message.contains("Schlüssel"), "{err}");
    assert_clean(&err); // der Server hat den Schluessel zurueckgeschickt: er wird entfernt

    // Beim Pollen (Schluessel widerrufen) und beim Blob (403).
    for (path, status) in [(POLL, 401u16), (BLOB, 403)] {
        let fake = FakeServer::start(move |req| match (req.method.as_str(), req.path.as_str()) {
            ("POST", CREATE) => Reply::json(200, json!({"uid": "vox_123"})),
            ("GET", POLL) if path == POLL => Reply::text(status, "no"),
            ("GET", POLL) => Reply::json(200, json!({"found": true})),
            ("GET", BLOB) => Reply::text(status, "no"),
            _ => Reply::text(404, ""),
        })
        .await;
        let err = client(Config::for_fake(&fake))
            .clone_voice(&job(), &flag(), &noop)
            .await
            .unwrap_err();
        assert_eq!(err.kind, GradiumErrorKind::Unauthorized, "{path}");
        assert_clean(&err);
    }
}

#[tokio::test]
async fn ungeeignetes_audio_nennt_die_antwort_des_servers() {
    for (status, body) in [
        (422u16, json!({"detail": "Audio too short"}).to_string()),
        (
            400,
            json!({"detail": [{"msg": "language not supported"}]}).to_string(),
        ),
        (413, "Request Entity Too Large".to_string()),
        (415, "plain".to_string()),
    ] {
        let b = body.clone();
        let fake = FakeServer::start(move |_| Reply::text(status, &b)).await;
        let err = client(Config::for_fake(&fake))
            .clone_voice(&job(), &flag(), &noop)
            .await
            .unwrap_err();
        assert_eq!(err.kind, GradiumErrorKind::BadAudio, "{status}");
        assert!(err.message.contains(&status.to_string()), "{err}");
        assert_clean(&err);
    }
    // Der Hinweis des Servers ist dabei.
    let fake = FakeServer::start(|_| Reply::json(422, json!({"detail": "Audio too short"}))).await;
    let err = client(Config::for_fake(&fake))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert!(err.message.contains("Audio too short"), "{err}");
}

#[tokio::test]
async fn ein_langer_oder_steuerzeichenhaltiger_servertext_wird_gekuerzt_und_bereinigt() {
    let long = format!("{}\u{7}\u{1b}[31mrot\n{KEY}", "x".repeat(2000));
    let fake = FakeServer::start(move |_| Reply::json(422, json!({"detail": long.clone()}))).await;
    let err = client(Config::for_fake(&fake))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert!(err.message.chars().count() < 600, "{}", err.message.len());
    assert!(
        !err.message.chars().any(|c| c.is_control()),
        "{:?}",
        err.message
    );
    assert_clean(&err);
}

#[tokio::test]
async fn kontingent_tempolimit_und_serverfehler_haben_eigene_klassen() {
    for (status, kind) in [
        (429u16, GradiumErrorKind::QuotaOrRate),
        (402, GradiumErrorKind::QuotaOrRate),
        (500, GradiumErrorKind::Server),
        (503, GradiumErrorKind::Server),
    ] {
        let fake = FakeServer::start(move |_| Reply::text(status, "nein")).await;
        let err = client(Config::for_fake(&fake))
            .clone_voice(&job(), &flag(), &noop)
            .await
            .unwrap_err();
        assert_eq!(err.kind, kind, "{status}");
        assert!(err.message.contains(&status.to_string()), "{err}");
        assert_clean(&err);
    }
}

#[tokio::test]
async fn ein_haengender_server_endet_mit_zeitueberschreitung() {
    let fake = FakeServer::start(|_| Reply::Hang).await;
    let mut cfg = Config::for_fake(&fake);
    cfg.request_timeout = Duration::from_millis(300);
    let t = Instant::now();
    let err = client(cfg)
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::Timeout);
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert_clean(&err);
}

#[tokio::test]
async fn das_pollen_gibt_nach_den_versuchen_auf_und_nennt_die_stimme_im_konto() {
    let polls = Arc::new(AtomicUsize::new(0));
    let p = polls.clone();
    let fake = FakeServer::start(move |req| match (req.method.as_str(), req.path.as_str()) {
        ("POST", CREATE) => Reply::json(200, json!({"uid": "vox_123"})),
        ("GET", POLL) => {
            p.fetch_add(1, Ordering::SeqCst);
            Reply::json(200, json!({"found": false}))
        }
        _ => Reply::text(404, ""),
    })
    .await;
    let mut cfg = Config::for_fake(&fake);
    cfg.max_polls = 4;
    let err = client(cfg)
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::Timeout);
    assert!(err.message.contains("vox_123"), "{err}");
    assert_eq!(
        polls.load(Ordering::SeqCst),
        4,
        "genau so viele Versuche wie erlaubt"
    );
    assert_eq!(fake.count(BLOB), 0, "ohne found kein Blob");
    assert_clean(&err);
}

#[tokio::test]
async fn abbruch_beendet_das_pollen_und_haengende_anfragen() {
    // Beim Pollen.
    let fake = FakeServer::start(|req| match req.method.as_str() {
        "POST" => Reply::json(200, json!({"uid": "vox_123"})),
        _ => Reply::json(200, json!({"found": false})),
    })
    .await;
    let mut cfg = Config::for_fake(&fake);
    cfg.max_polls = 10_000;
    cfg.poll_interval = Duration::from_millis(20);
    let cancel = flag();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        c2.store(true, Ordering::SeqCst);
    });
    let t = Instant::now();
    let err = client(cfg)
        .clone_voice(&job(), &cancel, &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::Cancelled);
    assert!(t.elapsed() < Duration::from_secs(3));
    let polls_at_stop = fake.requests().len();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        fake.requests().len(),
        polls_at_stop,
        "nach dem Abbruch kommt keine Anfrage mehr"
    );

    // Waehrend ein Server nicht antwortet.
    let hang = FakeServer::start(|_| Reply::Hang).await;
    let mut cfg = Config::for_fake(&hang);
    cfg.request_timeout = Duration::from_secs(60);
    let cancel = flag();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        c2.store(true, Ordering::SeqCst);
    });
    let t = Instant::now();
    let err = client(cfg)
        .clone_voice(&job(), &cancel, &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::Cancelled);
    assert!(t.elapsed() < Duration::from_secs(3));

    // Schon vor dem Start abgebrochen: keine einzige Anfrage.
    let fresh = FakeServer::start(happy(0, b"X".to_vec())).await;
    let already = Arc::new(AtomicBool::new(true));
    let err = client(Config::for_fake(&fresh))
        .clone_voice(&job(), &already, &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::Cancelled);
    assert!(fresh.requests().is_empty());
}

#[tokio::test]
async fn abgerissene_verbindung_und_fehlender_server_sind_netzfehler() {
    let fake = FakeServer::start(|_| Reply::Close).await;
    let err = client(Config::for_fake(&fake))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::Network);
    assert_clean(&err);

    // Nichts hoert zu.
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let cfg = Config {
        base: Url::parse(&format!("http://127.0.0.1:{closed}/api")).unwrap(),
        origin: Origin::plain_http("127.0.0.1", closed),
        request_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(10),
        max_polls: 3,
        https_only: false,
    };
    let err = client(cfg)
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::Network);
    assert_clean(&err);
}

#[tokio::test]
async fn eine_kennung_mit_pfadzeichen_wird_nie_in_eine_adresse_gesetzt() {
    for uid in ["../../x", "a/b", "a?b=1", "", "a b", &"x".repeat(65)] {
        let u = uid.to_string();
        let fake = FakeServer::start(move |_| Reply::json(200, json!({"uid": u.clone()}))).await;
        let err = client(Config::for_fake(&fake))
            .clone_voice(&job(), &flag(), &noop)
            .await
            .unwrap_err();
        assert_eq!(err.kind, GradiumErrorKind::BadResponse, "{uid:?}");
        assert_eq!(
            fake.requests().len(),
            1,
            "nach der Anlage keine weitere Anfrage ({uid:?})"
        );
        assert_clean(&err);
    }
}

#[tokio::test]
async fn unbrauchbare_antworten_sind_bad_response() {
    // Keine Kennung: der Grund des Servers wird genannt (wie clone-voice.py).
    let fake = FakeServer::start(|_| Reply::json(200, json!({"error": "quota exhausted"}))).await;
    let err = client(Config::for_fake(&fake))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::BadResponse);
    assert!(err.message.contains("quota exhausted"), "{err}");
    // Kein JSON.
    let fake = FakeServer::start(|_| Reply::text(200, "<html>Wartung</html>")).await;
    let err = client(Config::for_fake(&fake))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::BadResponse);
    assert_clean(&err);
    // Poll-Antwort ohne `found` zaehlt als „noch nicht fertig“.
    let fake = FakeServer::start(|req| match req.method.as_str() {
        "POST" => Reply::json(200, json!({"uid": "vox_123"})),
        _ => Reply::json(200, json!({"irgendwas": 1})),
    })
    .await;
    let mut cfg = Config::for_fake(&fake);
    cfg.max_polls = 2;
    let err = client(cfg)
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::Timeout);
}

#[tokio::test]
async fn eine_zu_grosse_antwort_wird_nicht_geladen() {
    let fake = FakeServer::start(happy(0, vec![0u8; MAX_BLOB_BYTES + 1])).await;
    let err = client(Config::for_fake(&fake))
        .clone_voice(&job(), &flag(), &noop)
        .await
        .unwrap_err();
    assert_eq!(err.kind, GradiumErrorKind::BadResponse);
    assert_clean(&err);
}
