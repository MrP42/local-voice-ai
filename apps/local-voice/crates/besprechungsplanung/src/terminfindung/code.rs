//! Vier Base36-Zeichen haben nur 36^4 Moeglichkeiten (20,7 Bit), keine
//! Authentisierung. TF2 muss den Absender binden, Versuche begrenzen und
//! Weiterleitungen zur Pruefung stellen. ID-Kollisionen beim Anlegen erkennen;
//! gleiche Codes verschiedener Adressen sind moeglich, nie allein danach suchen.

use super::modell::{gueltige_adresse, gueltige_id, gueltiger_kurzcode, normalisiere_adresse};
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const MAX_BETREFF_BYTES: usize = 4096;
type HmacSha256 = Hmac<Sha256>;

fn authentikator(schluessel: &[u8], daten: &[u8]) -> HmacSha256 {
    let mut mac =
        HmacSha256::new_from_slice(schluessel).expect("HMAC akzeptiert jede Schluessellaenge");
    mac.update(daten);
    mac
}

/// Lokal zufaellig erzeugter Schluessel mit mindestens 32 Bytes. Versioniertes,
/// durch NUL getrenntes Eingabeformat; weder Schluessel noch Adressen protokollieren.
pub fn persoenlicher_code(
    schluessel: &[u8],
    umfrage_id: &str,
    adresse: &str,
) -> Result<String, &'static str> {
    if schluessel.len() < 32 || !gueltige_id(umfrage_id) || !gueltige_adresse(adresse) {
        return Err("code_eingabe");
    }
    let daten = format!(
        "terminfindung:v1\0{umfrage_id}\0{}",
        normalisiere_adresse(adresse)
    );
    let digest = authentikator(schluessel, daten.as_bytes())
        .finalize()
        .into_bytes();
    // 128 Bit modulo 36^4; die maximale Modulo-Verzerrung ist kleiner als 2^-107.
    let mut zahl = 0_u32;
    for b in &digest[..16] {
        zahl = (zahl * 256 + u32::from(*b)) % 36_u32.pow(4);
    }
    let mut code = [b'0'; 4];
    for b in code.iter_mut().rev() {
        *b = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"[(zahl % 36) as usize];
        zahl /= 36;
    }
    Ok(code.iter().map(|b| char::from(*b)).collect())
}

/// Der Inhalt des Codes wird durch HMACs gleicher Laenge in konstanter Zeit
/// verglichen (`verify_slice`). Format und Laenge sind oeffentlich.
pub fn pruefe_code(schluessel: &[u8], id: &str, adresse: &str, code: &str) -> bool {
    let Ok(erwartet) = persoenlicher_code(schluessel, id, adresse) else {
        return false;
    };
    if !gueltiger_kurzcode(code) {
        return false;
    }
    let tag = authentikator(schluessel, erwartet.as_bytes())
        .finalize()
        .into_bytes();
    authentikator(schluessel, code.as_bytes())
        .verify_slice(&tag)
        .is_ok()
}

pub fn betreff_marke(id: &str, code: &str) -> Result<String, &'static str> {
    if !gueltige_id(id) || !gueltiger_kurzcode(code) {
        return Err("marke_format");
    }
    Ok(format!("[Terminfindung {id}-{code}]"))
}

/// Genau eine vollstaendige Marke; mehrdeutige Betreffzeilen werden abgelehnt.
pub fn finde_marke(betreff: &str) -> Option<(String, String)> {
    if betreff.len() > MAX_BETREFF_BYTES {
        return None;
    }
    let mut fund = None;
    for (pos, _) in betreff.match_indices("[Terminfindung ") {
        let rest = &betreff[pos + "[Terminfindung ".len()..];
        let kandidat = rest.as_bytes().get(..12)?;
        if kandidat[6] != b'-' || kandidat[11] != b']' {
            return None;
        }
        let id = std::str::from_utf8(&kandidat[..6]).ok()?;
        let code = std::str::from_utf8(&kandidat[7..11]).ok()?;
        if !gueltige_id(id) || !gueltiger_kurzcode(code) || fund.is_some() {
            return None;
        }
        fund = Some((id.into(), code.into()));
    }
    fund
}
