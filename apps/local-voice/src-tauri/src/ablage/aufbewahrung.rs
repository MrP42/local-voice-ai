//! Aufbewahrung von Fassungen (Spec §6, ADR-0029 E9), reine Funktion ohne Uhr und I/O.
//!
//! Eine Fassung bleibt, wenn sie Kopf ist, von einer aktiven Wahl gewählt wurde, jüngste gemeinsame
//! Vorgängerin offener Köpfe ist, zu den [`JUENGSTE`] jüngsten der Datei gehört oder jünger als
//! [`TAGE`] Tage ist. Alles andere wird entfernt (`fas` kommt in `entfernt`).
//!
//! Zusätzlich gilt der Pfadschluss: liegt eine behaltene Fassung unter einer anderen behaltenen
//! (Vorgängerkette), bleiben auch alle dazwischen. Sonst hätte die behaltene ältere Fassung keinen
//! lebenden Nachfolger mehr und erschiene fälschlich als Kopf (Konflikt ohne Anlass).

// D4 (Sync) und D3 (KI) nutzen diese Funktionen; D1 liefert Logik und Tests (Spec §9).
#![allow(dead_code)]

use super::zusammenfuehren::{aktive_wahlen, gemeinsame_vorgaengerin, koepfe, Datei};
use std::collections::{BTreeMap, BTreeSet};

pub const JUENGSTE: usize = 10;
pub const TAGE: i64 = 90;
const TAG_MS: i64 = 24 * 60 * 60 * 1000;

/// IDs der Fassungen, die behalten werden.
pub fn behalten(d: &Datei, jetzt_ms: i64) -> BTreeSet<String> {
    let mut b: BTreeSet<String> = BTreeSet::new();
    b.extend(koepfe(d));
    b.extend(aktive_wahlen(d).into_iter().map(|w| w.gewaehlt.clone()));
    b.extend(gemeinsame_vorgaengerin(d));
    let mut nach_zeit: Vec<(&i64, &str)> = d
        .fassungen
        .iter()
        .map(|f| (&f.zeit, f.fas.as_str()))
        .collect();
    nach_zeit.sort_by(|a, b| b.cmp(a));
    b.extend(nach_zeit.iter().take(JUENGSTE).map(|(_, f)| f.to_string()));
    b.extend(
        d.fassungen
            .iter()
            .filter(|f| jetzt_ms - f.zeit < TAGE * TAG_MS)
            .map(|f| f.fas.clone()),
    );
    pfadschluss(d, b)
}

fn pfadschluss(d: &Datei, mut b: BTreeSet<String>) -> BTreeSet<String> {
    let je: BTreeMap<&str, Option<&str>> = d
        .fassungen
        .iter()
        .map(|f| (f.fas.as_str(), f.vorgaenger.as_deref()))
        .collect();
    let start: Vec<String> = b.iter().cloned().collect();
    for k in start {
        let mut pfad: Vec<&str> = Vec::new();
        let mut aktuell = je.get(k.as_str()).copied().flatten();
        let mut schritte = 0usize;
        while let Some(id) = aktuell {
            if !je.contains_key(id) || schritte > je.len() {
                break;
            }
            if b.contains(id) {
                b.extend(pfad.iter().map(|s| s.to_string()));
                break;
            }
            pfad.push(id);
            aktuell = je.get(id).copied().flatten();
            schritte += 1;
        }
    }
    b
}

/// IDs der Fassungen, die jetzt entfernt werden (nach `fas` sortiert).
pub fn zu_entfernen(d: &Datei, jetzt_ms: i64) -> Vec<String> {
    let b = behalten(d, jetzt_ms);
    let mut weg: Vec<String> = d
        .fassungen
        .iter()
        .filter(|f| !b.contains(&f.fas))
        .map(|f| f.fas.clone())
        .collect();
    weg.sort();
    weg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ablage::zusammenfuehren::{blob_refs_aus, Fassung, Register};

    const JETZT: i64 = 1_800_000_000_000;

    fn alt(tage: i64) -> i64 {
        JETZT - tage * TAG_MS
    }

    fn fas(id: &str, vorgaenger: Option<&str>, tage: i64) -> Fassung {
        Fassung {
            fas: id.into(),
            vorgaenger: vorgaenger.map(Into::into),
            blob: format!("b-{id}"),
            size: 1,
            original_sha256: id.into(),
            typ: "pdf".into(),
            geraet: "g_1".into(),
            zeit: alt(tage),
            texte: vec![],
        }
    }

    fn datei(fassungen: Vec<Fassung>) -> Datei {
        Datei {
            id: "dat_1".into(),
            ordner: Register::neu("ord_1".into(), 1, "g_1"),
            name: Register::neu("x.pdf".into(), 1, "g_1"),
            klasse_min: Register::neu("internal".into(), 1, "g_1"),
            nur_geraet_min: false,
            blob_refs: blob_refs_aus(&fassungen),
            fassungen,
            wahlen: vec![],
            entfernt: vec![],
            papierkorb: Register::neu(None, 0, "g_1"),
        }
    }

    #[test]
    fn einzelne_alte_kette_behaelt_zehn_juengste_und_kopf() {
        // 14 Fassungen, alle älter als 90 Tage: die 10 jüngsten bleiben, 4 gehen.
        let mut v = vec![fas("f00", None, 400)];
        for i in 1..14 {
            let vor = format!("f{:02}", i - 1);
            v.push(fas(&format!("f{i:02}"), Some(&vor), 400 - i as i64));
        }
        let d = datei(v);
        let weg = zu_entfernen(&d, JETZT);
        assert_eq!(weg, ["f00", "f01", "f02", "f03"]);
    }

    #[test]
    fn juenger_als_90_tage_bleibt() {
        let mut v = vec![fas("a0", None, 200)];
        for i in 1..15 {
            let vor = format!("a{}", i - 1);
            v.push(fas(&format!("a{i}"), Some(&vor), 80));
        }
        // a0 ist alt und gehört nicht zu den 10 jüngsten (14 jüngere), a1..a14 sind jünger als 90 Tage.
        let weg = zu_entfernen(&datei(v), JETZT);
        assert_eq!(weg, ["a0"]);
    }

    #[test]
    fn gemeinsame_vorgaengerin_bleibt_bei_12_neueren_und_120_tagen() {
        // x0 (130 Tage) <- F (120 Tage); zwei Zweige mit je 6 Fassungen, alle 100..110 Tage alt:
        // 12 neuere als F, alle älter als 90 Tage. F ist nicht unter den 10 jüngsten.
        let mut v = vec![fas("x0", None, 130), fas("F", Some("x0"), 120)];
        let mut vor_a = "F".to_string();
        let mut vor_b = "F".to_string();
        for i in 1..=6 {
            let a = format!("A{i}");
            let b = format!("B{i}");
            v.push(fas(&a, Some(&vor_a), 110 - i as i64));
            v.push(fas(&b, Some(&vor_b), 110 - i as i64));
            vor_a = a;
            vor_b = b;
        }
        let d = datei(v);
        assert_eq!(koepfe(&d), ["A6", "B6"]);
        assert_eq!(gemeinsame_vorgaengerin(&d).as_deref(), Some("F"));
        let weg = zu_entfernen(&d, JETZT);
        assert!(
            !weg.contains(&"F".to_string()),
            "gemeinsame Vorgängerin bleibt"
        );
        assert!(weg.contains(&"x0".to_string()), "ältere Vorgängerin geht");
        // Nach dem Entfernen bleiben dieselben zwei Köpfe (kein falscher Kopf durch die Lücke).
        let mut danach = d.clone();
        danach.entfernt = weg.clone();
        danach.fassungen.retain(|f| !weg.contains(&f.fas));
        assert_eq!(koepfe(&danach), ["A6", "B6"]);
        assert_eq!(gemeinsame_vorgaengerin(&danach).as_deref(), Some("F"));
    }

    #[test]
    fn pfadschluss_haelt_die_kette_zur_behaltenen_vorgaengerin() {
        let mut v = vec![fas("F", None, 200)];
        let mut vor_a = "F".to_string();
        let mut vor_b = "F".to_string();
        for i in 1..=8 {
            let a = format!("A{i}");
            let b = format!("B{i}");
            v.push(fas(&a, Some(&vor_a), 150 - i as i64));
            v.push(fas(&b, Some(&vor_b), 150 - i as i64));
            vor_a = a;
            vor_b = b;
        }
        let d = datei(v);
        let b = behalten(&d, JETZT);
        // Auch die alten Mitglieder der Zweige bleiben, weil F (gemeinsame Vorgängerin) bleibt.
        assert_eq!(b.len(), d.fassungen.len());
    }
}
