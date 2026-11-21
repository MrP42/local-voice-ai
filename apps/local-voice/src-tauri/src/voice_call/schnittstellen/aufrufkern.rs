//! Plattformneutrale Aufrufhuelle mit Freigabepolitik ohne eigene Netzadapter.
use std::cell::Cell;

use super::anbindung::{
    self, Abbruch, Antwortrunde, Ausgang, Namensaufloesung, Schnappschuss, Verbindungsaufbau,
};
use super::bestaetigung::Freigabe;
use super::kodierung::{lies_json, Wert};
use super::richtlinie::{Richtlinie, Widerrufspruefung};
use super::{rahmung, werkzeug};

pub(crate) struct Aufrufergebnis {
    pub ergebnis: &'static str,
    pub daten: Option<Wert>,
    pub meldung: Option<String>,
    pub protokollzeile: String,
}

fn ohne_lauf(
    ergebnis: &'static str,
    sprache: &str,
    untercode: Option<&str>,
    parameter: Option<&str>,
) -> Aufrufergebnis {
    Aufrufergebnis {
        ergebnis,
        daten: None,
        meldung: rahmung::meldung(ergebnis, sprache, untercode, parameter),
        protokollzeile: String::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fuehre_aus(
    verzeichnis_eintraege: &[Wert],
    aktive_ids: &[String],
    werkzeugname: &str,
    arguments_json: &str,
    runde: &mut Antwortrunde,
    abbruch: &Abbruch,
    frage: Option<(&Freigabe, u64)>,
    richtlinie: &impl Richtlinie,
    dns: &mut impl Namensaufloesung,
    verbindung: &mut impl Verbindungsaufbau,
    assistent: &str,
    sprache: &str,
    zeit: &str,
) -> Aufrufergebnis {
    let eintrag = werkzeug::aufloesen(aktive_ids, werkzeugname).and_then(|id| {
        verzeichnis_eintraege
            .iter()
            .find(|e| e["id"].text() == Some(id.as_str()))
    });
    let Some(eintrag) = eintrag else {
        return ohne_lauf("nicht_vorhanden", sprache, None, None);
    };
    let kennung = eintrag["id"].s();
    if richtlinie.widerrufen(kennung)
        || (eintrag["abfluss"].text().unwrap_or("netz") == "netz"
            && !richtlinie.netzabfluss_erlaubt(kennung))
    {
        return ohne_lauf("abgelehnt", sprache, None, None);
    }
    let argumente = match lies_json(arguments_json) {
        Ok(wert) if wert.map().is_some() => wert,
        _ => return ohne_lauf("abgelehnt", sprache, Some("argumente_typ"), None),
    };
    let s = match Schnappschuss::neu(eintrag, &argumente, None) {
        Ok(s) => s,
        Err(fehler) => {
            return ohne_lauf(
                "abgelehnt",
                sprache,
                fehler["code"].text(),
                fehler["parameter"].text(),
            );
        }
    };
    let dummy = Ausgang {
        ergebnis: "abgelehnt",
        status: None,
        koerper: vec![],
        dauer_ms: 0,
        gesendet_bytes: 0,
        empfangen_bytes: 0,
    };
    if anbindung::vertragsprotokoll(&s, &dummy, assistent, zeit, false).is_err() {
        return ohne_lauf("abgelehnt", sprache, Some("protokoll_ungueltig"), None);
    }
    // Widerruf nochmals direkt vor Lauf und Versand pruefen, ohne einen Platz zu verbrauchen.
    if richtlinie.widerrufen(kennung) {
        return ohne_lauf("abgelehnt", sprache, None, None);
    }
    let blockiert = Cell::new(false);
    let mut pruefung = Widerrufspruefung {
        verbindung,
        richtlinie,
        kennung,
        blockiert: &blockiert,
    };
    let mut ausgang = match runde.naechster_aufruf(&s, abbruch) {
        Ok(lauf) => anbindung::versende(&s, frage, dns, &mut pruefung, &lauf),
        Err(_) => dummy,
    };
    if blockiert.get()
        && ausgang.gesendet_bytes == 0
        && matches!(ausgang.ergebnis, "netzfehler" | "unklar")
    {
        // Die Freigabe ist verbraucht, aber ohne Versand wird nichts als bestaetigt protokolliert.
        ausgang.ergebnis = "abgebrochen";
    }
    // Das Transportergebnis belegt das Einloesen, auch ohne gesendete Bytes.
    let bestaetigt = s.schreibend()
        && matches!(
            ausgang.ergebnis,
            "ok" | "http_fehler" | "umleitung" | "unklar" | "zu_gross"
        );
    let mut daten = None;
    if !ausgang.koerper.is_empty() {
        let limit = ausgang.status.map_or(s.antwort_max_bytes(), |status| {
            rahmung::antwort_limit(status, s.antwort_max_bytes())
        });
        match rahmung::rahme_zufaellig(s.kennung(), &ausgang.koerper, limit) {
            Ok(block) => daten = Some(block),
            Err(_) => ausgang.ergebnis = "antwort_ungueltig",
        }
    }
    // Eine einzige Zeile zurueckgeben; das Loggen uebernimmt der spaetere Aufrufer.
    Aufrufergebnis {
        ergebnis: ausgang.ergebnis,
        daten,
        meldung: rahmung::meldung(ausgang.ergebnis, sprache, None, None),
        protokollzeile: anbindung::protokolltext(&s, &ausgang, assistent, zeit, bestaetigt),
    }
}
