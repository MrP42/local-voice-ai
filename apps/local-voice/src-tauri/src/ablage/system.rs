//! Reservierter Kennungsraum für den privaten Stimmtransport (lva#898).
//! Die Kennung zählt, niemals der sichtbare Ordnername. Normale ULIDs tragen einen Zeitstempel;
//! die reservierten Kennungen beginnen stattdessen mit der festen Domäne `LVSO` bzw. `LVSF`.
use sha2::{Digest, Sha256};

fn ist(id: &str, prefix: &str, magic: &[u8; 4]) -> bool {
    id.strip_prefix(prefix)
        .and_then(|s| ulid::Ulid::from_string(s).ok())
        .is_some_and(|u| u.to_bytes()[..4] == magic[..])
}

pub(crate) fn ist_ordner(id: &str) -> bool { ist(id, "ord_", b"LVSO") }
pub(crate) fn ist_datei(id: &str) -> bool { ist(id, "dat_", b"LVSF") }

/// Kontoschlüssel und Domäne verhindern kontenübergreifende Inhalts- oder Namenskennung.
pub(crate) fn kennung(key: &[u8; 32], ordner: bool, generation: u32, salt: &str) -> String {
    let mut h = Sha256::new();
    h.update(key);
    h.update(b"lv-phonon-sync-v1");
    h.update([u8::from(ordner)]);
    h.update(generation.to_be_bytes());
    h.update(salt.as_bytes());
    let hash = h.finalize();
    let mut bytes = [0u8; 16];
    bytes[..4].copy_from_slice(if ordner { b"LVSO" } else { b"LVSF" });
    bytes[4..].copy_from_slice(&hash[..12]);
    format!("{}{}", if ordner { "ord_" } else { "dat_" }, ulid::Ulid::from_bytes(bytes))
}

pub(crate) fn ablehnen(id: &str) -> super::Res<()> {
    if ist_ordner(id) || ist_datei(id) {
        Err(super::Fehler::neu("ablage.nicht_gefunden", "Datei nicht gefunden."))
    } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kennungen_sind_kontogebunden_deterministisch_und_keine_namenssperre() {
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() { *b = i as u8; }
        // Unabhängig mit Python hashlib/int.to_bytes berechnete Vertragsvektoren.
        assert_eq!(kennung(&key, true, 0, ""), "ord_2CAS9MY90GTRQEBVKFKP5TXZAB");
        assert_eq!(kennung(&key, true, 1, ""), "ord_2CAS9MYNV00X4X0TY6MQE0ECCH");
        assert_eq!(kennung(&key, false, 0, "00112233445566778899aabbccddeeff"),
            "dat_2CAS9MD268N1T7K9V0N4GCDVT8");
        let folder = kennung(&key, true, 0, "");
        assert!(ist_ordner(&folder));
        assert!(super::super::id_gueltig("ord_", &folder));
        key[0] ^= 1;
        assert_ne!(folder, kennung(&key, true, 0, ""));
        assert!(!ist_ordner(".lv-stimmen"));
        assert!(!ist_ordner(&super::super::neue_id("ord_")));
    }
}
