//! Minimale Zeitumrechnung (RFC 3339, UTC mit `Z`) ohne Systemuhr und ohne Zusatzabhaengigkeit.
//! Der Kern rechnet in Unix-Sekunden; die Uhr liefert der Aufrufer.

/// Unix-Sekunden als `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(sekunden: u64) -> String {
    let tage = (sekunden / 86_400) as i64;
    let rest = sekunden % 86_400;
    let (j, m, t) = datum_aus_tagen(tage);
    format!(
        "{j:04}-{m:02}-{t:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Liest `YYYY-MM-DDTHH:MM:SS[.f]Z`; Bruchteile werden verworfen. `None` bei jeder Abweichung.
pub fn aus_rfc3339(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || *b.last()? != b'Z' {
        return None;
    }
    let ziffern = |von: usize, bis: usize| -> Option<i64> {
        let teil = s.get(von..bis)?;
        if teil.bytes().all(|c| c.is_ascii_digit()) {
            teil.parse().ok()
        } else {
            None
        }
    };
    let trenner_ok =
        b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':' && b[16] == b':';
    if !trenner_ok {
        return None;
    }
    let (j, m, t) = (ziffern(0, 4)?, ziffern(5, 7)?, ziffern(8, 10)?);
    let (h, mi, se) = (ziffern(11, 13)?, ziffern(14, 16)?, ziffern(17, 19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&t) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let bruch_ok = b.len() == 20
        || (b[19] == b'.'
            && b.len() > 21
            && s[20..b.len() - 1].bytes().all(|c| c.is_ascii_digit()));
    if !bruch_ok {
        return None;
    }
    let tage = tage_aus_datum(j, m, t);
    u64::try_from(tage * 86_400 + h * 3600 + mi * 60 + se).ok()
}

fn tage_aus_datum(j: i64, m: i64, t: i64) -> i64 {
    let j = if m <= 2 { j - 1 } else { j };
    let era = j.div_euclid(400);
    let jahr_der_era = j - era * 400;
    let tag_im_jahr = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + t - 1;
    let tag_der_era = jahr_der_era * 365 + jahr_der_era / 4 - jahr_der_era / 100 + tag_im_jahr;
    era * 146_097 + tag_der_era - 719_468
}

fn datum_aus_tagen(tage: i64) -> (i64, i64, i64) {
    let z = tage + 719_468;
    let era = z.div_euclid(146_097);
    let tag_der_era = z - era * 146_097;
    let jahr_der_era =
        (tag_der_era - tag_der_era / 1460 + tag_der_era / 36_524 - tag_der_era / 146_096) / 365;
    let tag_im_jahr = tag_der_era - (365 * jahr_der_era + jahr_der_era / 4 - jahr_der_era / 100);
    let mp = (5 * tag_im_jahr + 2) / 153;
    let t = tag_im_jahr - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let j = jahr_der_era + era * 400 + i64::from(m <= 2);
    (j, m, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bekannte_werte() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_794_081_600), "2026-11-07T20:00:00Z");
        assert_eq!(aus_rfc3339("2026-11-07T20:00:00Z"), Some(1_794_081_600));
        assert_eq!(aus_rfc3339("2026-11-07T20:00:00.123Z"), Some(1_794_081_600));
    }

    #[test]
    fn rundlauf_ueber_schaltjahr() {
        for s in [
            0u64,
            951_782_400,
            1_709_251_199,
            1_709_251_200,
            4_102_444_800,
        ] {
            assert_eq!(aus_rfc3339(&rfc3339(s)), Some(s));
        }
    }

    #[test]
    fn ungueltiges_wird_abgewiesen() {
        for s in [
            "",
            "2026-11-07",
            "2026-13-07T20:00:00Z",
            "2026-11-07 20:00:00Z",
            "2026-11-07T20:00:00",
            "2026-11-07T20:00:00.Z",
        ] {
            assert_eq!(aus_rfc3339(s), None, "{s}");
        }
    }
}
