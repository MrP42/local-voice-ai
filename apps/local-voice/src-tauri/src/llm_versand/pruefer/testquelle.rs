//! Testquelle des Prüfers (lva#604). Nur unter `cfg(test)`; produktiv gibt es sie nicht.
//!
//! Frei einstellbare Lesung und Sichtbarkeit, mit Zählern, und ein **Haken**, der am Anfang
//! jeder Lesung läuft. Damit lassen sich Widerrufe an genau einer Stelle des Ablaufs
//! einschieben, ohne Schlaf und ohne Wettlauf: Der Prüfer ruft `lesen()` in fester
//! Reihenfolge (R1, Quellenprüfung, R2, ggf. R3); die Nummer der Lesung bestimmt, **zwischen
//! welchen** Schritten der Haken wirkt.
//!
//! Wie in der echten Datenbank steigt die Revision bei jeder Änderung an der Freigabe
//! ([`TestQuelle::freigeben`], [`TestQuelle::widerrufen`], [`TestQuelle::anheben`]) und sinkt nie.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::{FreigabeQuelle, KiSperre, Lesung, VersandFehler};

/// Wird mit der Quelle und der Nummer (ab 1) der Lesung gerufen, **bevor** sie ihr Ergebnis
/// liefert. Läuft ohne gehaltene Sperre und darf die Quelle ändern.
type Haken = Arc<dyn Fn(&TestQuelle, usize) + Send + Sync>;

pub(in crate::llm_versand) struct TestQuelle {
    lesung: Mutex<Result<Lesung, VersandFehler>>,
    sichtbar: Mutex<BTreeSet<String>>,
    haken: Mutex<Option<Haken>>,
    gelesen: AtomicUsize,
    abgefragt: AtomicUsize,
}

/// Eine vergiftete Sperre (Panic in einem anderen Test-Thread) bleibt lesbar.
fn sperren<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl super::versiegelt::Versiegelt for TestQuelle {}

impl TestQuelle {
    /// Eine Quelle mit dieser Revision und Richtlinie, ohne sichtbare Besprechung.
    pub(in crate::llm_versand) fn neu(revision: i64, sperre: KiSperre) -> Self {
        Self {
            lesung: Mutex::new(Ok(Lesung { revision, sperre })),
            sichtbar: Mutex::new(BTreeSet::new()),
            haken: Mutex::new(None),
            gelesen: AtomicUsize::new(0),
            abgefragt: AtomicUsize::new(0),
        }
    }

    /// Gibt eine Besprechung für KI-Clients frei (die Revision steigt).
    pub(in crate::llm_versand) fn freigeben(&self, id: &str) {
        sperren(&self.sichtbar).insert(id.to_owned());
        self.anheben();
    }

    /// Widerruft die Freigabe einer Besprechung (die Revision steigt).
    pub(in crate::llm_versand) fn widerrufen(&self, id: &str) {
        sperren(&self.sichtbar).remove(id);
        self.anheben();
    }

    /// Erhöht die Revision um eins, wie ein Trigger der Datenbank bei einer Änderung.
    pub(in crate::llm_versand) fn anheben(&self) {
        if let Ok(lesung) = &mut *sperren(&self.lesung) {
            lesung.revision += 1;
        }
    }

    /// Stellt die nächsten Lesungen ein (neuer Stand oder Fehler), ohne die Revision zu ändern.
    pub(in crate::llm_versand) fn setze_lesung(&self, lesung: Result<Lesung, VersandFehler>) {
        *sperren(&self.lesung) = lesung;
    }

    /// Der Stand, den die nächste Lesung liefert (ohne zu zählen).
    pub(in crate::llm_versand) fn stand(&self) -> Result<Lesung, VersandFehler> {
        sperren(&self.lesung).clone()
    }

    /// Setzt den Haken, der am Anfang **jeder** Lesung läuft; wer nur an einer Stelle eingreifen
    /// will, prüft die Nummer (`nr == 2`: zwischen Quellenprüfung und R2 des ersten Versuchs).
    pub(in crate::llm_versand) fn bei_lesung(
        &self,
        haken: impl Fn(&TestQuelle, usize) + Send + Sync + 'static,
    ) {
        *sperren(&self.haken) = Some(Arc::new(haken));
    }

    /// Wie oft wurde `lesen()` aufgerufen?
    pub(in crate::llm_versand) fn lesungen(&self) -> usize {
        self.gelesen.load(Ordering::SeqCst)
    }

    /// Wie oft wurde `sichtbar()` aufgerufen?
    pub(in crate::llm_versand) fn sichtbarkeitsabfragen(&self) -> usize {
        self.abgefragt.load(Ordering::SeqCst)
    }
}

impl FreigabeQuelle for TestQuelle {
    fn lesen(&self) -> Result<Lesung, VersandFehler> {
        let nr = self.gelesen.fetch_add(1, Ordering::SeqCst) + 1;
        let haken = sperren(&self.haken).clone();
        if let Some(haken) = haken {
            haken(self, nr);
        }
        self.stand()
    }

    fn sichtbar(&self, _sperre: &KiSperre, besprechung_id: &str) -> bool {
        self.abgefragt.fetch_add(1, Ordering::SeqCst);
        sperren(&self.sichtbar).contains(besprechung_id)
    }
}
