//! Schutzbereiche der Sprachausgabe (lva#556): Stücke eines Sprechtexts, die eine Ausspracheregel
//! eines Assistenten eingesetzt hat.
//!
//! Die Ersetzung selbst macht das Frontend (`zerlegeAussprache`, Vertrag in
//! `apps/shared/assistenten/README.md`, Abschnitt „Aussprache“). Es schickt den zusammengesetzten Text
//! und seine Stücke (`teile`); Rust prüft, dass beides zusammenpasst, und hält die eingesetzten Stücke
//! aus der **ganzen** Aufbereitung heraus, die der Text auf dem Weg zur Engine durchläuft (Tags und
//! Leerraum, Datum, Lexikon, Abkürzungstabelle, Punktregeln). Wer ein Wort in eine Aussprache
//! umgeschrieben hat, will sie genau so hören: „Beurlaubung“ als Regel bleibt „Beurlaubung“, auch
//! wenn dasselbe Wort ohne Regel „Be-urlaubung“ würde.
//!
//! Vereinbarungen:
//!
//! * **Bytes aus Rust.** Die Grenzen der Stücke bildet Rust aus den kumulierten UTF-8-Längen
//!   (`String::len`), nie aus Zählungen des Frontends (UTF-16).
//! * **Marken zuerst, auf dem zusammengesetzten Text.** Tags `[…]` und Sprechermarker `<…>` werden
//!   auf dem fertigen Text erkannt; `[` + `lachen` + `]` ist ein Tag, auch wenn ein Stück davon
//!   eingesetzt wurde. Die Behandlung der Marken hat Vorrang vor dem Schutz: Wo eine Engine sie
//!   entfernt, verschwindet auch ein eingesetztes Stück darin.
//! * **Der Schutz wandert mit.** Jeder Eingriff, der Zeichen entfernt (Trimmen, Kappen, Tags,
//!   Leerraumglättung), bildet die Bereiche auf den neuen Text ab; was ganz wegfällt, ist weg, und
//!   leere Bereiche gibt es nicht.
//! * **Die Umgebung bleibt sichtbar.** Die Regeln der Aufbereitung sehen den Text neben einem
//!   geschützten Stück wie jeden anderen (ein Punkt vor einem geschützten Wort ist kein Satzende
//!   mitten im Text, ein Datum nach einem geschützten Wort ist ein Datum). Nur verändert wird nie ein
//!   geschütztes Zeichen, und keine Regel greift über eines hinweg.
//! * **Nichts bleibt zurück.** Der Schutz wird verbraucht, bevor der Schlüssel des Zwischenspeichers
//!   entsteht: der Schlüssel hängt allein am fertig aufbereiteten Engine-Text.

use std::ops::Range;

use serde::Deserialize;
use specta::Type;

/// Meldung, wenn Stücke und Text nicht zusammenpassen. Ohne Inhalt: Texte stehen nie im Log.
pub const TEILE_PASSEN_NICHT: &str = "Aussprache: Die Teile passen nicht zum Text.";

/// Ein Stück des Sprechtexts: unveränderter Text oder die Aussprache eines Treffers. Dieselbe Form
/// liefert `zerlegeAussprache` im Frontend.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
pub struct AusspracheTeil {
    pub text: String,
    pub ersetzt: bool,
}

/// Die Byte-Bereiche (UTF-8) eines Textes, die aus einer Ausspracheregel stammen. Immer sortiert,
/// ohne Überlappung, ohne Berührung (Aneinandergrenzendes ist vereinigt) und nie leer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schutz(Vec<Range<usize>>);

impl Schutz {
    /// Nichts ist geschützt: der Text läuft wie ohne Ausspracheregeln.
    pub fn leer() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn ist_leer(&self) -> bool {
        self.0.is_empty()
    }

    pub fn bereiche(&self) -> &[Range<usize>] {
        &self.0
    }

    /// Bringt beliebige Bereiche in die Form oben: sortieren, leere weglassen, Überlappendes und
    /// Aneinandergrenzendes vereinigen.
    pub fn aus_bereichen(mut bereiche: Vec<Range<usize>>) -> Self {
        bereiche.retain(|b| b.start < b.end);
        bereiche.sort_by_key(|b| b.start);
        let mut fertig: Vec<Range<usize>> = Vec::with_capacity(bereiche.len());
        for b in bereiche {
            match fertig.last_mut() {
                Some(letzter) if b.start <= letzter.end => letzter.end = letzter.end.max(b.end),
                _ => fertig.push(b),
            }
        }
        Self(fertig)
    }

    /// Die Bereiche aus den Stücken des Frontends. Die Stücke müssen **genau** den Text ergeben
    /// (Byte für Byte); sonst ist das Ergebnis ein Fehler, und es wird nichts gesprochen. Leere
    /// Stücke sind harmlos und zählen nicht. Die Grenzen ergeben sich aus `String::len` der Stücke:
    /// jedes Stück ist gültiges UTF-8, die Grenzen liegen also auf Zeichengrenzen des Textes.
    pub fn aus_teilen(text: &str, teile: &[AusspracheTeil]) -> Result<Self, String> {
        let bytes = text.as_bytes();
        let mut stelle = 0usize;
        let mut bereiche = Vec::new();
        for teil in teile {
            let ende = stelle
                .checked_add(teil.text.len())
                .ok_or_else(|| TEILE_PASSEN_NICHT.to_string())?;
            if bytes.get(stelle..ende) != Some(teil.text.as_bytes()) {
                return Err(TEILE_PASSEN_NICHT.to_string());
            }
            if teil.ersetzt {
                bereiche.push(stelle..ende);
            }
            stelle = ende;
        }
        if stelle != text.len() {
            return Err(TEILE_PASSEN_NICHT.to_string());
        }
        Ok(Self::aus_bereichen(bereiche))
    }

    /// Liegt das Byte `stelle` in einem geschützten Bereich?
    #[cfg(test)]
    pub fn enthaelt(&self, stelle: usize) -> bool {
        let nach = self.0.partition_point(|b| b.end <= stelle);
        self.0.get(nach).is_some_and(|b| b.start <= stelle)
    }

    /// Berührt `bereich` einen geschützten Bereich (mindestens ein gemeinsames Byte)?
    pub fn beruehrt(&self, bereich: &Range<usize>) -> bool {
        if bereich.start >= bereich.end {
            return false;
        }
        let nach = self.0.partition_point(|b| b.end <= bereich.start);
        self.0.get(nach).is_some_and(|b| b.start < bereich.end)
    }

    /// Der Schutz im Ausschnitt `von..bis` des Textes, bezogen auf den Anfang des Ausschnitts.
    pub fn ausschnitt(&self, von: usize, bis: usize) -> Self {
        self.durch_auswahl(&[von..bis.max(von)])
    }

    /// Der Schutz nach einem Eingriff, bei dem der neue Text aus den (sortierten, überlappungsfreien)
    /// Bereichen `behalten` des alten Textes besteht, hintereinandergelegt und Byte für Byte gleich
    /// lang. Geschütztes in verworfenen Teilen fällt weg, Bereiche, die dabei leer würden, ebenso.
    pub fn durch_auswahl(&self, behalten: &[Range<usize>]) -> Self {
        let mut neu = Vec::new();
        let mut versatz = 0usize;
        for bereich in behalten {
            uebernimm(self, bereich, versatz, &mut neu);
            versatz += bereich.end.saturating_sub(bereich.start);
        }
        Self::aus_bereichen(neu)
    }

    /// Je Zeichen von `text`: Liegt es in einem geschützten Bereich? Für Regeln, die mit
    /// Zeichenlisten arbeiten.
    pub fn maske(&self, text: &str) -> Vec<bool> {
        let mut maske = Vec::with_capacity(text.len());
        let mut nach = 0usize;
        for (stelle, _) in text.char_indices() {
            while nach < self.0.len() && self.0[nach].end <= stelle {
                nach += 1;
            }
            maske.push(self.0.get(nach).is_some_and(|b| b.start <= stelle));
        }
        maske
    }

    /// Umkehrung von [`Self::maske`]: die Bereiche der Zeichen, die in `maske` gesetzt sind.
    pub fn aus_maske(text: &str, maske: &[bool]) -> Self {
        debug_assert_eq!(text.chars().count(), maske.len());
        let mut bereiche: Vec<Range<usize>> = Vec::new();
        for ((stelle, zeichen), &fest) in text.char_indices().zip(maske) {
            if !fest {
                continue;
            }
            let ende = stelle + zeichen.len_utf8();
            match bereiche.last_mut() {
                Some(letzter) if letzter.end == stelle => letzter.end = ende,
                _ => bereiche.push(stelle..ende),
            }
        }
        Self(bereiche)
    }
}

/// Hängt den Anteil von `schutz` in `bereich` an `ziel`, verschoben auf die Stelle `versatz` im neuen
/// Text.
fn uebernimm(
    schutz: &Schutz,
    bereich: &Range<usize>,
    versatz: usize,
    ziel: &mut Vec<Range<usize>>,
) {
    if bereich.start >= bereich.end {
        return;
    }
    let erster = schutz.0.partition_point(|b| b.end <= bereich.start);
    for b in &schutz.0[erster..] {
        if b.start >= bereich.end {
            break;
        }
        let von = b.start.max(bereich.start);
        let bis = b.end.min(bereich.end);
        ziel.push(versatz + (von - bereich.start)..versatz + (bis - bereich.start));
    }
}

/// Baut aus Stücken eines alten Textes und neu eingesetztem Text einen neuen Text und führt dabei
/// die Schutzbereiche mit: Was von `alt` übernommen wird, behält seinen Schutz, was neu eingesetzt
/// wird, ist ungeschützt. Eingesetzt werden darf nur an Stellen, an denen kein Schutz liegt; das ist
/// Sache des Aufrufers (er prüft [`Schutz::beruehrt`] vor jedem Eingriff).
pub struct Aufbau<'a> {
    alt: &'a str,
    schutz: &'a Schutz,
    text: String,
    bereiche: Vec<Range<usize>>,
}

impl<'a> Aufbau<'a> {
    pub fn neu(alt: &'a str, schutz: &'a Schutz) -> Self {
        Self {
            alt,
            schutz,
            text: String::with_capacity(alt.len() + 16),
            bereiche: Vec::new(),
        }
    }

    /// Übernimmt `alt[bereich]` unverändert, samt der Schutzbereiche darin.
    pub fn behalte(&mut self, bereich: Range<usize>) {
        if bereich.start >= bereich.end {
            return;
        }
        uebernimm(self.schutz, &bereich, self.text.len(), &mut self.bereiche);
        self.text.push_str(&self.alt[bereich]);
    }

    /// Setzt neuen, ungeschützten Text ein.
    pub fn setze(&mut self, neu: &str) {
        self.text.push_str(neu);
    }

    pub fn fertig(self) -> (String, Schutz) {
        (self.text, Schutz::aus_bereichen(self.bereiche))
    }
}

/// Für Tests: der Text und seine Schutzbereiche aus Paaren (Stück, eingesetzt).
#[cfg(test)]
pub(crate) fn aus_paaren(teile: &[(&str, bool)]) -> (String, Schutz) {
    let text: String = teile.iter().map(|(t, _)| *t).collect();
    let liste: Vec<AusspracheTeil> = teile
        .iter()
        .map(|(t, e)| AusspracheTeil {
            text: (*t).to_string(),
            ersetzt: *e,
        })
        .collect();
    let schutz = Schutz::aus_teilen(&text, &liste).expect("Stücke und Text stimmen überein");
    (text, schutz)
}

/// Für Tests: die Stücke von `text`, die `schutz` bezeichnet.
#[cfg(test)]
pub(crate) fn stuecke<'a>(text: &'a str, schutz: &Schutz) -> Vec<&'a str> {
    schutz.bereiche().iter().map(|b| &text[b.clone()]).collect()
}

#[cfg(test)]
mod tests {
    use super::super::{abbreviations, protocol};
    use super::*;

    fn teil(text: &str, ersetzt: bool) -> AusspracheTeil {
        AusspracheTeil {
            text: text.to_string(),
            ersetzt,
        }
    }

    fn schutz(bereiche: &[Range<usize>]) -> Schutz {
        Schutz::aus_bereichen(bereiche.to_vec())
    }

    #[test]
    fn bereiche_werden_sortiert_vereinigt_und_nie_leer() {
        let s = Schutz::aus_bereichen(vec![5..7, 0..2, 2..3, 6..9, 4..4, 12..13]);
        assert_eq!(s.bereiche(), &[0..3, 5..9, 12..13]);
        assert!(Schutz::aus_bereichen(vec![3..3, 7..2]).ist_leer());
        assert!(Schutz::leer().ist_leer());
    }

    #[test]
    fn die_grenzen_kommen_aus_utf8_bytes_nicht_aus_zeichen_oder_utf16() {
        // „é“ (2 Byte), „😀“ (4 Byte, 2 UTF-16-Einheiten), „東“ (3 Byte), zerlegtes e (1 + 2 Byte).
        let text = "Café 😀 東京 e\u{301}!";
        let teile = [
            teil("Café ", false),
            teil("😀", true),
            teil(" ", false),
            teil("東京", true),
            teil(" e\u{301}!", false),
        ];
        let s = Schutz::aus_teilen(text, &teile).unwrap();
        let emoji = "Café ".len();
        let tokio = emoji + "😀 ".len();
        assert_eq!(s.bereiche(), &[emoji..emoji + 4, tokio..tokio + 6]);
        assert_eq!(&text[s.bereiche()[0].clone()], "😀");
        assert_eq!(&text[s.bereiche()[1].clone()], "東京");
        // Die Grenzen liegen alle auf Zeichengrenzen.
        for b in s.bereiche() {
            assert!(text.is_char_boundary(b.start) && text.is_char_boundary(b.end));
        }
    }

    #[test]
    fn aneinandergrenzende_stuecke_werden_ein_bereich_und_leere_zaehlen_nicht() {
        let text = "WolfWolf!";
        let teile = [
            teil("Wolf", true),
            teil("", true),
            teil("Wolf", true),
            teil("", false),
            teil("!", false),
        ];
        assert_eq!(
            Schutz::aus_teilen(text, &teile).unwrap().bereiche(),
            &[0..8]
        );
        // Nur unersetzte Stücke: nichts ist geschützt, der alte Textpfad.
        let s = Schutz::aus_teilen("a b", &[teil("a", false), teil(" b", false)]).unwrap();
        assert!(s.ist_leer());
    }

    #[test]
    fn stuecke_die_nicht_zum_text_passen_sind_ein_fehler() {
        let fall = |text: &str, teile: &[AusspracheTeil]| Schutz::aus_teilen(text, teile);
        // Zu kurz, zu lang, anderer Inhalt, andere Reihenfolge, leere Liste bei vorhandenem Text.
        assert!(fall("abc", &[teil("ab", true)]).is_err());
        assert!(fall("abc", &[teil("abc", true), teil("d", false)]).is_err());
        assert!(fall("abc", &[teil("abd", true)]).is_err());
        assert!(fall("abc", &[teil("c", false), teil("ab", true)]).is_err());
        assert!(fall("abc", &[]).is_err());
        // Gleiche Bytelänge, anderer Inhalt: „é“ ist nicht „e“ + U+0301 und nicht „ä“.
        assert!(fall("é", &[teil("ä", true)]).is_err());
        assert!(fall("é", &[teil("e\u{301}", true)]).is_err());
        // Leerer Text mit leerer Liste ist stimmig (gesprochen wird dann nichts).
        assert!(fall("", &[]).unwrap().ist_leer());
        assert_eq!(
            fall("abc", &[teil("ab", true)]).unwrap_err(),
            TEILE_PASSEN_NICHT
        );
    }

    #[test]
    fn enthaelt_und_beruehrt_zaehlen_bytes_genau() {
        let s = schutz(&[2..4, 10..12]);
        assert!(!s.enthaelt(1) && s.enthaelt(2) && s.enthaelt(3) && !s.enthaelt(4));
        assert!(s.enthaelt(10) && s.enthaelt(11) && !s.enthaelt(12));
        assert!(!s.beruehrt(&(0..2)) && s.beruehrt(&(0..3)) && s.beruehrt(&(3..9)));
        assert!(!s.beruehrt(&(4..10)) && s.beruehrt(&(4..11)));
        assert!(!s.beruehrt(&(3..3)), "ein leerer Bereich berührt nichts");
        assert!(!Schutz::leer().beruehrt(&(0..100)));
    }

    #[test]
    fn auswahl_verschiebt_beschneidet_und_entfernt_leere_bereiche() {
        let s = schutz(&[2..6, 8..9]);
        // Von 0..10 bleiben 1..3 und 5..10: aus 2..6 werden 1..2 (im ersten Stück) und 2..3 (im zweiten),
        // vereinigt 1..3; aus 8..9 wird 5..6.
        let n = s.durch_auswahl(&[1..3, 5..10]);
        assert_eq!(n.bereiche(), &[1..3, 5..6]);
        // Bereich ganz in verworfenem Teil: weg.
        assert!(s.durch_auswahl(&[0..2, 6..8]).ist_leer());
        // Ausschnitt: bezogen auf den Anfang.
        assert_eq!(s.ausschnitt(3, 9).bereiche(), &[0..3, 5..6]);
        assert!(s.ausschnitt(6, 8).ist_leer());
        assert!(s.ausschnitt(5, 5).ist_leer());
        // Ein verkehrter Ausschnitt ist leer, kein Absturz.
        assert!(s.ausschnitt(9, 3).ist_leer());
    }

    #[test]
    fn maske_und_bereiche_sind_umkehrbar_auch_mit_mehrbyte_zeichen() {
        let text = "a😀é東b";
        let s = Schutz::aus_teilen(
            text,
            &[teil("a", false), teil("😀é", true), teil("東b", false)],
        )
        .unwrap();
        let maske = s.maske(text);
        assert_eq!(maske, vec![false, true, true, false, false]);
        assert_eq!(Schutz::aus_maske(text, &maske), s);
        assert!(Schutz::leer().maske(text).iter().all(|f| !f));
    }

    #[test]
    fn aufbau_fuehrt_den_schutz_ueber_eingesetzten_text_mit() {
        let alt = "ab[CD]ef";
        let s = schutz(&[2..6]);
        let mut aus = Aufbau::neu(alt, &s);
        aus.behalte(0..2);
        aus.setze("XYZ");
        aus.behalte(2..6);
        aus.setze("_");
        aus.behalte(6..8);
        let (text, neu) = aus.fertig();
        assert_eq!(text, "abXYZ[CD]_ef");
        assert_eq!(neu.bereiche(), &[5..9]);
        assert_eq!(&text[neu.bereiche()[0].clone()], "[CD]");
    }

    // ------------------------------------------------------------------
    // Gemeinsame Prüfstände: dieselben Fälle wie das Frontend
    // ------------------------------------------------------------------

    /// Die Fälle des Abschnitts `ersetzung` (LF, auch unter Windows: `.gitattributes`).
    const PRUEFSTAENDE: &str =
        include_str!("../../../../../shared/assistenten/aussprache-pruefstaende.json");

    struct Fall {
        name: String,
        erwartet: String,
        teile: Vec<AusspracheTeil>,
    }

    fn faelle() -> Vec<Fall> {
        let wurzel: serde_json::Value = serde_json::from_str(PRUEFSTAENDE).expect("JSON");
        wurzel["ersetzung"]
            .as_array()
            .expect("Abschnitt ersetzung")
            .iter()
            .map(|f| Fall {
                name: f["name"].as_str().unwrap().to_string(),
                erwartet: f["erwartet"].as_str().unwrap().to_string(),
                teile: f["teile"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| AusspracheTeil {
                        text: t["text"].as_str().unwrap().to_string(),
                        ersetzt: t["ersetzt"].as_bool().unwrap(),
                    })
                    .collect(),
            })
            .collect()
    }

    fn ohne_leerraum(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Alle Fälle der gemeinsamen Datei, so wie das Frontend sie schickt: die Stücke ergeben den
    /// erwarteten Text, und die Bereiche aus Bytes treffen genau die eingesetzten Stücke.
    #[test]
    fn die_teile_der_gemeinsamen_faelle_ergeben_text_und_bereiche() {
        let alle = faelle();
        assert_eq!(alle.len(), 44, "die Datei hat sich geändert: Fälle prüfen");
        for fall in &alle {
            let zusammen: String = fall.teile.iter().map(|t| t.text.as_str()).collect();
            assert_eq!(zusammen, fall.erwartet, "{}", fall.name);
            let s = Schutz::aus_teilen(&fall.erwartet, &fall.teile)
                .unwrap_or_else(|e| panic!("{}: {e}", fall.name));
            let eingesetzt: String = s
                .bereiche()
                .iter()
                .map(|b| &fall.erwartet[b.clone()])
                .collect();
            let soll: String = fall
                .teile
                .iter()
                .filter(|t| t.ersetzt)
                .map(|t| t.text.as_str())
                .collect();
            assert_eq!(eingesetzt, soll, "{}", fall.name);
            // Ein Stück weniger oder mehr ist ein Widerspruch.
            if !fall.erwartet.is_empty() {
                let mut zu_kurz = fall.teile.clone();
                zu_kurz.pop();
                assert!(
                    Schutz::aus_teilen(&fall.erwartet, &zu_kurz).is_err(),
                    "{}: fehlendes letztes Stück",
                    fall.name
                );
            }
            let mut zu_lang = fall.teile.clone();
            zu_lang.push(teil("x", false));
            assert!(
                Schutz::aus_teilen(&fall.erwartet, &zu_lang).is_err(),
                "{}: überzähliges Stück",
                fall.name
            );
        }
    }

    /// Dieselben Fälle durch die Aufbereitung, die der Text auf dem Weg zur Engine durchläuft
    /// (Trimmen und Kappen, Tags, Leerraum, Ausschreiben), für jede Tag-Fähigkeit: Die eingesetzten
    /// Stücke kommen unverändert und in Reihenfolge an. Mit Stil-Tags (Fish) Byte für Byte; wo der
    /// Leerraum geglättet wird (Piper, Phonon), bis auf Leerraum.
    #[test]
    fn die_teile_der_gemeinsamen_faelle_laufen_unveraendert_durch_die_aufbereitung() {
        for fall in &faelle() {
            let s = Schutz::aus_teilen(&fall.erwartet, &fall.teile).unwrap();
            let eingesetzt: Vec<&str> = s
                .bereiche()
                .iter()
                .map(|b| &fall.erwartet[b.clone()])
                .collect();
            let Some((vorbereitet, schutz)) =
                protocol::prepare_text_mit_schutz(&fall.erwartet, 100_000, &s)
            else {
                assert!(
                    fall.erwartet.trim().is_empty(),
                    "{}: nur Leerraum fällt weg",
                    fall.name
                );
                continue;
            };
            // Stil-Tags bleiben stehen (Fish): nichts außer dem Ausschreiben.
            let (fish, fish_schutz) =
                abbreviations::expand_if_german_mit_schutz(&vorbereitet.text, &schutz);
            // Piper: alle Tags und der Leerraum.
            let (ohne, ohne_schutz) =
                protocol::strip_tag_spans_mit_schutz(&vorbereitet.text, &schutz);
            let (piper, piper_schutz) =
                abbreviations::expand_if_german_mit_schutz(&ohne, &ohne_schutz);
            // Phonon: nur Tempo-Tags bleiben.
            let (tempo, tempo_schutz) =
                protocol::strip_tag_spans_keep_pace_mit_schutz(&vorbereitet.text, &schutz);
            let (phonon, phonon_schutz) =
                abbreviations::expand_if_german_mit_schutz(&tempo, &tempo_schutz);

            // Fish: die Stücke stehen Byte für Byte an den abgebildeten Stellen, außer wo ein Tag
            // (Vorrang der Marken) ein eingesetztes Stück mit umschließt.
            let tags = protocol::tag_spans(&vorbereitet.text);
            let im_tag = |b: &Range<usize>| tags.iter().any(|t| t.start < b.end && b.start < t.end);
            if !s.bereiche().iter().any(|b| im_tag(b)) {
                let gefunden: Vec<&str> = fish_schutz
                    .bereiche()
                    .iter()
                    .map(|b| &fish[b.clone()])
                    .collect();
                assert_eq!(gefunden, eingesetzt, "{} (Stil-Tags)", fall.name);
            }
            // Mit Glättung: bis auf Leerraum, und nie mehr, als eingesetzt wurde.
            for (name, text, bleibt) in [
                ("Piper", &piper, &piper_schutz),
                ("Phonon", &phonon, &phonon_schutz),
            ] {
                let gefunden: String = bleibt.bereiche().iter().map(|b| &text[b.clone()]).collect();
                let soll: String = s
                    .bereiche()
                    .iter()
                    .map(|b| &fall.erwartet[b.clone()])
                    .collect();
                if s.bereiche().iter().any(|b| im_tag(b)) {
                    assert!(
                        ohne_leerraum(&soll).contains(&ohne_leerraum(&gefunden)),
                        "{} ({name})",
                        fall.name
                    );
                } else {
                    assert_eq!(
                        ohne_leerraum(&gefunden),
                        ohne_leerraum(&soll),
                        "{} ({name})",
                        fall.name
                    );
                }
            }
        }
    }
}
