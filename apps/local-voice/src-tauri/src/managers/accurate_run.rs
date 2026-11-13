//! Genauer Lauf: eine einzelne Aufnahme aus dem Verlauf mit dem genauen Modell
//! (`accurate_model`) erneut transkribieren, neben dem Diktatmodell (lva#579).
//!
//! Regeln, die dieses Modul durchsetzt (der Lauf selbst steht in
//! `TranscriptionManager::transcribe_accurate`):
//! - Der Lauf rechnet IMMER mit einer eigenen Engine, nie mit der gemeinsamen. Er
//!   veraendert weder deren Ladezustand noch deren Leerlauf-Frist.
//! - Hoechstens ein genauer Lauf zur Zeit ([`Slot`]); solange er laeuft, bekommt die
//!   Import-Warteschlange keine weitere eigene Engine ([`active`]).
//! - Live-Arbeit geht vor: waehrend Diktat, Besprechungsaufnahme, Live-Transkription,
//!   Modellwechsel oder Besprechungsverarbeitung beginnt kein genauer Lauf ([`admit`]).
//!   Beginnt die Live-Arbeit erst WAEHREND des Laufs, laeuft er zu Ende und teilt sich
//!   Rechenzeit mit ihr; er wird nicht abgebrochen. Die Oberflaeche sagt das so.
//! - Das Speicher-Tor ist streng ([`memory_gate`]): unbekannte Modellgroesse oder nicht
//!   messbarer Speicher oeffnen es nicht, und beim Grafikspeicher zaehlt die Karte mit
//!   dem WENIGSTEN freien Speicher (welche Karte der Lader nimmt, ist hier nicht
//!   bekannt).
//!
//! Grenze, ehrlich: ein Wechsel des Diktatmodells WAEHREND eines genauen Laufs prueft
//! keinen Speicher (das tut er auch ohne diesen Lauf nicht); der Lauf haelt sein Modell
//! dann zusaetzlich im Speicher, bis er endet.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::managers::meetings::final_pass::Hardware;
use crate::managers::meetings::queue::plan_extra_slot;

pub const ERR_BUSY: &str = "accurate_model_busy";
pub const ERR_LIVE: &str = "accurate_model_live";
pub const ERR_MEMORY: &str = "accurate_model_memory";
pub const ERR_MISSING: &str = "accurate_model_missing";
pub const ERR_LOAD: &str = "accurate_model_load_failed";

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Der eine Platz fuer einen genauen Lauf. Faellt mit dem Wert (auch bei Fehler,
/// Panik im Lauf oder Abbruch des Arbeitsauftrags).
#[derive(Debug)]
pub struct Slot(());

impl Slot {
    pub fn try_take() -> Option<Slot> {
        ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Slot(()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

/// Laeuft gerade ein genauer Lauf (er haelt ein zweites Modell im Speicher)?
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// Was gerade live oder im Hintergrund an der Spracherkennung arbeitet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveState {
    pub dictation_recording: bool,
    pub meeting_recording: bool,
    pub streaming: bool,
    pub model_loading: bool,
    pub meeting_work: bool,
}

/// Darf ein genauer Lauf jetzt beginnen? Live-Arbeit geht vor.
pub fn admit(live: &LiveState) -> Result<(), &'static str> {
    let LiveState {
        dictation_recording,
        meeting_recording,
        streaming,
        model_loading,
        meeting_work,
    } = *live;
    if dictation_recording || meeting_recording || streaming || model_loading || meeting_work {
        return Err(ERR_LIVE);
    }
    Ok(())
}

/// Strenges Speicher-Tor fuer das zweite Modell. `size_mb` = 0 heisst unbekannt.
/// Anders als das Tor der Warteschlange ([`plan_extra_slot`]) laesst Unbekanntes
/// hier nicht durch.
pub fn memory_gate(size_mb: u64, gpu_model: bool, hw: &Hardware) -> Result<(), &'static str> {
    if size_mb == 0 || hw.free_ram_mb.is_none() {
        return Err(ERR_MEMORY);
    }
    if gpu_model && hw.gpu && hw.free_vram_mb.is_none() {
        return Err(ERR_MEMORY);
    }
    plan_extra_slot(size_mb, gpu_model, hw).map_err(|_| ERR_MEMORY)
}

/// Der Stand des Rechners fuer [`memory_gate`]: wie `final_pass::probe_hardware`,
/// aber beim Grafikspeicher die Karte mit dem wenigsten freien Speicher, und ohne
/// Rueckgriff auf Schaetzwerte (nicht Messbares bleibt `None`).
pub fn measure(app: &tauri::AppHandle) -> Hardware {
    let settings = crate::settings::get_settings(app);
    let allowed =
        settings.transcribe_accelerator != crate::settings::TranscribeAcceleratorSetting::Cpu;
    let gpus = crate::managers::transcription::transcribe_gpu_memory();
    let ram = crate::process_guard::available_ram_mb();
    Hardware {
        gpu: allowed && !gpus.is_empty(),
        free_vram_mb: least_free_vram(&gpus.iter().map(|(_, free)| *free).collect::<Vec<_>>()),
        free_ram_mb: (ram > 0).then_some(ram),
    }
}

/// Der kleinste gemessene freie Grafikspeicher; meldet auch nur EINE Karte nichts
/// (0), ist das Ergebnis unbekannt.
pub fn least_free_vram(free_mb: &[u64]) -> Option<u64> {
    if free_mb.is_empty() || free_mb.contains(&0) {
        return None;
    }
    free_mb.iter().copied().min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::queue::{ram_gate_mb, vram_gate_mb};

    fn hw(ram: Option<u64>, gpu: bool, vram: Option<u64>) -> Hardware {
        Hardware {
            gpu,
            free_vram_mb: vram,
            free_ram_mb: ram,
        }
    }

    /// Ein Test fuer den ganzen Lebenslauf des Platzes: er ist prozessweit, zwei
    /// Tests daran wuerden sich gegenseitig stoeren.
    #[test]
    fn only_one_accurate_run_holds_the_slot_and_every_way_out_frees_it() {
        assert!(!active());
        let first = Slot::try_take().expect("frei");
        assert!(active());
        assert!(
            Slot::try_take().is_none(),
            "ein zweiter Lauf bekommt keinen Platz"
        );
        drop(first);
        assert!(!active());

        // Fehlerweg: der Platz faellt mit dem fruehen Ruecksprung.
        let failing = || -> Result<(), &'static str> {
            let _slot = Slot::try_take().ok_or(ERR_BUSY)?;
            Err(ERR_MEMORY)
        };
        assert_eq!(failing(), Err(ERR_MEMORY));
        assert!(!active());

        // Panik im Lauf (Unwind): ebenfalls frei.
        let panicked = std::panic::catch_unwind(|| {
            let _slot = Slot::try_take().expect("frei");
            panic!("Engine");
        });
        assert!(panicked.is_err());
        assert!(!active());

        // Ein anderer Thread sieht denselben Platz.
        let held = Slot::try_take().expect("frei");
        std::thread::spawn(|| assert!(Slot::try_take().is_none()))
            .join()
            .unwrap();
        drop(held);
        assert!(Slot::try_take().is_some());
        assert!(!active());
    }

    #[test]
    fn live_work_of_any_kind_keeps_an_accurate_run_from_starting() {
        assert_eq!(admit(&LiveState::default()), Ok(()));
        let cases = [
            LiveState {
                dictation_recording: true,
                ..Default::default()
            },
            LiveState {
                meeting_recording: true,
                ..Default::default()
            },
            LiveState {
                streaming: true,
                ..Default::default()
            },
            LiveState {
                model_loading: true,
                ..Default::default()
            },
            LiveState {
                meeting_work: true,
                ..Default::default()
            },
        ];
        for live in cases {
            assert_eq!(admit(&live), Err(ERR_LIVE), "{live:?}");
        }
    }

    #[test]
    fn the_memory_gate_stays_shut_for_anything_it_cannot_measure() {
        let size = 1_600;
        let ram = ram_gate_mb(size);
        let vram = vram_gate_mb(size);
        // Alles bekannt und genug: offen (genau auf der Grenze).
        assert_eq!(
            memory_gate(size, true, &hw(Some(ram), true, Some(vram))),
            Ok(())
        );
        assert_eq!(
            memory_gate(size, false, &hw(Some(ram), false, None)),
            Ok(())
        );
        // Unbekannte Modellgroesse (eigene Datei ohne Groesse).
        assert_eq!(
            memory_gate(0, false, &hw(Some(64_000), false, None)),
            Err(ERR_MEMORY)
        );
        // Arbeitsspeicher nicht messbar.
        assert_eq!(
            memory_gate(size, false, &hw(None, false, None)),
            Err(ERR_MEMORY)
        );
        // Grafikspeicher nicht messbar, das Modell wuerde aber auf die Karte gehen.
        assert_eq!(
            memory_gate(size, true, &hw(Some(64_000), true, None)),
            Err(ERR_MEMORY)
        );
        // Dasselbe ohne GPU-Nutzung: der Grafikspeicher spielt keine Rolle.
        assert_eq!(memory_gate(size, true, &hw(Some(ram), false, None)), Ok(()));
        assert_eq!(memory_gate(size, false, &hw(Some(ram), true, None)), Ok(()));
        // Zu wenig.
        assert_eq!(
            memory_gate(size, false, &hw(Some(ram - 1), false, None)),
            Err(ERR_MEMORY)
        );
        assert_eq!(
            memory_gate(size, true, &hw(Some(ram), true, Some(vram - 1))),
            Err(ERR_MEMORY)
        );
    }

    #[test]
    fn with_several_cards_the_one_with_the_least_free_memory_counts() {
        assert_eq!(least_free_vram(&[]), None);
        assert_eq!(least_free_vram(&[20_000]), Some(20_000));
        // Kleine Karte neben grosser: die kleine entscheidet.
        assert_eq!(least_free_vram(&[20_000, 1_500]), Some(1_500));
        // Meldet eine Karte nichts, ist der Stand unbekannt.
        assert_eq!(least_free_vram(&[20_000, 0]), None);
        let size = 1_600;
        let small = least_free_vram(&[24_000, 1_500]);
        assert_eq!(
            memory_gate(size, true, &hw(Some(64_000), true, small)),
            Err(ERR_MEMORY)
        );
    }
}
