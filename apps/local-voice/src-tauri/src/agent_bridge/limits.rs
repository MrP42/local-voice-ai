//! Aufruf-Obergrenzen (QG5): je Zugang `tools/call` und alle Anfragen je Minute, dazu
//! eine Obergrenze fuer fehlgeschlagene Anmeldungen. Ein Gleitfenster von einer Minute,
//! rein (die Uhr kommt als Parameter), damit Tests ohne Warten auskommen.
//!
//! Ein Agent in einer Schleife kann die App so nicht beschaeftigen und nicht das Audit fuellen:
//! der erste abgewiesene Aufruf je Fenster meldet `first = true` (der Aufrufer schreibt dafuer EINE
//! Audit-Zeile), alle weiteren sind still. Speicher: hoechstens `max` Zeitstempel je Schluessel,
//! hoechstens `MAX_KEYS` Schluessel.

use std::collections::{HashMap, VecDeque};

/// Breite des Fensters.
pub const WINDOW_MS: i64 = 60_000;
/// Hoechstzahl verfolgter Schluessel (Zugaenge): ueber der Grenze wird der aelteste vergessen.
pub const MAX_KEYS: usize = 256;

/// Antwort des Begrenzers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    Allowed,
    /// Abgewiesen. `first`: erster abgewiesener Aufruf in diesem Fenster.
    Limited { first: bool },
}

#[derive(Default)]
struct Bucket {
    stamps: VecDeque<i64>,
    /// Zeitpunkt der letzten gemeldeten Abweisung.
    reported_at: Option<i64>,
}

/// Gleitfenster-Begrenzer je Schluessel.
#[derive(Default)]
pub struct RateLimiter {
    buckets: HashMap<String, Bucket>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Zaehlt einen Aufruf von `key` und entscheidet. `max` = erlaubte Aufrufe je Fenster.
    pub fn hit(&mut self, key: &str, max: u32, now_ms: i64) -> Limit {
        if !self.buckets.contains_key(key) && self.buckets.len() >= MAX_KEYS {
            // Nicht unbegrenzt wachsen: Schluessel ohne Aufruf im Fenster vergessen, sonst den
            // beliebig ersten.
            self.buckets
                .retain(|_, b| b.stamps.back().is_some_and(|t| now_ms - t < WINDOW_MS));
            if self.buckets.len() >= MAX_KEYS {
                if let Some(k) = self.buckets.keys().next().cloned() {
                    self.buckets.remove(&k);
                }
            }
        }
        let b = self.buckets.entry(key.to_string()).or_default();
        while b.stamps.front().is_some_and(|t| now_ms - t >= WINDOW_MS) {
            b.stamps.pop_front();
        }
        if b.stamps.len() as u32 >= max {
            let first = b.reported_at.is_none_or(|r| now_ms - r >= WINDOW_MS);
            if first {
                b.reported_at = Some(now_ms);
            }
            return Limit::Limited { first };
        }
        b.stamps.push_back(now_ms);
        Limit::Allowed
    }

    /// Anzahl verfolgter Schluessel (Tests).
    pub fn keys(&self) -> usize {
        self.buckets.len()
    }
}

#[cfg(test)]
mod tests;
