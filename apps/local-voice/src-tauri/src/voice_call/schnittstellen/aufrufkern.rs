//! Plattformneutrale Aufrufhuelle ohne Freigabeentscheidung oder eigene Netzadapter.
use super::anbindung::{
    self, Abbruch, Antwortrunde, Ausgang, Namensaufloesung, Schnappschuss, Verbindungsaufbau,
};
use super::bestaetigung::Freigabe;
use super::kodierung::{lies_json, Wert};
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
    let mut ausgang = match runde.naechster_aufruf(&s, abbruch) {
        Ok(lauf) => anbindung::versende(&s, frage, dns, verbindung, &lauf),
        Err(_) => Ausgang {
            ergebnis: "abgelehnt",
            status: None,
            koerper: vec![],
            dauer_ms: 0,
            gesendet_bytes: 0,
            empfangen_bytes: 0,
        },
    };
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
    let bestaetigt = s.schreibend() && frage.is_some() && ausgang.gesendet_bytes > 0;
    Aufrufergebnis {
        ergebnis: ausgang.ergebnis,
        daten,
        meldung: rahmung::meldung(ausgang.ergebnis, sprache, None, None),
        protokollzeile: anbindung::protokolltext(&s, &ausgang, assistent, zeit, bestaetigt),
    }
}
