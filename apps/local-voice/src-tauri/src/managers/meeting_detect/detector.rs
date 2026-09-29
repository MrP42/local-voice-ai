//! Entscheidungslogik der Ad-hoc-Erkennung (M5 F16), rein: `Detector::step`
//! bekommt einen Snapshot der Mikrofonnutzung und sagt, ob ein Hinweis
//! erscheinen oder sich schliessen soll. Keine Uhr, keine Registry, keine
//! Prozessliste im Kern; alles kommt ueber Argumente, damit die Regeln ohne
//! Rechner-Zustand testbar sind.
//!
//! Regeln:
//! - Nur aktive Eintraege (`Stop == 0`) zaehlen. Sitzung = (`app_key`,
//!   `LastUsedTimeStart`); je Sitzung hoechstens ein Hinweis.
//! - Ein Hinweis erscheint erst, wenn die Sitzung seit mindestens 5 s laeuft
//!   (Teams-Geraetetest, kurze Mikrofonproben). Massgeblich ist der Start aus
//!   der Registry, nicht der erste Blick: so kostet die Abfrageluecke keine
//!   Zeit.
//! - Nie gemeldet werden: die eigene App, `msedgewebview2.exe`, Eintraege der
//!   Ignorierliste, klassische Programme ohne laufenden Prozess (Leiche nach
//!   Absturz: `Stop` blieb 0) und, je nach Modus, Programme ausserhalb des
//!   Katalogs.
//! - Laeuft eine Aufnahme, gibt es keinen Hinweis; die dann gerade laufenden
//!   Sitzungen bleiben auch nach dem Stopp stumm (der Anwender hat ja
//!   aufgenommen).
//! - Ein offener Hinweis schliesst sich, sobald seine Sitzung endet (Mikrofon
//!   frei), die Erkennung ausgeschaltet oder eine Aufnahme gestartet wird.
//! - Mehrere Apps zugleich: ein Hinweis, Vorrang Meeting-App vor Browser vor
//!   Rest, bei Gleichstand die aeltere Sitzung. Zugleich fertige Sitzungen
//!   gelten damit als abgedeckt.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::catalog::{self, AppClass};
use super::source::{filetime_to_unix_ms, MicUsage};

/// So lange muss eine Sitzung mindestens laufen, bevor ein Hinweis erscheint.
pub const DEBOUNCE_MS: u64 = 5_000;

/// Einstellung `meeting_detect_mode`.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type, Default)]
#[serde(rename_all = "snake_case")]
pub enum DetectMode {
    /// Keine Erkennung; die Registry wird nicht einmal gelesen.
    Off,
    /// Katalog-Apps und Browser (Standard).
    #[default]
    MeetingApps,
    /// Jedes Programm mit Mikrofonzugriff, auch ausserhalb des Katalogs.
    AllApps,
}

pub struct DetectCtx<'a> {
    pub mode: DetectMode,
    /// Aufnahme oder Diktat laeuft.
    pub recording: bool,
    /// Nutzerliste (Dateiname, Etikett oder Teil des Schluessels, ohne Gross-/Kleinschreibung).
    pub ignored: &'a [String],
    /// Schluessel der eigenen EXE (`np:...`), zusaetzlich zur Namensregel.
    pub self_keys: &'a [String],
    /// Laeuft zu diesem Pfad ein Prozess?
    pub process_alive: &'a dyn Fn(&Path) -> bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectEvent {
    /// Hinweis zeigen. `class`: `meeting_app` | `browser` | `other`.
    Started {
        app_key: String,
        label: String,
        class: &'static str,
    },
    /// Der Hinweis zu dieser App schliesst sich.
    Ended { app_key: String },
}

type Session = (String, u64);

#[derive(Default)]
pub struct Detector {
    /// Sitzungen, zu denen schon ein Hinweis kam oder die abgedeckt sind.
    handled: HashSet<Session>,
    /// Sitzung des offenen Hinweises.
    open: Option<Session>,
}

impl Detector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sitzung des offenen Hinweises (Schluessel), falls einer offen ist.
    pub fn open_app(&self) -> Option<&str> {
        self.open.as_ref().map(|(key, _)| key.as_str())
    }

    /// Warum wird dieser Eintrag nicht gemeldet? `None` = er ist ein Kandidat.
    /// Die Gruende stehen im JSON von `--detect-mic`.
    pub fn exclusion(u: &MicUsage, ctx: &DetectCtx) -> Option<&'static str> {
        if catalog::is_own_app(&u.app_key)
            || ctx
                .self_keys
                .iter()
                .any(|k| k.eq_ignore_ascii_case(&u.app_key))
        {
            return Some("own_app");
        }
        match catalog::classify(&u.app_key) {
            AppClass::Ignored => return Some("ignored_component"),
            AppClass::Other if ctx.mode != DetectMode::AllApps => return Some("not_in_catalog"),
            _ => {}
        }
        let key = u.app_key.to_ascii_lowercase();
        let label = catalog::label_for(&u.app_key).to_ascii_lowercase();
        let listed = ctx.ignored.iter().any(|entry| {
            let e = entry.trim().to_ascii_lowercase();
            !e.is_empty() && (key.contains(&e) || label.contains(&e))
        });
        if listed {
            return Some("ignore_list");
        }
        // Zuletzt, weil die Prozessliste am teuersten ist.
        match &u.exe_path {
            Some(path) if !(ctx.process_alive)(path) => Some("no_process"),
            _ => None,
        }
    }

    fn allowed(u: &MicUsage, ctx: &DetectCtx) -> bool {
        Self::exclusion(u, ctx).is_none()
    }

    /// Ein Schritt. `now_ms` sind Unix-Millisekunden.
    pub fn step(&mut self, now_ms: u64, snap: &[MicUsage], ctx: &DetectCtx) -> Vec<DetectEvent> {
        let mut events = Vec::new();

        if ctx.mode == DetectMode::Off {
            self.handled.clear();
            if let Some((app_key, _)) = self.open.take() {
                events.push(DetectEvent::Ended { app_key });
            }
            return events;
        }

        let live: Vec<&MicUsage> = snap.iter().filter(|u| u.active).collect();
        // Beendete Sitzungen vergessen; eine neue hat einen neuen Start.
        self.handled.retain(|(key, start)| {
            live.iter()
                .any(|u| &u.app_key == key && u.last_start == *start)
        });

        let eligible: Vec<&MicUsage> = live.into_iter().filter(|u| Self::allowed(u, ctx)).collect();

        // Hinweis schliessen: Sitzung vorbei oder Aufnahme laeuft.
        if let Some((key, start)) = &self.open {
            let still = eligible
                .iter()
                .any(|u| &u.app_key == key && u.last_start == *start);
            if !still || ctx.recording {
                events.push(DetectEvent::Ended {
                    app_key: key.clone(),
                });
                self.open = None;
            }
        }

        let mature: Vec<&MicUsage> = eligible
            .into_iter()
            .filter(|u| {
                !self.handled.contains(&(u.app_key.clone(), u.last_start))
                    && now_ms.saturating_sub(filetime_to_unix_ms(u.last_start)) >= DEBOUNCE_MS
            })
            .collect();
        if mature.is_empty() {
            return events;
        }

        if ctx.recording {
            for u in mature {
                self.handled.insert((u.app_key.clone(), u.last_start));
            }
            return events;
        }
        if self.open.is_some() {
            return events;
        }

        let winner = mature
            .iter()
            .min_by_key(|u| {
                (
                    catalog::rank(catalog::classify(&u.app_key)),
                    u.last_start,
                    u.app_key.clone(),
                )
            })
            .copied()
            .expect("mature is not empty");
        let class = catalog::classify(&winner.app_key);
        events.push(DetectEvent::Started {
            app_key: winner.app_key.clone(),
            label: catalog::label_for(&winner.app_key),
            class: catalog::kind_name(class),
        });
        self.open = Some((winner.app_key.clone(), winner.last_start));
        for u in mature {
            self.handled.insert((u.app_key.clone(), u.last_start));
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meeting_detect::source::unix_ms_to_filetime;

    const NOW: u64 = 1_790_000_000_000;
    const TEAMS: &str = "pkg:MSTeams_8wekyb3d8bbwe";
    const CHROME: &str = "np:C:#Program Files#Google#Chrome#Application#chrome.exe";
    const PYTHON: &str = "np:C:#Program Files#Python311#python.exe";
    const OWN: &str = "np:C:#Users#w#AppData#Local#Local Voice AI#local-voice-ai.exe";
    const WEBVIEW: &str =
        "np:C:#Program Files (x86)#Microsoft#EdgeWebView#Application#153.0#msedgewebview2.exe";

    /// Aktiver Eintrag, der vor `age_ms` Millisekunden startete.
    fn usage(key: &str, age_ms: u64) -> MicUsage {
        MicUsage {
            app_key: key.to_string(),
            exe_path: crate::managers::meeting_detect::catalog::exe_path_of(key),
            last_start: unix_ms_to_filetime(NOW - age_ms),
            active: true,
        }
    }

    fn idle(mut u: MicUsage) -> MicUsage {
        u.active = false;
        u
    }

    struct Setup {
        mode: DetectMode,
        recording: bool,
        ignored: Vec<String>,
        alive: bool,
    }

    fn setup(mode: DetectMode) -> Setup {
        Setup {
            mode,
            recording: false,
            ignored: vec![],
            alive: true,
        }
    }

    fn step(d: &mut Detector, now: u64, snap: &[MicUsage], s: &Setup) -> Vec<DetectEvent> {
        let alive = s.alive;
        let process_alive = move |_: &Path| alive;
        let ctx = DetectCtx {
            mode: s.mode,
            recording: s.recording,
            ignored: &s.ignored,
            self_keys: &[],
            process_alive: &process_alive,
        };
        d.step(now, snap, &ctx)
    }

    fn started_keys(events: &[DetectEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| match e {
                DetectEvent::Started { app_key, .. } => Some(app_key.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_meeting_app_active_for_five_seconds_gets_one_notice() {
        let mut d = Detector::new();
        let s = setup(DetectMode::MeetingApps);
        let ev = step(&mut d, NOW, &[usage(TEAMS, 6_000)], &s);
        assert_eq!(
            ev,
            vec![DetectEvent::Started {
                app_key: TEAMS.into(),
                label: "Microsoft Teams".into(),
                class: "meeting_app",
            }]
        );
    }

    #[test]
    fn a_shorter_session_stays_quiet_until_it_lasts_five_seconds() {
        let mut d = Detector::new();
        let s = setup(DetectMode::MeetingApps);
        let start = unix_ms_to_filetime(NOW);
        let snap = [MicUsage {
            app_key: TEAMS.into(),
            exe_path: None,
            last_start: start,
            active: true,
        }];
        assert!(step(&mut d, NOW + 2_000, &snap, &s).is_empty());
        assert!(step(&mut d, NOW + 4_900, &snap, &s).is_empty());
        assert_eq!(started_keys(&step(&mut d, NOW + 5_000, &snap, &s)), [TEAMS]);
    }

    #[test]
    fn a_short_mic_test_never_gets_a_notice() {
        let mut d = Detector::new();
        let s = setup(DetectMode::MeetingApps);
        let start = unix_ms_to_filetime(NOW);
        let running = [MicUsage {
            app_key: TEAMS.into(),
            exe_path: None,
            last_start: start,
            active: true,
        }];
        assert!(step(&mut d, NOW + 3_000, &running, &s).is_empty());
        let done = [idle(running[0].clone())];
        assert!(step(&mut d, NOW + 4_000, &done, &s).is_empty());
        assert!(step(&mut d, NOW + 9_000, &done, &s).is_empty());
    }

    #[test]
    fn the_same_session_is_announced_only_once() {
        let mut d = Detector::new();
        let s = setup(DetectMode::MeetingApps);
        let snap = [usage(TEAMS, 10_000)];
        assert_eq!(started_keys(&step(&mut d, NOW, &snap, &s)), [TEAMS]);
        assert!(step(&mut d, NOW + 2_000, &snap, &s).is_empty());
        assert!(step(&mut d, NOW + 4_000, &snap, &s).is_empty());
    }

    #[test]
    fn a_new_session_of_the_same_app_gets_a_new_notice() {
        let mut d = Detector::new();
        let s = setup(DetectMode::MeetingApps);
        let first = usage(TEAMS, 10_000);
        assert_eq!(
            started_keys(&step(&mut d, NOW, &[first.clone()], &s)),
            [TEAMS]
        );
        // Mikrofon frei: Hinweis schliesst.
        let ended = step(&mut d, NOW + 2_000, &[idle(first)], &s);
        assert_eq!(
            ended,
            vec![DetectEvent::Ended {
                app_key: TEAMS.into()
            }]
        );
        // Spaeter ein neuer Anruf (neuer Start).
        let later = NOW + 600_000;
        let mut second = usage(TEAMS, 0);
        second.last_start = unix_ms_to_filetime(later);
        assert!(step(&mut d, later + 1_000, &[second.clone()], &s).is_empty());
        assert_eq!(
            started_keys(&step(&mut d, later + 6_000, &[second], &s)),
            [TEAMS]
        );
    }

    #[test]
    fn the_end_of_the_session_closes_the_open_notice() {
        let mut d = Detector::new();
        let s = setup(DetectMode::MeetingApps);
        let u = usage(TEAMS, 10_000);
        step(&mut d, NOW, &[u.clone()], &s);
        assert_eq!(d.open_app(), Some(TEAMS));
        let ev = step(&mut d, NOW + 2_000, &[idle(u)], &s);
        assert_eq!(
            ev,
            vec![DetectEvent::Ended {
                app_key: TEAMS.into()
            }]
        );
        assert_eq!(d.open_app(), None);
        // Ein zweites Mal schliesst nichts mehr.
        assert!(step(&mut d, NOW + 4_000, &[], &s).is_empty());
    }

    #[test]
    fn the_own_app_is_never_reported_even_in_all_apps_mode() {
        let mut d = Detector::new();
        let s = setup(DetectMode::AllApps);
        assert!(step(&mut d, NOW, &[usage(OWN, 60_000)], &s).is_empty());
        // Auch ein Test-Executable und der Schluessel aus `current_exe`.
        let lib = "np:C:#x#target#debug#deps#local_voice_ai_lib-f4e6.exe";
        assert!(step(&mut d, NOW, &[usage(lib, 60_000)], &s).is_empty());
        let process_alive = |_: &Path| true;
        let selfs = vec!["np:D:#Portable#meine-app.exe".to_string()];
        let ctx = DetectCtx {
            mode: DetectMode::AllApps,
            recording: false,
            ignored: &[],
            self_keys: &selfs,
            process_alive: &process_alive,
        };
        let mine = usage("np:D:#Portable#Meine-App.exe", 60_000);
        assert!(d.step(NOW, &[mine], &ctx).is_empty());
    }

    #[test]
    fn webview2_is_ignored_in_every_mode() {
        for mode in [DetectMode::MeetingApps, DetectMode::AllApps] {
            let mut d = Detector::new();
            assert!(step(&mut d, NOW, &[usage(WEBVIEW, 60_000)], &setup(mode)).is_empty());
        }
    }

    #[test]
    fn a_dead_process_with_a_stuck_entry_is_ignored() {
        let mut d = Detector::new();
        let mut s = setup(DetectMode::MeetingApps);
        s.alive = false;
        assert!(step(&mut d, NOW, &[usage(CHROME, 60_000)], &s).is_empty());
        s.alive = true;
        assert_eq!(
            started_keys(&step(&mut d, NOW + 2_000, &[usage(CHROME, 62_000)], &s)),
            [CHROME]
        );
    }

    #[test]
    fn a_store_app_is_not_checked_against_the_process_list() {
        let mut d = Detector::new();
        let mut s = setup(DetectMode::MeetingApps);
        s.alive = false;
        assert_eq!(
            started_keys(&step(&mut d, NOW, &[usage(TEAMS, 60_000)], &s)),
            [TEAMS]
        );
    }

    #[test]
    fn browsers_carry_the_call_label() {
        let mut d = Detector::new();
        let s = setup(DetectMode::MeetingApps);
        let ev = step(&mut d, NOW, &[usage(CHROME, 10_000)], &s);
        assert_eq!(
            ev,
            vec![DetectEvent::Started {
                app_key: CHROME.into(),
                label: "Browser-Call (z. B. Google Meet)".into(),
                class: "browser",
            }]
        );
    }

    #[test]
    fn unknown_apps_need_all_apps_mode() {
        let mut d = Detector::new();
        assert!(step(
            &mut d,
            NOW,
            &[usage(PYTHON, 10_000)],
            &setup(DetectMode::MeetingApps)
        )
        .is_empty());
        let ev = step(
            &mut d,
            NOW,
            &[usage(PYTHON, 10_000)],
            &setup(DetectMode::AllApps),
        );
        assert_eq!(
            ev,
            vec![DetectEvent::Started {
                app_key: PYTHON.into(),
                label: "python.exe".into(),
                class: "other",
            }]
        );
    }

    #[test]
    fn mode_off_reports_nothing_and_closes_an_open_notice() {
        let mut d = Detector::new();
        let mut s = setup(DetectMode::MeetingApps);
        let snap = [usage(TEAMS, 10_000)];
        step(&mut d, NOW, &snap, &s);
        s.mode = DetectMode::Off;
        let ev = step(&mut d, NOW + 2_000, &snap, &s);
        assert_eq!(
            ev,
            vec![DetectEvent::Ended {
                app_key: TEAMS.into()
            }]
        );
        assert!(step(&mut d, NOW + 4_000, &snap, &s).is_empty());
    }

    #[test]
    fn the_ignore_list_matches_file_name_label_and_key_part_case_insensitively() {
        for entry in ["CHROME.EXE", "browser-call", "google#chrome", "  chrome  "] {
            let mut d = Detector::new();
            let mut s = setup(DetectMode::MeetingApps);
            s.ignored = vec![entry.to_string(), String::new()];
            assert!(
                step(&mut d, NOW, &[usage(CHROME, 10_000)], &s).is_empty(),
                "{entry}"
            );
        }
        // Ein Eintrag, der nichts trifft, aendert nichts; eine leere Zeile ignoriert nicht alles.
        let mut d = Detector::new();
        let mut s = setup(DetectMode::MeetingApps);
        s.ignored = vec!["zoom".into(), " ".into()];
        assert_eq!(
            started_keys(&step(&mut d, NOW, &[usage(CHROME, 10_000)], &s)),
            [CHROME]
        );
    }

    #[test]
    fn a_running_recording_suppresses_the_notice_for_that_session() {
        let mut d = Detector::new();
        let mut s = setup(DetectMode::MeetingApps);
        s.recording = true;
        let snap = [usage(TEAMS, 10_000)];
        assert!(step(&mut d, NOW, &snap, &s).is_empty());
        // Nach dem Stopp bleibt dieselbe Sitzung stumm ...
        s.recording = false;
        assert!(step(&mut d, NOW + 2_000, &snap, &s).is_empty());
        // ... ein spaeterer Anruf meldet sich wieder.
        let later = NOW + 900_000;
        let mut next = usage(TEAMS, 0);
        next.last_start = unix_ms_to_filetime(later - 10_000);
        assert_eq!(started_keys(&step(&mut d, later, &[next], &s)), [TEAMS]);
    }

    #[test]
    fn starting_a_recording_closes_an_open_notice() {
        let mut d = Detector::new();
        let mut s = setup(DetectMode::MeetingApps);
        let snap = [usage(TEAMS, 10_000)];
        step(&mut d, NOW, &snap, &s);
        s.recording = true;
        let ev = step(&mut d, NOW + 2_000, &snap, &s);
        assert_eq!(
            ev,
            vec![DetectEvent::Ended {
                app_key: TEAMS.into()
            }]
        );
    }

    #[test]
    fn several_apps_give_one_notice_with_meeting_app_before_browser_before_rest() {
        let mut d = Detector::new();
        let s = setup(DetectMode::AllApps);
        let (python, chrome, teams) = (
            usage(PYTHON, 40_000),
            usage(CHROME, 30_000),
            usage(TEAMS, 10_000),
        );
        let snap = [python.clone(), chrome.clone(), teams.clone()];
        assert_eq!(started_keys(&step(&mut d, NOW, &snap, &s)), [TEAMS]);
        // Die uebrigen sind abgedeckt: auch nach dem Ende von Teams kein zweiter Hinweis.
        let after = [python, chrome, idle(teams)];
        let ev = step(&mut d, NOW + 2_000, &after, &s);
        assert_eq!(
            ev,
            vec![DetectEvent::Ended {
                app_key: TEAMS.into()
            }]
        );
    }

    #[test]
    fn with_equal_rank_the_older_session_wins() {
        let mut d = Detector::new();
        let s = setup(DetectMode::MeetingApps);
        let zoom = "np:C:#Users#x#AppData#Roaming#Zoom#bin#Zoom.exe";
        let snap = [usage(TEAMS, 10_000), usage(zoom, 20_000)];
        assert_eq!(started_keys(&step(&mut d, NOW, &snap, &s)), [zoom]);
    }

    #[test]
    fn a_second_app_waits_while_a_notice_is_open_and_follows_when_it_closes() {
        let mut d = Detector::new();
        let s = setup(DetectMode::MeetingApps);
        let teams = usage(TEAMS, 10_000);
        assert_eq!(
            started_keys(&step(&mut d, NOW, &[teams.clone()], &s)),
            [TEAMS]
        );
        let chrome = usage(CHROME, 10_000);
        assert!(step(&mut d, NOW + 2_000, &[teams.clone(), chrome.clone()], &s).is_empty());
        let ev = step(&mut d, NOW + 4_000, &[idle(teams), chrome], &s);
        assert_eq!(
            ev.iter()
                .map(|e| matches!(e, DetectEvent::Ended { .. }))
                .collect::<Vec<_>>(),
            vec![true, false]
        );
        assert_eq!(started_keys(&ev), [CHROME]);
    }
}
