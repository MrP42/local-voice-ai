//! Sitzungssteuerung und getrennt ausführbare, serielle Fensterarbeit.

use std::collections::VecDeque;

use super::cache::{Ablage, Cache, CacheArt, Schluessel};
use super::events::{Bereich, Ereignis, EreignisArt, FehlerCode, Grund};
use super::planner::{Modus, Planer, Pufferzustand, Schritt, STARTPUFFER_MS};
use super::sentences::Satzbildner;
use super::{MediaSegmentSource, Uebersetzer, ZeitGuete};

pub use super::planner::Fenster as Auftrag;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SatzInhalt {
    pub original: String,
    pub uebersetzung: String,
    pub zeiten: Bereich,
    pub guete: ZeitGuete,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct Status {
    pub epoch: u64,
    pub position_ms: u64,
    pub bereit: Bereich,
    pub puffer: Pufferzustand,
    pub laeuft: bool,
    pub fehler: Option<FehlerCode>,
}

/// Ausschließlich außerhalb der Sitzungssperre und seriell verwenden.
pub struct Werkbank<Q, U, A: Ablage> {
    quelle: Q,
    satzbildner: Satzbildner,
    uebersetzer: U,
    cache: Cache<A>,
    session_id: String,
    dauer_ms: u64,
    ziel_sprache: String,
    epoch: Option<u64>,
    gesperrt: bool,
}

/// Enthält keine ungefilterten Quellen- oder Übersetzungsfehler.
pub struct Ergebnis {
    auftrag: Auftrag,
    inhalt: Result<Vec<(String, SatzInhalt)>, ArbeitsFehler>,
}

enum ArbeitsFehler {
    Abgebrochen,
    Fehler(FehlerCode),
}

/// Die Werkbank braucht weder Planer noch Sitzungssperre.
pub fn ausfuehren<Q: MediaSegmentSource, U: Uebersetzer, A: Ablage>(
    werkbank: &mut Werkbank<Q, U, A>,
    auftrag: Auftrag,
) -> Ergebnis {
    let inhalt = werkbank.ausfuehren(auftrag);
    Ergebnis { auftrag, inhalt }
}

impl<Q: MediaSegmentSource, U: Uebersetzer, A: Ablage> Werkbank<Q, U, A> {
    fn ausfuehren(&mut self, auftrag: Auftrag) -> Result<Vec<(String, SatzInhalt)>, ArbeitsFehler> {
        if self.epoch.is_some_and(|epoch| epoch > auftrag.epoch) {
            return Err(ArbeitsFehler::Abgebrochen);
        }
        if self.epoch != Some(auftrag.epoch) {
            self.satzbildner = Satzbildner::default();
            self.epoch = Some(auftrag.epoch);
            self.gesperrt = false;
        }
        if self.gesperrt {
            return Err(ArbeitsFehler::Fehler(FehlerCode::KeinLokalesModell));
        }
        let (segmente, guete) = self
            .quelle
            .segmente(auftrag.bereich.von_ms, auftrag.bereich.bis_ms)
            .map_err(|fehler| {
                if fehler.0 == "abgebrochen" {
                    ArbeitsFehler::Abgebrochen
                } else {
                    ArbeitsFehler::Fehler(FehlerCode::Quelle)
                }
            })?;
        let saetze = self.satzbildner.fenster(
            &self.session_id,
            segmente,
            guete,
            auftrag.bereich.bis_ms >= self.dauer_ms,
        );
        let mut inhalte = Vec::new();
        for satz in saetze {
            let schluessel = Schluessel {
                art: CacheArt::Uebersetzung,
                spur: String::new(),
                modellrevision: self.uebersetzer.kennung(),
                quell_sprache: String::new(),
                ziel_sprache: self.ziel_sprache.clone(),
                kontext: String::new(),
                prompt_version: String::new(),
                pipeline_version: "sitzung-v1".into(),
                eingabe: satz.text.clone(),
            };
            let treffer = self
                .cache
                .holen(&schluessel)
                .ok()
                .flatten()
                .and_then(|bytes| String::from_utf8(bytes).ok());
            let uebersetzung = match treffer {
                Some(text) => text,
                None => {
                    let text = self.uebersetzer.uebersetzen(&satz.text).map_err(|fehler| {
                        let code = if fehler.0 == "kein_lokales_modell" {
                            self.gesperrt = true;
                            FehlerCode::KeinLokalesModell
                        } else {
                            FehlerCode::Uebersetzung
                        };
                        ArbeitsFehler::Fehler(code)
                    })?;
                    let _ = self.cache.ablegen(&schluessel, text.as_bytes(), 0);
                    text
                }
            };
            // Die Bildner-ID enthält Zeit, Inhaltsdigest und einen fortlaufenden Index.
            let id = format!("{}-{}-{}", self.session_id, auftrag.epoch, satz.id);
            inhalte.push((
                id,
                SatzInhalt {
                    original: satz.text,
                    uebersetzung,
                    zeiten: Bereich {
                        von_ms: satz.start_ms,
                        bis_ms: satz.end_ms,
                    },
                    guete: satz.guete,
                },
            ));
        }
        Ok(inhalte)
    }
}

pub struct Sitzung<Q, U, A: Ablage> {
    planer: Planer,
    werkbank: Option<Werkbank<Q, U, A>>,
    session_id: String,
    dauer_ms: u64,
    laeuft: bool,
    fehler: Option<FehlerCode>,
    puffer: Pufferzustand,
    offen: Option<Auftrag>,
    wiederholen: bool,
    wiederholungen: u8,
    texte: VecDeque<(String, SatzInhalt)>,
}

impl<Q: MediaSegmentSource, U: Uebersetzer, A: Ablage> Sitzung<Q, U, A> {
    pub fn neu(
        session_id: String,
        dauer_ms: u64,
        modus: Modus,
        ziel_sprache: String,
        quelle: Q,
        uebersetzer: U,
        cache: Cache<A>,
    ) -> Self {
        Self {
            planer: Planer::neu(session_id.clone(), dauer_ms, modus),
            werkbank: Some(Werkbank {
                quelle,
                satzbildner: Satzbildner::default(),
                uebersetzer,
                cache,
                session_id: session_id.clone(),
                dauer_ms,
                ziel_sprache,
                epoch: None,
                gesperrt: false,
            }),
            session_id,
            dauer_ms,
            laeuft: false,
            fehler: None,
            puffer: Pufferzustand::Leer,
            offen: None,
            wiederholen: false,
            wiederholungen: 0,
            texte: VecDeque::new(),
        }
    }

    pub fn werkbank_entnehmen(&mut self) -> Option<Werkbank<Q, U, A>> {
        self.werkbank.take()
    }

    /// Eine bereits vorhandene Werkbank wird nicht überschrieben; dann kommt die neue zurück.
    pub fn werkbank_zurueckgeben(
        &mut self,
        werkbank: Werkbank<Q, U, A>,
    ) -> Option<Werkbank<Q, U, A>> {
        if self.werkbank.is_some() {
            return Some(werkbank);
        }
        self.werkbank = Some(werkbank);
        None
    }

    pub fn starten(&mut self, position_ms: u64, spielt: bool) {
        self.planer.start(position_ms, spielt);
        self.zuruecksetzen();
        self.laeuft = true;
    }

    pub fn spulen(&mut self, position_ms: u64) {
        self.planer.position_setzen(position_ms);
        self.zuruecksetzen();
    }

    fn zuruecksetzen(&mut self) {
        self.fehler = None;
        self.puffer = Pufferzustand::Leer;
        self.offen = None;
        self.wiederholen = false;
        self.wiederholungen = 0;
        self.texte.clear();
        if let Some(werkbank) = &mut self.werkbank {
            werkbank.satzbildner = Satzbildner::default();
        }
    }

    pub fn stoppen(&mut self) {
        self.laeuft = false;
        self.planer.abspielen_setzen(false);
        self.offen = None;
    }

    pub fn pausieren(&mut self) {
        self.planer.abspielen_setzen(false);
    }

    pub fn fortsetzen(&mut self) {
        if self.laeuft {
            self.planer.abspielen_setzen(true);
        }
    }

    pub fn tick(&mut self, jetzt_ms: u64) -> Schritt {
        if !self.laeuft {
            let status = self.status();
            return Schritt {
                puffer: status.puffer,
                bereit: status.bereit,
                aktion: None,
                ereignisse: Vec::new(),
            };
        }
        let schritt = self.planer.tick(jetzt_ms);
        self.puffer = schritt.puffer;
        schritt
    }

    pub fn status(&self) -> Status {
        let bereit = self.planer.bereit();
        let vorrat = bereit.bis_ms - bereit.von_ms;
        let erforderlich = STARTPUFFER_MS.min(self.dauer_ms - bereit.von_ms);
        let puffer = if self.laeuft
            && (vorrat >= erforderlich || (self.puffer == Pufferzustand::Bereit && vorrat > 0))
        {
            Pufferzustand::Bereit
        } else if vorrat == 0 {
            Pufferzustand::Leer
        } else {
            Pufferzustand::Fuellend
        };
        Status {
            epoch: self.planer.epoch(),
            position_ms: bereit.von_ms,
            bereit,
            puffer,
            laeuft: self.laeuft,
            fehler: self.fehler,
        }
    }

    /// Ein offenes Fenster hält die serielle Satzbildung in Medienreihenfolge.
    pub fn naechster_auftrag(&mut self) -> Option<Auftrag> {
        if !self.laeuft || self.fehler.is_some() {
            return None;
        }
        if let Some(auftrag) = self.offen {
            if self.wiederholen {
                self.wiederholen = false;
                return self.planer.fenster_wiederholen(auftrag.epoch, auftrag.id);
            }
            return None;
        }
        let auftrag = self.planer.neues_fenster()?;
        self.offen = Some(auftrag);
        self.wiederholungen = 0;
        Some(auftrag)
    }

    pub fn uebernehmen(&mut self, auftrag: Auftrag, ergebnis: Ergebnis) -> Vec<Ereignis> {
        if !self.laeuft
            || auftrag.epoch != self.planer.epoch()
            || self.offen != Some(auftrag)
            || ergebnis.auftrag != auftrag
            || self.wiederholen
            || self.fehler.is_some()
        {
            return Vec::new();
        }
        let inhalte = match ergebnis.inhalt {
            Ok(inhalte) => inhalte,
            Err(ArbeitsFehler::Abgebrochen) => {
                self.wiederholen = true;
                return Vec::new();
            }
            Err(ArbeitsFehler::Fehler(FehlerCode::Quelle)) if self.wiederholungen < 2 => {
                self.wiederholungen += 1;
                self.wiederholen = true;
                return Vec::new();
            }
            Err(ArbeitsFehler::Fehler(code)) => {
                self.fehler = Some(code);
                return vec![self.ereignis(EreignisArt::Fehler { code })];
            }
        };
        let mut ereignisse = self
            .planer
            .ergebnis_melden(auftrag.epoch, auftrag.id, false);
        self.offen = None;
        for (satz_id, inhalt) in inhalte {
            ereignisse.push(self.ereignis(EreignisArt::Satz {
                satz_id: satz_id.clone(),
                zeiten: inhalt.zeiten,
                guete: inhalt.guete,
            }));
            if inhalt.guete == ZeitGuete::Grob {
                ereignisse.push(self.ereignis(EreignisArt::SynchronNichtVerfuegbar {
                    satz_id: satz_id.clone(),
                    zeiten: inhalt.zeiten,
                    grund: Grund::GrobeZeitmarken,
                }));
            }
            self.texte.push_back((satz_id, inhalt));
            if self.texte.len() > 2000 {
                self.texte.pop_front();
            }
        }
        ereignisse
    }

    fn ereignis(&self, art: EreignisArt) -> Ereignis {
        Ereignis {
            session_id: self.session_id.clone(),
            epoch: self.planer.epoch(),
            art,
        }
    }

    pub fn satz_text(&self, satz_id: &str) -> Option<SatzInhalt> {
        self.texte
            .iter()
            .find(|(id, _)| id == satz_id)
            .map(|(_, inhalt)| inhalt.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    use super::super::cache::SpeicherAblage;
    use super::super::{QuellenFehler, Segment, UebersetzungsFehler};

    type QuellenErgebnis = Result<(Vec<Segment>, ZeitGuete), QuellenFehler>;
    type TestSitzung = Sitzung<Quelle, Lokal, SpeicherAblage>;

    #[derive(Default)]
    struct Quelle {
        ergebnisse: VecDeque<QuellenErgebnis>,
        aufrufe: Vec<(u64, u64)>,
    }

    impl MediaSegmentSource for Quelle {
        fn segmente(&mut self, von_ms: u64, bis_ms: u64) -> QuellenErgebnis {
            self.aufrufe.push((von_ms, bis_ms));
            self.ergebnisse
                .pop_front()
                .unwrap_or_else(|| Ok((Vec::new(), ZeitGuete::Satz)))
        }
    }

    #[derive(Default)]
    struct Lokal {
        aufrufe: usize,
        fehler: Option<&'static str>,
        kennung: String,
    }

    impl Uebersetzer for Lokal {
        fn uebersetzen(&mut self, satz: &str) -> Result<String, UebersetzungsFehler> {
            self.aufrufe += 1;
            match self.fehler {
                Some(fehler) => Err(UebersetzungsFehler(fehler.into())),
                None => Ok(format!("Ziel: {satz}")),
            }
        }

        fn kennung(&self) -> String {
            self.kennung.clone()
        }
    }

    fn segment(start_ms: u64, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms: start_ms + 100,
            text: text.into(),
            woerter: None,
        }
    }

    fn daten(start_ms: u64, text: &str) -> QuellenErgebnis {
        Ok((vec![segment(start_ms, text)], ZeitGuete::Satz))
    }

    fn sitzung(ergebnisse: Vec<QuellenErgebnis>) -> TestSitzung {
        Sitzung::neu(
            "test".into(),
            30_000,
            Modus::Untertitel,
            "en".into(),
            Quelle {
                ergebnisse: ergebnisse.into(),
                ..Quelle::default()
            },
            Lokal::default(),
            Cache::neu(SpeicherAblage::default(), 1_000_000),
        )
    }

    fn auswerten(sitzung: &mut TestSitzung, auftrag: Auftrag) -> Ergebnis {
        let mut werkbank = sitzung.werkbank_entnehmen().expect("Werkbank");
        let ergebnis = ausfuehren(&mut werkbank, auftrag);
        assert!(sitzung.werkbank_zurueckgeben(werkbank).is_none());
        ergebnis
    }

    fn schritt(sitzung: &mut TestSitzung) -> Vec<Ereignis> {
        let auftrag = sitzung.naechster_auftrag().expect("Auftrag");
        let ergebnis = auswerten(sitzung, auftrag);
        sitzung.uebernehmen(auftrag, ergebnis)
    }

    fn satz_ids(ereignisse: &[Ereignis]) -> Vec<String> {
        ereignisse
            .iter()
            .filter_map(|ereignis| match &ereignis.art {
                EreignisArt::Satz { satz_id, .. } => Some(satz_id.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn status_vor_und_nach_start() {
        let mut s = sitzung(vec![]);
        // Gegenprobe: Ohne Startsperre würde schon der Konstruktor Aufträge zulassen.
        assert!(!s.status().laeuft);
        assert_eq!(s.status().epoch, 0);
        assert!(s.naechster_auftrag().is_none());
        s.starten(500, false);
        assert!(s.status().laeuft);
        assert_eq!(s.status().epoch, 1);
        assert_eq!(s.status().position_ms, 500);
        assert_eq!(s.status().bereit.bis_ms, 500);
        assert_eq!(s.status().puffer, Pufferzustand::Leer);
    }

    #[test]
    fn fenster_liefert_saetze_texte_und_bereitschaft() {
        let mut s = sitzung(vec![daten(100, "Hallo.")]);
        s.starten(0, false);
        let ereignisse = schritt(&mut s);
        let ids = satz_ids(&ereignisse);
        assert_eq!(ids.len(), 1);
        let inhalt = s.satz_text(&ids[0]).expect("Text");
        assert_eq!(inhalt.original, "Hallo.");
        assert_eq!(inhalt.uebersetzung, "Ziel: Hallo.");
        assert_eq!(inhalt.zeiten.von_ms, 100);
        // Gegenprobe: Satzende statt Fensterende würde Stille nicht bestätigen.
        assert_eq!(s.status().bereit.bis_ms, 15_000);
        assert_eq!(s.status().puffer, Pufferzustand::Fuellend);
        assert!(matches!(ereignisse[0].art, EreignisArt::Bereitschaft(_)));
        assert!(ereignisse
            .iter()
            .all(|e| e.session_id == "test" && e.epoch == 1));
    }

    #[test]
    fn zwei_fenster_uebergeben_rest_ohne_doppelsatz() {
        let anfang = segment(14_900, "Hallo");
        let mut s = sitzung(vec![
            Ok((vec![anfang.clone()], ZeitGuete::Satz)),
            Ok((vec![anfang, segment(15_000, "Welt.")], ZeitGuete::Satz)),
        ]);
        s.starten(0, false);
        assert!(satz_ids(&schritt(&mut s)).is_empty());
        let ids = satz_ids(&schritt(&mut s));
        // Gegenprobe: Ein frischer Bildner pro Fenster verliert den Anfang.
        assert_eq!(ids.len(), 1);
        assert_eq!(s.satz_text(&ids[0]).unwrap().original, "Hallo Welt.");
        assert_eq!(s.status().bereit.bis_ms, 30_000);
        assert_eq!(s.status().puffer, Pufferzustand::Bereit);
    }

    #[test]
    fn seek_verwirft_laufendes_ergebnis() {
        let mut s = sitzung(vec![daten(0, "Alt.")]);
        s.starten(0, false);
        let alt = s.naechster_auftrag().unwrap();
        let ergebnis = auswerten(&mut s, alt);
        s.spulen(20_000);
        let vorher = s.status();
        // Gegenprobe: Ohne Sitzungsfilter würden trotz Planerfilter Satzereignisse entstehen.
        assert!(s.uebernehmen(alt, ergebnis).is_empty());
        assert_eq!(s.status(), vorher);
        assert!(s.texte.is_empty());
    }

    #[test]
    fn stoppen_verwirft_ergebnisse_und_ticks() {
        let mut s = sitzung(vec![daten(0, "Alt.")]);
        s.starten(0, true);
        let auftrag = s.naechster_auftrag().unwrap();
        let ergebnis = auswerten(&mut s, auftrag);
        s.stoppen();
        // Gegenprobe: Die Epoche allein schützt nicht vor Ergebnissen nach Stoppen.
        assert!(s.uebernehmen(auftrag, ergebnis).is_empty());
        assert!(s.naechster_auftrag().is_none());
        assert!(s.tick(100).ereignisse.is_empty());
        s.fortsetzen();
        s.spulen(1_000);
        assert!(s.naechster_auftrag().is_none());
        assert!(!s.status().laeuft);
    }

    #[test]
    fn drei_seeks_nur_letztes_ergebnis_zaehlt() {
        let mut s = sitzung(vec![]);
        s.starten(0, false);
        let mut alte = Vec::new();
        for position in [1_000, 2_000, 3_000] {
            let auftrag = s.naechster_auftrag().unwrap();
            alte.push((auftrag, auswerten(&mut s, auftrag)));
            s.spulen(position);
        }
        for (auftrag, ergebnis) in alte {
            // Gegenprobe: Nur die direkt vorherige Epoche zu sperren reicht nicht.
            assert!(s.uebernehmen(auftrag, ergebnis).is_empty());
        }
        assert_eq!(s.status().bereit.bis_ms, 3_000);
        assert!(!schritt(&mut s).is_empty());
        assert_eq!(s.status().bereit.bis_ms, 18_000);
    }

    #[test]
    fn seek_setzt_ausgelagerten_satzbildner_zurueck() {
        let mut s = sitzung(vec![daten(0, "Alt"), daten(20_000, "Neu.")]);
        let mut w = s.werkbank_entnehmen().unwrap();
        s.starten(0, false);
        let a = s.naechster_auftrag().unwrap();
        let e = ausfuehren(&mut w, a);
        s.uebernehmen(a, e);
        s.spulen(20_000);
        let a = s.naechster_auftrag().unwrap();
        let e = ausfuehren(&mut w, a);
        let ids = satz_ids(&s.uebernehmen(a, e));
        // Gegenprobe: Nur die eingelagerte Werkbank zurückzusetzen lässt „Alt“ stehen.
        assert_eq!(s.satz_text(&ids[0]).unwrap().original, "Neu.");
    }

    #[test]
    fn kein_lokales_modell_sperrt_bis_seek() {
        let mut s = sitzung(vec![daten(0, "Hallo."), daten(20_000, "Neu.")]);
        s.werkbank.as_mut().unwrap().uebersetzer.fehler = Some("kein_lokales_modell");
        s.starten(0, false);
        let ereignisse = schritt(&mut s);
        assert_eq!(ereignisse.len(), 1);
        assert_eq!(
            ereignisse[0].art,
            EreignisArt::Fehler {
                code: FehlerCode::KeinLokalesModell
            }
        );
        assert_eq!(s.status().fehler, Some(FehlerCode::KeinLokalesModell));
        assert_eq!(s.status().bereit.bis_ms, 0);
        // Gegenprobe: Ohne Sperre würde ein weiteres Fenster das Modell erneut aufrufen.
        assert!(s.naechster_auftrag().is_none());
        assert_eq!(s.werkbank.as_ref().unwrap().uebersetzer.aufrufe, 1);
        s.spulen(20_000);
        s.werkbank.as_mut().unwrap().uebersetzer.fehler = None;
        assert_eq!(satz_ids(&schritt(&mut s)).len(), 1);
        assert_eq!(s.status().fehler, None);
        assert_eq!(s.werkbank.as_ref().unwrap().uebersetzer.aufrufe, 2);
    }

    #[test]
    fn quellenfehler_hoechstens_zweimal_wiederholen() {
        let mut s = sitzung(vec![Err(QuellenFehler("privater Pfad".into())); 3]);
        s.starten(0, false);
        let mut erster = None;
        for versuch in 0..3 {
            let a = s.naechster_auftrag().unwrap();
            assert_eq!(*erster.get_or_insert(a), a);
            let e = auswerten(&mut s, a);
            let ereignisse = s.uebernehmen(a, e);
            if versuch < 2 {
                assert!(ereignisse.is_empty());
            } else {
                assert_eq!(
                    ereignisse[0].art,
                    EreignisArt::Fehler {
                        code: FehlerCode::Quelle
                    }
                );
            }
        }
        // Gegenprobe: Ohne Limit würde hier ein vierter Versuch erscheinen.
        assert!(s.naechster_auftrag().is_none());
        assert_eq!(s.status().fehler, Some(FehlerCode::Quelle));
        assert_eq!(s.status().bereit.bis_ms, 0);
    }

    #[test]
    fn erfolgreiche_wiederholung_bestaetigt_dasselbe_fenster() {
        let mut s = sitzung(vec![Err(QuellenFehler("Quelle".into())), daten(0, "Ja.")]);
        s.starten(0, false);
        assert!(schritt(&mut s).is_empty());
        assert_eq!(satz_ids(&schritt(&mut s)).len(), 1);
        // Gegenprobe: Ein neues Fenster würde eine Bereitschaftslücke hinterlassen.
        assert_eq!(s.status().bereit.bis_ms, 15_000);
        assert_eq!(s.status().fehler, None);
    }

    #[test]
    fn abbruch_bleibt_still_und_verbraucht_keinen_versuch() {
        let mut ergebnisse = vec![Err(QuellenFehler("abgebrochen".into())); 4];
        ergebnisse.push(daten(0, "Ja."));
        let mut s = sitzung(ergebnisse);
        s.starten(0, false);
        for _ in 0..4 {
            assert!(schritt(&mut s).is_empty());
            assert_eq!(s.status().fehler, None);
            assert_eq!(s.status().bereit.bis_ms, 0);
        }
        // Gegenprobe: Als Quellenfehler gezählte Abbrüche würden vor dem Erfolg sperren.
        assert_eq!(satz_ids(&schritt(&mut s)).len(), 1);
    }

    #[test]
    fn cache_treffer_vermeidet_uebersetzer() {
        let mut s = sitzung(vec![daten(0, "Ja."), daten(15_000, "Ja.")]);
        s.starten(0, false);
        schritt(&mut s);
        let ids = satz_ids(&schritt(&mut s));
        // Gegenprobe: Ohne Cache-Lesen würde der zweite Satz erneut übersetzt.
        assert_eq!(s.werkbank.as_ref().unwrap().uebersetzer.aufrufe, 1);
        assert_eq!(s.satz_text(&ids[0]).unwrap().uebersetzung, "Ziel: Ja.");
    }

    #[test]
    fn zielsprache_invalidiert_cache() {
        let mut s = sitzung(vec![daten(0, "Ja."), daten(15_000, "Ja.")]);
        s.starten(0, false);
        schritt(&mut s);
        s.werkbank.as_mut().unwrap().ziel_sprache = "fr".into();
        schritt(&mut s);
        // Gegenprobe: Ohne Zielsprache im Schlüssel wäre dies ein Treffer.
        assert_eq!(s.werkbank.as_ref().unwrap().uebersetzer.aufrufe, 2);
    }

    #[test]
    fn modellkennung_invalidiert_cache() {
        let mut s = sitzung(vec![daten(0, "Ja."), daten(15_000, "Ja.")]);
        s.starten(0, false);
        schritt(&mut s);
        s.werkbank.as_mut().unwrap().uebersetzer.kennung = "revision2".into();
        schritt(&mut s);
        // Gegenprobe: Ohne Modellkennung wird eine fremde Revision wiederverwendet.
        assert_eq!(s.werkbank.as_ref().unwrap().uebersetzer.aufrufe, 2);
    }

    #[test]
    fn cache_io_fehler_verhindert_uebersetzung_nicht() {
        struct Defekt;
        impl Ablage for Defekt {
            fn namen(&self) -> io::Result<Vec<String>> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
            fn lesen(&self, _name: &str) -> io::Result<Vec<u8>> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
            fn schreiben(&mut self, _name: &str, _daten: &[u8]) -> io::Result<()> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
            fn umbenennen(&mut self, _von: &str, _nach: &str) -> io::Result<()> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
            fn entfernen(&mut self, _name: &str) -> io::Result<()> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
        }
        let mut s = Sitzung::neu(
            "test".into(),
            15_000,
            Modus::Untertitel,
            "en".into(),
            Quelle {
                ergebnisse: vec![daten(0, "Ja.")].into(),
                ..Quelle::default()
            },
            Lokal::default(),
            Cache::neu(Defekt, 10_000),
        );
        s.starten(0, false);
        let a = s.naechster_auftrag().unwrap();
        let mut w = s.werkbank_entnehmen().unwrap();
        let e = ausfuehren(&mut w, a);
        // Gegenprobe: Durchgereichte Cachefehler würden keinen Satz veröffentlichen.
        assert_eq!(satz_ids(&s.uebernehmen(a, e)).len(), 1);
        assert_eq!(w.uebersetzer.aufrufe, 1);
        assert_eq!(s.status().fehler, None);
    }

    #[test]
    fn grobe_zeiten_melden_synchron_nicht_verfuegbar() {
        let mut s = sitzung(vec![Ok((vec![segment(100, "Ja.")], ZeitGuete::Grob))]);
        s.starten(0, false);
        let ereignisse = schritt(&mut s);
        let ids = satz_ids(&ereignisse);
        // Gegenprobe: Nur Satzereignisse würden die fehlende Synchronqualität verbergen.
        assert!(ereignisse.iter().any(|e| matches!(
            &e.art,
            EreignisArt::SynchronNichtVerfuegbar { satz_id, zeiten, grund }
                if satz_id == &ids[0]
                    && zeiten.von_ms == 100
                    && *grund == Grund::GrobeZeitmarken
        )));
    }

    #[test]
    fn textablage_begrenzt_auf_2000_saetze() {
        let segmente = (0..2001)
            .map(|i| Segment {
                end_ms: i * 2 + 1,
                ..segment(i * 2, "Ja.")
            })
            .collect();
        let mut s = sitzung(vec![Ok((segmente, ZeitGuete::Satz))]);
        s.starten(0, false);
        let ids = satz_ids(&schritt(&mut s));
        assert_eq!(ids.len(), 2001);
        // Gegenprobe: Ohne FIFO-Limit bliebe auch der erste Satz abrufbar.
        assert!(s.satz_text(&ids[0]).is_none());
        assert!(s.satz_text(&ids[1]).is_some());
        assert!(s.satz_text(&ids[2000]).is_some());
        assert_eq!(s.texte.len(), 2000);
    }

    #[test]
    fn ereignisse_und_fehler_tragen_keinen_text() {
        let mut s = sitzung(vec![daten(0, "GEHEIMINHALT.")]);
        s.starten(0, false);
        let ereignisse = schritt(&mut s);
        let json = serde_json::to_string(&ereignisse).unwrap();
        // Gegenprobe: Ein Textfeld oder Klartext in der Satz-ID verrät diesen Marker.
        assert!(!json.contains("GEHEIMINHALT"));
        assert!(!json.contains("original"));
        assert!(!json.contains("uebersetzung"));
        s.spulen(15_000);
        s.werkbank
            .as_mut()
            .unwrap()
            .quelle
            .ergebnisse
            .push_back(daten(15_000, "Neu."));
        s.werkbank.as_mut().unwrap().uebersetzer.fehler = Some("GEHEIMER FEHLERPFAD");
        let ereignisse = schritt(&mut s);
        assert_eq!(s.status().fehler, Some(FehlerCode::Uebersetzung));
        assert!(!serde_json::to_string(&ereignisse)
            .unwrap()
            .contains("GEHEIMER"));
    }

    #[test]
    fn pause_fortsetzung_und_injizierter_tick() {
        let mut s = sitzung(vec![]);
        s.starten(0, true);
        schritt(&mut s);
        schritt(&mut s);
        s.tick(0);
        assert_eq!(s.tick(100).bereit.von_ms, 100);
        s.pausieren();
        s.tick(200);
        // Gegenprobe: Ohne abspielen_setzen würde die Position weiterlaufen.
        assert_eq!(s.tick(300).bereit.von_ms, 100);
        s.fortsetzen();
        s.tick(400);
        assert_eq!(s.tick(500).bereit.von_ms, 200);
    }

    #[test]
    fn auftraege_seriell_und_ergebnisse_zugeordnet() {
        let mut s = sitzung(vec![daten(0, "Ja."), daten(15_000, "Weiter.")]);
        s.starten(0, false);
        let a = s.naechster_auftrag().unwrap();
        // Gegenprobe: Mehrere ausgegebene Fenster erlauben vertauschte Satzbildung.
        assert!(s.naechster_auftrag().is_none());
        let falsch = Auftrag { id: a.id + 1, ..a };
        let e = auswerten(&mut s, a);
        assert!(s.uebernehmen(falsch, e).is_empty());
        assert_eq!(s.status().bereit.bis_ms, 0);
        s.starten(15_000, false);
        assert_eq!(satz_ids(&schritt(&mut s)).len(), 1);
        assert_eq!(s.status().bereit.bis_ms, 30_000);
    }

    #[test]
    fn dateiende_schliesst_rest_auch_ohne_satzzeichen() {
        let mut s = sitzung(vec![daten(0, "Anfang"), daten(15_000, "Ende")]);
        s.starten(0, false);
        assert!(satz_ids(&schritt(&mut s)).is_empty());
        let ids = satz_ids(&schritt(&mut s));
        // Gegenprobe: Ohne Dateiende-Markierung bleibt der letzte Satz im Bildner hängen.
        assert_eq!(ids.len(), 1);
        assert_eq!(s.satz_text(&ids[0]).unwrap().original, "Anfang Ende");
        assert!(s.naechster_auftrag().is_none());
    }

    #[test]
    fn alte_fehler_bleiben_stumm_und_fehlercodes_sind_serialisierbar() {
        let mut s = sitzung(vec![daten(0, "Alt.")]);
        s.werkbank.as_mut().unwrap().uebersetzer.fehler = Some("kein_lokales_modell");
        s.starten(0, false);
        let a = s.naechster_auftrag().unwrap();
        let e = auswerten(&mut s, a);
        s.spulen(15_000);
        // Gegenprobe: Fehler vor der Epochenprüfung auszuwerten sperrt die neue Epoche.
        assert!(s.uebernehmen(a, e).is_empty());
        assert_eq!(s.status().fehler, None);
        for code in [
            FehlerCode::KeinLokalesModell,
            FehlerCode::Quelle,
            FehlerCode::Uebersetzung,
        ] {
            let art = EreignisArt::Fehler { code };
            let json = serde_json::to_string(&art).unwrap();
            assert_eq!(serde_json::from_str::<EreignisArt>(&json).unwrap(), art);
            assert!(super::super::events::aktuell("test", 2, 1, art).is_none());
        }
    }
}
