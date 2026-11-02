//! Deterministische Belegpruefung: Ist ein Satz in einem zitierten Auszug gestuetzt?
//!
//! Regeln (alle muessen zutreffen):
//! * Schluesselwoerter (ab 4 Zeichen, keine Stoppwoerter, keine Negation) muessen zu mindestens `schwelle`
//!   im Auszug vorkommen. Wortvergleich: gleich, oder beide mit mindestens 8 Zeichen und gleichen ersten 8.
//! * Ein Satz ohne Schluesselwort ist nie gestuetzt (auch nicht, wenn nur eine Zahl darin steht).
//! * Jede Zahl wird kanonisiert (1.000 = 1000, 1,5 = 1.5) und muss im Auszug mit passendem Nachbarwort
//!   stehen; eine Praeposition (ab/bis/seit/vor ...) direkt vor der Zahl muss ebenso davor stehen.
//! * Negationsparitaet: Negation im Satz genau dann, wenn im passendsten Auszugssatz.

const STOPP: &[&str] = &[
    "dass", "sich", "sind", "wird", "werden", "wurde", "wurden", "eine", "einer", "einem", "einen",
    "eines", "oder", "auch", "nach", "von", "dem", "den", "der", "die", "das", "und", "mit",
    "fuer", "für", "kann", "koennen", "können", "nur", "aber", "wie", "als", "zur", "zum", "ist",
    "hat", "haben", "that", "with", "this", "from", "have", "been", "were", "which", "their",
    "there", "than", "will", "would", "about", "into", "over", "also", "the", "and", "for", "are",
    "was",
];

const NEGATION: &[&str] = &[
    "nicht", "kein", "keine", "keinen", "keinem", "keiner", "keines", "ohne", "nie", "niemals",
    "nichts", "weder", "no", "not", "never", "none", "without",
];

const PRAEP: &[&str] = &[
    "ab", "bis", "seit", "vor", "nach", "ueber", "über", "unter", "um", "von", "in", "from",
    "until", "since", "before", "after",
];

const VERGLEICH: &[&str] = &[
    "mehr",
    "weniger",
    "als",
    "groesser",
    "größer",
    "kleiner",
    "hoeher",
    "höher",
    "niedriger",
    "besser",
    "schlechter",
    "doppelt",
    "mindestens",
    "hoechstens",
    "höchstens",
    "more",
    "less",
    "than",
    "higher",
    "lower",
    "greater",
    "fewer",
    "most",
    "least",
];

const STAMM: usize = 8;

fn ist_zahl(w: &str) -> bool {
    w.starts_with(|c: char| c.is_ascii_digit())
        && w.chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
}

/// 1.000 und 1000 gleich; 1,5 und 1.5 gleich (letztes gemischtes Trennzeichen oder ein einzelnes
/// Trennzeichen ohne genau drei Folgeziffern ist Dezimaltrenner).
pub fn kanonische_zahl(s: &str) -> String {
    let ziffern = |t: &str| -> String { t.chars().filter(char::is_ascii_digit).collect() };
    let punkt = s.matches('.').count();
    let komma = s.matches(',').count();
    let (ganz, dez) = match (punkt, komma) {
        (0, 0) => (ziffern(s), String::new()),
        (p, k) if p > 0 && k > 0 => {
            let l = s.rfind(['.', ',']).unwrap_or(0);
            (ziffern(&s[..l]), ziffern(&s[l + 1..]))
        }
        (p, k) => {
            let sep = if p > 0 { '.' } else { ',' };
            let l = s.rfind(sep).unwrap_or(0);
            let nach = &s[l + 1..];
            if p + k > 1 || nach.len() == 3 {
                (ziffern(s), String::new())
            } else {
                (ziffern(&s[..l]), ziffern(nach))
            }
        }
    };
    let ganz = ganz.trim_start_matches('0');
    let ganz = if ganz.is_empty() { "0" } else { ganz };
    let dez = dez.trim_end_matches('0');
    if dez.is_empty() {
        ganz.to_string()
    } else {
        format!("{ganz}.{dez}")
    }
}

/// Zerlegt in Woerter und Zahlen; `.` und `,` zwischen Ziffern bleiben Teil der Zahl; Zahlen sind kanonisiert.
fn zerlege(text: &str) -> Vec<String> {
    let zeichen: Vec<char> = text.chars().collect();
    let mut aus = Vec::new();
    let mut akt = String::new();
    let abschliessen = |akt: &mut String, aus: &mut Vec<String>| {
        if akt.is_empty() {
            return;
        }
        let w = std::mem::take(akt);
        aus.push(if ist_zahl(&w) { kanonische_zahl(&w) } else { w });
    };
    for (i, &c) in zeichen.iter().enumerate() {
        let zwischen_ziffern = (c == '.' || c == ',')
            && i > 0
            && zeichen[i - 1].is_ascii_digit()
            && zeichen.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if c.is_alphanumeric() || zwischen_ziffern {
            akt.extend(c.to_lowercase());
        } else {
            abschliessen(&mut akt, &mut aus);
        }
    }
    abschliessen(&mut akt, &mut aus);
    aus
}

fn zahl_token(w: &str) -> bool {
    w.starts_with(|c: char| c.is_ascii_digit()) && w.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn ist_neg(w: &str) -> bool {
    NEGATION.contains(&w)
}

/// Gleich, oder beide mindestens `STAMM` Zeichen lang mit gleichem Anfang dieser Laenge.
fn passt(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (la, lb) = (a.chars().count(), b.chars().count());
    la >= STAMM && lb >= STAMM && a.chars().take(STAMM).eq(b.chars().take(STAMM))
}

fn schluesselwoerter(w: &[String]) -> Vec<&String> {
    w.iter()
        .filter(|x| {
            !zahl_token(x) && x.chars().count() >= 4 && !STOPP.contains(&x.as_str()) && !ist_neg(x)
        })
        .collect()
}

/// Saetze eines Auszugs (Ende bei `.`, `!`, `?`, `;` vor Leerraum oder Zeilenende).
fn saetze(text: &str) -> Vec<&str> {
    let mut aus = Vec::new();
    let mut start = 0;
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        let ende = matches!(c, '.' | '!' | '?' | ';' | '\n')
            && it.peek().is_none_or(|(_, n)| n.is_whitespace());
        if ende {
            let stop = i + c.len_utf8();
            if !text[start..stop].trim().is_empty() {
                aus.push(&text[start..stop]);
            }
            start = stop;
        }
    }
    if !text[start..].trim().is_empty() {
        aus.push(&text[start..]);
    }
    aus
}

fn zahl_belegt(satz: &[String], i: usize, aus: &[String]) -> bool {
    let nachbar = |v: &[String], k: Option<usize>| -> Option<String> {
        k.and_then(|k| v.get(k)).filter(|w| !zahl_token(w)).cloned()
    };
    let prev = nachbar(satz, i.checked_sub(1));
    let next = nachbar(satz, Some(i + 1));
    if prev.is_none() && next.is_none() {
        return false;
    }
    let praep = prev.as_deref().filter(|p| PRAEP.contains(p));
    aus.iter().enumerate().any(|(j, w)| {
        if *w != satz[i] {
            return false;
        }
        let ap = nachbar(aus, j.checked_sub(1));
        let an = nachbar(aus, Some(j + 1));
        if let Some(p) = praep
            && ap.as_deref() != Some(p)
        {
            return false;
        }
        let gleich = |x: &Option<String>, y: &Option<String>| match (x, y) {
            (Some(x), Some(y)) => passt(x, y),
            _ => false,
        };
        gleich(&prev, &ap) || gleich(&next, &an)
    })
}

fn einzeln(satz: &str, auszug: &str, schwelle: f64) -> bool {
    let sw = zerlege(satz);
    let aw = zerlege(auszug);
    let schluessel = schluesselwoerter(&sw);
    if schluessel.is_empty() {
        return false;
    }
    let treffer = schluessel
        .iter()
        .filter(|k| aw.iter().any(|a| passt(k, a)))
        .count();
    if (treffer as f64 / schluessel.len() as f64) < schwelle {
        return false;
    }
    for (i, w) in sw.iter().enumerate() {
        if zahl_token(w) && !zahl_belegt(&sw, i, &aw) {
            return false;
        }
    }
    // Negationsparitaet mit dem passendsten Satz des Auszugs.
    let bester = saetze(auszug)
        .into_iter()
        .max_by_key(|s| {
            let w = zerlege(s);
            let n = schluessel
                .iter()
                .filter(|k| w.iter().any(|a| passt(k, a)))
                .count();
            (n, std::cmp::Reverse(s.as_ptr() as usize))
        })
        .map(zerlege)
        .unwrap_or_default();
    sw.iter().any(|w| ist_neg(w)) == bester.iter().any(|w| ist_neg(w))
}

/// `true`, wenn `satz` durch mindestens einen der `auszuege` gestuetzt ist. `schwelle` in 0.0..=1.0.
pub fn gestuetzt(satz: &str, auszuege: &[&str], schwelle: f64) -> bool {
    auszuege.iter().any(|a| einzeln(satz, a, schwelle))
}

/// Saetze mit Negation oder Vergleich werden immer zusaetzlich per Modell geprueft.
pub fn braucht_modellpruefung(satz: &str) -> bool {
    zerlege(satz)
        .iter()
        .any(|w| ist_neg(w) || VERGLEICH.contains(&w.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "Die kommunale Waermeplanung ist bis 2028 verpflichtend. Gemeinden mit mehr als 100.000 Einwohnern muessen frueher fertig sein (2026).";

    fn g(satz: &str) -> bool {
        gestuetzt(satz, &[A], 0.6)
    }

    #[test]
    fn wortgleicher_satz_ist_gestuetzt() {
        assert!(g("Die Waermeplanung ist bis 2028 verpflichtend."));
    }

    #[test]
    fn erfundene_zahl_ist_nicht_gestuetzt() {
        assert!(!g("Die Waermeplanung ist bis 2031 verpflichtend."));
    }

    #[test]
    fn zahlen_werden_kanonisiert() {
        assert!(g("Gemeinden mit mehr als 100.000 Einwohnern"));
        assert!(g("Gemeinden mit mehr als 100000 Einwohnern"));
        assert!(g("Gemeinden mit mehr als 100,000 Einwohnern"));
        assert!(!g("Gemeinden mit mehr als 10.000 Einwohnern"));
        assert_eq!(kanonische_zahl("1,5"), "1.5");
        assert_eq!(kanonische_zahl("1.5"), "1.5");
        assert_eq!(kanonische_zahl("1.000"), "1000");
        assert_eq!(kanonische_zahl("1.234,50"), "1234.5");
        assert_eq!(kanonische_zahl("1,234.50"), "1234.5");
        assert_eq!(kanonische_zahl("1.000.000"), "1000000");
    }

    #[test]
    fn praeposition_muss_zur_zahl_passen() {
        assert!(!g("Die Waermeplanung ist ab 2028 verpflichtend."));
        assert!(!g("Die Waermeplanung ist seit 2028 verpflichtend."));
    }

    #[test]
    fn zahl_braucht_nachbarwort() {
        // 2026 steht im Auszug in Klammern neben "sein", nicht neben "Waermeplanung".
        assert!(!g("Die Waermeplanung endet 2026."));
    }

    #[test]
    fn satz_nur_mit_zahl_ist_nicht_gestuetzt() {
        assert!(!g("Es sind 2028."));
        assert!(!g("2028"));
    }

    #[test]
    fn negation_muss_uebereinstimmen() {
        assert!(!g("Die Waermeplanung ist bis 2028 nicht verpflichtend."));
        assert!(!g("Die Waermeplanung ist niemals verpflichtend."));
        let neg = "Die Waermeplanung ist nicht verpflichtend. Gemeinden planen frei.";
        assert!(gestuetzt(
            "Die Waermeplanung ist nicht verpflichtend.",
            &[neg],
            0.6
        ));
        assert!(!gestuetzt(
            "Die Waermeplanung ist verpflichtend.",
            &[neg],
            0.6
        ));
        assert!(!gestuetzt(
            "Die Waermeplanung ist keine Pflicht.",
            &[A],
            0.5
        ));
    }

    #[test]
    fn wortstamm_erst_ab_acht_zeichen() {
        let a = "Waermeplanungen werden erstellt. Gemeinde plant.";
        assert!(gestuetzt("Die Waermeplanung wird erstellt", &[a], 0.6));
        // kurze Woerter nur exakt: "plant" gegen "planen"
        assert!(!gestuetzt("Gemeinden planen", &["Gemeinde plant."], 0.9));
    }

    #[test]
    fn fremdes_thema_faellt_durch() {
        assert!(!g("Windkraftanlagen benoetigen Abstandsflaechen"));
    }

    #[test]
    fn satz_ohne_merkmale_ist_nicht_gestuetzt() {
        assert!(!g("Ja, so ist es."));
    }

    #[test]
    fn jeder_auszug_der_quelle_zaehlt() {
        assert!(gestuetzt(
            "Windkraftanlagen benoetigen Abstandsflaechen",
            &[
                A,
                "Windkraftanlagen benoetigen Abstandsflaechen zu Wohnhaeusern."
            ],
            0.6
        ));
    }

    #[test]
    fn modellpruefung_bei_negation_und_vergleich() {
        assert!(braucht_modellpruefung("Das ist nicht erlaubt"));
        assert!(braucht_modellpruefung("Mehr als 5 Gemeinden"));
        assert!(!braucht_modellpruefung("Die Frist endet 2028"));
    }
}
