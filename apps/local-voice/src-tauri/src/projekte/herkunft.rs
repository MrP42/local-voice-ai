//! Verweise auf Inhalte von Local Voice AI und die Momentaufnahme fuer `wai-projekte`.
//!
//! Ein Projekteintrag ist nur ein Verweis (`wai://lva/besprechung/<id>`, `wai://lva/buch/<id>`,
//! `wai://lva/recherche/<id>`); Inhalte bleiben in der App. `wai-projekte` fragt bei jeder Zuordnung
//! und jedem Auflisten nach dem Leserecht und beim Abgleich nach dem Bestand. Beide Traits sind
//! synchron, die Speicher nicht immer: darum wird vor jedem Aufruf eine Momentaufnahme gebaut.
//!
//! Beurteilt werden nur die eigenen Praefixe. Alles andere, auch Unbekanntes unter `wai://lva/` und
//! jede fremde App, bleibt `Unbekannt` und wird damit nie geloescht.

use std::collections::HashMap;
use wai_projekte::{Herkunftsbestand, Herkunftsrecht, Klasse, Vorhanden};
use wai_security::Principal;

/// App-Kennung in den Verweisen (`wai://lva/...`).
pub const LINK_APP: &str = "lva";
const BESPRECHUNG: &str = "wai://lva/besprechung/";
const BUCH: &str = "wai://lva/buch/";
const RECHERCHE: &str = "wai://lva/recherche/";
/// Lose Vorlesen-Seite („Texte“, lva#365 P4).
const SEITE: &str = "wai://lva/seite/";
/// Eigene Datei eines Projekts: `wai://lva/projektdatei/<projekt>/<datei>` (lva#365 P4).
const PROJEKTDATEI: &str = "wai://lva/projektdatei/";
/// Eintrag der Dokumentenablage (Ordner oder Datei): `wai://lva/ablage/<id>` (Spec Ablage §11).
const ABLAGE: &str = "wai://lva/ablage/";

/// Datenklasse einer Besprechung: personenbezogen (Sprache, Namen). Vertrag: Besprechungen `confidential`.
pub const KLASSE_BESPRECHUNG: Klasse = Klasse::Personal;
/// Datenklasse eines Buchs der Bibliothek: intern.
pub const KLASSE_BUCH: Klasse = Klasse::Internal;
/// Datenklasse einer Vorlesen-Seite: intern.
pub const KLASSE_SEITE: Klasse = Klasse::Internal;
/// Datenklasse einer Projektdatei: intern (die Datenklasse des Projekts begrenzt weiter).
pub const KLASSE_DATEI: Klasse = Klasse::Internal;

/// Id nur aus Buchstaben, Ziffern, `-`, `_`, `.`: so verlangt es `wai-links` fuer einen Verweis.
pub fn id_gueltig(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub fn besprechung_link(id: &str) -> String {
    format!("{BESPRECHUNG}{id}")
}

pub fn buch_link(id: &str) -> String {
    format!("{BUCH}{id}")
}

pub fn recherche_link(id: &str) -> String {
    format!("{RECHERCHE}{id}")
}

pub fn seite_link(id: &str) -> String {
    format!("{SEITE}{id}")
}

pub fn projektdatei_link(projekt: &str, datei: &str) -> String {
    format!("{PROJEKTDATEI}{projekt}/{datei}")
}

pub fn ablage_link(id: &str) -> String {
    format!("{ABLAGE}{id}")
}

/// Kennung eines Ablage-Verweises (`dat_…` oder `ord_…`, streng geprüft).
pub fn ablage_id(ref_: &str) -> Option<&str> {
    use crate::ablage::{id_gueltig, PRAEFIX_DATEI, PRAEFIX_ORDNER};
    ref_.strip_prefix(ABLAGE)
        .filter(|id| id_gueltig(PRAEFIX_DATEI, id) || id_gueltig(PRAEFIX_ORDNER, id))
}

/// Klasse eines Ablage-Eintrags für `wai-projekte` (fail-closed): `confidential` und `personal` →
/// `Personal`; unbekannt → `Restricted`.
pub fn ablage_klasse(k: crate::ablage::zusammenfuehren::Klasse) -> Klasse {
    use crate::ablage::zusammenfuehren::Klasse as A;
    match k {
        A::Public => Klasse::Public,
        A::Internal => Klasse::Internal,
        A::Confidential | A::Personal => Klasse::Personal,
        A::Unbekannt => Klasse::Restricted,
    }
}

pub fn seite_id(ref_: &str) -> Option<&str> {
    ref_.strip_prefix(SEITE).filter(|id| id_gueltig(id))
}

/// `(projekt, datei)` eines Projektdatei-Verweises, beide geprüft.
pub fn projektdatei_id(ref_: &str) -> Option<(&str, &str)> {
    let (p, d) = ref_.strip_prefix(PROJEKTDATEI)?.split_once('/')?;
    (suite_id_gueltig(p) && id_gueltig(d)).then_some((p, d))
}

pub fn besprechung_id(ref_: &str) -> Option<&str> {
    ref_.strip_prefix(BESPRECHUNG)
}

pub fn buch_id(ref_: &str) -> Option<&str> {
    ref_.strip_prefix(BUCH)
}

pub fn recherche_id(ref_: &str) -> Option<&str> {
    ref_.strip_prefix(RECHERCHE)
}

/// Suite-Projekt-Kennung: Praefix, Unterstrich, 26 Zeichen Crockford-Base32 (`prj_<ULID>`).
pub fn suite_id_gueltig(id: &str) -> bool {
    let Some((prefix, ulid)) = id.split_once('_') else {
        return false;
    };
    (2..=12).contains(&prefix.len())
        && prefix.bytes().all(|b| b.is_ascii_lowercase())
        && ulid.len() == 26
        && ulid
            .bytes()
            .all(|b| b.is_ascii_digit() || (b.is_ascii_uppercase() && !b"ILOU".contains(&b)))
}

/// Ein Eintrag der Momentaufnahme: existiert immer, ist nur mit Leserecht lesbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HerkunftsEintrag {
    pub lesbar: bool,
    pub klasse: Klasse,
}

/// Momentaufnahme der Urspruenge von Local Voice AI (Besprechungen, Buecher, Recherchen) eines
/// Mandanten. Fremde Mandanten und fremde Apps beurteilt sie nie als "nicht vorhanden".
#[derive(Debug, Clone)]
pub struct LvaHerkunft {
    mandant: String,
    eintraege: HashMap<String, HerkunftsEintrag>,
    /// Zusätzlich beurteilte Präfixe (Seiten, Projektdateien): nur wenn ihr Bestand vollständig
    /// gelesen wurde. Sonst bleiben sie `Unbekannt` und werden nie als verwaist entfernt.
    beurteilt: Vec<String>,
}

impl LvaHerkunft {
    pub fn neu(mandant: &str) -> Self {
        Self {
            mandant: mandant.to_string(),
            eintraege: HashMap::new(),
            beurteilt: Vec::new(),
        }
    }

    /// Seiten wurden vollständig gelesen: ihr Präfix wird beurteilt.
    pub fn seiten_beurteilen(&mut self) {
        self.beurteilt.push(SEITE.to_string());
    }

    /// Die Projektdateien dieses Projekts wurden vollständig gelesen.
    pub fn projektdateien_beurteilen(&mut self, projekt: &str) {
        self.beurteilt.push(format!("{PROJEKTDATEI}{projekt}/"));
    }

    pub fn hinzufuegen(&mut self, ref_: String, lesbar: bool, klasse: Klasse) {
        self.eintraege
            .insert(ref_, HerkunftsEintrag { lesbar, klasse });
    }

    pub fn erweitern(&mut self, andere: LvaHerkunft) {
        if andere.mandant == self.mandant {
            self.eintraege.extend(andere.eintraege);
            self.beurteilt.extend(andere.beurteilt);
        }
    }

    pub fn eintrag(&self, ref_: &str) -> Option<HerkunftsEintrag> {
        self.eintraege.get(ref_).copied()
    }
}

impl Herkunftsrecht for LvaHerkunft {
    fn darf_lesen(&self, principal: &Principal, ref_: &str) -> bool {
        principal.tenant().0 == self.mandant && self.eintrag(ref_).is_some_and(|e| e.lesbar)
    }

    fn klasse(&self, principal: &Principal, ref_: &str) -> Option<Klasse> {
        if principal.tenant().0 != self.mandant {
            return None;
        }
        self.eintrag(ref_).map(|e| e.klasse)
    }
}

impl Herkunftsbestand for LvaHerkunft {
    /// Beurteilt nur die Verweisarten, die diese App kennt und vollständig gelesen hat. Alles
    /// andere bleibt `Unbekannt`.
    fn existiert(&self, mandant: &str, ref_: &str) -> Vorhanden {
        let bekannt = [BESPRECHUNG, BUCH, RECHERCHE]
            .iter()
            .any(|p| ref_.starts_with(p))
            || self.beurteilt.iter().any(|p| ref_.starts_with(p.as_str()));
        if mandant != self.mandant || !bekannt {
            return Vorhanden::Unbekannt;
        }
        if self.eintraege.contains_key(ref_) {
            Vorhanden::Ja
        } else {
            Vorhanden::Nein
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projekte::lokaler_principal;

    #[test]
    fn links_und_ids_rundlauf() {
        assert_eq!(besprechung_link("M1"), "wai://lva/besprechung/M1");
        assert_eq!(besprechung_id("wai://lva/besprechung/M1"), Some("M1"));
        assert_eq!(buch_id("wai://lva/buch/book_1"), Some("book_1"));
        assert_eq!(recherche_id("wai://lva/recherche/rch_1"), Some("rch_1"));
        assert_eq!(besprechung_id("wai://lva/buch/x"), None);
        assert!(id_gueltig("01HZZZZZZZZZZZZZZZZZZZZZZZ"));
        assert!(id_gueltig("book_1730000000000"));
        assert!(!id_gueltig("../x"));
        assert!(!id_gueltig("a/b"));
        assert!(!id_gueltig(""));
        assert_eq!(seite_id(&seite_link("p_1")), Some("p_1"));
        assert_eq!(seite_id("wai://lva/seite/../x"), None);
        let p = "prj_01HZZZZZZZZZZZZZZZZZZZZZZZ";
        assert_eq!(
            projektdatei_id(&projektdatei_link(p, "pd_1")),
            Some((p, "pd_1"))
        );
        assert_eq!(projektdatei_id("wai://lva/projektdatei/../pd_1"), None);
        assert_eq!(
            projektdatei_id(&format!("wai://lva/projektdatei/{p}/a/b")),
            None
        );
    }

    #[test]
    fn seiten_und_projektdateien_nur_beurteilt_wenn_gelesen() {
        let p = "prj_01HZZZZZZZZZZZZZZZZZZZZZZZ";
        let mut h = LvaHerkunft::neu("solo");
        assert_eq!(h.existiert("solo", &seite_link("s1")), Vorhanden::Unbekannt);
        assert_eq!(
            h.existiert("solo", &projektdatei_link(p, "d1")),
            Vorhanden::Unbekannt
        );
        h.seiten_beurteilen();
        h.projektdateien_beurteilen(p);
        h.hinzufuegen(seite_link("s1"), true, KLASSE_SEITE);
        assert_eq!(h.existiert("solo", &seite_link("s1")), Vorhanden::Ja);
        assert_eq!(h.existiert("solo", &seite_link("s2")), Vorhanden::Nein);
        assert_eq!(
            h.existiert("solo", &projektdatei_link(p, "d1")),
            Vorhanden::Nein
        );
        let anderes = "prj_01HYYYYYYYYYYYYYYYYYYYYYYY";
        assert_eq!(
            h.existiert("solo", &projektdatei_link(anderes, "d1")),
            Vorhanden::Unbekannt
        );
    }

    #[test]
    fn suite_kennung_wird_streng_geprueft() {
        assert!(suite_id_gueltig("prj_01HZZZZZZZZZZZZZZZZZZZZZZZ"));
        assert!(!suite_id_gueltig("prj_01hzzzzzzzzzzzzzzzzzzzzzzz"));
        assert!(!suite_id_gueltig("prj_ILOU"));
        assert!(!suite_id_gueltig("../prj"));
    }

    #[test]
    fn nur_eigene_praefixe_werden_beurteilt_alles_andere_ist_unbekannt() {
        let mut h = LvaHerkunft::neu("solo");
        h.hinzufuegen(besprechung_link("M1"), true, Klasse::Personal);
        assert_eq!(
            h.existiert("solo", "wai://lva/besprechung/M1"),
            Vorhanden::Ja
        );
        assert_eq!(
            h.existiert("solo", "wai://lva/besprechung/M2"),
            Vorhanden::Nein
        );
        for fremd in [
            "wai://corporate-llm/unterhaltung/c1",
            "wai://lva/unbekannt/x",
            "wai://presenter/vortrag/v1",
            "wai://planner/plan/p1",
        ] {
            assert_eq!(h.existiert("solo", fremd), Vorhanden::Unbekannt, "{fremd}");
        }
        assert_eq!(
            h.existiert("anderer", "wai://lva/besprechung/M1"),
            Vorhanden::Unbekannt
        );
    }

    #[test]
    fn leserecht_und_klasse_gelten_nur_fuer_den_eigenen_mandanten() {
        let p = lokaler_principal();
        let mut h = LvaHerkunft::neu(&p.tenant().0);
        h.hinzufuegen(buch_link("b1"), true, Klasse::Internal);
        h.hinzufuegen(buch_link("b2"), false, Klasse::Internal);
        assert!(h.darf_lesen(&p, "wai://lva/buch/b1"));
        assert!(!h.darf_lesen(&p, "wai://lva/buch/b2"));
        assert_eq!(h.klasse(&p, "wai://lva/buch/b1"), Some(Klasse::Internal));
        let fremd = LvaHerkunft::neu("anderer");
        assert!(!fremd.darf_lesen(&p, "wai://lva/buch/b1"));
        assert_eq!(fremd.klasse(&p, "wai://lva/buch/b1"), None);
    }
}
