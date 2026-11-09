//! Kennungen (ULID), Zeitstempel und Eingabepruefung.

use std::time::{SystemTime, UNIX_EPOCH};

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

pub fn jetzt_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Praefix-ULID `<praefix>_<26 Zeichen Crockford>` (48 Bit Zeit, 80 Bit Zufall).
pub fn neue_id(praefix: &str) -> String {
    let mut zufall = [0u8; 10];
    getrandom::fill(&mut zufall).expect("OS-Zufall nicht verfuegbar");
    let mut wert: u128 = (jetzt_ms() as u128 & 0xFFFF_FFFF_FFFF) << 80;
    wert |= zufall.iter().fold(0u128, |a, b| (a << 8) | u128::from(*b));
    let mut s = String::with_capacity(praefix.len() + 27);
    s.push_str(praefix);
    s.push('_');
    for i in (0..26).rev() {
        s.push(CROCKFORD[((wert >> (i * 5)) & 31) as usize] as char);
    }
    s
}

pub fn neue_trace_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS-Zufall nicht verfuegbar");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// RFC 3339 in UTC mit `Z` aus Millisekunden seit der Epoche.
pub fn zeitstempel(ms: i64) -> String {
    let tage = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000);
    let (j, m, t) = datum(tage);
    format!(
        "{j:04}-{m:02}-{t:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60,
        rest % 1000
    )
}

/// Tage seit 1970-01-01 in (Jahr, Monat, Tag), proleptisch gregorianisch.
fn datum(tage: i64) -> (i64, i64, i64) {
    let z = tage + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let t = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, t)
}

pub fn slug_ok(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 64 || !b[0].is_ascii_lowercase() {
        return false;
    }
    let mut vorher_trenner = false;
    for (i, c) in b.iter().enumerate() {
        let trenner = matches!(c, b'.' | b'_' | b'-');
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || trenner) {
            return false;
        }
        if trenner && (vorher_trenner || i == b.len() - 1) {
            return false;
        }
        vorher_trenner = trenner;
    }
    true
}

/// Schema-Muster `^wai://[a-z0-9-]+/[^\s]+$`, 8..=500 Zeichen.
pub fn wai_muster_ok(s: &str) -> bool {
    let n = s.chars().count();
    if !(8..=500).contains(&n) || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let Some(rest) = s.strip_prefix("wai://") else {
        return false;
    };
    match rest.split_once('/') {
        Some((app, pfad)) => {
            !app.is_empty()
                && !pfad.is_empty()
                && app
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeit_und_id_format() {
        assert_eq!(zeitstempel(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(zeitstempel(1_782_950_400_123), "2026-07-02T00:00:00.123Z");
        let id = neue_id("prj");
        assert_eq!(id.len(), 30);
        assert!(id.starts_with("prj_"));
        assert!(slug_ok("corporate-llm") && !slug_ok("Corp") && !slug_ok("a--b"));
        assert!(wai_muster_ok("wai://lva/buch/abc") && !wai_muster_ok("http://x/y"));
    }
}
