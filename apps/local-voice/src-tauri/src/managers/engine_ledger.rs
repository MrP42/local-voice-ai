//! Gemeinsame Speicher-Reservierung fuer das Laden von Spracherkennungs-Engines
//! (lva#579).
//!
//! Freier Speicher wird gemessen, bevor eine Engine laedt; bis das Laden fertig
//! ist, sieht eine zweite Messung denselben freien Speicher noch einmal. Dieses
//! Buch haelt deshalb fest, was gerade laedt. Jeder Ladeweg traegt sich ein:
//! - der Modellwechsel der gemeinsamen Engine ([`Ledger::announce`], ohne Tor:
//!   er fragt wie bisher kein Speicher-Tor, es wird ihm also nichts zugesagt);
//! - die eigene Engine der Import-Warteschlange ([`Ledger::announce_unless`]);
//! - der genaue Lauf ([`Ledger::reserve`], mit Tor).
//!
//! Wer ein Tor fragt, bekommt den gemessenen freien Speicher ABZUEGLICH aller
//! laufenden Ladevorgaenge; laedt etwas unbekannter Groesse, ist der freie Speicher
//! fuer das Tor unbekannt. Messen, Entscheiden und Eintragen geschehen unter
//! einer Sperre: derselbe freie Speicher wird nicht zweimal zugesagt. Der Eintrag
//! faellt mit der [`Reservation`], also wenn das Laden zu Ende ist (dann steckt
//! das Modell im gemessenen Verbrauch) oder gescheitert ist.
//!
//! Die Sperre wird nur fuer Messen und Rechnen gehalten, nie ueber ein Laden.

use std::sync::{Mutex, MutexGuard};

use crate::managers::meetings::final_pass::Hardware;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Pending {
    ram_mb: u64,
    vram_mb: u64,
    /// Laufende Ladevorgaenge, deren Groesse nicht bekannt ist.
    unknown: u32,
}

#[derive(Debug, Default)]
pub struct Ledger {
    pending: Mutex<Pending>,
}

/// Ein laufender Ladevorgang im Buch. Faellt mit dem Wert (Erfolg, Fehler, Panik).
#[derive(Debug)]
pub struct Reservation<'a> {
    ledger: &'a Ledger,
    held: Pending,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let mut p = self.ledger.lock();
        p.ram_mb = p.ram_mb.saturating_sub(self.held.ram_mb);
        p.vram_mb = p.vram_mb.saturating_sub(self.held.vram_mb);
        p.unknown = p.unknown.saturating_sub(self.held.unknown);
    }
}

impl Ledger {
    pub const fn new() -> Self {
        Self {
            pending: Mutex::new(Pending {
                ram_mb: 0,
                vram_mb: 0,
                unknown: 0,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn enter(&self, p: &mut Pending, size_mb: u64, gpu_model: bool) -> Reservation<'_> {
        // Ein GPU-faehiges Modell zaehlt in beiden Speichern: wohin der Lader es
        // legt, ist hier nicht bekannt.
        // Groesse 0 heisst unbekannt, nicht "kein Bedarf": so ein Ladevorgang macht
        // den freien Speicher fuer alle, die ein Tor fragen, unbekannt.
        let held = Pending {
            ram_mb: size_mb,
            vram_mb: if gpu_model { size_mb } else { 0 },
            unknown: u32::from(size_mb == 0),
        };
        p.ram_mb = p.ram_mb.saturating_add(held.ram_mb);
        p.vram_mb = p.vram_mb.saturating_add(held.vram_mb);
        p.unknown = p.unknown.saturating_add(held.unknown);
        Reservation { ledger: self, held }
    }

    /// Traegt einen Ladevorgang ohne Tor ein (Modellwechsel der gemeinsamen Engine).
    pub fn announce(&self, size_mb: u64, gpu_model: bool) -> Reservation<'_> {
        let mut p = self.lock();
        self.enter(&mut p, size_mb, gpu_model)
    }

    /// Wie [`Self::announce`], aber nur, wenn `blocked` unter der Sperre `false`
    /// meldet (Warteschlange: nicht, solange ein genauer Lauf seinen Platz haelt).
    pub fn announce_unless(
        &self,
        size_mb: u64,
        gpu_model: bool,
        blocked: impl FnOnce() -> bool,
    ) -> Option<Reservation<'_>> {
        let mut p = self.lock();
        if blocked() {
            return None;
        }
        Some(self.enter(&mut p, size_mb, gpu_model))
    }

    /// Misst, zieht alle laufenden Ladevorgaenge ab, fragt `gate` und traegt bei
    /// Erfolg ein; alles unter einer Sperre.
    pub fn reserve<E>(
        &self,
        size_mb: u64,
        gpu_model: bool,
        measure: impl FnOnce() -> Hardware,
        gate: impl FnOnce(&Hardware) -> Result<(), E>,
    ) -> Result<Reservation<'_>, E> {
        let mut p = self.lock();
        let hw = less_pending(measure(), *p);
        gate(&hw)?;
        Ok(self.enter(&mut p, size_mb, gpu_model))
    }

    /// Was gerade laedt: (Arbeitsspeicher, Grafikspeicher) in MB, ohne Ladevorgaenge
    /// unbekannter Groesse ([`Self::pending_unknown`]).
    pub fn pending_mb(&self) -> (u64, u64) {
        let p = self.lock();
        (p.ram_mb, p.vram_mb)
    }

    /// Laedt gerade etwas, dessen Groesse nicht bekannt ist?
    pub fn pending_unknown(&self) -> bool {
        self.lock().unknown > 0
    }
}

fn less_pending(hw: Hardware, p: Pending) -> Hardware {
    if p.unknown > 0 {
        // Von einem gemessenen Wert laesst sich nichts Unbekanntes abziehen: der
        // freie Speicher ist dann selbst unbekannt (das strenge Tor schliesst).
        return Hardware {
            gpu: hw.gpu,
            free_ram_mb: None,
            free_vram_mb: None,
        };
    }
    Hardware {
        gpu: hw.gpu,
        free_ram_mb: hw.free_ram_mb.map(|f| f.saturating_sub(p.ram_mb)),
        free_vram_mb: hw.free_vram_mb.map(|f| f.saturating_sub(p.vram_mb)),
    }
}

/// Das Buch der App.
pub fn global() -> &'static Ledger {
    static LEDGER: Ledger = Ledger::new();
    &LEDGER
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::accurate_run::{memory_gate, ERR_MEMORY};
    use crate::managers::meetings::queue::{plan_extra_slot, ram_gate_mb, vram_gate_mb};

    const SIZE: u64 = 1_600;

    /// Genau genug fuer EINE weitere Engine dieser Groesse.
    fn room_for_one() -> Hardware {
        Hardware {
            gpu: true,
            free_ram_mb: Some(ram_gate_mb(SIZE)),
            free_vram_mb: Some(vram_gate_mb(SIZE)),
        }
    }

    #[test]
    fn a_queue_load_that_was_admitted_first_is_not_promised_again_to_an_accurate_run() {
        let ledger = Ledger::new();
        // Die Warteschlange ist zugelassen und steht unmittelbar vor der Allokation:
        // die Messung sieht ihr Modell noch nicht.
        let queue = ledger
            .announce_unless(SIZE, true, || false)
            .expect("kein genauer Lauf");
        assert_eq!(
            ledger
                .reserve(SIZE, true, room_for_one, |hw| memory_gate(SIZE, true, hw))
                .unwrap_err(),
            ERR_MEMORY
        );
        // Abgelehntes hinterlaesst nichts im Buch.
        assert_eq!(ledger.pending_mb(), (SIZE, SIZE));
        // Die Warteschlange ist fertig geladen; die Messung zeigt ihr Modell jetzt
        // selbst: weiterhin kein Platz.
        drop(queue);
        assert_eq!(ledger.pending_mb(), (0, 0));
        let after_load = || Hardware {
            gpu: true,
            free_ram_mb: Some(ram_gate_mb(SIZE) - SIZE),
            free_vram_mb: Some(vram_gate_mb(SIZE) - SIZE),
        };
        assert!(ledger
            .reserve(SIZE, true, after_load, |hw| memory_gate(SIZE, true, hw))
            .is_err());
    }

    #[test]
    fn while_an_accurate_run_holds_its_place_the_queue_loads_nothing() {
        let ledger = Ledger::new();
        let accurate = ledger
            .reserve(SIZE, true, room_for_one, |hw| memory_gate(SIZE, true, hw))
            .expect("Platz fuer einen");
        // `blocked` steht fuer `accurate_run::active()`.
        assert!(ledger.announce_unless(SIZE, true, || true).is_none());
        assert_eq!(ledger.pending_mb(), (SIZE, SIZE));
        // Auch ein Tor der Warteschlange saehe den Platz nicht noch einmal.
        assert!(ledger
            .reserve(SIZE, true, room_for_one, |hw| plan_extra_slot(
                SIZE, true, hw
            ))
            .is_err());
        drop(accurate);
        assert!(ledger.announce_unless(SIZE, true, || false).is_some());
    }

    #[test]
    fn a_model_switch_in_progress_counts_against_an_accurate_run() {
        let ledger = Ledger::new();
        // Modellwechsel ohne GPU-Modell: zaehlt nur im Arbeitsspeicher.
        let switch = ledger.announce(SIZE, false);
        assert_eq!(ledger.pending_mb(), (SIZE, 0));
        assert_eq!(
            ledger
                .reserve(SIZE, true, room_for_one, |hw| memory_gate(SIZE, true, hw))
                .unwrap_err(),
            ERR_MEMORY
        );
        // Mit genug Platz fuer beide geht es, und beide stehen im Buch.
        let room_for_two = || Hardware {
            gpu: true,
            free_ram_mb: Some(ram_gate_mb(SIZE) + SIZE),
            free_vram_mb: Some(vram_gate_mb(SIZE)),
        };
        let accurate = ledger
            .reserve(SIZE, true, room_for_two, |hw| memory_gate(SIZE, true, hw))
            .expect("Platz fuer beide");
        assert_eq!(ledger.pending_mb(), (2 * SIZE, SIZE));
        drop(switch);
        drop(accurate);
        assert_eq!(ledger.pending_mb(), (0, 0));
    }

    #[test]
    fn a_load_of_unknown_size_is_never_taken_for_no_demand() {
        let plenty = || Hardware {
            gpu: true,
            free_ram_mb: Some(512_000),
            free_vram_mb: Some(512_000),
        };
        let gate = |hw: &Hardware| memory_gate(SIZE, true, hw);
        let cpu_gate = |hw: &Hardware| memory_gate(SIZE, false, hw);

        // Modellwechsel mit eigener Datei ohne Groesse, vor der Allokation.
        let ledger = Ledger::new();
        let switch = ledger.announce(0, true);
        assert!(ledger.pending_unknown());
        assert_eq!(
            ledger.reserve(SIZE, true, plenty, gate).unwrap_err(),
            ERR_MEMORY
        );
        // Auch ein reines CPU-Modell bekommt keine Zusage: der Arbeitsspeicher ist
        // genauso unbekannt.
        assert_eq!(
            ledger.reserve(SIZE, false, plenty, cpu_gate).unwrap_err(),
            ERR_MEMORY
        );
        drop(switch);
        assert!(!ledger.pending_unknown());
        drop(
            ledger
                .reserve(SIZE, true, plenty, gate)
                .expect("wieder messbar"),
        );

        // Eigene Engine der Warteschlange ohne Groesse.
        let queue = ledger
            .announce_unless(0, false, || false)
            .expect("kein genauer Lauf");
        assert_eq!(
            ledger.reserve(SIZE, true, plenty, gate).unwrap_err(),
            ERR_MEMORY
        );
        // Neben einem bekannten Ladevorgang bleibt es unbekannt, bis der
        // unbekannte zu Ende ist.
        let known = ledger.announce(SIZE, false);
        assert_eq!(
            ledger.reserve(SIZE, true, plenty, gate).unwrap_err(),
            ERR_MEMORY
        );
        drop(queue);
        drop(
            ledger
                .reserve(SIZE, true, plenty, gate)
                .expect("nur noch Bekanntes"),
        );
        drop(known);
        assert_eq!(ledger.pending_mb(), (0, 0));
        assert!(!ledger.pending_unknown());
    }

    #[test]
    fn every_way_out_of_a_load_clears_its_entry() {
        let ledger = Ledger::new();
        let failing = |l: &Ledger| -> Result<(), &'static str> {
            let _pending = l.announce(SIZE, true);
            Err("Laden gescheitert")
        };
        assert!(failing(&ledger).is_err());
        assert_eq!(ledger.pending_mb(), (0, 0));
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _pending = ledger.announce(SIZE, true);
            panic!("Lader");
        }));
        assert!(panicked.is_err());
        assert_eq!(ledger.pending_mb(), (0, 0));
        // Nicht Messbares bleibt nicht messbar (das strenge Tor schliesst dann).
        let _held = ledger.announce(SIZE, true);
        let unknown = || Hardware {
            gpu: true,
            free_ram_mb: None,
            free_vram_mb: None,
        };
        assert!(ledger
            .reserve(SIZE, true, unknown, |hw| memory_gate(SIZE, true, hw))
            .is_err());
    }
}
