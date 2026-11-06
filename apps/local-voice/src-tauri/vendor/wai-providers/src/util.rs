//! Kleine Helfer: Zeitstempel (RFC 3339 UTC) und praefixierte ULID-artige IDs.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `<prefix>_` + 26 Crockford-Zeichen (10 Zeit, 16 Zufall/Zaehler).
pub fn new_id(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut out = String::with_capacity(prefix.len() + 27);
    out.push_str(prefix);
    out.push('_');
    let t = now_ms();
    for i in (0..10).rev() {
        out.push(CROCKFORD[((t >> (5 * i)) & 31) as usize] as char);
    }
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u64(t);
    let mut r = h.finish() as u128;
    let mut h2 = RandomState::new().build_hasher();
    h2.write_u64(r as u64);
    r = (r << 64) | h2.finish() as u128;
    for i in 0..16 {
        out.push(CROCKFORD[((r >> (5 * i)) & 31) as usize] as char);
    }
    out
}

/// 32 Hex-Zeichen (W3C trace id).
pub fn new_trace_id() -> String {
    let mut h = RandomState::new().build_hasher();
    h.write_u64(now_ms());
    let a = h.finish();
    let mut h2 = RandomState::new().build_hasher();
    h2.write_u64(a);
    format!("{a:016x}{:016x}", h2.finish())
}

/// Aktueller Zeitpunkt als `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_rfc3339() -> String {
    let secs = (now_ms() / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_732), (2026, 10, 6));
    }

    #[test]
    fn ids_have_contract_shape() {
        let id = new_id("evt");
        assert_eq!(id.len(), 30);
        assert!(id[4..].chars().all(|c| CROCKFORD.contains(&(c as u8))));
        assert_ne!(new_id("evt"), id);
        assert_eq!(new_trace_id().len(), 32);
    }
}
