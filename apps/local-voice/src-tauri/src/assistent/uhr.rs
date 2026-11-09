//! Systemuhr fuer die Einstufung (ADR-0028 A2): lokale Zeit des Geraets.
//!
//! Der Kern `wai-assistent` kennt keine Systemuhr; Uhrzeit und Datum der S0-Antworten kommen
//! ausschliesslich ueber dieses [`Uhr`]-Objekt, in Tests ueber `FesteUhr`.

use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Timelike, Weekday};
use wai_assistent::{Uhr, Wochentag, Zeitpunkt};

/// Lokale Zeit des Geraets (`chrono::Local`).
pub struct SystemUhr;

impl Uhr for SystemUhr {
    fn jetzt(&self) -> Zeitpunkt {
        zeitpunkt_lokal(chrono::Local::now())
    }
}

/// Die Wandzeit einer Zeit mit Zone (Sommerzeit ist im Versatz der Zone enthalten).
pub fn zeitpunkt_lokal<Tz: TimeZone>(zeit: DateTime<Tz>) -> Zeitpunkt {
    zeitpunkt_aus(zeit.naive_local())
}

/// Uebersetzt eine lokale Zeit in den Zeitpunkt des Kerns (getrennt, damit Tests keine Uhr brauchen).
pub fn zeitpunkt_aus(zeit: NaiveDateTime) -> Zeitpunkt {
    Zeitpunkt {
        jahr: zeit.year(),
        // chrono liefert Monat 1-12, Tag 1-31, Stunde 0-23, Minute 0-59: immer im Bereich von u8.
        monat: zeit.month() as u8,
        tag: zeit.day() as u8,
        stunde: zeit.hour() as u8,
        minute: zeit.minute() as u8,
        wochentag: match zeit.weekday() {
            Weekday::Mon => Wochentag::Montag,
            Weekday::Tue => Wochentag::Dienstag,
            Weekday::Wed => Wochentag::Mittwoch,
            Weekday::Thu => Wochentag::Donnerstag,
            Weekday::Fri => Wochentag::Freitag,
            Weekday::Sat => Wochentag::Samstag,
            Weekday::Sun => Wochentag::Sonntag,
        },
    }
}
