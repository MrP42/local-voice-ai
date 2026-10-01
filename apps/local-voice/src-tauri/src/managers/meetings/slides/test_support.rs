//! D1: gemeinsame Hilfen der Tests (nur `cfg(test)`): ein ffmpeg-Check, ein
//! Prozess-Check und ein per lavfi erzeugtes Testvideo.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};

use super::store::{MeetingSlide, NewSlide, ORIGIN_VIDEO};
use super::{dhash64, SlideOccurrence, FRAME_BYTES, SAMPLE_H, SAMPLE_W};
use crate::managers::meetings::store::MeetingStore;

/// Serialisiert die Tests, die Kindprozesse starten: unter Windows erbt ein
/// gleichzeitig gestarteter Prozess gelegentlich fremde Pipe-Enden, und die
/// Lauefe sollen den Rechner nicht zusaetzlich belasten.
pub(crate) fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ist ffmpeg erreichbar? (Tests ueberspringen sich sonst, wie in `media.rs`.)
pub(crate) fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Lebt der Prozess mit dieser PID noch?
#[cfg(windows)]
pub(crate) fn process_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.process(pid).is_some()
}

/// Das Testvideo: 30 s, 640x360, 25 fps, drei Folien zu je 10 s, dazu ein dauernd
/// bewegtes Feld (128x72, unten rechts, "Webcam") ueber die ganze Laenge.
pub(crate) struct SlideVideo {
    pub path: PathBuf,
}

/// Versionsmarke im Dateinamen des zwischengespeicherten Testvideos: bei jeder
/// Aenderung am Erzeugungsbefehl hochzaehlen.
const VIDEO_VERSION: u32 = 1;

/// Das Testvideo, einmal je Testprozess erzeugt (ffmpeg-lavfi, ca. 5 s) und im
/// Temp-Ordner abgelegt: viele Tests nutzen es gleichzeitig, und je Test ein
/// eigenes zu bauen hielte die Maschine minutenlang beschaeftigt. Panikt, wenn
/// ffmpeg scheitert: die Aufrufer pruefen vorher [`ffmpeg_available`].
pub(crate) fn shared_slide_video() -> SlideVideo {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    let path = PATH
        .get_or_init(|| {
            let path = std::env::temp_dir().join(format!("lva-d1-slide-test-v{VIDEO_VERSION}.mp4"));
            if std::fs::metadata(&path).is_ok_and(|m| m.len() > 100_000) {
                return path;
            }
            // Erst unter eigenem Namen, dann umbenennen: ein zweiter Testprozess
            // sieht nie ein halbes Video.
            let tmp = path.with_file_name(format!(
                "lva-d1-slide-test-v{VIDEO_VERSION}.{}.tmp.mp4",
                std::process::id()
            ));
            generate_slide_video(&tmp);
            if std::fs::rename(&tmp, &path).is_err() {
                // Ein anderer Prozess war schneller: sein Video gilt.
                let _ = std::fs::remove_file(&tmp);
                if !path.is_file() {
                    panic!("das Testvideo liess sich nicht ablegen");
                }
            }
            path
        })
        .clone();
    SlideVideo { path }
}

fn generate_slide_video(path: &Path) {
    let slide1 = "color=c=white:s=640x360:r=25:d=10,drawbox=x=40:y=30:w=560:h=40:color=black:t=fill,drawbox=x=60:y=110:w=400:h=18:color=gray:t=fill,drawbox=x=60:y=160:w=450:h=18:color=gray:t=fill,drawbox=x=60:y=210:w=300:h=18:color=gray:t=fill,drawbox=x=60:y=260:w=380:h=18:color=gray:t=fill";
    let slide2 = "color=c=0x1F2A44:s=640x360:r=25:d=10,drawbox=x=330:y=60:w=270:h=240:color=white:t=fill,drawbox=x=40:y=70:w=220:h=30:color=0xAAAAAA:t=fill,drawbox=x=40:y=140:w=180:h=30:color=0xAAAAAA:t=fill,drawbox=x=40:y=210:w=240:h=30:color=0xAAAAAA:t=fill";
    let slide3 = "color=c=0xE0E0E0:s=640x360:r=25:d=10,drawbox=x=60:y=40:w=60:h=280:color=black:t=fill,drawbox=x=180:y=120:w=60:h=200:color=0x336699:t=fill,drawbox=x=300:y=200:w=60:h=120:color=black:t=fill,drawbox=x=420:y=80:w=60:h=240:color=0x993333:t=fill,drawbox=x=540:y=160:w=60:h=160:color=black:t=fill";
    let field = "testsrc2=s=128x72:r=25:d=30";
    for codec in ["libx264", "mpeg4"] {
        let output = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "lavfi", "-i", slide1])
            .args(["-f", "lavfi", "-i", slide2])
            .args(["-f", "lavfi", "-i", slide3])
            .args(["-f", "lavfi", "-i", field])
            .args(["-filter_threads", "2", "-filter_complex_threads", "2"])
            .args([
                "-filter_complex",
                "[0:v][1:v][2:v]concat=n=3:v=1:a=0[bg];[bg][3:v]overlay=x=W-w-8:y=H-h-8[out]",
                "-map",
                "[out]",
                "-c:v",
                codec,
                "-threads",
                "2",
                "-pix_fmt",
                "yuv420p",
                "-t",
                "30",
            ])
            .args(if codec == "libx264" {
                ["-preset", "ultrafast"]
            } else {
                ["-q:v", "4"]
            })
            .args(["-f", "mp4"])
            .arg(path)
            .output()
            .expect("ffmpeg startet");
        if output.status.success() {
            return;
        }
        eprintln!(
            "Testvideo mit {codec} gescheitert: {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("")
        );
    }
    panic!("das Testvideo liess sich mit keinem Codec erzeugen");
}

/// dHash eines Bildes (JPEG o. ae.) auf demselben Weg wie die Abtastung: 160x90
/// Graustufen, ein Bild.
pub(crate) fn gray_hash_of_image(path: &Path) -> u64 {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!(
            "scale={SAMPLE_W}:{SAMPLE_H}:flags=area,format=gray"
        ))
        .args(["-f", "rawvideo", "-pix_fmt", "gray", "pipe:1"])
        .output()
        .expect("ffmpeg startet");
    assert_eq!(
        out.stdout.len(),
        FRAME_BYTES,
        "genau ein Abtastbild erwartet"
    );
    dhash64(&out.stdout, SAMPLE_W, SAMPLE_H)
}

/// D5: legt eine Folie an einer Zeit an (Sichtung 20 s lang), mit Text (`text`) und
/// Art `text`, oder ohne. Fuer die Tests von Protokoll, KI-Notizen, Suche und Chat.
pub(crate) fn add_slide(
    store: &MeetingStore,
    meeting_id: &str,
    start_ms: u64,
    text: Option<&str>,
) -> MeetingSlide {
    let made = store
        .slide_insert(&NewSlide {
            meeting_id: meeting_id.to_string(),
            number: None,
            origin: ORIGIN_VIDEO,
            image_path: "slides/0001.jpg".to_string(),
            thumb_path: None,
            dhash: start_ms,
            occurrences: vec![SlideOccurrence {
                start_ms,
                end_ms: start_ms + 20_000,
            }],
        })
        .expect("Folie anlegen");
    if let Some(text) = text {
        store
            .slide_set_text(&made.id, Some(text), Some("windows-ocr"), Some("text"))
            .expect("Folientext setzen");
    }
    store
        .slide_get(&made.id)
        .expect("Folie lesen")
        .expect("Folie da")
}
