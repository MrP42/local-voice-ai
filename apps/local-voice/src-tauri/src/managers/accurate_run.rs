//! Genauer Lauf: eine einzelne Aufnahme aus dem Verlauf mit dem genauen Modell
//! (`accurate_model`) erneut transkribieren, neben dem Diktatmodell (lva#579).
//!
//! Regeln, die dieses Modul durchsetzt (der Lauf selbst steht in
//! `TranscriptionManager::transcribe_accurate`):
//! - Der Lauf rechnet IMMER mit einer eigenen Engine, nie mit der gemeinsamen. Er
//!   veraendert weder deren Ladezustand noch deren Leerlauf-Frist.
//! - Hoechstens ein genauer Lauf zur Zeit ([`Slot`]); solange er laeuft, bekommt die
//!   Import-Warteschlange keine weitere eigene Engine ([`active`]).
//! - Live-Arbeit geht vor, soweit eine Zustandspruefung das leisten kann: laeuft beim
//!   Start oder unmittelbar vor dem Laden ein Diktat, eine Besprechungsaufnahme, eine
//!   Live-Transkription, ein Modellwechsel oder eine Besprechungsverarbeitung, beginnt
//!   der genaue Lauf nicht ([`admit`]). Das ist KEINE Sperre: Live-Arbeit, die nach der
//!   letzten Pruefung beginnt (auch noch waehrend des Ladens), trifft auf einen
//!   laufenden genauen Lauf. Er wird nicht abgebrochen und teilt sich Rechenzeit mit
//!   ihr. Die Oberflaeche sagt das so.
//! - Das Speicher-Tor ist streng ([`memory_gate`]): unbekannte Modellgroesse oder nicht
//!   messbarer Arbeitsspeicher oeffnen es nicht, und beim Grafikspeicher zaehlt die
//!   Karte mit dem WENIGSTEN freien Speicher (welche Karte der Lader nimmt, ist hier
//!   nicht bekannt).
//! - Die Grafikkarte wird nur benutzt, wenn ihr freier Speicher gemessen ist. Ohne
//!   Messung (GPU abgeschaltet, kein Geraet gemeldet, Geraet ohne Angabe) laedt der
//!   genaue Lauf sein Modell ausdruecklich auf der CPU ([`must_load_on_cpu`]); ob eine
//!   Karte fehlt oder nur die Messung, wird nicht geraten.
//! - Der freie Speicher wird nicht zweimal zugesagt: Messen, Abziehen aller laufenden
//!   Ladevorgaenge (Modellwechsel, eigene Engine der Warteschlange), Entscheiden und
//!   Eintragen geschehen unter einer Sperre ([`crate::managers::engine_ledger`]).

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
/// Rueckgriff auf Schaetzwerte. Siehe [`hardware_from`].
pub fn measure(app: &tauri::AppHandle) -> Hardware {
    let settings = crate::settings::get_settings(app);
    let allowed =
        settings.transcribe_accelerator != crate::settings::TranscribeAcceleratorSetting::Cpu;
    let free: Vec<u64> = crate::managers::transcription::transcribe_gpu_memory()
        .iter()
        .map(|(_, free)| *free)
        .collect();
    hardware_from(allowed, &free, crate::process_guard::available_ram_mb())
}

/// Baut den Stand fuer das Tor. `gpu` ist nur dann `true`, wenn die GPU-Nutzung
/// erlaubt ist UND der freie Grafikspeicher JEDER gemeldeten Karte gemessen ist.
/// In jedem anderen Fall (GPU abgeschaltet, kein Geraet gemeldet, ein Geraet ohne
/// Angabe) ist `gpu` = false, und das heisst hier nicht "es gibt keine Grafikkarte",
/// sondern: der genaue Lauf darf die Grafikkarte nicht benutzen. Ob eine Karte
/// fehlt oder nur die Messung, wird nicht geraten. Der Aufrufer MUSS dann auf der
/// CPU laden ([`must_load_on_cpu`]); das Tor prueft dafuer den Arbeitsspeicher.
///
/// `ram_mb` = 0 heisst nicht messbar.
pub fn hardware_from(gpu_allowed: bool, transcribe_free_mb: &[u64], ram_mb: u64) -> Hardware {
    let free_vram_mb = least_free_vram(transcribe_free_mb);
    Hardware {
        gpu: gpu_allowed && free_vram_mb.is_some(),
        free_vram_mb: if gpu_allowed { free_vram_mb } else { None },
        free_ram_mb: (ram_mb > 0).then_some(ram_mb),
    }
}

/// Muss der genaue Lauf sein Modell ausdruecklich auf der CPU laden? Ja, sobald der
/// Stand keinen gemessenen Grafikspeicher ausweist: ohne Messung gibt es keine
/// Zusage fuer die Grafikkarte, also auch kein Laden dorthin.
pub fn must_load_on_cpu(hw: &Hardware) -> bool {
    !(hw.gpu && hw.free_vram_mb.is_some())
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
    fn without_a_measured_gpu_the_run_is_admitted_for_the_cpu_only() {
        let size = 1_600;
        let ram = 64_000;
        let need = vram_gate_mb(size);
        let gate = |hw: &Hardware| memory_gate(size, true, hw);

        // Gemessen: die Grafikkarte darf benutzt werden, es zaehlt der Wert.
        let measured = hardware_from(true, &[need], ram);
        assert_eq!(gate(&measured), Ok(()));
        assert!(!must_load_on_cpu(&measured));
        let short = hardware_from(true, &[need - 1], ram);
        assert_eq!(gate(&short), Err(ERR_MEMORY));
        assert!(!must_load_on_cpu(&short));

        // GPU erlaubt, GPU-faehiges Modell, aber keine Messung: gleich ob kein
        // Geraet gemeldet wird (Karte fehlt ODER Probe fiel aus) oder ein Geraet
        // nichts angibt. Zugelassen wird nur der CPU-Ladeweg, nie die Karte.
        for free in [&[][..], &[0][..], &[need, 0][..]] {
            let hw = hardware_from(true, free, ram);
            assert!(must_load_on_cpu(&hw), "{free:?}");
            assert_eq!(gate(&hw), Ok(()), "{free:?}");
            // Und fuer die CPU zaehlt der Arbeitsspeicher wie immer.
            let tight = hardware_from(true, free, ram_gate_mb(size) - 1);
            assert_eq!(gate(&tight), Err(ERR_MEMORY), "{free:?}");
            assert_eq!(gate(&hardware_from(true, free, 0)), Err(ERR_MEMORY));
        }

        // GPU abgeschaltet: CPU, auch wenn eine Karte Speicher meldet.
        let forced = hardware_from(false, &[need], ram);
        assert!(must_load_on_cpu(&forced));
        assert_eq!(forced.free_vram_mb, None);
        assert_eq!(gate(&forced), Ok(()));

        // Ein von Hand gebauter Stand "GPU ja, Speicher unbekannt" bleibt zu und
        // verlangt trotzdem die CPU: keine Luecke zwischen Tor und Ladeweg.
        let odd = hw(Some(ram), true, None);
        assert_eq!(gate(&odd), Err(ERR_MEMORY));
        assert!(must_load_on_cpu(&odd));
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
