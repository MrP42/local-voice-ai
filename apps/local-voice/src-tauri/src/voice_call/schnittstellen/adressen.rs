use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub fn unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"._~-".contains(&b)
}
pub fn pfadwert_ok(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.bytes().all(unreserved)
}
pub fn parametername(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

const V4: &[(&str, u32, &str, &str)] = &[
    ("0.0.0.0", 8, "nie", "dieses_netz"),
    ("10.0.0.0", 8, "privat", "privat"),
    ("100.64.0.0", 10, "privat", "geteilt"),
    ("127.0.0.0", 8, "geraet", "loopback"),
    ("169.254.0.0", 16, "nie", "link_lokal"),
    ("172.16.0.0", 12, "privat", "privat"),
    ("192.0.0.0", 24, "nie", "protokoll"),
    ("192.0.2.0", 24, "nie", "dokumentation"),
    ("192.88.99.0", 24, "nie", "uebergang"),
    ("192.168.0.0", 16, "privat", "privat"),
    ("198.18.0.0", 15, "nie", "benchmark"),
    ("198.51.100.0", 24, "nie", "dokumentation"),
    ("203.0.113.0", 24, "nie", "dokumentation"),
    ("224.0.0.0", 4, "nie", "multicast"),
    ("240.0.0.0", 4, "nie", "reserviert"),
];
const V6: &[(&str, u32, &str, &str)] = &[
    ("::", 128, "nie", "unspezifiziert"),
    ("::1", 128, "geraet", "loopback"),
    ("::ffff:0:0", 96, "@v4", ""),
    ("64:ff9b::", 96, "@nat64", ""),
    ("64:ff9b:1::", 48, "nie", "nat64_lokal"),
    ("100::", 64, "nie", "verwerfen"),
    ("2001::", 23, "nie", "protokoll"),
    ("2001:db8::", 32, "nie", "dokumentation"),
    ("2002::", 16, "nie", "uebergang"),
    ("3fff::", 20, "nie", "dokumentation"),
    ("fc00::", 7, "privat", "privat"),
    ("fe80::", 10, "nie", "link_lokal"),
    ("ff00::", 8, "nie", "multicast"),
];
fn klasse_v4(a: u32) -> (&'static str, &'static str) {
    for &(n, bits, k, b) in V4 {
        let n = u32::from(n.parse::<Ipv4Addr>().expect("konstantes Netz"));
        if a >> (32 - bits) == n >> (32 - bits) {
            return (k, b);
        }
    }
    ("oeffentlich", "oeffentlich")
}
pub fn klasse(ip: &str) -> Option<(&'static str, &'static str)> {
    // Python ipaddress akzeptiert eine IPv6-Zonenkennung, die Klasse bleibt dieselbe.
    let ip = if let Some((adresse, zone)) = ip.split_once('%') {
        if !ip.contains(':') || zone.is_empty() || zone.contains(['%', '/']) {
            return None;
        }
        adresse
    } else {
        ip
    };
    match ip.parse::<IpAddr>().ok()? {
        IpAddr::V4(a) => Some(klasse_v4(u32::from(a))),
        IpAddr::V6(a) => {
            let a = u128::from(a);
            for &(n, bits, k, b) in V6 {
                let n = u128::from(n.parse::<Ipv6Addr>().expect("konstantes Netz"));
                if a >> (128 - bits) != n >> (128 - bits) {
                    continue;
                }
                return Some(match k {
                    "@v4" => klasse_v4(a as u32),
                    "@nat64" => (
                        if klasse_v4(a as u32).0 == "oeffentlich" {
                            "oeffentlich"
                        } else {
                            "nie"
                        },
                        "nat64",
                    ),
                    _ => (k, b),
                });
            }
            Some(if a >> 125 == 1 {
                ("oeffentlich", "oeffentlich")
            } else {
                ("nie", "reserviert")
            })
        }
    }
}
pub fn pruefe_netz(netz: &str, adressen: &[String]) -> &'static str {
    if adressen.is_empty() {
        return "netzfehler";
    }
    if adressen
        .iter()
        .all(|a| klasse(a).is_some_and(|(k, _)| k == netz))
    {
        "ok"
    } else {
        "adresse_abgewiesen"
    }
}
fn dns(h: &str) -> bool {
    if h.is_empty() || h.len() > 253 {
        return false;
    }
    let labels: Vec<_> = h.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        && labels
            .last()
            .is_some_and(|l| l.as_bytes().first().is_some_and(u8::is_ascii_lowercase))
}
fn zeichen(s: &str, pfad: bool) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if unreserved(b[i]) || (!pfad && b"=&+".contains(&b[i])) {
            i += 1;
            continue;
        }
        if b[i] != b'%'
            || i + 2 >= b.len()
            || !b[i + 1..i + 3]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(b))
        {
            return false;
        }
        let v = u8::from_str_radix(&s[i + 1..i + 3], 16).expect("Hex geprueft");
        if unreserved(v) || v <= 31 || v == 127 || (pfad && [47, 92].contains(&v)) {
            return false;
        }
        i += 3;
    }
    true
}
pub fn pruefe_adresse(text: &str, art: &str, netz: &str) -> bool {
    if text.is_empty()
        || text.len() > 500
        || !text.bytes().all(|b| (0x21..=0x7e).contains(&b))
        || text.bytes().any(|b| b"@#\\[]".contains(&b))
    {
        return false;
    }
    let (schema, rest) = if let Some(r) = text.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = text.strip_prefix("http://").filter(|_| netz == "geraet") {
        ("http", r)
    } else {
        return false;
    };
    let Some(slash) = rest.find('/') else {
        return false;
    };
    let (autoritaet, tail) = rest.split_at(slash);
    let host = if let Some((host, port)) = autoritaet.split_once(':') {
        if port.is_empty()
            || port.len() > 5
            || port.starts_with('0')
            || !port.bytes().all(|b| b.is_ascii_digit())
        {
            return false;
        }
        let Ok(port) = port.parse::<u16>() else {
            return false;
        };
        if port == if schema == "https" { 443 } else { 80 } {
            return false;
        }
        host
    } else {
        autoritaet
    };
    let host_ok = match netz {
        "oeffentlich" => dns(host),
        "privat" => {
            dns(host)
                || host.parse::<Ipv4Addr>().is_ok()
                    && klasse(host).is_some_and(|(k, _)| k == "privat")
        }
        "geraet" => host == "localhost" || host == "127.0.0.1",
        _ => false,
    };
    if !host_ok {
        return false;
    }
    let pfad = if let Some((p, q)) = tail.split_once('?') {
        if art != "web" || q.is_empty() || !zeichen(q, false) {
            return false;
        }
        p
    } else {
        tail
    };
    let segmente: Vec<_> = pfad[1..].split('/').collect();
    let mut platzhalter = BTreeSet::new();
    for (i, s) in segmente.iter().enumerate() {
        if s.is_empty() {
            if i + 1 != segmente.len() {
                return false;
            }
        } else if *s == "." || *s == ".." {
            return false;
        } else if art == "http" && s.starts_with('{') && s.ends_with('}') {
            if !parametername(&s[1..s.len() - 1]) || !platzhalter.insert(s) {
                return false;
            }
        } else if !zeichen(s, true) {
            return false;
        }
    }
    true
}
pub fn prozent_kodiere(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if unreserved(b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
