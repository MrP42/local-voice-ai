//! `freigabe.json`: gerätelokale KI-Freigaben je Ordner (Spec §5.1, ADR-0029 E6). Nur Speicherung und
//! Auswertung; die KI-Anbindung folgt in D3.
//!
//! * Je Ordner `an`, `aus` oder `gesperrt`; fehlender Eintrag = `aus`. Vererbung nach unten; ein näherer
//!   Eintrag gewinnt, `gesperrt` gewinnt immer.
//! * Vor jedem Schreiben wird die letzte gültige Fassung als `freigabe.json.vorher` abgelegt (Temp +
//!   Umbenennen).
//! * Defekt, nicht lesbar oder neuer als bekannt = „Freigabe unklar“: die ganze Ablage gilt auf beiden
//!   KI-Wegen als gesperrt. Ist die Datei defekt (oder fehlt sie trotz vorhandener `.vorher`) und die
//!   `.vorher` gültig, wird sie automatisch wiederhergestellt (neue Epoche, die defekte Datei wird
//!   beiseitegelegt). Sonst nur über [`neu_festlegen`] (alle Ordner `aus`).
//!
//! Die Funktionen sperren nicht selbst; der Aufrufer hält die Sperre der Ablage.

// `global_erlaubt` und `projekt_erlaubt` sind die Abfragen für D3 (KI-Zugriff).
#![allow(dead_code)]

use super::{jetzt_ms, schreibe_atomar, Fehler, Res};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const DATEI: &str = "freigabe.json";
pub const VORHER: &str = "freigabe.json.vorher";
const VERSION: u32 = 1;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum KiStufe {
    An,
    Aus,
    Gesperrt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Freigaben {
    pub v: u32,
    pub epoche: u64,
    #[serde(default)]
    pub ordner: BTreeMap<String, KiStufe>,
}

impl Freigaben {
    fn leer(epoche: u64) -> Self {
        Self {
            v: VERSION,
            epoche,
            ordner: BTreeMap::new(),
        }
    }
}

/// Warum die Freigabe unklar ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum KiUnklar {
    /// Kaputt, ungültig oder gelöscht, obwohl es eine `.vorher` gibt.
    Defekt,
    /// Gerade nicht lesbar (Rechte, Sperre, Datenträger): später erneut.
    Unlesbar,
    /// Von einer neueren App-Version.
    Neuer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lage {
    Gueltig(Freigaben),
    Unklar(KiUnklar),
}

impl Lage {
    pub fn ist_unklar(&self) -> bool {
        matches!(self, Lage::Unklar(_))
    }
}

enum Lesen {
    Ok(Freigaben),
    Fehlt,
    Mangel(KiUnklar),
}

fn lesen(pfad: &Path) -> Lesen {
    let roh = match fs::read(pfad) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Lesen::Fehlt,
        Err(_) => return Lesen::Mangel(KiUnklar::Unlesbar),
    };
    #[derive(Deserialize)]
    struct NurVersion {
        v: u32,
    }
    match serde_json::from_slice::<NurVersion>(&roh) {
        Ok(n) if n.v > VERSION => return Lesen::Mangel(KiUnklar::Neuer),
        Ok(_) => {}
        Err(_) => return Lesen::Mangel(KiUnklar::Defekt),
    }
    match serde_json::from_slice::<Freigaben>(&roh) {
        Ok(f)
            if f.ordner
                .keys()
                .all(|k| super::id_gueltig(super::PRAEFIX_ORDNER, k)) =>
        {
            Lesen::Ok(f)
        }
        _ => Lesen::Mangel(KiUnklar::Defekt),
    }
}

fn beiseitelegen(dir: &Path) -> Res<()> {
    let pfad = dir.join(DATEI);
    if fs::symlink_metadata(&pfad).is_err() {
        return Ok(());
    }
    let ziel = dir.join(format!("{DATEI}.defekt-{}", jetzt_ms()));
    if fs::symlink_metadata(&ziel).is_ok() {
        return Err(Fehler::speicher(
            "Defekte Freigabedatei nicht beiseitelegbar.",
        ));
    }
    fs::rename(&pfad, &ziel)
        .map_err(|_| Fehler::speicher("Defekte Freigabedatei nicht beiseitelegbar."))
}

fn schreiben(dir: &Path, f: &Freigaben) -> Res<()> {
    let roh = serde_json::to_vec_pretty(f)
        .map_err(|_| Fehler::speicher("Freigaben nicht speicherbar."))?;
    schreibe_atomar(&dir.join(DATEI), &roh)
}

/// Lädt die Freigaben. Eine fehlende Datei ohne `.vorher` ist ein gültiger leerer Stand (alles `aus`).
/// Defekt (oder gelöscht trotz `.vorher`) und gültige `.vorher`: automatische Wiederherstellung.
pub fn laden(dir: &Path) -> Lage {
    let vorher = dir.join(VORHER);
    match lesen(&dir.join(DATEI)) {
        Lesen::Ok(f) => Lage::Gueltig(f),
        Lesen::Mangel(KiUnklar::Defekt) => wiederherstellen_oder(dir, KiUnklar::Defekt),
        Lesen::Mangel(g) => Lage::Unklar(g),
        Lesen::Fehlt => {
            if fs::symlink_metadata(&vorher).is_ok() {
                wiederherstellen_oder(dir, KiUnklar::Defekt)
            } else {
                Lage::Gueltig(Freigaben::leer(0))
            }
        }
    }
}

fn wiederherstellen_oder(dir: &Path, sonst: KiUnklar) -> Lage {
    match lesen(&dir.join(VORHER)) {
        Lesen::Ok(v) => {
            let neu = Freigaben {
                v: VERSION,
                epoche: v.epoche + 2,
                // Fail-closed (Spec §5.1): alle wiederhergestellten Einträge stehen auf `gesperrt`.
                ordner: v
                    .ordner
                    .into_keys()
                    .map(|k| (k, KiStufe::Gesperrt))
                    .collect(),
            };
            if beiseitelegen(dir).is_ok() && schreiben(dir, &neu).is_ok() {
                Lage::Gueltig(neu)
            } else {
                Lage::Unklar(sonst)
            }
        }
        _ => Lage::Unklar(sonst),
    }
}

/// Setzt die Stufe eines Ordners (`None` entfernt den Eintrag: der Ordner erbt wieder). Verweigert bei
/// unklarer Freigabe. Legt vor dem Schreiben die letzte gültige Fassung als `.vorher` ab.
pub fn setzen(dir: &Path, ordner: &str, stufe: Option<KiStufe>) -> Res<Freigaben> {
    let Lage::Gueltig(mut f) = laden(dir) else {
        return Err(Fehler::neu(
            "ablage.freigabe_unklar",
            "Die KI-Freigabe ist unklar; sie muss zuerst neu festgelegt werden.",
        ));
    };
    if !super::id_gueltig(super::PRAEFIX_ORDNER, ordner) {
        return Err(Fehler::ungueltig("Kennung des Ordners ungültig."));
    }
    vorher_ablegen(dir)?;
    match stufe {
        Some(s) => {
            f.ordner.insert(ordner.to_string(), s);
        }
        None => {
            f.ordner.remove(ordner);
        }
    }
    f.epoche += 1;
    schreiben(dir, &f)?;
    Ok(f)
}

/// Entfernt Einträge gelöschter Ordner (Aufräumen nach endgültigem Löschen); ändert sonst nichts.
pub fn vergessen(dir: &Path, ordner: &[String]) -> Res<()> {
    let Lage::Gueltig(mut f) = laden(dir) else {
        return Ok(());
    };
    if !ordner.iter().any(|o| f.ordner.contains_key(o)) {
        return Ok(());
    }
    vorher_ablegen(dir)?;
    for o in ordner {
        f.ordner.remove(o);
    }
    f.epoche += 1;
    schreiben(dir, &f)
}

fn vorher_ablegen(dir: &Path) -> Res<()> {
    let pfad = dir.join(DATEI);
    match fs::read(&pfad) {
        Ok(roh) => schreibe_atomar(&dir.join(VORHER), &roh),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Fehler::speicher("Freigaben nicht lesbar; nichts geändert.")),
    }
}

/// „Freigaben neu festlegen“: nur bei unklarer, defekter Freigabe (nie bei neuerer Version oder
/// vorübergehend nicht lesbarer Datei). Die defekte Datei wird beiseitegelegt, nie gelöscht; danach
/// stehen alle Ordner auf `aus`.
pub fn neu_festlegen(dir: &Path, alle_ordner: &[String]) -> Res<Freigaben> {
    match laden(dir) {
        Lage::Gueltig(_) => {
            return Err(Fehler::neu(
                "ablage.freigabe_klar",
                "Die KI-Freigabe ist lesbar; nichts neu festzulegen.",
            ))
        }
        Lage::Unklar(KiUnklar::Neuer) | Lage::Unklar(KiUnklar::Unlesbar) => {
            return Err(Fehler::neu(
                "ablage.freigabe_nicht_neu",
                "Die KI-Freigabe wird so nicht neu festgelegt; nichts angefasst.",
            ))
        }
        Lage::Unklar(KiUnklar::Defekt) => {}
    }
    let letzte = match lesen(&dir.join(VORHER)) {
        Lesen::Ok(v) => v.epoche,
        _ => 0,
    };
    beiseitelegen(dir)?;
    let neu = Freigaben {
        v: VERSION,
        epoche: letzte + 2,
        ordner: alle_ordner
            .iter()
            .map(|o| (o.clone(), KiStufe::Aus))
            .collect(),
    };
    schreiben(dir, &neu)?;
    Ok(neu)
}

/// Wirksamer Zustand eines Ordners für die KI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Wirkung {
    pub stufe: KiStufe,
    /// Ordner, von dem der Zustand stammt, wenn nicht der Ordner selbst (geerbt).
    pub von: Option<String>,
    /// Freigabe unklar: alles gesperrt.
    pub unklar: bool,
    /// Kein Eintrag in der Kette: Vorgabe `aus`.
    pub vorgabe: bool,
}

/// `kette` ist der Ordner selbst bis zur Wurzel. `gesperrt` gewinnt immer; sonst der nächste Eintrag;
/// ohne Eintrag `aus`. Bei unklarer Freigabe gilt alles als gesperrt.
pub fn wirksam(lage: &Lage, kette: &[String]) -> Wirkung {
    let f = match lage {
        Lage::Unklar(_) => {
            return Wirkung {
                stufe: KiStufe::Gesperrt,
                von: None,
                unklar: true,
                vorgabe: false,
            }
        }
        Lage::Gueltig(f) => f,
    };
    let von = |id: &String| (kette.first() != Some(id)).then(|| id.clone());
    if let Some(g) = kette
        .iter()
        .find(|id| f.ordner.get(*id) == Some(&KiStufe::Gesperrt))
    {
        return Wirkung {
            stufe: KiStufe::Gesperrt,
            von: von(g),
            unklar: false,
            vorgabe: false,
        };
    }
    match kette.iter().find(|id| f.ordner.contains_key(*id)) {
        Some(id) => Wirkung {
            stufe: f.ordner[id],
            von: von(id),
            unklar: false,
            vorgabe: false,
        },
        None => Wirkung {
            stufe: KiStufe::Aus,
            von: None,
            unklar: false,
            vorgabe: true,
        },
    }
}

/// Globaler Weg (Assistent): nur Unterbäume mit `an`.
pub fn global_erlaubt(w: &Wirkung) -> bool {
    !w.unklar && w.stufe == KiStufe::An
}

/// Projektweg: der Mitgliedschaftsschalter zählt; `gesperrt` (und unklar) schließt aus.
pub fn projekt_erlaubt(w: &Wirkung) -> bool {
    !w.unklar && w.stufe != KiStufe::Gesperrt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(n: u8) -> String {
        // Feste, gültige ULIDs.
        format!(
            "ord_01ARZ3NDEKTSV4RRFFQ69G5FA{}",
            ['V', 'W', 'X', 'Y', 'Z'][n as usize % 5]
        )
    }

    fn kette(ids: &[&String]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn fehlend_ist_aus_und_vererbung_naechster_gewinnt() {
        let d = tempfile::tempdir().unwrap();
        let (eltern, kind) = (o(0), o(1));
        assert_eq!(
            wirksam(&laden(d.path()), &kette(&[&kind, &eltern])).stufe,
            KiStufe::Aus
        );
        setzen(d.path(), &eltern, Some(KiStufe::An)).unwrap();
        let w = wirksam(&laden(d.path()), &kette(&[&kind, &eltern]));
        assert_eq!(
            (w.stufe, w.von.as_deref()),
            (KiStufe::An, Some(eltern.as_str()))
        );
        setzen(d.path(), &kind, Some(KiStufe::Aus)).unwrap();
        assert_eq!(
            wirksam(&laden(d.path()), &kette(&[&kind, &eltern])).stufe,
            KiStufe::Aus
        );
        setzen(d.path(), &kind, None).unwrap();
        assert_eq!(
            wirksam(&laden(d.path()), &kette(&[&kind, &eltern])).stufe,
            KiStufe::An
        );
    }

    #[test]
    fn gesperrt_gewinnt_immer() {
        let d = tempfile::tempdir().unwrap();
        let (eltern, kind) = (o(0), o(1));
        setzen(d.path(), &eltern, Some(KiStufe::Gesperrt)).unwrap();
        setzen(d.path(), &kind, Some(KiStufe::An)).unwrap();
        let w = wirksam(&laden(d.path()), &kette(&[&kind, &eltern]));
        assert_eq!(w.stufe, KiStufe::Gesperrt);
        assert_eq!(w.von.as_deref(), Some(eltern.as_str()));
        assert!(!global_erlaubt(&w) && !projekt_erlaubt(&w));
    }

    #[test]
    fn aus_sperrt_nur_den_globalen_weg() {
        let w = Wirkung {
            stufe: KiStufe::Aus,
            von: None,
            unklar: false,
            vorgabe: true,
        };
        assert!(!global_erlaubt(&w) && projekt_erlaubt(&w));
    }

    #[test]
    fn vor_jedem_schreiben_entsteht_vorher() {
        let d = tempfile::tempdir().unwrap();
        let a = o(0);
        setzen(d.path(), &a, Some(KiStufe::An)).unwrap();
        assert!(
            !d.path().join(VORHER).exists(),
            "vor dem ersten Schreiben gab es nichts"
        );
        setzen(d.path(), &a, Some(KiStufe::Gesperrt)).unwrap();
        let v: Freigaben =
            serde_json::from_slice(&fs::read(d.path().join(VORHER)).unwrap()).unwrap();
        assert_eq!(v.ordner[&a], KiStufe::An, "die vorherige gültige Fassung");
    }

    #[test]
    fn defekt_ohne_vorher_ist_unklar_und_alles_gesperrt() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join(DATEI), b"{kaputt").unwrap();
        let l = laden(d.path());
        assert_eq!(l, Lage::Unklar(KiUnklar::Defekt));
        let w = wirksam(&l, &[o(0)]);
        assert!(w.unklar && w.stufe == KiStufe::Gesperrt);
        assert!(
            !global_erlaubt(&w) && !projekt_erlaubt(&w),
            "auch Projektmitgliedschaften"
        );
        assert!(setzen(d.path(), &o(0), Some(KiStufe::An)).is_err());
    }

    #[test]
    fn defekt_mit_gueltiger_vorher_stellt_wieder_her_und_erhaelt_gesperrt() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (o(0), o(1));
        setzen(d.path(), &a, Some(KiStufe::Gesperrt)).unwrap();
        setzen(d.path(), &b, Some(KiStufe::An)).unwrap();
        setzen(d.path(), &b, Some(KiStufe::An)).unwrap();
        let epoche = match laden(d.path()) {
            Lage::Gueltig(f) => f.epoche,
            _ => panic!(),
        };
        fs::write(d.path().join(DATEI), b"\x00\x01kaputt").unwrap();
        let Lage::Gueltig(f) = laden(d.path()) else {
            panic!("keine Wiederherstellung")
        };
        assert_eq!(f.ordner[&a], KiStufe::Gesperrt, "gesperrt bleibt erhalten");
        assert_eq!(
            f.ordner[&b],
            KiStufe::Gesperrt,
            "auch `an` wird bei der Wiederherstellung gesperrt"
        );
        assert!(f.epoche > epoche, "neue Epoche");
        let beiseite = fs::read_dir(d.path()).unwrap().flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("freigabe.json.defekt-")
        });
        assert!(
            beiseite,
            "die defekte Datei wird beiseitegelegt, nie gelöscht"
        );
    }

    #[test]
    fn geloeschte_datei_mit_vorher_wird_wiederhergestellt() {
        let d = tempfile::tempdir().unwrap();
        let a = o(0);
        setzen(d.path(), &a, Some(KiStufe::Gesperrt)).unwrap();
        setzen(d.path(), &a, Some(KiStufe::Gesperrt)).unwrap();
        fs::remove_file(d.path().join(DATEI)).unwrap();
        let Lage::Gueltig(f) = laden(d.path()) else {
            panic!()
        };
        assert_eq!(f.ordner[&a], KiStufe::Gesperrt);
    }

    #[test]
    fn geloeschte_datei_mit_vorher_ist_nie_leer_gueltig_und_nie_an() {
        let d = tempfile::tempdir().unwrap();
        let a = o(0);
        setzen(d.path(), &a, Some(KiStufe::An)).unwrap();
        setzen(d.path(), &a, Some(KiStufe::An)).unwrap();
        fs::remove_file(d.path().join(DATEI)).unwrap();
        match laden(d.path()) {
            Lage::Gueltig(f) => assert!(f.ordner.values().all(|s| *s == KiStufe::Gesperrt)),
            Lage::Unklar(_) => {}
        }
        // `.vorher` defekt und Datei weg: unklar, nicht leer-gültig.
        fs::write(d.path().join(VORHER), b"{kaputt").unwrap();
        let d2 = tempfile::tempdir().unwrap();
        fs::write(d2.path().join(VORHER), b"{kaputt").unwrap();
        assert_eq!(laden(d2.path()), Lage::Unklar(KiUnklar::Defekt));
    }

    #[test]
    fn neuere_version_ist_unklar_und_nie_neu_festlegbar() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join(DATEI), br#"{"v":99,"epoche":1,"neu":true}"#).unwrap();
        assert_eq!(laden(d.path()), Lage::Unklar(KiUnklar::Neuer));
        assert!(neu_festlegen(d.path(), &[o(0)]).is_err());
        assert!(d.path().join(DATEI).exists(), "nichts angefasst");
    }

    #[test]
    fn unlesbar_ist_unklar() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(DATEI)).unwrap();
        assert_eq!(laden(d.path()), Lage::Unklar(KiUnklar::Unlesbar));
        assert!(neu_festlegen(d.path(), &[]).is_err());
    }

    #[test]
    fn neu_festlegen_setzt_alle_ordner_auf_aus_und_legt_defekte_beiseite() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join(DATEI), b"nicht json").unwrap();
        let (a, b) = (o(0), o(1));
        let f = neu_festlegen(d.path(), &[a.clone(), b.clone()]).unwrap();
        assert_eq!(f.ordner[&a], KiStufe::Aus);
        assert_eq!(f.ordner[&b], KiStufe::Aus);
        assert!(matches!(laden(d.path()), Lage::Gueltig(_)));
        assert!(fs::read_dir(d.path()).unwrap().flatten().any(|e| e
            .file_name()
            .to_string_lossy()
            .starts_with("freigabe.json.defekt-")));
        assert!(
            neu_festlegen(d.path(), &[]).is_err(),
            "nur bei unklarer Freigabe"
        );
    }

    #[test]
    fn unbekannte_stufe_oder_ungueltige_id_ist_defekt() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join(DATEI),
            br#"{"v":1,"epoche":1,"ordner":{"x":"an"}}"#,
        )
        .unwrap();
        assert!(laden(d.path()).ist_unklar());
        fs::write(
            d.path().join(DATEI),
            format!(
                r#"{{"v":1,"epoche":1,"ordner":{{"{}":"vielleicht"}}}}"#,
                o(0)
            ),
        )
        .unwrap();
        assert!(laden(d.path()).ist_unklar());
    }
}
