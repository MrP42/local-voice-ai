//! Reine HTTP/1.1-Rahmung ohne Netz, Weiterleitungen oder Protokollwechsel.
use std::io::{ErrorKind, Read};

use super::anbindung::Ziel;
use super::kodierung::Wert;

const MAX_KOPF: usize = 16 * 1024;

/// Baut genau eine Anfrage an die urspruengliche Autoritaet.
pub(crate) fn baue_anfragebytes(ziel: &Ziel, anfrage: &Wert) -> Result<Vec<u8>, &'static str> {
    let methode = anfrage["methode"].text().ok_or("netzfehler")?;
    if !matches!(methode, "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return Err("netzfehler");
    }
    let adresse = anfrage["adresse"].text().ok_or("netzfehler")?;
    let (rest, standard) = if let Some(rest) = adresse.strip_prefix("https://") {
        (rest, 443)
    } else if let Some(rest) = adresse.strip_prefix("http://") {
        (rest, 80)
    } else {
        return Err("netzfehler");
    };
    let (autoritaet, pfad) = rest.split_once('/').unwrap_or((rest, ""));
    if autoritaet.is_empty()
        || autoritaet.contains(['@', '#', '?'])
        || autoritaet.bytes().any(|b| b <= b' ' || b == 127)
        || ziel.host.is_empty()
        || ziel.host.bytes().any(|b| b <= b' ' || b >= 127)
        || ziel.host.contains(['@', '/', '#', '?'])
    {
        return Err("netzfehler");
    }
    let pfad = format!("/{pfad}");
    if pfad.bytes().any(|b| b <= b' ' || b == 127 || b == b'#') {
        return Err("netzfehler");
    }
    let host = if ziel.port == standard {
        ziel.host.clone()
    } else {
        format!("{}:{}", ziel.host, ziel.port)
    };
    let mut kopf = format!("{methode} {pfad} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    let Wert::Liste(zeilen) = &anfrage["kopfzeilen"] else {
        return Err("netzfehler");
    };
    for zeile in zeilen {
        let Wert::Liste(paar) = zeile else {
            return Err("netzfehler");
        };
        if paar.len() != 2 {
            return Err("netzfehler");
        }
        let name = paar[0].text().ok_or("netzfehler")?;
        let wert = paar[1].text().ok_or("netzfehler")?;
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || !wert.bytes().all(|b| (32..=126).contains(&b))
            || matches!(
                name.to_ascii_lowercase().as_str(),
                "host"
                    | "connection"
                    | "content-length"
                    | "transfer-encoding"
                    | "upgrade"
                    | "te"
                    | "expect"
                    | "accept-encoding"
                    | "proxy-connection"
                    | "keep-alive"
                    | "trailer"
            )
        {
            return Err("netzfehler");
        }
        kopf.push_str(&format!("{name}: {wert}\r\n"));
    }
    let koerper = match &anfrage["koerper"] {
        Wert::Null => None,
        Wert::Text(text) => Some(text.as_bytes()),
        _ => return Err("netzfehler"),
    };
    if let Some(bytes) = koerper {
        kopf.push_str(&format!("Content-Length: {}\r\n", bytes.len()));
    }
    kopf.push_str("\r\n");
    let mut bytes = kopf.into_bytes();
    if let Some(koerper) = koerper {
        bytes.extend_from_slice(koerper);
    }
    Ok(bytes)
}

/// Nur Status und Koerper verlassen die Rahmung.
#[derive(Debug, PartialEq)]
pub(crate) struct Rohantwort {
    pub status: u16,
    pub koerper: Vec<u8>,
}

struct Leser<'a, R> {
    quelle: &'a mut R,
    warten: &'a mut dyn FnMut() -> Result<(), &'static str>,
}

impl<R: Read> Leser<'_, R> {
    fn lesen(&mut self, bytes: &mut [u8]) -> Result<usize, &'static str> {
        loop {
            match self.quelle.read(bytes) {
                Ok(n) => return Ok(n),
                Err(e) => match e.kind() {
                    ErrorKind::Interrupted => continue,
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => (self.warten)()?,
                    ErrorKind::UnexpectedEof => return Ok(0),
                    _ => return Err("netzfehler"),
                },
            }
        }
    }

    // Ein Byte je Schritt verhindert Vorablesen hinter Kopf und Groessenzeile.
    fn zeile(&mut self, rest: &mut usize) -> Result<Vec<u8>, &'static str> {
        let mut zeile = Vec::new();
        loop {
            if *rest == 0 {
                return Err("netzfehler");
            }
            let mut byte = [0];
            if self.lesen(&mut byte)? == 0 {
                return Err("netzfehler");
            }
            *rest -= 1;
            zeile.push(byte[0]);
            if byte[0] == b'\n' {
                if zeile.len() < 2 || zeile[zeile.len() - 2] != b'\r' {
                    return Err("netzfehler");
                }
                zeile.truncate(zeile.len() - 2);
                if zeile.contains(&b'\r') {
                    return Err("netzfehler");
                }
                return Ok(zeile);
            }
        }
    }

    fn genau(&mut self, anzahl: usize, koerper: &mut Vec<u8>) -> Result<(), &'static str> {
        let mut rest = anzahl;
        let mut puffer = [0; 8192];
        while rest > 0 {
            let n = self.lesen(&mut puffer[..rest.min(8192)])?;
            if n == 0 {
                return Err("netzfehler");
            }
            koerper.extend_from_slice(&puffer[..n]);
            rest -= n;
        }
        Ok(())
    }
}

fn dezimal(text: &str) -> Result<usize, &'static str> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err("netzfehler");
    }
    text.parse().map_err(|_| "netzfehler")
}

/// Liest begrenzt und meldet Wartefehler unveraendert an den Aufrufer.
pub(crate) fn lies_antwort(
    quelle: &mut impl Read,
    max_koerper: usize,
    warten: &mut dyn FnMut() -> Result<(), &'static str>,
) -> Result<Rohantwort, &'static str> {
    let max_koerper = max_koerper.min(1_048_576);
    let mut leser = Leser { quelle, warten };
    let mut rest = MAX_KOPF;
    let zeile = leser.zeile(&mut rest)?;
    let statuszeile = std::str::from_utf8(&zeile).map_err(|_| "netzfehler")?;
    let status = statuszeile
        .strip_prefix("HTTP/1.1 ")
        .or_else(|| statuszeile.strip_prefix("HTTP/1.0 "))
        .ok_or("netzfehler")?;
    let (code, _) = status.split_once(' ').ok_or("netzfehler")?;
    if code.len() != 3 {
        return Err("netzfehler");
    }
    let status = dezimal(code)?;
    if !(100..=599).contains(&status) {
        return Err("netzfehler");
    }
    let mut laenge = None;
    let mut chunked = false;
    loop {
        let zeile = leser.zeile(&mut rest)?;
        if zeile.is_empty() {
            break;
        }
        let trenner = zeile.iter().position(|b| *b == b':').ok_or("netzfehler")?;
        let name = &zeile[..trenner];
        if name.is_empty()
            || !name
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(b))
        {
            return Err("netzfehler");
        }
        let name = std::str::from_utf8(name).map_err(|_| "netzfehler")?;
        let name = name.to_ascii_lowercase();
        if !matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "content-encoding"
        ) {
            continue;
        }
        let wert = std::str::from_utf8(&zeile[trenner + 1..]).map_err(|_| "netzfehler")?;
        let wert = wert.trim_matches([' ', '\t']);
        match name.as_str() {
            "content-length" => {
                let n = dezimal(wert)?;
                if laenge.is_some_and(|alt| alt != n) {
                    return Err("netzfehler");
                }
                laenge = Some(n);
            }
            "transfer-encoding" => {
                if !wert.eq_ignore_ascii_case("chunked") || chunked {
                    return Err("antwort_ungueltig");
                }
                chunked = true;
            }
            "content-encoding" if !wert.eq_ignore_ascii_case("identity") => {
                return Err("antwort_ungueltig");
            }
            _ => {}
        }
    }
    if chunked && laenge.is_some() {
        return Err("netzfehler");
    }
    let mut antwort = Rohantwort {
        status: status as u16,
        koerper: Vec::new(),
    };
    if status < 200 || matches!(status, 204 | 304) {
        return Ok(antwort);
    }
    if chunked {
        loop {
            let mut rest = MAX_KOPF;
            let zeile = leser.zeile(&mut rest)?;
            let groesse = zeile.split(|b| *b == b';').next().ok_or("netzfehler")?;
            if groesse.is_empty() || !groesse.iter().all(u8::is_ascii_hexdigit) {
                return Err("netzfehler");
            }
            let groesse = std::str::from_utf8(groesse).map_err(|_| "netzfehler")?;
            let n = usize::from_str_radix(groesse, 16).map_err(|_| "netzfehler")?;
            if n == 0 {
                let mut rest = MAX_KOPF;
                while !leser.zeile(&mut rest)?.is_empty() {}
                break;
            }
            if n > max_koerper - antwort.koerper.len() {
                return Err("zu_gross");
            }
            leser.genau(n, &mut antwort.koerper)?;
            let mut rest = 2;
            if !leser.zeile(&mut rest)?.is_empty() {
                return Err("netzfehler");
            }
        }
    } else if let Some(n) = laenge {
        if n > max_koerper {
            return Err("zu_gross");
        }
        leser.genau(n, &mut antwort.koerper)?;
    } else {
        let mut puffer = [0; 8192];
        loop {
            let rest = max_koerper - antwort.koerper.len();
            let n = leser.lesen(&mut puffer[..rest.saturating_add(1).min(8192)])?;
            if n == 0 {
                break;
            }
            if n > rest {
                return Err("zu_gross");
            }
            antwort.koerper.extend_from_slice(&puffer[..n]);
        }
    }
    Ok(antwort)
}
