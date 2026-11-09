use crate::KontoFehler;

const MAX_NAME: usize = 100;
const MAX_EMAIL: usize = 254;
const MAX_TELEFON: usize = 40;

/// Obergrenze der Zeichenzahl je Feld.
pub fn limit(feld: &str) -> usize {
    match feld {
        "email" => MAX_EMAIL,
        "telefon" | "mobil" => MAX_TELEFON,
        _ => MAX_NAME,
    }
}

/// Trimmt, prüft Steuerzeichen und Länge; leerer Text ergibt `None`.
pub fn text(feld: &'static str, wert: &str) -> Result<Option<String>, KontoFehler> {
    let w = wert.trim();
    if w.is_empty() {
        return Ok(None);
    }
    if w.chars().any(char::is_control) {
        return Err(KontoFehler::Ungueltig {
            feld,
            grund: "Steuerzeichen nicht erlaubt",
        });
    }
    if w.chars().count() > limit(feld) {
        return Err(KontoFehler::Ungueltig {
            feld,
            grund: "zu lang",
        });
    }
    if feld == "email" && !email_ok(w) {
        return Err(KontoFehler::Ungueltig {
            feld,
            grund: "keine gültige E-Mail-Adresse",
        });
    }
    Ok(Some(w.to_string()))
}

/// Pflichtfeld: leer ist ein Fehler.
pub fn pflicht(feld: &'static str, wert: &str) -> Result<String, KontoFehler> {
    text(feld, wert)?.ok_or(KontoFehler::Ungueltig {
        feld,
        grund: "darf nicht leer sein",
    })
}

fn email_ok(w: &str) -> bool {
    let mut teile = w.split('@');
    let (Some(lokal), Some(domain), None) = (teile.next(), teile.next(), teile.next()) else {
        return false;
    };
    !lokal.is_empty() && !domain.is_empty() && !w.chars().any(char::is_whitespace)
}

/// Profil-IDs sind 26 Zeichen Crockford-Base32; schützt Dateinamen vor Pfadtricks.
pub fn id_ok(id: &str) -> bool {
    id.len() == 26
        && id
            .bytes()
            .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
}
