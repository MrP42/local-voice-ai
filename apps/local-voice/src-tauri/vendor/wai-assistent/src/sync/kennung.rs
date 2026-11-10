//! 128-Bit-Kennungen (32 Hexzeichen) und Zufall.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::fehler::SyncFehler;

/// Kennung aus 32 kleinen Hexzeichen (`Hex128` des Vertrags `assistent-sync-object`).
///
/// Zufaellig erzeugt oder per [`Hex128::abgeleitet`] stabil aus einer vorhandenen Kennung.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hex128(String);

impl Hex128 {
    /// Prueft Form (genau 32 Zeichen aus `0-9a-f`).
    pub fn parse(s: &str) -> Result<Self, SyncFehler> {
        let gueltig = s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if gueltig {
            Ok(Self(s.to_owned()))
        } else {
            Err(SyncFehler::UngueltigeKennung)
        }
    }

    /// Neue zufaellige Kennung (128 Bit).
    pub fn zufall() -> Result<Self, SyncFehler> {
        Ok(Self::aus_bytes(&zufall::<16>()?))
    }

    /// Stabile, schluesselabhaengige Kennung: `SHA-256(salz, zweck, eingabe)`, auf 128 Bit gekuerzt.
    ///
    /// Gleiche Eingabe und gleiches Salz ergeben auf jedem Geraet dieselbe Kennung (Migration, E11);
    /// ohne das Kontosalz ist kein Bezug zur Eingabe herstellbar.
    pub fn abgeleitet(salz: &[u8], zweck: &str, eingabe: &str) -> Self {
        let mut h = Sha256::new();
        h.update(b"wai-assistent-sync/id/v1");
        for teil in [salz, zweck.as_bytes(), eingabe.as_bytes()] {
            h.update((teil.len() as u32).to_be_bytes());
            h.update(teil);
        }
        Self::aus_bytes(&h.finalize()[..16])
    }

    pub(crate) fn aus_bytes(b: &[u8]) -> Self {
        Self(hex(b))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Hex128 {
    type Error = SyncFehler;
    fn try_from(s: String) -> Result<Self, SyncFehler> {
        Self::parse(&s)
    }
}

impl From<Hex128> for String {
    fn from(h: Hex128) -> String {
        h.0
    }
}

impl fmt::Display for Hex128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Hex128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hex128({})", self.0)
    }
}

/// Bytes als kleine Hexzeichen.
pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Liest genau `N` Bytes aus `2 * N` kleinen Hexzeichen.
pub(crate) fn aus_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != 2 * N || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// Liest ein Transportobjekt erst, nachdem die Rohlaenge geprueft ist (E3.8, Must-fix 7): ein Hub kann so
/// keine uebergrossen Zeichenketten oder Listen in den Speicher zwingen, bevor eine Vertragspruefung greift.
pub(crate) fn begrenzt_lesen<T: serde::de::DeserializeOwned>(
    roh: &[u8],
    max: usize,
) -> Result<T, SyncFehler> {
    if roh.len() > max {
        return Err(SyncFehler::Vertrag("transportgrenze ueberschritten"));
    }
    serde_json::from_slice(roh).map_err(|_| SyncFehler::Vertrag("transportobjekt unlesbar"))
}

/// Zaehlt geschriebene Bytes (Groesse einer Serialisierung vorab bestimmen).
struct Laenge(usize);

impl std::io::Write for Laenge {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0 += b.len();
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Serialisiert Klartext oder Geheimnisse als JSON in einen genau bemessenen, beim Freigeben
/// ueberschriebenen Puffer (E3.8, Must-fix 7, Runde 3): erst zaehlen, dann in den vorab reservierten
/// Puffer schreiben. Der Puffer waechst nie, also bleiben keine unueberschriebenen Teilkopien im
/// freigegebenen Speicher (anders als bei `serde_json::to_vec`, das beim Wachsen umkopiert).
pub(crate) fn json_geheim<T: Serialize + ?Sized>(
    wert: &T,
) -> Result<zeroize::Zeroizing<Vec<u8>>, serde_json::Error> {
    let mut n = Laenge(0);
    serde_json::to_writer(&mut n, wert)?;
    let mut puffer = zeroize::Zeroizing::new(Vec::with_capacity(n.0));
    serde_json::to_writer(&mut *puffer, wert)?;
    debug_assert_eq!(puffer.len(), n.0);
    Ok(puffer)
}

/// Geheimnis als Hex, ohne ungeschuetzte Zwischenkopie: eine vorab bemessene, beim Freigeben
/// ueberschriebene Zeichenkette (kein `format!` je Byte).
pub(crate) fn hex_geheim(b: &[u8]) -> zeroize::Zeroizing<String> {
    const ZIFFERN: &[u8; 16] = b"0123456789abcdef";
    let mut s = zeroize::Zeroizing::new(String::with_capacity(2 * b.len()));
    for x in b {
        s.push(char::from(ZIFFERN[usize::from(x >> 4)]));
        s.push(char::from(ZIFFERN[usize::from(x & 0x0f)]));
    }
    s
}

/// 32 geheime Bytes, als Hex gesichert (E3.8, Must-fix 7).
///
/// Beim Lesen wird direkt aus der Eingabe dekodiert, ohne eigene `String`-Kopie; eine vom Deserializer
/// uebergebene eigene Zeichenkette wird sofort ueberschrieben. Bricht das Lesen eines umgebenden Objekts
/// spaeter ab, wird jedes schon gelesene Feld beim Freigeben ueberschrieben.
pub(crate) struct GeheimHex(pub(crate) zeroize::Zeroizing<[u8; 32]>);

impl serde::Serialize for GeheimHex {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex_geheim(self.0.as_ref()))
    }
}

impl<'de> serde::Deserialize<'de> for GeheimHex {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Besucher;
        impl serde::de::Visitor<'_> for Besucher {
            type Value = GeheimHex;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("64 kleine Hexzeichen")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<GeheimHex, E> {
                let mut b = zeroize::Zeroizing::new([0u8; 32]);
                *b = aus_hex::<32>(v).ok_or_else(|| E::custom("geheimnis"))?;
                Ok(GeheimHex(b))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<GeheimHex, E> {
                let v = zeroize::Zeroizing::new(v);
                self.visit_str(&v)
            }
        }
        d.deserialize_str(Besucher)
    }
}

/// Serde fuer `[u8; N]` als Hexzeichenkette (oeffentliche Schluessel, Hashes, Signaturen).
pub(crate) mod hexfeld {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer, const N: usize>(v: &[u8; N], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::hex(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        d: D,
    ) -> Result<[u8; N], D::Error> {
        let s = String::deserialize(d)?;
        super::aus_hex::<N>(&s).ok_or_else(|| D::Error::custom("Hexfeld hat falsche Form"))
    }
}

/// SHA-256 ueber Teile mit 4-Byte-Laengenpraefix hinter einer Domaenenkennung.
pub(crate) fn hash_teile(kennung: &[u8], teile: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(kennung);
    for t in teile {
        h.update((t.len() as u32).to_be_bytes());
        h.update(t);
    }
    h.finalize().into()
}

/// `N` Zufallsbytes aus dem Betriebssystem.
pub(crate) fn zufall<const N: usize>() -> Result<[u8; N], SyncFehler> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|_| SyncFehler::Zufall)?;
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_geheim_schreibt_ohne_wachsen_in_den_genau_bemessenen_puffer() {
        // Gross genug, dass `to_vec` mehrfach umkopieren wuerde.
        let wert = serde_json::json!({
            "geheim": "x".repeat(10_000),
            "liste": (0..500).collect::<Vec<u32>>(),
        });
        let puffer = json_geheim(&wert).unwrap();
        assert_eq!(*puffer, serde_json::to_vec(&wert).unwrap());
        assert_eq!(
            puffer.capacity(),
            puffer.len(),
            "genau einmal, genau passend reserviert"
        );
    }

    #[test]
    fn parse_prueft_form() {
        assert!(Hex128::parse("a1b2c3d4e5f60718293a4b5c6d7e8f90").is_ok());
        assert!(Hex128::parse("A1b2c3d4e5f60718293a4b5c6d7e8f90").is_err());
        assert!(Hex128::parse("abc").is_err());
        assert!(Hex128::parse("g1b2c3d4e5f60718293a4b5c6d7e8f90").is_err());
    }

    #[test]
    fn zufall_ist_gueltig_und_verschieden() {
        let (a, b) = (Hex128::zufall().unwrap(), Hex128::zufall().unwrap());
        assert_ne!(a, b);
        assert!(Hex128::parse(a.as_str()).is_ok());
    }

    #[test]
    fn abgeleitet_ist_stabil_und_getrennt() {
        let a = Hex128::abgeleitet(b"salz", "beitrag", "msg_1");
        assert_eq!(a, Hex128::abgeleitet(b"salz", "beitrag", "msg_1"));
        assert_ne!(a, Hex128::abgeleitet(b"salz", "beitrag", "msg_2"));
        assert_ne!(a, Hex128::abgeleitet(b"salz", "unterhaltung", "msg_1"));
        assert_ne!(a, Hex128::abgeleitet(b"anderes", "beitrag", "msg_1"));
        // Laengenpraefix verhindert Grenzverschiebung zwischen den Teilen.
        assert_ne!(
            Hex128::abgeleitet(b"s", "ab", "c"),
            Hex128::abgeleitet(b"s", "a", "bc")
        );
    }

    #[test]
    fn serde_prueft_beim_lesen() {
        let ok: Hex128 = serde_json::from_str("\"a1b2c3d4e5f60718293a4b5c6d7e8f90\"").unwrap();
        assert_eq!(ok.as_str().len(), 32);
        assert!(serde_json::from_str::<Hex128>("\"zz\"").is_err());
    }
}
