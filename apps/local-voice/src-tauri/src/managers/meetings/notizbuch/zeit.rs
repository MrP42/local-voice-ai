//! Blockzeiten als Intervall mit Güte und ihre Abbildung auf die Zeitachse einer Aufnahme
//! (Spec §5.1 und §5.3). Alle Zeiten in Millisekunden seit Unix-Beginn.

use chrono::{DateTime, NaiveDateTime};
use wai_notizbuch_zugang::{Block, Seitenkopf};

/// Fenster vor dem Beginn der Besprechung (15 min).
pub const FENSTER_VOR_MS: i64 = 15 * 60_000;
/// Fenster nach dem Ende der Besprechung (30 min).
pub const FENSTER_NACH_MS: i64 = 30 * 60_000;

/// Güte einer Blockzeit; stärkere Güte steht vorn (`Genau < Version < Seite`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Guete {
    /// Attribut `zeit` des Blocks.
    Genau,
    /// Altseite: Block erscheint zuerst in Version n (noch nicht befüllt).
    Version,
    /// `[erstellt, geaendert]` der Seite.
    Seite,
}

impl Guete {
    /// Gewicht (Prozent) für den Anteil der Blöcke im Fenster.
    pub fn gewicht(self) -> u32 {
        match self {
            Guete::Genau => 100,
            Guete::Version => 60,
            Guete::Seite => 40,
        }
    }
}

/// Zeitintervall `[frueh_ms, spaet_ms]` eines Blocks (absolut).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blockzeit {
    pub frueh_ms: i64,
    pub spaet_ms: i64,
    pub guete: Guete,
}

/// ISO-8601/RFC-3339-Zeit in Millisekunden. Ohne Zeitzone gilt UTC. Ungültig → `None`.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.timestamp_millis());
    }
    [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
    ]
    .iter()
    .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
    .map(|t| t.and_utc().timestamp_millis())
}

/// Blockzeit eines Blocks: `Genau` aus `zeit`; ist sie ungültig oder fehlt sie, die nächst
/// schwächere Güte `Seite` (beide Seitenzeiten gültig, sonst `None`). `Version` wird noch nicht
/// befüllt.
pub fn blockzeit(block: &Block, kopf: &Seitenkopf) -> Option<Blockzeit> {
    if let Some(t) = block.zeit.as_deref().and_then(parse_iso_ms) {
        return Some(Blockzeit {
            frueh_ms: t,
            spaet_ms: t,
            guete: Guete::Genau,
        });
    }
    let a = parse_iso_ms(&kopf.erstellt)?;
    let b = parse_iso_ms(&kopf.geaendert)?;
    Some(Blockzeit {
        frueh_ms: a.min(b),
        spaet_ms: a.max(b),
        guete: Guete::Seite,
    })
}

/// Zeitachse einer Aufnahme (absolut, Wanduhr).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zeitachse {
    pub beginn_ms: i64,
    pub ende_ms: i64,
}

impl Zeitachse {
    /// Aus `started_at` (Unix-Sekunden) und `duration_ms`; negative Dauer zählt als 0.
    pub fn neu(started_at_s: i64, duration_ms: i64) -> Self {
        let beginn_ms = started_at_s.saturating_mul(1000);
        Self {
            beginn_ms,
            ende_ms: beginn_ms.saturating_add(duration_ms.max(0)),
        }
    }

    pub fn dauer_ms(&self) -> i64 {
        self.ende_ms - self.beginn_ms
    }

    /// Fenster `[Beginn − 15 min, Ende + 30 min]`.
    pub fn fenster(&self) -> (i64, i64) {
        (
            self.beginn_ms.saturating_sub(FENSTER_VOR_MS),
            self.ende_ms.saturating_add(FENSTER_NACH_MS),
        )
    }

    /// Schneidet das Intervall mit dem Fenster (Ränder eingeschlossen).
    pub fn im_fenster(&self, z: &Blockzeit) -> bool {
        let (von, bis) = self.fenster();
        z.spaet_ms >= von && z.frueh_ms <= bis
    }
}

/// Auf die Aufnahme abgebildetes Intervall (relativ zum Beginn, an den Rändern abgeschnitten).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Abbild {
    pub von_ms: i64,
    pub bis_ms: i64,
    pub guete: Guete,
}

/// Lage einer Blockzeit zur Aufnahme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lage {
    Vor,
    Nach,
    Innerhalb(Abbild),
}

/// Die eine Regel aus §5.1: ganz außerhalb → `Vor`/`Nach`, sonst Intervall minus Beginn,
/// an den Rändern abgeschnitten.
pub fn lage(z: &Blockzeit, achse: &Zeitachse) -> Lage {
    if z.spaet_ms < achse.beginn_ms {
        return Lage::Vor;
    }
    if z.frueh_ms > achse.ende_ms {
        return Lage::Nach;
    }
    Lage::Innerhalb(Abbild {
        von_ms: z.frueh_ms.max(achse.beginn_ms) - achse.beginn_ms,
        bis_ms: z.spaet_ms.min(achse.ende_ms) - achse.beginn_ms,
        guete: z.guete,
    })
}

/// Wie [`lage`], aber als `at_ms`-artiges `Option`: ganz außerhalb → `None`.
pub fn abbilden(z: &Blockzeit, achse: &Zeitachse) -> Option<Abbild> {
    match lage(z, achse) {
        Lage::Innerhalb(a) => Some(a),
        _ => None,
    }
}

/// Anzeigetext ohne Scheingenauigkeit: Minute nur bei `Genau`, sonst Spanne bzw.
/// „während der Besprechung“.
pub fn anzeige(l: &Lage, achse: &Zeitachse) -> String {
    let a = match l {
        Lage::Vor => return "vor der Besprechung".into(),
        Lage::Nach => return "nach der Besprechung".into(),
        Lage::Innerhalb(a) => a,
    };
    let min = |ms: i64| ms / 60_000;
    if a.guete == Guete::Genau {
        return format!("bei Minute {}", min(a.von_ms));
    }
    if a.von_ms <= 0 && a.bis_ms >= achse.dauer_ms() {
        return "während der Besprechung".into();
    }
    if min(a.von_ms) == min(a.bis_ms) {
        return format!("etwa in Minute {}", min(a.von_ms));
    }
    format!(
        "zwischen Minute {} und Minute {}",
        min(a.von_ms),
        min(a.bis_ms)
    )
}
