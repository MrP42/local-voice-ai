use super::*;

/// Deterministischer Buchtext: Sätze aus einem kleinen Wortschatz (viele
/// Wiederholungen wie in echter Prosa) plus seltene Wörter.
fn book_text(sentences: usize) -> String {
    const COMMON: [&str; 24] = [
        "der", "die", "das", "und", "sie", "er", "war", "nicht", "mit", "auf", "ein", "eine", "zu",
        "dem", "Haus", "Meer", "Licht", "Nacht", "lange", "immer", "wieder", "sah", "ging",
        "hörte",
    ];
    const RARE: [&str; 12] = [
        "Leuchtturm",
        "Klippe",
        "Wärterin",
        "Stufen",
        "Brandung",
        "Nebelhorn",
        "Kompass",
        "Laterne",
        "Möwen",
        "Treibholz",
        "Gezeiten",
        "Fernglas",
    ];
    let mut seed: u64 = 42;
    let mut rnd = |n: usize| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as usize) % n
    };
    let mut out = String::new();
    for s in 0..sentences {
        if s % 12 == 0 {
            out.push_str(&format!("\n\n## Kapitel {}\n\n", s / 12 + 1));
        }
        let len = 6 + rnd(9);
        for w in 0..len {
            let word = if rnd(4) == 0 {
                RARE[rnd(RARE.len())]
            } else {
                COMMON[rnd(COMMON.len())]
            };
            if w == 0 {
                let mut c = word.chars();
                let first = c.next().unwrap().to_uppercase().collect::<String>();
                out.push_str(&(first + c.as_str()));
            } else {
                out.push(' ');
                out.push_str(word);
            }
        }
        out.push_str(if s % 5 == 0 { "! " } else { ". " });
        if s % 7 == 3 {
            out.push_str("{{S. 12}} ");
        }
    }
    out
}

/// „Gehört“: Buchwörter ab `from`, je Wort 400 ms, mit Störungen.
fn hear(
    book: &[BookWord],
    text: &str,
    from: usize,
    count: usize,
    t0: f64,
) -> (Vec<HeardWord>, Vec<(usize, f64)>) {
    let chars: Vec<char> = text.chars().collect();
    let mut heard = Vec::new();
    let mut truth = Vec::new();
    let mut t = t0;
    for (k, w) in book[from..from + count].iter().enumerate() {
        let i = from + k;
        // Jedes 17. Wort fehlt im Hörbuch (gekürzt), jedes 11. ist verhört.
        if k % 17 == 9 {
            continue;
        }
        let raw: String = chars[w.start as usize..w.end as usize].iter().collect();
        let raw = if k % 11 == 5 && raw.chars().count() > 5 {
            let mut s: Vec<char> = raw.chars().collect();
            s[2] = 'x';
            s.into_iter().collect()
        } else {
            raw
        };
        truth.push((i, t));
        heard.push(HeardWord {
            norm: normalize(&raw),
            text: raw,
            start_ms: t,
            end_ms: t + 380.0,
        });
        t += 400.0;
    }
    (heard, truth)
}

fn check_points(a: &Alignment, book: &[BookWord], truth: &[(usize, f64)]) -> usize {
    let mut good = 0;
    for p in &a.points {
        let expected = truth
            .iter()
            .find(|(_, ms)| (*ms - p.ms).abs() < 1.0)
            .map(|(i, _)| book[*i].start);
        assert_eq!(
            expected,
            Some(p.chars),
            "Stützpunkt bei {} ms zeigt auf falsches Wort",
            p.ms
        );
        good += 1;
    }
    good
}

#[test]
fn normalisiert_umlaute_und_satzzeichen() {
    assert_eq!(normalize("„Straße!“"), "strasse");
    assert_eq!(normalize("Über,"), "über");
    assert_eq!(normalize("42."), "42");
    assert_eq!(normalize("—"), "");
}

#[test]
fn buchwoerter_tragen_zeichenlage_und_ueberspringen_seitenmarken() {
    let text = "## Kapitel 1\n\nDer {{S. 12}} Leuchtturm stand.";
    let w = book_words(text);
    let norms: Vec<&str> = w.iter().map(|w| w.norm.as_str()).collect();
    assert_eq!(norms, ["kapitel", "1", "der", "leuchtturm", "stand"]);
    let chars: Vec<char> = text.chars().collect();
    let s: String = chars[w[3].start as usize..w[3].end as usize]
        .iter()
        .collect();
    assert_eq!(s, "Leuchtturm");
}

#[test]
fn segmente_verteilen_zeit_nach_zeichen() {
    let w = heard_words(&[Segment {
        text: "Ein Leuchtturm".into(),
        start_ms: 1000.0,
        end_ms: 2500.0,
    }]);
    assert_eq!(w.len(), 2);
    assert_eq!(w[0].start_ms, 1000.0);
    // „Ein“ + Leerraum = 4 von 15 Zeichen: 1000 + 1500 · 4/15.
    assert!((w[1].start_ms - 1400.0).abs() < 1.0);
    assert!((w[1].end_ms - 2500.0).abs() < 1.0);
}

#[test]
fn abgleich_mit_hinweis_trifft_die_woerter_trotz_hoerfehlern_und_kuerzungen() {
    let text = book_text(400);
    let book = book_words(&text);
    let from = 1200;
    let (heard, truth) = hear(&book, &text, from, 90, 600_000.0);
    let a = align(&book, &heard, Some(from - 30), 600);
    let good = check_points(&a, &book, &truth);
    assert!(
        good as f32 >= heard.len() as f32 * 0.8,
        "nur {good} von {} Wörtern zugeordnet",
        heard.len()
    );
    assert!(a.coverage > 0.8);
    assert!(
        a.inserts.is_empty(),
        "keine Einschübe erwartet: {:?}",
        a.inserts
    );
    // Stützpunkte steigen in beiden Achsen.
    assert!(a
        .points
        .windows(2)
        .all(|p| p[0].chars < p[1].chars && p[0].ms < p[1].ms));
    let (b0, b1) = a.book_span.unwrap();
    assert!(b0 >= from && b1 < from + 90);
}

#[test]
fn abgleich_ohne_hinweis_findet_die_stelle_im_ganzen_buch() {
    let text = book_text(400);
    let book = book_words(&text);
    let from = book.len() - 200;
    let (heard, truth) = hear(&book, &text, from, 60, 3_000_000.0);
    let a = align(&book, &heard, None, 0);
    let good = check_points(&a, &book, &truth);
    assert!(good >= 40, "nur {good} Treffer");
}

#[test]
fn einschub_aus_dem_hoerbuch_wird_an_der_buchstelle_gemeldet() {
    let text = book_text(200);
    let book = book_words(&text);
    let from = 500;
    let (mut heard, _) = hear(&book, &text, from, 60, 0.0);
    let extra = [
        "Anmerkung",
        "des",
        "Sprechers",
        "zur",
        "Neuausgabe",
        "dieses",
        "Buches",
    ];
    let at_word = 30;
    let t = heard[at_word].start_ms;
    let ins: Vec<HeardWord> = extra
        .iter()
        .enumerate()
        .map(|(k, w)| HeardWord {
            text: (*w).into(),
            norm: normalize(w),
            start_ms: t + k as f64,
            end_ms: t + k as f64 + 0.5,
        })
        .collect();
    heard.splice(at_word..at_word, ins);
    let a = align(&book, &heard, Some(from), 400);
    assert_eq!(a.inserts.len(), 1, "{:?}", a.inserts);
    assert_eq!(a.inserts[0].text, extra.join(" "));
    // Eingefügt vor dem Buchwort, das nach dem Einschub gehört wird.
    let after = &heard[at_word + extra.len()];
    let p = a
        .points
        .iter()
        .find(|p| p.ms == after.start_ms)
        .expect("Wort nach Einschub zugeordnet");
    assert_eq!(a.inserts[0].at, p.chars);
}

#[test]
fn ohne_gemeinsame_woerter_kein_ergebnis() {
    let book = book_words("Der Leuchtturm stand seit über hundert Jahren auf der Klippe.");
    let heard = heard_words(&[Segment {
        text: "völlig anderes Gerede hier und dort".into(),
        start_ms: 0.0,
        end_ms: 3000.0,
    }]);
    let a = align(&book, &heard, Some(0), 100);
    assert!(a.points.is_empty());
    assert_eq!(a.book_span, None);
}

#[test]
fn aehnlichkeit() {
    assert_eq!(similarity("haus", "haus"), 1.0);
    assert!(same("leuchtturm", "leuchtturms"));
    assert!(same("brandung", "brandunk"));
    assert!(!same("er", "es"));
    assert!(!same("meer", "mehr") || similarity("meer", "mehr") >= 0.75);
}

#[test]
fn ganzes_buch_mit_hunderttausend_woertern_bleibt_schnell() {
    let text = book_text(10_000);
    let book = book_words(&text);
    assert!(book.len() > 90_000);
    let from = book.len() / 2;
    let (heard, truth) = hear(&book, &text, from, 75, 1.0);
    let t = std::time::Instant::now();
    let a = align(&book, &heard, None, 0);
    let took = t.elapsed();
    assert!(check_points(&a, &book, &truth) >= 50);
    // Großzügig für Debug-Builds auf langsamen Rechnern.
    assert!(took.as_millis() < 3_000, "{took:?}");
}

#[test]
fn lange_luecke_ohne_anker_wird_in_stuecken_abgeglichen() {
    let filler = std::iter::repeat_n("rot blau gelb", 300)
        .collect::<Vec<_>>()
        .join(" ");
    let text = format!("alpha beta gamma {filler} delta epsilon zeta");
    let book = book_words(&text);
    let heard = heard_words(&[Segment {
        text: text.clone(),
        start_ms: 0.0,
        end_ms: 400_000.0,
    }]);
    let a = align(&book, &heard, Some(0), usize::MAX);
    assert!(a.inserts.is_empty(), "{} Einschübe", a.inserts.len());
    assert!(a.coverage > 0.99, "Abdeckung {}", a.coverage);
}

#[test]
fn offenes_doppelklammer_ist_gewoehnlicher_text() {
    let w = book_words("Er sagte {{ und ging weg");
    let norms: Vec<&str> = w.iter().map(|w| w.norm.as_str()).collect();
    assert_eq!(norms, ["er", "sagte", "und", "ging", "weg"]);
}

#[test]
fn lage_in_utf16_wie_im_frontend() {
    let text = "Ein 🚢 fährt";
    let w = book_words(text);
    // JS: "Ein 🚢 fährt".indexOf("fährt") === 7 (das Emoji zählt doppelt).
    assert_eq!(w[1].start, 7);
}

#[test]
fn segmente_ohne_dauer_ergeben_steigende_zeiten_ohne_falsche_einschuebe() {
    let text = "eins zwei drei vier fünf sechs sieben acht neun zehn";
    let book = book_words(text);
    let heard = heard_words(&[
        Segment {
            text: "eins zwei drei vier fünf".into(),
            start_ms: 1000.0,
            end_ms: 1000.0,
        },
        Segment {
            text: "sechs sieben acht neun zehn".into(),
            start_ms: 900.0,
            end_ms: 3000.0,
        },
    ]);
    let a = align(&book, &heard, Some(0), 100);
    assert!(
        a.points
            .windows(2)
            .all(|p| p[0].chars < p[1].chars && p[0].ms < p[1].ms),
        "{:?}",
        a.points
    );
    assert!(a.inserts.is_empty());
}

#[test]
fn riesiges_fenster_laeuft_nicht_ueber() {
    let book = book_words("Der Leuchtturm stand seit über hundert Jahren auf der Klippe.");
    let heard = heard_words(&[Segment {
        text: "stand seit über hundert Jahren".into(),
        start_ms: 0.0,
        end_ms: 2000.0,
    }]);
    let a = align(&book, &heard, Some(2), usize::MAX);
    assert_eq!(a.points.len(), 5);
}
