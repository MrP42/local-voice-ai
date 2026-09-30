//! Anteil der App am Speicher: RAM und VRAM von Local Voice AI samt allen
//! Kindprozessen (llama-server, Fish-Speech, ffmpeg, WebView2).
//!
//! RAM: Arbeitssatz der Prozesse im Prozessbaum der App (Summe ueber alle
//! Nachfahren). Der Arbeitssatz, nicht "Private Bytes": ein per mmap
//! eingeblendetes GGUF liegt im Arbeitssatz und damit auch in der
//! systemweiten Belegung, die daneben steht.
//!
//! VRAM (nur Windows): Der Leistungsindikator "GPU Process Memory" liefert
//! je Prozess und Adapter den dedizierten Speicher -- fuer ALLE Prozesse,
//! nicht nur den eigenen (DXGI `QueryVideoMemoryInfo` kennt nur den
//! eigenen Prozess und sahe den llama-server nicht). Die Abfrage laeuft
//! ueber die PDH-API im Prozess: kein Kindprozess, kein `nvidia-smi`.
//! Herstellerunabhaengig. Ist der Indikator nicht verfuegbar, ist der
//! Wert `None` ("nicht messbar"), nie eine erfundene Null.
//!
//! Die Messung kostet einige Millisekunden (Prozessliste, PDH) und wird
//! deshalb hoechstens alle [`MIN_INTERVAL`] wiederholt; dazwischen gilt der
//! letzte Wert. Der Aufrufer liegt auf einem Blocking-Thread.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Hoechstens so oft wird wirklich gemessen.
pub const MIN_INTERVAL: Duration = Duration::from_secs(5);

/// Eine Zeile der Prozessliste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: u32,
    pub ppid: u32,
    /// Arbeitssatz in Bytes.
    pub mem_bytes: u64,
}

/// GPU-Speicher eines Prozesses auf einem Adapter (PDH-Instanz).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuInstance {
    pub pid: u32,
    /// `0x00000000_0x0001EE10` (klein geschrieben), wie in `GpuMemory::luid`.
    pub luid: String,
    pub bytes: u64,
}

/// Ergebnis einer Messung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AppUsage {
    pub ram_mb: u64,
    /// `None`: nicht messbar (kein Indikator, kein Adapter).
    pub gpu_mb: Option<u64>,
}

/// `root` und alle seine Nachfahren. Gegen Zyklen (wiederverwendete PIDs)
/// abgesichert: jede PID kommt hoechstens einmal vor.
pub fn tree_pids(root: u32, rows: &[ProcRow]) -> Vec<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for row in rows {
        if row.pid != row.ppid {
            children.entry(row.ppid).or_default().push(row.pid);
        }
    }
    let mut seen: HashSet<u32> = HashSet::from([root]);
    let mut order = vec![root];
    let mut next = 0;
    while next < order.len() {
        let pid = order[next];
        next += 1;
        for child in children.get(&pid).into_iter().flatten() {
            if seen.insert(*child) {
                order.push(*child);
            }
        }
    }
    order
}

/// Arbeitssatz des Baums ab `root` in MB (abgerundet auf ganze MB).
pub fn sum_tree_ram_mb(root: u32, rows: &[ProcRow]) -> u64 {
    let pids: HashSet<u32> = tree_pids(root, rows).into_iter().collect();
    let bytes: u64 = rows
        .iter()
        .filter(|r| pids.contains(&r.pid))
        .map(|r| r.mem_bytes)
        .sum();
    bytes / (1024 * 1024)
}

/// Zerlegt einen PDH-Instanznamen `pid_10600_luid_0x00000000_0x00018AE6_phys_0`
/// in PID und LUID (klein geschrieben). `None` bei jeder anderen Form
/// (z. B. `_total`).
pub fn parse_gpu_instance(name: &str) -> Option<(u32, String)> {
    let rest = name.strip_prefix("pid_")?;
    let (pid, rest) = rest.split_once("_luid_")?;
    let pid = pid.parse::<u32>().ok()?;
    let luid = rest.split("_phys_").next()?;
    let (high, low) = luid.split_once('_')?;
    let hex = |s: &str| {
        s.strip_prefix("0x")
            .is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
    };
    if !hex(high) || !hex(low) {
        return None;
    }
    Some((pid, luid.to_ascii_lowercase()))
}

/// Dedizierter GPU-Speicher der Prozesse `pids` in MB. Mit `luid` nur auf
/// diesem Adapter, sonst ueber alle.
pub fn sum_gpu_mb(pids: &[u32], luid: Option<&str>, instances: &[GpuInstance]) -> u64 {
    let pids: HashSet<u32> = pids.iter().copied().collect();
    let bytes: u64 = instances
        .iter()
        .filter(|i| pids.contains(&i.pid))
        .filter(|i| luid.is_none_or(|l| i.luid == l))
        .map(|i| i.bytes)
        .sum();
    bytes / (1024 * 1024)
}

/// Prozessliste ohne Speicherangaben (`mem_bytes` 0): PID und Elternprozess
/// liefert sysinfo immer, ohne je Prozess ein Handle zu oeffnen.
pub(crate) fn process_rows() -> Vec<ProcRow> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    sys.processes()
        .values()
        .map(|p| ProcRow {
            pid: p.pid().as_u32(),
            ppid: p.parent().map(|pp| pp.as_u32()).unwrap_or(0),
            mem_bytes: 0,
        })
        .collect()
}

/// Arbeitssatz der Prozesse `pids` in Bytes, in `rows` eingetragen. Nur diese
/// Prozesse werden geoeffnet (ein Dutzend statt Hunderte); der Arbeitssatz
/// verlangt bei sysinfo ausdruecklich `with_memory`.
fn fill_memory(rows: &mut [ProcRow], pids: &[u32]) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let ids: Vec<Pid> = pids.iter().map(|p| Pid::from_u32(*p)).collect();
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&ids),
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    for row in rows.iter_mut() {
        if let Some(p) = sys.process(Pid::from_u32(row.pid)) {
            row.mem_bytes = p.memory();
        }
    }
}

/// GPU-Speicher je Prozess und Adapter. `None`, wenn der Indikator fehlt.
#[cfg(windows)]
fn gpu_instances() -> Option<Vec<GpuInstance>> {
    use windows::core::w;
    use windows::Win32::System::Performance::{
        PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
        PdhOpenQueryW, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_LARGE,
        PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    };

    // SAFETY: PDH-Aufrufe mit gueltigen Zeigern auf lokale Variablen; das
    // Puffer-Vec ist mit u64 ausgerichtet (die Eintraege enthalten Zeiger) und
    // lebt, solange die Eintraege gelesen werden; die Abfrage wird auf jedem
    // Weg geschlossen.
    unsafe {
        let mut query = PDH_HQUERY::default();
        if PdhOpenQueryW(windows::core::PCWSTR::null(), 0, &mut query) != 0 {
            return None;
        }
        let result = (|| {
            let mut counter = PDH_HCOUNTER::default();
            if PdhAddEnglishCounterW(
                query,
                w!("\\GPU Process Memory(*)\\Dedicated Usage"),
                0,
                &mut counter,
            ) != 0
            {
                return None;
            }
            if PdhCollectQueryData(query) != 0 {
                return None;
            }
            let mut size = 0u32;
            let mut count = 0u32;
            let status =
                PdhGetFormattedCounterArrayW(counter, PDH_FMT_LARGE, &mut size, &mut count, None);
            if status != PDH_MORE_DATA || size == 0 {
                return None;
            }
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_LARGE,
                &mut size,
                &mut count,
                Some(items),
            ) != 0
            {
                return None;
            }
            let mut out = Vec::with_capacity(count as usize);
            for i in 0..count as usize {
                let item = &*items.add(i);
                if item.FmtValue.CStatus != PDH_CSTATUS_VALID_DATA || item.szName.is_null() {
                    continue;
                }
                let Ok(name) = item.szName.to_string() else {
                    continue;
                };
                let Some((pid, luid)) = parse_gpu_instance(&name) else {
                    continue;
                };
                let value = item.FmtValue.Anonymous.largeValue;
                out.push(GpuInstance {
                    pid,
                    luid,
                    bytes: value.max(0) as u64,
                });
            }
            Some(out)
        })();
        let _ = PdhCloseQuery(query);
        result
    }
}

#[cfg(not(windows))]
fn gpu_instances() -> Option<Vec<GpuInstance>> {
    None
}

struct Cached {
    at: Instant,
    luid: String,
    usage: AppUsage,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

/// Darf nach `age` neu gemessen werden (oder wechselte der Adapter)?
fn is_stale(age: Duration, same_adapter: bool) -> bool {
    age >= MIN_INTERVAL || !same_adapter
}

/// RAM und VRAM der App samt Kindprozessen. `gpu_luid`: der Adapter, dessen
/// Belegung die Fussleiste zeigt (leer: nicht einschraenken, sofern
/// messbar). Blockierend; hoechstens alle [`MIN_INTERVAL`] echte Messung.
pub fn measure(gpu_luid: &str) -> AppUsage {
    {
        let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = cache.as_ref() {
            if !is_stale(c.at.elapsed(), c.luid == gpu_luid) {
                return c.usage;
            }
        }
    }
    let mut rows = process_rows();
    let root = std::process::id();
    let pids = tree_pids(root, &rows);
    fill_memory(&mut rows, &pids);
    let usage = AppUsage {
        ram_mb: sum_tree_ram_mb(root, &rows),
        gpu_mb: gpu_instances().map(|inst| {
            let luid = (!gpu_luid.is_empty()).then_some(gpu_luid);
            sum_gpu_mb(&pids, luid, &inst)
        }),
    };
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Cached {
        at: Instant::now(),
        luid: gpu_luid.to_string(),
        usage,
    });
    usage
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    fn row(pid: u32, ppid: u32, mb: u64) -> ProcRow {
        ProcRow {
            pid,
            ppid,
            mem_bytes: mb * MB,
        }
    }

    #[test]
    fn tree_sums_root_and_all_descendants_only() {
        let rows = [
            row(1, 0, 10_000),    // fremd
            row(100, 1, 1_000),   // App
            row(200, 100, 3_000), // llama-server
            row(300, 200, 500),   // Enkel
            row(400, 100, 250),   // ffmpeg
            row(500, 1, 9_000),   // fremd, anderer Elternteil
        ];
        let mut pids = tree_pids(100, &rows);
        pids.sort();
        assert_eq!(pids, vec![100, 200, 300, 400]);
        assert_eq!(sum_tree_ram_mb(100, &rows), 4_750);
    }

    #[test]
    fn tree_survives_cycles_and_unknown_root() {
        // Wiederverwendete PIDs koennen einen Kreis bilden.
        let rows = [row(100, 200, 10), row(200, 100, 20), row(300, 300, 40)];
        assert_eq!(sum_tree_ram_mb(100, &rows), 30);
        // Die Wurzel selbst fehlt in der Liste: nichts zu zaehlen, kein Absturz.
        assert_eq!(sum_tree_ram_mb(999, &rows), 0);
        assert_eq!(sum_tree_ram_mb(300, &rows), 40);
        assert_eq!(sum_tree_ram_mb(1, &[]), 0);
    }

    #[test]
    fn instance_names_are_parsed_or_rejected() {
        assert_eq!(
            parse_gpu_instance("pid_10600_luid_0x00000000_0x0001EE10_phys_0"),
            Some((10600, "0x00000000_0x0001ee10".to_string()))
        );
        assert_eq!(
            parse_gpu_instance("pid_4_luid_0x00000000_0x00018AE6_phys_1").map(|p| p.0),
            Some(4)
        );
        for bad in [
            "_total",
            "",
            "pid__luid_0x0_0x1_phys_0",
            "pid_x_luid_0x0_0x1_phys_0",
            "pid_1_luid_0x00000000_phys_0",
            "pid_1_luid_0xZZ_0x1_phys_0",
            "pid_1",
        ] {
            assert_eq!(parse_gpu_instance(bad), None, "{bad}");
        }
    }

    #[test]
    fn gpu_sum_filters_by_process_and_adapter() {
        let inst = |pid, luid: &str, mb: u64| GpuInstance {
            pid,
            luid: luid.into(),
            bytes: mb * MB,
        };
        let all = [
            inst(200, "0x0_0xa", 800),   // llama-server auf der dGPU
            inst(100, "0x0_0xa", 100),   // App (Vulkan-Kontext) auf der dGPU
            inst(100, "0x0_0xb", 50),    // App auf der iGPU
            inst(999, "0x0_0xa", 7_000), // fremder Prozess
        ];
        assert_eq!(sum_gpu_mb(&[100, 200], Some("0x0_0xa"), &all), 900);
        assert_eq!(sum_gpu_mb(&[100, 200], Some("0x0_0xb"), &all), 50);
        assert_eq!(sum_gpu_mb(&[100, 200], None, &all), 950);
        assert_eq!(sum_gpu_mb(&[], None, &all), 0);
        assert_eq!(sum_gpu_mb(&[100], Some("0x0_0xc"), &all), 0);
    }

    #[test]
    fn cache_is_reused_inside_the_interval_only() {
        assert!(!is_stale(Duration::from_secs(1), true));
        assert!(is_stale(MIN_INTERVAL, true));
        // Anderer Adapter: sofort neu, sonst stuende die Zahl der falschen Karte da.
        assert!(is_stale(Duration::from_millis(1), false));
    }

    /// Handprobe am echten Rechner: liest den PDH-Indikator und druckt die
    /// groessten GPU-Belegungen je Prozess. `cargo test gpu_counter_probe --
    /// --ignored --nocapture`. Kein Automatiktest: auf Rechnern ohne GPU ist
    /// `None` richtig.
    #[test]
    #[ignore]
    fn gpu_counter_probe() {
        let Some(mut inst) = gpu_instances() else {
            eprintln!("Indikator nicht verfuegbar (None)");
            return;
        };
        inst.sort_by_key(|i| std::cmp::Reverse(i.bytes));
        for i in inst.iter().take(6) {
            eprintln!("pid {} luid {} {} MB", i.pid, i.luid, i.bytes / MB);
        }
        eprintln!("{} Instanzen", inst.len());
    }

    /// Die eigene Testprozess-Messung: ein Kind mit bekanntem Speicher zaehlt
    /// mit, und der RAM-Anteil ist plausibel (> 0, < Gesamt-RAM).
    #[test]
    fn own_process_is_measured_and_a_child_counts() {
        let usage = measure("");
        assert!(usage.ram_mb > 0, "App-RAM: {}", usage.ram_mb);
        let total = {
            let mut s = sysinfo::System::new();
            s.refresh_memory();
            s.total_memory() / MB
        };
        assert!(usage.ram_mb < total);
        // Kind: das Tauri-freie Testprogramm startet `cmd /c ping` bzw. `sleep`.
        #[cfg(windows)]
        {
            let mut child = std::process::Command::new("ping")
                .args(["-n", "6", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .spawn()
                .expect("ping");
            let mut rows = process_rows();
            let pids = tree_pids(std::process::id(), &rows);
            assert!(pids.contains(&child.id()), "das Kind gehoert zum Baum");
            // Das Kind traegt seinen Arbeitssatz zur Summe bei.
            fill_memory(&mut rows, &pids);
            let child_bytes = rows
                .iter()
                .find(|r| r.pid == child.id())
                .map(|r| r.mem_bytes);
            assert!(
                child_bytes.unwrap_or(0) > 0,
                "Arbeitssatz des Kindes: {child_bytes:?}"
            );
            assert!(sum_tree_ram_mb(std::process::id(), &rows) >= child_bytes.unwrap() / MB);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
