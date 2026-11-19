use super::events::{aktuell, Bereich, Ereignis, EreignisArt, Grund, Zustand};

pub const STARTPUFFER_MS: u64 = 45_000;
pub const ZIELVORSPRUNG_MS: u64 = 90_000;
pub const FENSTER_MS: u64 = 15_000;
pub const MAX_FENSTER: usize = 3;
pub const MAX_KUERZUNGS_VERSUCHE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modus {
    Untertitel,
    Synchronsprechen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub enum Pufferzustand {
    Fuellend,
    Bereit,
    Leer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aktion {
    PausierenGemeinsam,
    FortsetzenGemeinsam,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schritt {
    pub puffer: Pufferzustand,
    pub bereit: Bereich,
    pub aktion: Option<Aktion>,
    pub ereignisse: Vec<Ereignis>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fenster {
    pub epoch: u64,
    pub id: u64,
    pub bereich: Bereich,
}

pub struct Planer {
    session_id: String,
    epoch: u64,
    modus: Modus,
    dauer_ms: u64,
    position_ms: u64,
    cursor_ms: u64,
    naechste_id: u64,
    offen: Vec<Fenster>,
    fertig: Vec<(Bereich, bool)>,
    spielt: bool,
    fortsetzen: bool,
    gestartet: bool,
    rate_promille: u32,
    letzter_tick: Option<u64>,
    rest_promille: u64,
    letzter_zustand: Option<Zustand>,
}

impl Planer {
    pub fn neu(session_id: String, dauer_ms: u64, modus: Modus) -> Self {
        Self {
            session_id,
            epoch: 0,
            modus,
            dauer_ms,
            position_ms: 0,
            cursor_ms: 0,
            naechste_id: 0,
            offen: Vec::new(),
            fertig: Vec::new(),
            spielt: false,
            fortsetzen: false,
            gestartet: false,
            rate_promille: 1000,
            letzter_tick: None,
            rest_promille: 0,
            letzter_zustand: None,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn start(&mut self, position_ms: u64, spielt: bool) {
        self.spielt = spielt;
        self.fortsetzen = false;
        self.position_setzen(position_ms);
    }

    /// Jeder Seek ersetzt sämtliche offenen Aufträge der vorherigen Epoche.
    pub fn position_setzen(&mut self, position_ms: u64) {
        self.epoch = self.epoch.saturating_add(1);
        self.position_ms = position_ms.min(self.dauer_ms);
        self.cursor_ms = self.position_ms;
        self.offen.clear();
        self.fertig.clear();
        self.gestartet = false;
        self.letzter_tick = None;
        self.rest_promille = 0;
        self.letzter_zustand = None;
    }

    pub fn rate_aendern(&mut self, rate_promille: u32) {
        if self.rate_promille != rate_promille && rate_promille > 0 {
            self.rate_promille = rate_promille;
            self.position_setzen(self.position_ms);
        }
    }

    /// Explizites Pausieren widerruft auch eine vorgemerkte automatische Fortsetzung.
    pub fn abspielen_setzen(&mut self, spielt: bool) {
        self.spielt = spielt;
        self.fortsetzen = false;
        self.letzter_tick = None;
    }

    pub fn neues_fenster(&mut self) -> Option<Fenster> {
        self.cursor_ms = self.cursor_ms.max(self.position_ms);
        let ziel = self
            .position_ms
            .saturating_add(ZIELVORSPRUNG_MS)
            .min(self.dauer_ms);
        if self.offen.len() >= MAX_FENSTER || self.cursor_ms >= ziel {
            return None;
        }
        self.naechste_id = self.naechste_id.saturating_add(1);
        let fenster = Fenster {
            epoch: self.epoch,
            id: self.naechste_id,
            bereich: Bereich {
                von_ms: self.cursor_ms,
                bis_ms: self.cursor_ms.saturating_add(FENSTER_MS).min(ziel),
            },
        };
        self.cursor_ms = fenster.bereich.bis_ms;
        self.offen.push(fenster);
        Some(fenster)
    }

    /// Erfolg bestätigt das ganze Fenster einschließlich Stille, niemals nur Satzenden.
    /// Im Sprechmodus zählt auch ein abgeschlossener Untertitel-Rückfall als clipbereit.
    /// Ein Fehler wird mit `fenster_wiederholen` erneut eingeplant.
    pub fn ergebnis_melden(
        &mut self,
        epoche: u64,
        fenster_id: u64,
        sprachclips_fertig: bool,
    ) -> Vec<Ereignis> {
        if epoche != self.epoch {
            return Vec::new();
        }
        if let Some(index) = self.offen.iter().position(|f| f.id == fenster_id) {
            let fenster = self.offen.remove(index);
            self.fertig.push((fenster.bereich, sprachclips_fertig));
        } else {
            return Vec::new();
        }
        aktuell(
            &self.session_id,
            self.epoch,
            epoche,
            EreignisArt::Bereitschaft(self.bereit()),
        )
        .into_iter()
        .collect()
    }

    pub fn sprachclips_melden(&mut self, epoche: u64, bereich: Bereich) -> Vec<Ereignis> {
        if epoche != self.epoch {
            return Vec::new();
        }
        let mut geaendert = false;
        for (fertig, clips) in &mut self.fertig {
            if *fertig == bereich && !*clips {
                *clips = true;
                geaendert = true;
            }
        }
        if !geaendert {
            return Vec::new();
        }
        aktuell(
            &self.session_id,
            self.epoch,
            epoche,
            EreignisArt::Bereitschaft(self.bereit()),
        )
        .into_iter()
        .collect()
    }

    pub fn fenster_wiederholen(&self, epoche: u64, id: u64) -> Option<Fenster> {
        self.offen
            .iter()
            .copied()
            .find(|f| f.epoch == epoche && f.id == id)
    }

    pub fn bereit(&self) -> Bereich {
        let mut bis_ms = self.position_ms;
        loop {
            let vorher = bis_ms;
            for (bereich, clips) in &self.fertig {
                if bereich.von_ms <= bis_ms && (self.modus == Modus::Untertitel || *clips) {
                    bis_ms = bis_ms.max(bereich.bis_ms);
                }
            }
            if bis_ms == vorher {
                break;
            }
        }
        Bereich {
            von_ms: self.position_ms,
            bis_ms,
        }
    }

    /// `jetzt_ms` stammt aus einer injizierten monotonen Uhr, nicht aus der Systemzeit.
    pub fn tick(&mut self, jetzt_ms: u64) -> Schritt {
        if let Some(vorher) = self.letzter_tick {
            if self.spielt && self.gestartet {
                let schritte = jetzt_ms
                    .saturating_sub(vorher)
                    .saturating_mul(u64::from(self.rate_promille))
                    .saturating_add(self.rest_promille);
                self.rest_promille = schritte % 1000;
                let pufferende = self.bereit().bis_ms;
                self.position_ms = self
                    .position_ms
                    .saturating_add(schritte / 1000)
                    .min(pufferende)
                    .min(self.dauer_ms);
                if self.position_ms == pufferende {
                    self.rest_promille = 0;
                }
                self.fertig
                    .retain(|(bereich, _)| bereich.bis_ms > self.position_ms);
            }
        }
        self.letzter_tick = Some(jetzt_ms);
        let bereit = self.bereit();
        let vorrat = bereit.bis_ms.saturating_sub(bereit.von_ms);
        let erforderlich = STARTPUFFER_MS.min(self.dauer_ms - self.position_ms);
        let genug = vorrat >= erforderlich || (self.gestartet && vorrat > 0);
        let puffer = if genug {
            self.gestartet = true;
            Pufferzustand::Bereit
        } else if vorrat == 0 {
            self.gestartet = false;
            Pufferzustand::Leer
        } else {
            Pufferzustand::Fuellend
        };
        let mut aktion = None;
        let zustand = if genug && self.fortsetzen {
            self.spielt = true;
            self.fortsetzen = false;
            aktion = Some(Aktion::FortsetzenGemeinsam);
            Zustand::Fortgesetzt
        } else if !genug && (self.spielt || self.fortsetzen) {
            if self.spielt {
                self.spielt = false;
                self.fortsetzen = true;
                aktion = Some(Aktion::PausierenGemeinsam);
            }
            Zustand::PausiertWegenPuffer
        } else if genug {
            Zustand::Bereit
        } else {
            Zustand::Fuellend
        };
        let mut ereignisse = Vec::new();
        if self.letzter_zustand != Some(zustand) {
            ereignisse.extend(aktuell(
                &self.session_id,
                self.epoch,
                self.epoch,
                EreignisArt::Zustandswechsel(zustand),
            ));
            self.letzter_zustand = Some(zustand);
        }
        Schritt {
            puffer,
            bereit,
            aktion,
            ereignisse,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaengenAktion {
    Passt,
    KuerzerNeuUebersetzen { versuch: u8 },
    SynchronNichtVerfuegbar(Grund),
}

/// Der Folgesatz behält seine Startzeit; lediglich vorhandene Pause darf genutzt werden.
pub fn laenge_pruefen(
    start_ms: u64,
    end_ms: u64,
    naechster_start_ms: u64,
    clip_ms: u64,
    versuche: u8,
    synchron_erlaubt: bool,
) -> LaengenAktion {
    if !synchron_erlaubt {
        return LaengenAktion::SynchronNichtVerfuegbar(Grund::GrobeZeitmarken);
    }
    let satz_ms = end_ms.saturating_sub(start_ms);
    let pause_ms = naechster_start_ms.saturating_sub(end_ms.max(start_ms));
    let platz_ms = satz_ms
        .saturating_add(pause_ms)
        .min(naechster_start_ms.saturating_sub(start_ms));
    if clip_ms <= platz_ms {
        LaengenAktion::Passt
    } else if versuche < MAX_KUERZUNGS_VERSUCHE {
        LaengenAktion::KuerzerNeuUebersetzen {
            versuch: versuche + 1,
        }
    } else {
        LaengenAktion::SynchronNichtVerfuegbar(Grund::Ueberlaenge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fuellen(planer: &mut Planer, clips: bool) {
        while let Some(fenster) = planer.neues_fenster() {
            planer.ergebnis_melden(fenster.epoch, fenster.id, clips);
        }
    }

    #[test]
    fn grenzen_luecken_stille_und_dateiende() {
        let mut planer = Planer::neu("s".into(), 100_000, Modus::Untertitel);
        planer.start(0, false);
        let a = planer.neues_fenster().expect("a");
        let b = planer.neues_fenster().expect("b");
        let c = planer.neues_fenster().expect("c");
        // Gegenprobe: Ohne Warteschlangenlimit wäre ein viertes Fenster möglich.
        assert!(planer.neues_fenster().is_none());
        planer.ergebnis_melden(b.epoch, b.id, false);
        // Gegenprobe: Das größte Satzende würde die unverarbeitete Lücke überspringen.
        assert_eq!(planer.bereit().bis_ms, 0);
        planer.ergebnis_melden(a.epoch, a.id, false);
        assert_eq!(planer.bereit().bis_ms, 30_000);
        // Gegenprobe: Ohne Startschwelle wäre schon der Teilpuffer bereit.
        assert_eq!(planer.tick(0).puffer, Pufferzustand::Fuellend);
        planer.ergebnis_melden(c.epoch, c.id, false);
        assert_eq!(planer.tick(0).puffer, Pufferzustand::Bereit);
        fuellen(&mut planer, false);
        assert_eq!(planer.bereit().bis_ms, ZIELVORSPRUNG_MS);
        planer.start(95_000, false);
        fuellen(&mut planer, false);
        // Gegenprobe: Ein starrer 45-s-Puffer würde am Dateiende nie bereit.
        assert_eq!(planer.bereit().bis_ms, 100_000);
        assert_eq!(planer.tick(0).puffer, Pufferzustand::Bereit);
    }

    #[test]
    fn seeks_und_rate_verwerfen_alte_ergebnisse() {
        let mut planer = Planer::neu("s".into(), 200_000, Modus::Untertitel);
        planer.start(0, true);
        assert_eq!(planer.epoch(), 1);
        let alt = planer.neues_fenster().expect("alt");
        planer.position_setzen(50_000);
        planer.position_setzen(80_000);
        assert_eq!(planer.epoch(), 3);
        // Gegenprobe: Ohne Epochenwechsel würde das alte Ergebnis ein Ereignis erzeugen.
        assert!(planer.ergebnis_melden(alt.epoch, alt.id, true).is_empty());
        let neu = planer.neues_fenster().expect("neu");
        assert_eq!(neu.bereich.von_ms, 80_000);
        assert!(planer.fenster_wiederholen(alt.epoch, alt.id).is_none());
        planer.rate_aendern(1500);
        assert_eq!(planer.epoch(), 4);
        assert!(planer.ergebnis_melden(neu.epoch, neu.id, true).is_empty());
    }

    #[test]
    fn pausierregel_und_modus() {
        for vorher_spielend in [false, true] {
            let mut planer = Planer::neu("s".into(), 200_000, Modus::Synchronsprechen);
            planer.start(0, vorher_spielend);
            // Gegenprobe: Ohne gespeicherte Abspielabsicht würde auch eine Pause fortgesetzt.
            assert_eq!(
                planer.tick(0).aktion,
                vorher_spielend.then_some(Aktion::PausierenGemeinsam)
            );
            fuellen(&mut planer, false);
            assert_eq!(planer.tick(1).puffer, Pufferzustand::Leer);
            let bereiche: Vec<_> = planer.fertig.iter().map(|(b, _)| *b).collect();
            for bereich in bereiche {
                let epoch = planer.epoch();
                // Gegenprobe: Auch verspätete Clipmeldungen dürfen nichts veröffentlichen.
                assert!(planer.sprachclips_melden(epoch - 1, bereich).is_empty());
                assert_eq!(planer.sprachclips_melden(epoch, bereich).len(), 1);
            }
            assert_eq!(
                planer.tick(2).aktion,
                vorher_spielend.then_some(Aktion::FortsetzenGemeinsam)
            );
            if vorher_spielend {
                // Gegenprobe: Ohne gemeinsamen Stopp läuft das Medium über das Pufferende.
                assert_eq!(
                    planer.tick(100_002).aktion,
                    Some(Aktion::PausierenGemeinsam)
                );
                assert_eq!(planer.bereit().von_ms, ZIELVORSPRUNG_MS);
                fuellen(&mut planer, true);
                assert_eq!(
                    planer.tick(100_003).aktion,
                    Some(Aktion::FortsetzenGemeinsam)
                );
            }
        }
    }

    #[test]
    fn injizierte_uhr_rate_und_explizite_pause() {
        let mut planer = Planer::neu("s".into(), 200_000, Modus::Untertitel);
        planer.start(0, true);
        planer.rate_aendern(1500);
        fuellen(&mut planer, false);
        planer.tick(10);
        // Gegenprobe: Ohne Rate oder mit Verlust der Bruchteile stimmt die Position nicht.
        assert_eq!(planer.tick(11).bereit.von_ms, 1);
        assert_eq!(planer.tick(12).bereit.von_ms, 3);
        planer.position_setzen(150_000);
        assert_eq!(planer.tick(20).aktion, Some(Aktion::PausierenGemeinsam));
        planer.abspielen_setzen(false);
        fuellen(&mut planer, false);
        // Gegenprobe: Ein explizites Pausieren muss die automatische Fortsetzung widerrufen.
        assert_eq!(planer.tick(30).aktion, None);
        assert_eq!(planer.tick(40).bereit.von_ms, 150_000);
    }

    #[test]
    fn ueberlaenge_begrenzt_ohne_verschieben() {
        // Gegenprobe: Ohne Pausennutzung würde ein passender Überlauf gekürzt.
        assert_eq!(
            laenge_pruefen(0, 100, 150, 140, 0, true),
            LaengenAktion::Passt
        );
        for versuche in 0..MAX_KUERZUNGS_VERSUCHE {
            assert_eq!(
                laenge_pruefen(0, 100, 150, 151, versuche, true),
                LaengenAktion::KuerzerNeuUebersetzen {
                    versuch: versuche + 1
                }
            );
        }
        // Gegenprobe: Ohne Versuchslimit würde die Neuübersetzung endlos wiederholt.
        assert_eq!(
            laenge_pruefen(0, 100, 150, 151, MAX_KUERZUNGS_VERSUCHE, true),
            LaengenAktion::SynchronNichtVerfuegbar(Grund::Ueberlaenge)
        );
        assert_eq!(
            laenge_pruefen(0, 100, 150, 50, 0, false),
            LaengenAktion::SynchronNichtVerfuegbar(Grund::GrobeZeitmarken)
        );
    }
}
