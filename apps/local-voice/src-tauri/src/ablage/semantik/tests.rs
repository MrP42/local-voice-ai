//! D6: Verschiebung, Hybrid mit `FakeEmbedder` über die echte Ablage (Import, Extraktor im Unterprozess,
//! Index), Rückfall auf lexikalisch, Papierkorb, und das Eval-Tor (mit echtem Modell ignoriert).
//! Nur synthetische Inhalte (`eval_korpus.json`, Domains `example.*`).

use super::modell::{Art, E5};
use super::zerleger::Zerleger;
use super::*;
use crate::ablage::index;
use crate::ablage::speicher::{Ablage, AblageArt, AblageImportStand};
use crate::ablage::tests_d2::{kind, TEST_GRENZEN};
use crate::ablage::textfassung;
use std::sync::atomic::AtomicBool;

// ---------------------------------------------------------------------------
// Verschiebung
// ---------------------------------------------------------------------------

fn skalar(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn einheit(winkel: f32) -> Vec<f32> {
    vec![winkel.cos(), winkel.sin()]
}

#[test]
fn verschiebung_bildet_die_schwelle_auf_min_cosine_ab() {
    let s = 0.83f32;
    let q = einheit(0.0);
    for cos in [0.5f32, 0.8, 0.83, 0.9, 1.0] {
        let p = einheit(cos.acos());
        let d = skalar(
            &verschieben(q.clone(), s, Art::Anfrage),
            &verschieben(p, s, Art::Stueck),
        );
        let a2 = (1.0 + wai_retrieval::MIN_COSINE) / (1.0 + s);
        assert!((d - (a2 * cos - (1.0 - a2))).abs() < 1e-5, "{cos}: {d}");
        assert_eq!(
            d >= wai_retrieval::MIN_COSINE - 1e-6,
            cos >= s - 1e-6,
            "{cos}: {d}"
        );
    }
    // Anfragevektor bleibt normiert, die Reihenfolge der Stücke bleibt gleich.
    let v = verschieben(q, s, Art::Anfrage);
    assert!((skalar(&v, &v) - 1.0).abs() < 1e-5);
}

#[test]
fn kennungen_trennen_lexikalisch_und_vektorraeume() {
    use wai_retrieval::Embedder;
    assert_eq!(AblageEinbettung::Keine.id(), "keiner");
    assert_ne!(
        AblageEinbettung::test(0.3).id(),
        AblageEinbettung::test(0.4).id()
    );
    assert!(!AblageEinbettung::Keine.ist_semantisch());
}

#[test]
fn modell_ist_im_katalog_mit_lizenz_und_pruefsummen() {
    let e = katalog().expect("Katalogeintrag");
    assert_eq!(e.license.as_deref(), Some("MIT"));
    let namen: Vec<&str> = e.files.iter().map(|f| f.filename.as_str()).collect();
    assert_eq!(namen, [DATEI_MODELL, DATEI_ZERLEGER]);
    // Die Revision steht nur im Katalog; Dateien und Kennung leiten sich daraus ab.
    let rev = e.revision.clone().expect("gepinnte Revision");
    assert_eq!(rev.len(), 40);
    for f in &e.files {
        assert!(f.sha256.as_deref().is_some_and(|s| s.len() == 64));
        assert!(f.url.contains(&format!("/resolve/{rev}/")));
    }
    assert_eq!(modell_kennung(), format!("e5-small@{}", &rev[..8]));
    let root = std::path::Path::new("/m");
    assert!(modell_ordner(root).starts_with(root.join(MODELL_ORDNER)));
    assert!(!modell_bereit(root));
}

#[test]
fn ohne_modell_bleibt_die_einbettung_lexikalisch() {
    let d = tempfile::tempdir().unwrap();
    // Eigener Prozesszustand: andere Tests nutzen ihn nicht (sie reichen die Einbettung herein).
    einrichten(d.path(), true);
    assert!(!aktuell().ist_semantisch());
    einrichten(d.path(), false);
    assert!(!aktuell().ist_semantisch());

    // Modell entfernen: Dateien und halbe Downloads weg, leerer Ordner weg, fremde Dateien bleiben.
    // (Im selben Test, weil der Prozesszustand geteilt ist.)
    let ordner = modell_ordner(d.path());
    std::fs::create_dir_all(&ordner).unwrap();
    for n in [DATEI_MODELL, DATEI_ZERLEGER, "model.onnx.partial"] {
        std::fs::write(ordner.join(n), b"x").unwrap();
    }
    assert!(modell_bereit(d.path()));
    assert!(reste_vorhanden(d.path()));
    let nachbar = d.path().join("anderes-modell.bin");
    std::fs::write(&nachbar, b"y").unwrap();
    modell_entfernen(d.path()).unwrap();
    assert!(!modell_bereit(d.path()));
    assert!(!reste_vorhanden(d.path()));
    assert!(!ordner.exists());
    assert!(nachbar.is_file());
    // Zweimal ist kein Fehler.
    modell_entfernen(d.path()).unwrap();
}

// ---------------------------------------------------------------------------
// Korpus und Messung
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct Korpus {
    dokumente: Vec<Dok>,
    fragen: Vec<Frage>,
}

#[derive(serde::Deserialize)]
struct Dok {
    name: String,
    absaetze: Vec<String>,
}

#[derive(serde::Deserialize)]
struct Frage {
    frage: String,
    dok: Option<usize>,
    absatz: Option<usize>,
}

fn korpus() -> Korpus {
    serde_json::from_str(include_str!("eval_korpus.json")).unwrap()
}

struct Aufbau {
    _d: tempfile::TempDir,
    a: Ablage,
    dats: Vec<String>,
}

/// Legt alle Korpusdokumente als TXT ab (ein Absatz je Zeile, Leerzeile dazwischen: Absatz `i` hat den
/// Anker `zeile=2i+1`) und erzeugt die Textfassungen im echten Unterprozess.
fn aufbauen(k: &Korpus) -> Aufbau {
    let d = tempfile::tempdir().unwrap();
    let a = Ablage::neu(d.path().join("ablage"));
    let ordner = a.ansicht().unwrap().ordner[0].id.clone();
    let mut dats = Vec::new();
    for dok in &k.dokumente {
        let q = d.path().join("q").join(&dok.name);
        std::fs::create_dir_all(q.parent().unwrap()).unwrap();
        std::fs::write(&q, dok.absaetze.join("\n\n")).unwrap();
        let z = a.importieren(&q, &ordner, None).unwrap();
        assert_eq!(z[0].stand, AblageImportStand::Abgelegt);
        dats.push(z[0].dat.clone().unwrap());
    }
    let l = textfassung::lauf(
        &a,
        &kind(None),
        TEST_GRENZEN,
        &AtomicBool::new(false),
        &|| {},
    )
    .unwrap();
    assert_eq!(l.ok as usize, k.dokumente.len());
    Aufbau { _d: d, a, dats }
}

#[derive(Debug, Default, Clone, Copy)]
struct Messung {
    beantwortbar: u32,
    unbeantwortbar: u32,
    recall5: f64,
    beleg: f64,
    kein_beleg: f64,
    /// Nur im Bericht (`{:?}`): erste Fundstelle ist die erwartete.
    #[allow(dead_code)]
    top1: f64,
}

impl Messung {
    fn tor(&self) -> bool {
        self.recall5 >= 0.9 && self.beleg >= 0.95 && self.kein_beleg >= 0.9
    }
}

/// Recall@5: erwartete Datei unter den ersten fünf Treffern. Belegkorrektheit: bei gefundener Datei
/// zeigt deren erste Fundstelle auf den erwarteten Absatz. Unbeantwortbar: keine Inhaltstreffer.
fn messen(k: &Korpus, b: &Aufbau, emb: &AblageEinbettung) -> Messung {
    let (mut n, mut nu, mut r5, mut bel, mut leer, mut t1) = (0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
    for f in &k.fragen {
        let t = index::suchen_mit(&b.a, &f.frage, emb).unwrap();
        let (Some(dok), Some(absatz)) = (f.dok, f.absatz) else {
            nu += 1;
            leer += u32::from(t.is_empty());
            continue;
        };
        n += 1;
        let anker = format!("zeile={}", 2 * absatz + 1);
        if let Some(tr) = t.iter().take(5).find(|tr| tr.dat == b.dats[dok]) {
            r5 += 1;
            bel += u32::from(tr.stellen.first().is_some_and(|s| s.anker == anker));
        }
        t1 += u32::from(
            t.first()
                .is_some_and(|tr| tr.dat == b.dats[dok] && tr.stellen[0].anker == anker),
        );
    }
    Messung {
        beantwortbar: n,
        unbeantwortbar: nu,
        recall5: f64::from(r5) / f64::from(n.max(1)),
        beleg: f64::from(bel) / f64::from(r5.max(1)),
        kein_beleg: f64::from(leer) / f64::from(nu.max(1)),
        top1: f64::from(t1) / f64::from(n.max(1)),
    }
}

fn vektoren(b: &Aufbau) -> (i64, i64) {
    let c = rusqlite::Connection::open(b.a.dir().join(index::INDEX)).unwrap();
    let z = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    (
        z("SELECT COUNT(*) FROM chunks"),
        z("SELECT COUNT(*) FROM vectors"),
    )
}

// ---------------------------------------------------------------------------
// Hybrid mit FakeEmbedder
// ---------------------------------------------------------------------------

#[test]
fn hybrid_mit_fake_nachziehen_rueckfall_und_papierkorb() {
    let k = korpus();
    let b = aufbauen(&k);
    let lex = AblageEinbettung::Keine;
    let fake = AblageEinbettung::test(0.5);

    // D2-Stand: lexikalisch, keine Vektoren.
    let vorher = messen(&k, &b, &lex);
    assert_eq!((vorher.beantwortbar, vorher.unbeantwortbar), (50, 20));
    assert!(vorher.recall5 > 0.5, "{vorher:?}");
    assert_eq!(vektoren(&b).1, 0);
    assert_eq!(index::vektor_stand(&b.a, &lex).unwrap(), (false, 0));

    // Einschalten: der Index liegt im Raum „keiner“ → nachziehen übernimmt den Vektorraum und bettet
    // alle Stücke schrittweise ein (64 Stücke, Schritte zu 32).
    assert!(index::vektor_stand(&b.a, &fake).unwrap().0);
    let n = index::vektoren_nachziehen(&b.a, &fake).unwrap();
    let (stuecke, vek) = vektoren(&b);
    assert_eq!((n as i64, vek), (stuecke, stuecke));
    assert_eq!(index::vektor_stand(&b.a, &fake).unwrap(), (false, 0));
    // Zweimal nachziehen ändert nichts.
    assert_eq!(index::vektoren_nachziehen(&b.a, &fake).unwrap(), 0);

    let hybrid = messen(&k, &b, &fake);
    assert_eq!(hybrid.beantwortbar, 50);
    // Der Hash-Behelf kennt keine Semantik: mindestens so gut wie lexikalisch.
    assert!(hybrid.recall5 >= vorher.recall5, "{hybrid:?} {vorher:?}");
    // Eine Anfrage ohne gemeinsames Wort findet nichts (Kandidatenschwelle wirkt).
    assert!(index::suchen_mit(&b.a, "Quokka Zitrone", &fake)
        .unwrap()
        .is_empty());

    // Neue Datei nach dem Einschalten.
    let ordner = b.a.ansicht().unwrap().ordner[0].id.clone();
    let q = b._d.path().join("q").join("Neu.txt");
    std::fs::write(&q, "Der Zebrafink singt im Innenhof.").unwrap();
    let neu = b.a.importieren(&q, &ordner, None).unwrap()[0]
        .dat
        .clone()
        .unwrap();
    textfassung::lauf(
        &b.a,
        &kind(None),
        TEST_GRENZEN,
        &AtomicBool::new(false),
        &|| {},
    )
    .unwrap();
    // Der Textlauf glich hier (Testprozess ohne Modell) lexikalisch ab: das neue Stück fehlt im
    // Vektorraum, bis nachgezogen wird; die Suche bleibt dabei vollständig (lexikalisch).
    assert_eq!(index::vektor_stand(&b.a, &fake).unwrap(), (false, 1));
    // Nur das fehlende Stück wird eingebettet, nicht alle.
    assert_eq!(index::vektoren_nachziehen(&b.a, &fake).unwrap(), 1);
    let (s2, v2) = vektoren(&b);
    assert_eq!(s2, v2);
    assert_eq!(
        index::suchen_mit(&b.a, "zebrafink", &fake).unwrap()[0].dat,
        neu
    );

    // Papierkorb: weder lexikalisch noch über Vektoren.
    b.a.in_papierkorb(AblageArt::Datei, &neu).unwrap();
    index::abgleichen_mit(&b.a, &fake).unwrap();
    // (Der Hash-Behelf kann über Kollisionen andere Dateien treffen; die abgelegte nie mehr.)
    assert!(index::suchen_mit(&b.a, "zebrafink", &fake)
        .unwrap()
        .iter()
        .all(|t| t.dat != neu));
    // Moduswechsel ohne Leeren: der Abfrage-Zwischenspeicher trägt die Kennung der Einbettung im
    // Schlüssel, die zwischengespeicherte Rangfolge der anderen Suchart kommt nicht zurück.
    assert!(index::suchen_mit(&b.a, "zebrafink", &lex)
        .unwrap()
        .is_empty());

    // Ausschalten: sofort wieder lexikalisch mit denselben Treffern wie vorher.
    let danach = messen(&k, &b, &lex);
    assert_eq!(
        (danach.recall5, danach.beleg, danach.kein_beleg),
        (vorher.recall5, vorher.beleg, vorher.kein_beleg)
    );
    // Ein lexikalischer Abgleich löscht keine Vektoren und mischt keine fremden hinein.
    index::abgleichen_mit(&b.a, &lex).unwrap();
    assert_eq!(index::vektor_stand(&b.a, &fake).unwrap(), (false, 0));
}

/// Das Nachziehen sperrt nur schrittweise und lässt einem wartenden Schreiber (Neuaufbau) den
/// Vortritt: Bekommt der Schreiber die Dateisperre, hat seit seiner Anmeldung kein neuer Schritt
/// begonnen (Handshake über die Schrittzahl, keine Zeitgrenze; gilt auch bei nicht fairen Sperren wie
/// unter Windows). Danach endet das Nachziehen trotz Neuaufbau sauber und füllt alle Stücke.
#[test]
fn neuaufbau_waehrend_des_nachziehens_wartet_nur_einen_schritt() {
    let k = korpus();
    let b = aufbauen(&k);
    let langsam = AblageEinbettung::test_langsam(0.5, std::time::Duration::from_millis(200));
    let (stuecke, _) = vektoren(&b);
    // 64 Stücke zu 32: mehrere Schritte.
    assert!(stuecke > 32, "{stuecke}");
    let start = index::schritte_begonnen();
    let a2 = Ablage::neu(b.a.dir().to_path_buf());
    let l2 = langsam.clone();
    let lauf = std::thread::spawn(move || index::vektoren_nachziehen(&a2, &l2));
    // Handshake: ein Schritt läuft.
    while index::schritte_begonnen() == start {
        std::thread::yield_now();
    }
    let (vorher, nachher) = index::dateisperre_uebergabe();
    assert!(
        nachher <= vorher,
        "Schritt nach der Anmeldung: {vorher} → {nachher}"
    );
    // Ein echter Neuaufbau mitten im Lauf: beide enden ohne Fehler.
    index::neu_aufbauen_mit(&b.a, &AblageEinbettung::Keine).unwrap();
    lauf.join().unwrap().unwrap();
    index::vektoren_nachziehen(&b.a, &langsam).unwrap();
    assert_eq!(index::vektor_stand(&b.a, &langsam).unwrap(), (false, 0));
    let (s2, v2) = vektoren(&b);
    assert_eq!(s2, v2);
}

#[test]
fn neuaufbau_mit_vektoren_braucht_kein_nachziehen() {
    let k = korpus();
    let b = aufbauen(&k);
    let fake = AblageEinbettung::test(0.5);
    index::neu_aufbauen_mit(&b.a, &fake).unwrap();
    let (s, v) = vektoren(&b);
    assert_eq!(s, v);
    assert_eq!(index::vektor_stand(&b.a, &fake).unwrap(), (false, 0));
    // Ein anderer Vektorraum (andere Schwelle) verlangt neues Einbetten.
    assert!(
        index::vektor_stand(&b.a, &AblageEinbettung::test(0.4))
            .unwrap()
            .0
    );
}

#[test]
fn vektoren_liegen_nur_im_abgeleiteten_index() {
    let k = korpus();
    let b = aufbauen(&k);
    let fake = AblageEinbettung::test(0.5);
    index::vektoren_nachziehen(&b.a, &fake).unwrap();
    // Journal und Datensätze (die Sync-Quelle) tragen keine Vektoren oder Kennungen des Embedders.
    let journal = std::fs::read_to_string(b.a.dir().join("ablage.json")).unwrap();
    assert!(!journal.contains("fake-hash") && !journal.contains("vectors"));
    for e in walk(b.a.dir()) {
        let name = e.file_name().unwrap().to_string_lossy().to_string();
        if name.starts_with("index.db") {
            continue;
        }
        let inhalt = std::fs::read(&e).unwrap();
        assert!(
            !String::from_utf8_lossy(&inhalt).contains("kand=0.50"),
            "{name}"
        );
    }
}

fn walk(d: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut aus = Vec::new();
    for e in std::fs::read_dir(d).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            aus.extend(walk(&p));
        } else {
            aus.push(p);
        }
    }
    aus
}

// ---------------------------------------------------------------------------
// Echtes Modell (ignoriert): `LVA_E5_MODELL=<ordner mit model.onnx und tokenizer.json>`
// ---------------------------------------------------------------------------

fn echtes_modell() -> Option<(std::path::PathBuf, E5)> {
    let d = std::path::PathBuf::from(std::env::var_os("LVA_E5_MODELL")?);
    let m = E5::laden(&d.join(DATEI_MODELL), &d.join(DATEI_ZERLEGER)).expect("Modell laden");
    Some((d, m))
}

/// IDs aus der Bibliothek `tokenizers` (Python, gleiche `tokenizer.json`).
#[test]
#[ignore = "braucht LVA_E5_MODELL"]
fn zerleger_stimmt_mit_tokenizers_ueberein() {
    let Some((d, m)) = echtes_modell() else {
        return;
    };
    let z = Zerleger::aus_json(&std::fs::read(d.join(DATEI_ZERLEGER)).unwrap()).unwrap();
    let faelle: [(&str, &[i64]); 3] = [
        (
            "query: Wie lange läuft der Wartungsvertrag?",
            &[
                0, 41, 1294, 12, 4887, 9614, 93974, 122, 5550, 8587, 7, 76869, 32, 2,
            ],
        ),
        (
            "passage: § 3 Vertragsdauer: 36 Monate, 80 g/m² – Größe Straße\u{a0}x  y\nz",
            &[
                0, 46692, 12, 5360, 138, 109119, 107536, 12, 4039, 84044, 4, 2248, 706, 64, 39,
                304, 46, 66461, 90515, 1022, 113, 97, 2,
            ],
        ),
        (
            "passage: Ünïcödé 😀🙂 Ⅻ ﬁ stoerung@aufzug.example.com",
            &[
                0, 46692, 12, 8578, 19, 9392, 238, 21166, 446, 21119, 207254, 39198, 809, 3474,
                42441, 981, 18876, 31621, 5, 3355, 11, 33209, 5, 277, 2,
            ],
        ),
    ];
    for (text, ids) in faelle {
        assert_eq!(z.kodieren(text, 512), ids, "{text}");
    }
    // Referenz aus onnxruntime (Python): erste Werte des normierten Anfragevektors.
    let v = m
        .einbetten(
            &["Wie lange läuft der Wartungsvertrag?".into()],
            Art::Anfrage,
        )
        .unwrap();
    assert_eq!(v[0].len(), 384);
    for (a, b) in v[0].iter().zip([0.05955f32, -0.03979, -0.07131, -0.02648]) {
        assert!((a - b).abs() < 1e-3, "{a} {b}");
    }
}

/// Eval-Tor (Spec §7): Recall@5 ≥ 0,9, Belegkorrektheit ≥ 0,95, unbeantwortbar ≥ 0,9 „kein Beleg“.
/// Das Ergebnis muss zu [`SEMANTIK_STANDARD`] passen.
#[test]
#[ignore = "braucht LVA_E5_MODELL"]
fn eval_tor_mit_echtem_modell() {
    let Some((_, m)) = echtes_modell() else {
        return;
    };
    let k = korpus();
    let b = aufbauen(&k);
    let lex = messen(&k, &b, &AblageEinbettung::Keine);
    let emb = AblageEinbettung::e5(std::sync::Arc::new(m));
    let start = std::time::Instant::now();
    let n = index::vektoren_nachziehen(&b.a, &emb).unwrap();
    let dauer = start.elapsed();
    let sem = messen(&k, &b, &emb);
    eprintln!("[eval] Stücke eingebettet: {n} in {dauer:?}");
    eprintln!("[eval] lexikalisch: {lex:?} Tor={}", lex.tor());
    eprintln!(
        "[eval] hybrid (e5, kand={KANDIDAT_COS}): {sem:?} Tor={}",
        sem.tor()
    );
    assert!(
        sem.recall5 >= lex.recall5,
        "hybrid schlechter als lexikalisch"
    );
    assert_eq!(
        sem.tor(),
        SEMANTIK_STANDARD,
        "Eval-Tor und SEMANTIK_STANDARD widersprechen sich"
    );
}

// ---------------------------------------------------------------------------
// Normierung (lva#508): nicht endliche Werte dürfen nie in den Index
// ---------------------------------------------------------------------------

#[test]
fn normieren_liefert_einheitsvektor_und_laesst_den_nullvektor() {
    let mut v = vec![3.0f32, 4.0];
    assert!(normieren(&mut v));
    assert!(
        (v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6,
        "{v:?}"
    );
    let mut null = vec![0.0f32; 4];
    assert!(normieren(&mut null));
    assert_eq!(null, vec![0.0; 4]);
    // Große endliche Werte: die Norm läuft in f64 und überläuft nicht.
    let mut gross = vec![f32::MAX, f32::MAX];
    assert!(normieren(&mut gross));
    assert!(
        (gross[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
        "{gross:?}"
    );
}

#[test]
fn normieren_weist_nan_und_unendlich_ab_und_aendert_nichts() {
    for schlecht in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut v = vec![1.0f32, schlecht, 2.0];
        let vorher = v.clone();
        assert!(!normieren(&mut v));
        // NaN != NaN: bitweise vergleichen.
        let bits = |x: &[f32]| x.iter().map(|f| f.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&v), bits(&vorher));
    }
}
