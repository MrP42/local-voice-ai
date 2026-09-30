use std::path::Path;

use super::*;
use crate::managers::youtube::fetch;
use crate::managers::youtube::tool::ToolStatus;

const ID: &str = "dQw4w9WgXcQ";

/// Ausgabe von `yt-dlp --list-subs` wie in der Praxis: automatische Spuren (mit
/// Uebersetzungen) vor den manuellen.
const LIST: &str = "[info] Available automatic captions for dQw4w9WgXcQ:
Language   Name                        Formats
af         Afrikaans                   vtt, ttml, srv3
de         German                      vtt, ttml, srv3
en-orig    English (Original)          vtt, ttml, srv3
en         English                     vtt, ttml, srv3
[info] Available subtitles for dQw4w9WgXcQ:
Language Name    Formats
de       German  vtt, ttml
fr       French  vtt
";

const MANUAL_DE: &str = "WEBVTT
Kind: captions
Language: de

00:00:01.000 --> 00:00:03.500
Willkommen zum Kanal &amp; zur Reihe.

00:00:03.500 --> 00:00:06.000
<c.colorE5E5E5>Heute</c> geht es um den Lastgang.
";

const AUTO_EN: &str = "WEBVTT
Kind: captions
Language: en

00:00:00.160 --> 00:00:02.310 align:start position:0%

hello<00:00:00.640><c> everyone</c><00:00:01.040><c> welcome</c>

00:00:02.310 --> 00:00:02.320 align:start position:0%
hello everyone welcome


00:00:02.320 --> 00:00:04.150 align:start position:0%
hello everyone welcome
today<00:00:02.800><c> we</c><00:00:03.040><c> talk</c>

00:00:04.150 --> 00:00:04.160 align:start position:0%
today we talk


00:00:04.160 --> 00:00:06.000 align:start position:0%
today we talk
about<00:00:04.500><c> load</c><00:00:05.000><c> profiles</c>
";

fn texts(segments: &[StoredSegment]) -> Vec<&str> {
    segments.iter().map(|s| s.text.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Tabelle und Sprachwahl
// ---------------------------------------------------------------------------

#[test]
fn the_track_table_lists_manual_tracks_first_and_only_the_original_auto_track() {
    let tracks = parse_list_subs(LIST);
    let summary: Vec<(&str, bool)> = tracks
        .iter()
        .map(|t| (t.language.as_str(), t.auto))
        .collect();
    assert_eq!(
        summary,
        vec![("de", false), ("fr", false), ("en-orig", true)],
        "manuell zuerst; von den automatischen nur die Originalsprache"
    );
    assert!(tracks[0].recommended && !tracks[1].recommended && !tracks[2].recommended);
    assert_eq!(tracks[0].name, "German");
    assert_eq!(tracks[2].name, "English (Original)");
}

/// Ausgabe des yt-dlp 2026.08.19 (am echten Video geprueft): die Originalsprache
/// der automatischen Untertitel steht ohne Namen da, dazu ~150 Uebersetzungen.
const LIST_2026: &str = "[info] Available automatic captions for jNQXAC9IVRw:
Language   Name                               Formats
en                                            vtt
de                                            vtt
af-en      Afrikaans from English             vtt, srt, ttml, srv3, srv2, srv1, json3
zh-Hans-en Chinese (Simplified) from English  vtt, srt, ttml, srv3, srv2, srv1, json3
en-de      English from German                vtt, srt, ttml, srv3, srv2, srv1, json3
[info] Available subtitles for jNQXAC9IVRw:
Language Name    Formats
en       English vtt, srt, ttml, srv3, srv2, srv1, json3
de       German  vtt, srt, ttml, srv3, srv2, srv1, json3
";

#[test]
fn current_ytdlp_lists_the_original_auto_track_without_name_and_many_translations() {
    let tracks = parse_list_subs(LIST_2026);
    let summary: Vec<(&str, &str, bool)> = tracks
        .iter()
        .map(|t| (t.language.as_str(), t.name.as_str(), t.auto))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("en", "English", false),
            ("de", "German", false),
            ("en", "en", true),
            ("de", "de", true),
        ],
        "Uebersetzungen (\"X from Y\") fallen weg, die Originale ohne Namen bleiben"
    );
    assert!(tracks[0].recommended, "manuell zuerst");
}

#[test]
fn only_translations_are_kept_when_nothing_else_exists() {
    let out = "[info] Available automatic captions for x:
Language Name Formats
fr-en French from English vtt
";
    let tracks = parse_list_subs(out);
    assert_eq!(tracks.len(), 1);
    assert!(tracks[0].auto);
}

#[test]
fn without_an_original_track_all_automatic_tracks_stay() {
    let out = "[info] Available automatic captions for x:
Language Name Formats
de German vtt
en English vtt
";
    let tracks = parse_list_subs(out);
    assert_eq!(tracks.len(), 2);
    assert!(tracks.iter().all(|t| t.auto));
}

#[test]
fn a_video_without_subtitles_gives_an_empty_list() {
    for out in [
        "",
        "dQw4w9WgXcQ has no automatic captions\ndQw4w9WgXcQ has no subtitles\n",
        "[info] Available subtitles for x:\nLanguage Name Formats\n",
        "ERROR: [youtube] x: Video unavailable\n",
        "<html>kein yt-dlp</html>",
    ] {
        assert!(parse_list_subs(out).is_empty(), "{out:?}");
    }
}

#[test]
fn language_codes_are_restricted_to_a_safe_alphabet() {
    for ok in ["de", "en-orig", "zh-Hans", "pt_BR", "a"] {
        assert!(valid_language(ok), "{ok}");
    }
    for bad in [
        "",
        "-de",
        "de,en",
        "de.*",
        "de en",
        "de;calc",
        "ä",
        "de
",
        &"x".repeat(36),
    ] {
        assert!(!valid_language(bad), "{bad:?}");
    }
}

// ---------------------------------------------------------------------------
// Argumente
// ---------------------------------------------------------------------------

#[test]
fn the_arguments_are_fixed_and_the_address_comes_from_the_id() {
    let list = list_args(ID);
    let fetch_manual = fetch_args(ID, "de", false);
    let fetch_auto = fetch_args(ID, "en-orig", true);
    let audio = fetch::audio_args(ID);
    for args in [&list, &fetch_manual, &fetch_auto, &audio] {
        assert_eq!(args[0], "--ignore-config", "Konfigdateien nie laden");
        assert!(args.contains(&"--no-playlist".to_string()));
        let dashdash = args.iter().position(|a| a == "--").expect("Optionsende");
        assert_eq!(args.len(), dashdash + 2, "nach -- nur die Adresse");
        assert_eq!(
            args[dashdash + 1],
            format!("https://www.youtube.com/watch?v={ID}")
        );
    }
    assert!(
        list.contains(&"--list-subs".to_string()) && list.contains(&"--skip-download".to_string())
    );
    assert!(fetch_manual.contains(&"--write-subs".to_string()));
    assert!(!fetch_manual.contains(&"--write-auto-subs".to_string()));
    assert!(fetch_auto.contains(&"--write-auto-subs".to_string()));
    assert!(fetch_manual
        .windows(2)
        .any(|w| w == ["--sub-format", "vtt"]));
    assert!(fetch_manual.windows(2).any(|w| w == ["--sub-langs", "de"]));
    assert!(audio.windows(2).any(|w| w == ["-f", "bestaudio/best"]));
    assert!(audio.windows(2).any(|w| w == ["--max-filesize", "500M"]));
    assert!(!audio.contains(&"--skip-download".to_string()));
    // Nichts, das die Ausgabe ausserhalb des Arbeitsordners anlegen koennte.
    for args in [&fetch_manual, &audio] {
        assert!(args.windows(2).any(|w| w == ["-o", "%(id)s.%(ext)s"]));
        assert!(!args
            .iter()
            .any(|a| a == "-P" || a == "--paths" || a == "--exec"));
    }
}

// ---------------------------------------------------------------------------
// VTT
// ---------------------------------------------------------------------------

#[test]
fn manual_subtitles_lose_tags_and_entities() {
    let segments = parse_youtube_vtt(MANUAL_DE, false).unwrap();
    assert_eq!(
        texts(&segments),
        vec![
            "Willkommen zum Kanal & zur Reihe.",
            "Heute geht es um den Lastgang."
        ]
    );
    assert_eq!((segments[0].start_ms, segments[0].end_ms), (1000, 3500));
    assert_eq!(segments[1].segment_index, 1);
    assert_eq!(segments[0].channel, 2, "Mischkanal wie beim Datei-Import");
}

#[test]
fn rolling_auto_captions_become_one_segment_per_new_line() {
    let segments = parse_youtube_vtt(AUTO_EN, true).unwrap();
    assert_eq!(
        texts(&segments),
        vec![
            "hello everyone welcome",
            "today we talk",
            "about load profiles"
        ]
    );
    assert_eq!(segments[0].start_ms, 160);
    assert_eq!(segments[2].end_ms, 6000);
    let indexes: Vec<u32> = segments.iter().map(|s| s.segment_index).collect();
    assert_eq!(indexes, vec![0, 1, 2], "fortlaufend nummeriert");
}

#[test]
fn timecodes_without_hours_are_accepted_and_empty_files_are_refused() {
    let short = "WEBVTT\n\n01:05.000 --> 01:07.500\nKurz ohne Stunden\n";
    let segments = parse_youtube_vtt(short, false).unwrap();
    assert_eq!((segments[0].start_ms, segments[0].end_ms), (65_000, 67_500));
    for bad in [
        "",
        "WEBVTT\n\n",
        "kein vtt",
        "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n \n",
    ] {
        assert!(parse_youtube_vtt(bad, true).is_err(), "{bad:?}");
    }
}

// ---------------------------------------------------------------------------
// Zugang, Fehler, Temp-Ordner
// ---------------------------------------------------------------------------

fn found_status(path: &Path) -> ToolStatus {
    ToolStatus {
        found: true,
        path: Some(path.display().to_string()),
        version: Some("2026.09.01".to_string()),
        source: Some("configured".to_string()),
        error: None,
    }
}

#[test]
fn the_way_is_closed_without_the_switch_or_the_tool() {
    let status = found_status(Path::new("C:/Tools/yt-dlp.exe"));
    assert_eq!(
        fetch::resolve(false, &status).unwrap_err(),
        YoutubeError::PrivateOff
    );
    let missing = ToolStatus {
        found: false,
        path: None,
        version: None,
        source: None,
        error: Some("not_found".into()),
    };
    assert_eq!(
        fetch::resolve(true, &missing).unwrap_err(),
        YoutubeError::ToolMissing
    );
    assert_eq!(
        fetch::resolve(true, &status).unwrap().display().to_string(),
        "C:/Tools/yt-dlp.exe"
    );
    // Der Hinweistext nennt, was zu tun ist.
    assert!(YoutubeError::PrivateOff
        .to_string()
        .contains("yt-dlp selbst installieren"));
    assert!(YoutubeError::PrivateOff.to_string().contains("privat"));
}

#[test]
fn failures_of_the_tool_are_classified_for_a_clear_message() {
    let finished = |code: i32, err: &str| RunEnd::Finished {
        code,
        stdout: Vec::new(),
        stderr_tail: err.to_string(),
    };
    assert_eq!(fetch::classify(RunEnd::TimedOut), YoutubeError::Timeout);
    assert_eq!(fetch::classify(RunEnd::Cancelled), YoutubeError::Cancelled);
    assert_eq!(
        fetch::classify(finished(
            1,
            "ERROR: [youtube] x: Unable to extract player response"
        )),
        YoutubeError::ToolOutdated
    );
    assert_eq!(
        fetch::classify(finished(
            1,
            "ERROR: unable to download: HTTP Error 403: Forbidden"
        )),
        YoutubeError::ToolOutdated
    );
    assert_eq!(
        fetch::classify(finished(1, "ERROR: [youtube] x: Private video. Sign in")),
        YoutubeError::Unavailable
    );
    assert_eq!(
        fetch::classify(finished(7, "irgendwas")),
        YoutubeError::ToolFailed(7)
    );
    assert!(YoutubeError::ToolOutdated.to_string().contains("veraltet"));
}

#[test]
fn stale_temp_folders_of_a_crashed_run_are_swept_and_nothing_else() {
    let base = tempfile::tempdir().unwrap();
    let old = base.path().join("lva-yt-old");
    let fresh = base.path().join("lva-yt-fresh");
    let foreign = base.path().join("other-old");
    let file = base.path().join("lva-yt-file.txt");
    for d in [&old, &fresh, &foreign] {
        std::fs::create_dir(d).unwrap();
        std::fs::write(d.join("x"), b"x").unwrap();
    }
    std::fs::write(&file, b"x").unwrap();
    // Alter 0 = alles mit dem Praefix, das ein Ordner ist; Fremdes bleibt.
    let removed = fetch::sweep_stale(base.path(), Duration::ZERO);
    assert_eq!(removed, 2, "beide lva-yt-Ordner");
    assert!(!old.exists() && !fresh.exists());
    assert!(foreign.exists(), "fremde Ordner bleiben");
    assert!(file.exists(), "Dateien bleiben");
    // Mit dem echten Alter (24 h) bleibt ein frischer Ordner stehen.
    let young = base.path().join("lva-yt-young");
    std::fs::create_dir(&young).unwrap();
    assert_eq!(
        fetch::sweep_stale(base.path(), Duration::from_secs(24 * 3600)),
        0
    );
    assert!(young.exists());
}

#[test]
fn the_temp_folder_disappears_with_its_owner() {
    let base = tempfile::tempdir().unwrap();
    let path = {
        let tmp = fetch::TempDir::create(Some(base.path())).unwrap();
        std::fs::write(tmp.path().join("audio.webm"), b"x").unwrap();
        assert!(tmp.path().starts_with(base.path()));
        assert!(tmp
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("lva-yt-"));
        tmp.path().to_path_buf()
    };
    assert!(!path.exists());
}

// ---------------------------------------------------------------------------
// Der echte Kindprozess (Windows: Batch-Dateien als Attrappen)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod process {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use super::*;
    use crate::managers::integrations::audit::{self, AuditFilter};
    use crate::managers::integrations::model::IntegrationPatch;
    use crate::managers::integrations::store as register;
    use crate::managers::integrations::test_support::Fx;
    use crate::managers::meetings::variants as fassungen;
    use crate::managers::provenance::{self, SubjectKind};
    use crate::managers::youtube::link::normalize_link;
    use crate::managers::youtube::oembed::VideoMeta;
    use crate::managers::youtube::{source, variants, INTEGRATION_ID};

    /// yt-dlp-Attrappe: schreibt jeden Aufruf in `calls.log`, antwortet auf
    /// `--list-subs` mit `list.txt` und legt je nach Schalter die VTT-Dateien bzw.
    /// Audio im Arbeitsordner ab.
    fn stub(dir: &Path) -> PathBuf {
        std::fs::write(dir.join("list.txt"), LIST.replace('\n', "\r\n")).unwrap();
        std::fs::write(dir.join("manual.vtt"), MANUAL_DE).unwrap();
        std::fs::write(dir.join("auto.vtt"), AUTO_EN).unwrap();
        let body = [
            "@echo off",
            "echo %* >> \"%~dp0calls.log\"",
            "echo %CD% >> \"%~dp0cwd.log\"",
            "echo %* | findstr /C:\"--list-subs\" >nul",
            "if not errorlevel 1 goto list",
            "echo %* | findstr /C:\"--write-auto-subs\" >nul",
            "if not errorlevel 1 goto auto",
            "echo %* | findstr /C:\"--write-subs\" >nul",
            "if not errorlevel 1 goto manual",
            "echo %* | findstr /C:\"bestaudio\" >nul",
            "if not errorlevel 1 goto audio",
            "exit /b 9",
            ":list",
            "type \"%~dp0list.txt\"",
            "exit /b 0",
            ":auto",
            "copy /y \"%~dp0auto.vtt\" \"dQw4w9WgXcQ.en-orig.vtt\" >nul",
            "exit /b 0",
            ":manual",
            "copy /y \"%~dp0manual.vtt\" \"dQw4w9WgXcQ.de.vtt\" >nul",
            "exit /b 0",
            ":audio",
            "echo 12345678901234567890123456789012345678901234567890> dQw4w9WgXcQ.webm.part",
            "echo audio> dQw4w9WgXcQ.webm",
            "exit /b 0",
            "",
        ]
        .join("\r\n");
        let exe = dir.join("yt-dlp-stub.cmd");
        std::fs::write(&exe, body).unwrap();
        exe
    }

    fn calls(dir: &Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .collect()
    }

    struct Tools {
        dir: tempfile::TempDir,
        temp: tempfile::TempDir,
        exe: PathBuf,
    }

    fn tools() -> Tools {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let exe = stub(dir.path());
        Tools { dir, temp, exe }
    }

    impl Tools {
        fn run(&self) -> ToolRun<'_> {
            ToolRun {
                exe: &self.exe,
                temp_base: Some(self.temp.path()),
            }
        }
        fn status(&self) -> ToolStatus {
            found_status(&self.exe)
        }
        fn leftovers(&self) -> usize {
            std::fs::read_dir(self.temp.path()).unwrap().count()
        }
    }

    #[test]
    fn the_track_list_is_read_from_the_tool_output() {
        let t = tools();
        let tracks = list_tracks(&t.run(), ID, None).unwrap();
        assert_eq!(tracks.len(), 3);
        let log = calls(t.dir.path());
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("--list-subs") && log[0].contains("--ignore-config"));
        assert!(log[0].contains(&format!("https://www.youtube.com/watch?v={ID}")));
        assert_eq!(t.leftovers(), 0, "Temp-Ordner weg");
    }

    #[test]
    fn two_language_tracks_are_fetched_as_two_different_transcripts() {
        let t = tools();
        let de = SubtitleTrack {
            language: "de".into(),
            name: "German".into(),
            auto: false,
            recommended: true,
        };
        let en = SubtitleTrack {
            language: "en-orig".into(),
            name: "English (Original)".into(),
            auto: true,
            recommended: false,
        };
        let manual = fetch_track(&t.run(), ID, &de, None).unwrap();
        let auto = fetch_track(&t.run(), ID, &en, None).unwrap();
        assert_eq!(manual.len(), 2);
        assert_eq!(
            texts(&auto),
            vec![
                "hello everyone welcome",
                "today we talk",
                "about load profiles"
            ]
        );
        let log = calls(t.dir.path());
        assert!(log[0].contains("--write-subs") && log[0].contains("--sub-langs de"));
        assert!(log[1].contains("--write-auto-subs") && log[1].contains("--sub-langs en-orig"));
        assert_eq!(t.leftovers(), 0);
        // Der Prozess lief im eigenen Temp-Ordner, nicht im Ordner der Attrappe.
        let cwd = std::fs::read_to_string(t.dir.path().join("cwd.log")).unwrap();
        assert!(cwd.contains("lva-yt-"), "{cwd}");
    }

    #[test]
    fn a_language_code_outside_the_alphabet_never_reaches_the_tool() {
        let t = tools();
        let bad = SubtitleTrack {
            language: "de,en".into(),
            name: "x".into(),
            auto: false,
            recommended: false,
        };
        assert!(fetch_track(&t.run(), ID, &bad, None).is_err());
        assert!(
            calls(t.dir.path()).is_empty(),
            "das Programm wurde nicht gestartet"
        );
    }

    #[test]
    fn a_tool_that_fails_is_reported_and_leaves_no_files() {
        let t = tools();
        std::fs::write(
            &t.exe,
            "@echo off\r\necho ERROR: Unable to extract player response 1>&2\r\nexit /b 1\r\n",
        )
        .unwrap();
        let err = list_tracks(&t.run(), ID, None).unwrap_err();
        assert_eq!(err, YoutubeError::ToolOutdated);
        assert_eq!(t.leftovers(), 0);
        // Erfolg ohne Datei: "keine Untertitel", nie ein stiller Erfolg.
        std::fs::write(&t.exe, "@echo off\r\nexit /b 0\r\n").unwrap();
        let track = SubtitleTrack {
            language: "de".into(),
            name: "German".into(),
            auto: false,
            recommended: true,
        };
        assert_eq!(
            fetch_track(&t.run(), ID, &track, None).unwrap_err(),
            YoutubeError::NoSubtitles
        );
        assert_eq!(t.leftovers(), 0);
    }

    #[test]
    fn a_hanging_tool_is_killed_at_the_limit_and_a_stop_ends_it_at_once() {
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("hang.cmd");
        std::fs::write(&exe, "@echo off\r\nping -n 60 127.0.0.1 >nul\r\n").unwrap();
        let started = Instant::now();
        let end = run::run(
            &exe,
            &[],
            &RunOpts {
                timeout: Duration::from_millis(1500),
                max_stdout: 100,
                cancel: None,
                cwd: None,
            },
        )
        .unwrap();
        assert_eq!(end, RunEnd::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10), "kein Haenger");

        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            flag.store(true, std::sync::atomic::Ordering::Release);
        });
        let started = Instant::now();
        let end = run::run(
            &exe,
            &[],
            &RunOpts {
                timeout: Duration::from_secs(60),
                max_stdout: 100,
                cancel: Some(&cancel),
                cwd: None,
            },
        )
        .unwrap();
        assert_eq!(end, RunEnd::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(10));
        // Schon gesetzt vor dem Start: das Programm startet gar nicht.
        let pre = AtomicBool::new(true);
        assert_eq!(
            run::run(
                &exe,
                &[],
                &RunOpts {
                    timeout: Duration::from_secs(60),
                    max_stdout: 1,
                    cancel: Some(&pre),
                    cwd: None
                }
            )
            .unwrap(),
            RunEnd::Cancelled
        );
    }

    #[test]
    fn a_flood_of_output_is_bounded() {
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("flood.cmd");
        std::fs::write(
            &exe,
            "@echo off\r\nfor /L %%i in (1,1,3000) do echo 0123456789012345678901234567890123456789\r\n",
        )
        .unwrap();
        let end = run::run(
            &exe,
            &[],
            &RunOpts {
                timeout: Duration::from_secs(60),
                max_stdout: 1000,
                cancel: None,
                cwd: None,
            },
        )
        .unwrap();
        match end {
            RunEnd::Finished {
                code: 0, stdout, ..
            } => assert_eq!(stdout.len(), 1000),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_audio_download_picks_the_finished_file_and_cleans_up() {
        let t = tools();
        let cancel = AtomicBool::new(false);
        let file = {
            let audio = fetch::download_audio(&t.run(), ID, &cancel).unwrap();
            assert!(
                audio.file.ends_with("dQw4w9WgXcQ.webm"),
                "nicht die .part-Datei"
            );
            assert!(audio.file.is_file());
            assert_eq!(t.leftovers(), 1, "waehrend der Nutzung ist der Ordner da");
            audio.file.clone()
        };
        assert!(!file.exists(), "Audio verschwindet mit dem Wert");
        assert_eq!(t.leftovers(), 0);
        let log = calls(t.dir.path());
        assert!(log[0].contains("bestaudio") && log[0].contains("--max-filesize 500M"));
        // Stopp vor dem Start: nichts wird geladen.
        let stop = AtomicBool::new(true);
        assert_eq!(
            fetch::download_audio(&t.run(), ID, &stop).err().unwrap(),
            YoutubeError::Cancelled
        );
        assert_eq!(calls(t.dir.path()).len(), 1);
    }

    // -- Ablauf mit Register, Audit, Fassung und Herkunft ---------------------

    fn youtube_meeting(fx: &Fx) -> String {
        let video = normalize_link(&format!("https://youtu.be/{ID}")).unwrap();
        let meta = VideoMeta {
            title: "Lastgang".into(),
            channel: "Wolff".into(),
            channel_url: None,
            thumbnail_url: None,
        };
        let mut conn = fx.conn();
        source::create_meeting(&mut conn, &video, &meta, None, 1_000).unwrap()
    }

    fn audits(fx: &Fx) -> Vec<crate::managers::integrations::model::AuditEntry> {
        audit::list(
            &fx.conn(),
            &AuditFilter {
                integration_id: Some(INTEGRATION_ID.to_string()),
                ..Default::default()
            },
            100,
        )
        .unwrap()
    }

    fn de_track() -> SubtitleTrack {
        SubtitleTrack {
            language: "de".into(),
            name: "German".into(),
            auto: false,
            recommended: true,
        }
    }

    #[test]
    fn subtitles_become_a_variant_with_provenance_and_an_audit_entry() {
        let fx = Fx::new();
        let t = tools();
        let id = youtube_meeting(&fx);
        let variant_id = variants::fetch_subtitles(
            &fx.store,
            &id,
            true,
            &t.status(),
            &de_track(),
            None,
            Some(t.temp.path()),
        )
        .unwrap();
        let mut conn = fx.conn();
        let list = fassungen::list(&mut conn, &id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, variant_id);
        assert_eq!(list[0].kind, fassungen::KIND_SUBTITLES_MANUAL);
        assert_eq!(list[0].language.as_deref(), Some("de"));
        assert!(list[0].active, "die erste Fassung wird aktiv");
        // Die Herkunft: Quelle Video, Sprache, Werkzeug.
        let entries = provenance::list(&conn, SubjectKind::TranscriptVariant, &variant_id).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].operation, "subtitles_import");
        assert_eq!(entries[0].sources[0].kind, "subtitle");
        // Audit: genau ein Eintrag fuer den Lauf, ok, Ziel ohne Adresse.
        let log = audits(&fx);
        let fetches: Vec<_> = log
            .iter()
            .filter(|e| e.capability.as_deref() == Some("media.fetch"))
            .collect();
        assert_eq!(fetches.len(), 1);
        assert_eq!(fetches[0].outcome, "ok");
        assert_eq!(
            fetches[0].target.as_deref(),
            Some(format!("youtube:{ID}").as_str())
        );
        assert!(fetches[0].detail_json.as_deref().unwrap().contains("done"));
    }

    #[test]
    fn a_second_language_becomes_a_second_variant_and_the_first_stays_active() {
        let fx = Fx::new();
        let t = tools();
        let id = youtube_meeting(&fx);
        let first = variants::fetch_subtitles(
            &fx.store,
            &id,
            true,
            &t.status(),
            &de_track(),
            None,
            Some(t.temp.path()),
        )
        .unwrap();
        let en = SubtitleTrack {
            language: "en-orig".into(),
            name: "English".into(),
            auto: true,
            recommended: false,
        };
        let second = variants::fetch_subtitles(
            &fx.store,
            &id,
            true,
            &t.status(),
            &en,
            None,
            Some(t.temp.path()),
        )
        .unwrap();
        let mut conn = fx.conn();
        let list = fassungen::list(&mut conn, &id).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(
            (list[0].id.as_str(), list[0].active),
            (first.as_str(), true)
        );
        assert_eq!(
            (list[1].id.as_str(), list[1].active),
            (second.as_str(), false)
        );
        assert_eq!(list[1].kind, fassungen::KIND_SUBTITLES_AUTO);
    }

    #[test]
    fn without_the_switch_nothing_is_started_and_nothing_is_audited() {
        let fx = Fx::new();
        let t = tools();
        let id = youtube_meeting(&fx);
        let err = variants::fetch_subtitles(
            &fx.store,
            &id,
            false,
            &t.status(),
            &de_track(),
            None,
            Some(t.temp.path()),
        )
        .unwrap_err();
        assert_eq!(err, YoutubeError::PrivateOff);
        assert!(calls(t.dir.path()).is_empty());
        assert!(audits(&fx)
            .iter()
            .all(|e| e.capability.as_deref() != Some("media.fetch")));
        let err = variants::tracks(
            &fx.store,
            &id,
            false,
            &t.status(),
            None,
            Some(t.temp.path()),
        )
        .unwrap_err();
        assert_eq!(err, YoutubeError::PrivateOff);
    }

    #[test]
    fn a_blocked_gate_stops_before_the_process_and_a_failed_run_is_audited_as_error() {
        let fx = Fx::new();
        let t = tools();
        let id = youtube_meeting(&fx);
        source::ensure_integration(&fx.conn(), 1_000).unwrap();
        register::update(
            &fx.conn(),
            INTEGRATION_ID,
            &IntegrationPatch {
                enabled: Some(false),
                ..Default::default()
            },
            2_000,
        )
        .unwrap();
        let err = variants::fetch_subtitles(
            &fx.store,
            &id,
            true,
            &t.status(),
            &de_track(),
            None,
            Some(t.temp.path()),
        )
        .unwrap_err();
        assert!(matches!(err, YoutubeError::Disabled(_)), "{err:?}");
        assert!(calls(t.dir.path()).is_empty(), "Tor zu: kein Prozess");
        register::update(
            &fx.conn(),
            INTEGRATION_ID,
            &IntegrationPatch {
                enabled: Some(true),
                ..Default::default()
            },
            3_000,
        )
        .unwrap();
        std::fs::write(&t.exe, "@echo off\r\nexit /b 4\r\n").unwrap();
        let err = variants::fetch_subtitles(
            &fx.store,
            &id,
            true,
            &t.status(),
            &de_track(),
            None,
            Some(t.temp.path()),
        )
        .unwrap_err();
        assert_eq!(err, YoutubeError::ToolFailed(4));
        let last = audits(&fx)
            .into_iter()
            .find(|e| e.capability.as_deref() == Some("media.fetch") && e.outcome == "error");
        assert!(last.is_some(), "der Fehlschlag steht im Audit");
        assert!(
            fassungen::list(&mut fx.conn(), &id).unwrap().is_empty(),
            "keine halbe Fassung"
        );
    }

    #[test]
    fn a_non_youtube_meeting_is_refused() {
        let fx = Fx::new();
        let t = tools();
        let other = fx
            .store
            .create_meeting(
                "Import",
                crate::managers::meetings::store::MeetingSource::Import,
                None,
            )
            .unwrap()
            .id;
        let err = variants::tracks(
            &fx.store,
            &other,
            true,
            &t.status(),
            None,
            Some(t.temp.path()),
        )
        .unwrap_err();
        assert!(matches!(err, YoutubeError::BadResponse(_)));
        assert!(calls(t.dir.path()).is_empty());
    }

    #[test]
    fn the_player_duration_is_added_once_and_only_when_sensible() {
        let fx = Fx::new();
        let id = youtube_meeting(&fx);
        let duration = |fx: &Fx| fx.store.get_meeting(&id).unwrap().unwrap().duration_ms;
        assert_eq!(duration(&fx), None);
        for bad in [f64::NAN, f64::INFINITY, -5.0, 0.0, 0.5, 1e12] {
            assert!(
                !variants::set_duration_if_missing(&fx.store, &id, bad).unwrap(),
                "{bad}"
            );
        }
        assert_eq!(duration(&fx), None);
        assert!(variants::set_duration_if_missing(&fx.store, &id, 1234.56).unwrap());
        assert_eq!(duration(&fx), Some(1_234_560));
        assert!(
            !variants::set_duration_if_missing(&fx.store, &id, 99.0).unwrap(),
            "nie ueberschreiben"
        );
        assert_eq!(duration(&fx), Some(1_234_560));
        // Andere Quellen bleiben unberuehrt.
        let other = fx
            .store
            .create_meeting(
                "Import",
                crate::managers::meetings::store::MeetingSource::Import,
                None,
            )
            .unwrap()
            .id;
        assert!(!variants::set_duration_if_missing(&fx.store, &other, 60.0).unwrap());
    }
}

// ---------------------------------------------------------------------------
// Manueller Nachweis mit dem echten yt-dlp (nur auf Zuruf: `-- --ignored`)
// ---------------------------------------------------------------------------

/// Braucht ein selbst installiertes yt-dlp und Netz; laeuft nie in der normalen
/// Suite. Nimmt das erste Video aus `LVA_YT_PROBE` (Standard: ein oeffentliches
/// Video mit Untertiteln), listet die Spuren, laedt die empfohlene und prueft, dass
/// daraus Segmente mit Zeiten werden.
#[test]
#[ignore = "braucht Netz und ein selbst installiertes yt-dlp"]
fn a_real_ytdlp_lists_and_fetches_subtitles() {
    use crate::managers::youtube::tool;
    let status = tool::detect(None);
    assert!(status.found, "yt-dlp nicht gefunden: {status:?}");
    let exe = std::path::PathBuf::from(status.path.clone().unwrap());
    let id = std::env::var("LVA_YT_PROBE").unwrap_or_else(|_| "jNQXAC9IVRw".to_string());
    let temp = tempfile::tempdir().unwrap();
    let run = ToolRun {
        exe: &exe,
        temp_base: Some(temp.path()),
    };
    let tracks = list_tracks(&run, &id, None).expect("Spuren");
    println!("yt-dlp {:?}: {} Spuren", status.version, tracks.len());
    for t in &tracks {
        println!(
            "  {} ({}) auto={} empfohlen={}",
            t.language, t.name, t.auto, t.recommended
        );
    }
    let track = tracks
        .iter()
        .find(|t| t.recommended)
        .expect("eine Spur")
        .clone();
    let segments = fetch_track(&run, &id, &track, None).expect("Untertitel");
    println!(
        "{} Segmente, erstes: {:?}",
        segments.len(),
        segments.first()
    );
    assert!(!segments.is_empty());
    assert!(segments.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
    // Die automatische Spur (Rolltext) ist der haertere Fall fuer die Bereinigung.
    if let Some(auto) = tracks.iter().find(|t| t.auto) {
        let rolled = fetch_track(&run, &id, auto, None).expect("automatische Untertitel");
        println!(
            "auto {}: {} Segmente, erste: {:?}",
            auto.language,
            rolled.len(),
            rolled
                .iter()
                .take(3)
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
        );
        assert!(!rolled.is_empty());
        assert!(
            rolled.windows(2).all(|w| w[0].text != w[1].text),
            "Rolltext ist bereinigt: keine zwei gleichen Zeilen hintereinander"
        );
        assert!(
            rolled.iter().all(|s| !s.text.contains('<')),
            "keine Zeitmarken-Reste"
        );
    }
    assert_eq!(
        std::fs::read_dir(temp.path()).unwrap().count(),
        0,
        "Temp leer"
    );
}
