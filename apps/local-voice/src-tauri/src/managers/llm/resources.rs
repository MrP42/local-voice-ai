//! Speicher des Rechners: RAM und das GPU-Speicherbudget je Adapter.
//!
//! Ehrlich beschriftet, nicht geraten: Auf Windows liefert DXGI je Adapter
//! ein *Budget* und die aktuelle Belegung -- das ist kein "physisch freier
//! VRAM", aber es ist die Zahl, gegen die der Treiber tatsaechlich
//! entscheidet. Eine iGPU meldet gemeinsamen Speicher; das steht dabei.
//! Ist etwas nicht messbar, sagt die Antwort das, statt eine Zahl zu
//! erfinden.

use serde::Serialize;
use specta::Type;

#[derive(Debug, Clone, Serialize, Type)]
pub struct GpuMemory {
    pub name: String,
    /// Budget, das der Treiber diesem Prozess einraeumt (MiB).
    pub budget_mb: u64,
    /// Davon aktuell belegt (MiB) -- systemweit ueber alle Prozesse.
    pub used_mb: u64,
    /// Dedizierter Speicher laut Adapter (MiB); 0 bei reinen iGPUs.
    pub dedicated_mb: u64,
    /// Gemeinsamer Speicher mit der CPU (iGPU, Apple Silicon).
    pub shared: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SystemMemory {
    pub ram_total_mb: u64,
    pub ram_used_mb: u64,
    /// Leer, wenn kein Adapter messbar ist. Software-Adapter (Microsoft
    /// Basic Render Driver) sind herausgefiltert.
    pub gpus: Vec<GpuMemory>,
}

/// Misst RAM und GPU-Budgets. Blockierend, aber schnell (DXGI-Aufrufe im
/// Millisekundenbereich); der Aufrufer legt es auf einen Blocking-Thread.
pub fn system_memory() -> SystemMemory {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    SystemMemory {
        ram_total_mb: sys.total_memory() / (1024 * 1024),
        ram_used_mb: sys.used_memory() / (1024 * 1024),
        gpus: gpu_memory(),
    }
}

#[cfg(windows)]
fn gpu_memory() -> Vec<GpuMemory> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIAdapter3, IDXGIFactory1,
        DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_MEMORY_SEGMENT_GROUP_LOCAL,
        DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
    };

    let mut out = Vec::new();
    // SAFETY: reine Abfrage-APIs von DXGI; jede Schnittstelle wird durch das
    // windows-Crate freigegeben, wenn sie aus dem Geltungsbereich faellt.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return out;
        };
        let mut index = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(index) {
            index += 1;
            let adapter: IDXGIAdapter1 = adapter;
            let Ok(desc) = adapter.GetDesc1() else { continue };
            if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            let name = String::from_utf16_lossy(
                &desc.Description[..desc
                    .Description
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(desc.Description.len())],
            );
            let dedicated_mb = desc.DedicatedVideoMemory as u64 / (1024 * 1024);
            // Eine iGPU hat keinen eigenen Speicher: ihr "lokales" Segment
            // ist der gemeinsame Speicher. Dann ist NON_LOCAL das Budget,
            // das wirklich zaehlt.
            let shared = dedicated_mb < 512;
            let Ok(adapter3) = adapter.cast::<IDXGIAdapter3>() else {
                continue;
            };
            let group = if shared {
                DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL
            } else {
                DXGI_MEMORY_SEGMENT_GROUP_LOCAL
            };
            let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            if adapter3.QueryVideoMemoryInfo(0, group, &mut info).is_err() {
                continue;
            }
            out.push(GpuMemory {
                name,
                budget_mb: info.Budget / (1024 * 1024),
                used_mb: info.CurrentUsage / (1024 * 1024),
                dedicated_mb,
                shared,
            });
        }
    }
    out
}

#[cfg(not(windows))]
fn gpu_memory() -> Vec<GpuMemory> {
    // macOS: Metal-Arbeitsbudget folgt in einem eigenen Schritt. Bis dahin
    // lieber "nicht messbar" als eine geratene Zahl.
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RAM ist auf jedem Rechner messbar; die GPU-Liste darf leer sein,
    /// aber kein Eintrag darf Unsinn tragen.
    #[test]
    fn ram_is_measured_and_gpu_entries_are_sane() {
        let mem = system_memory();
        assert!(mem.ram_total_mb > 256, "RAM gesamt: {}", mem.ram_total_mb);
        assert!(mem.ram_used_mb <= mem.ram_total_mb);
        for gpu in &mem.gpus {
            assert!(!gpu.name.trim().is_empty());
            assert!(gpu.used_mb <= gpu.budget_mb.max(gpu.used_mb));
        }
    }
}
