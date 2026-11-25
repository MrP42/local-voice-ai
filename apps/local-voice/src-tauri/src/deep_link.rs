// TF3 (lva#845): ausschliesslich Navigation, niemals eine Fachaktion.
use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Clone, PartialEq, Eq, serde::Serialize)]
struct Target {
    ziel: &'static str,
    id: String,
}

// Kein URL-Normalisieren oder Dekodieren: Query, Fragmente, Escapes und fremde
// Pfade sind keine alternativen Schreibweisen eines erlaubten Ziels.
fn parse(raw: &str) -> Option<Target> {
    const PREFIX: &str = "localvoice://terminfindung/";
    if raw.len() != PREFIX.len() + 6 {
        return None;
    }
    let (scheme, rest) = raw.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("localvoice") {
        return None;
    }
    let id = rest.strip_prefix("terminfindung/")?;
    if !id.starts_with("T-")
        || !id.as_bytes()[2..]
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return None;
    }
    Some(Target {
        ziel: "terminfindung",
        id: id.to_owned(),
    })
}

/// URL-Starts umgehen Clap; dessen Fehlerausgabe wuerde das Argument verraten.
/// Optionen mit URL-Werten (z.B. --calendar-dump) bleiben normale CLI-Aufrufe.
pub fn is_url_launch(args: &[String]) -> bool {
    args.first().is_some_and(|arg| {
        arg.contains("://") || arg.get(..10).is_some_and(|s| s.eq_ignore_ascii_case("localvoice"))
    })
}

#[derive(Default)]
struct Inbox {
    ready: bool,
    last: Option<Target>,
    ignored: u64,
}

impl Inbox {
    fn accept_args(&mut self, args: &[String]) -> bool {
        // Das Windows-Single-Instance-Plugin trennt mit '|'. Mehrere Teile
        // duerfen deshalb niemals als gueltiger Praefix einer URL gelten.
        if args.len() != 1 {
            self.ignored = self.ignored.saturating_add(1);
            return false;
        }
        self.accept(&args[0])
    }

    fn accept(&mut self, raw: &str) -> bool {
        if let Some(target) = parse(raw) {
            self.last = Some(target);
            true
        } else {
            self.ignored = self.ignored.saturating_add(1);
            false
        }
    }

    fn flush(&mut self, send: impl FnOnce(&Target) -> bool) {
        if self.ready && self.last.as_ref().is_some_and(send) {
            self.last = None;
        }
    }
}

#[derive(Default)]
pub(crate) struct State(Mutex<Inbox>);

fn flush(app: &tauri::AppHandle, inbox: &mut Inbox) {
    // Unter demselben Lock wie accept/ready: ein neueres Ziel kann kein altes
    // ueberholen. Nur das Hauptfenster bekommt die ID, niemals Overlay/Logs.
    inbox.flush(|target| {
        app.get_webview_window("main")
            .is_some_and(|window| window.emit("deep-link-open", target).is_ok())
    });
}

pub(crate) fn receive(app: &tauri::AppHandle, raw: &str) {
    let state = app.state::<State>();
    let Ok(mut inbox) = state.0.lock() else {
        return;
    };
    if inbox.accept(raw) {
        flush(app, &mut inbox);
        drop(inbox);
        crate::show_main_window(app);
    } else {
        log::debug!("deep-link ignored count={}", inbox.ignored);
    }
}

pub(crate) fn receive_args(
    app: &tauri::AppHandle,
    args: impl IntoIterator<Item = impl AsRef<str>>,
) {
    let args: Vec<String> = args.into_iter().map(|arg| arg.as_ref().to_owned()).collect();
    let state = app.state::<State>();
    let Ok(mut inbox) = state.0.lock() else {
        return;
    };
    if inbox.accept_args(&args) {
        flush(app, &mut inbox);
        drop(inbox);
        crate::show_main_window(app);
    } else {
        log::debug!("deep-link ignored count={}", inbox.ignored);
    }
}

pub(crate) fn page_started(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload<'_>) {
    if webview.label() == "main" && payload.event() == tauri::webview::PageLoadEvent::Started {
        let state = webview.state::<State>();
        if let Ok(mut inbox) = state.0.lock() {
            inbox.ready = false;
        };
    }
}

#[tauri::command]
#[specta::specta]
pub(crate) fn deep_link_ready(window: tauri::WebviewWindow) {
    if window.label() != "main" {
        return;
    }
    let app = window.app_handle();
    let state = app.state::<State>();
    let Ok(mut inbox) = state.0.lock() else {
        return;
    };
    inbox.ready = true;
    flush(app, &mut inbox);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_accepts_only_the_contract() {
        for raw in ["localvoice://terminfindung/T-TEST", "LOCALVOICE://terminfindung/T-AB12"] {
            let target = parse(raw).expect("synthetic valid target");
            assert_eq!(target.ziel, "terminfindung");
            assert_eq!(target.id, raw.rsplit('/').next().unwrap());
        }
        for raw in [
            "", "https://terminfindung/T-TEST", "localvoice://Terminfindung/T-TEST",
            "localvoice://terminfindung/t-TEST", "localvoice://terminfindung/T-test",
            "localvoice://terminfindung/T-ABC", "localvoice://terminfindung/T-ABCDE",
            "localvoice://anders/T-TEST", "localvoice://terminfindung/T-TEST/",
            "localvoice://terminfindung/T-TEST?a=1", "localvoice://terminfindung/T-TEST#x",
            "localvoice://terminfindung/../T-TEST", "localvoice://terminfindung/x/../T-TEST",
            "localvoice://terminfindung/%54-TEST", "localvoice://terminfindung/T-%54EST",
            "localvoice://terminfindung/T-TE%2f", "localvoice://terminfindung/T-ＴEST",
            "localvoice://terminfindung/T-TE\0T", "localvoice://terminfindung/T-TEST\n",
            "localvoice://user@terminfindung/T-TEST", "localvoice://terminfindung:80/T-TEST",
            " localvoice://terminfindung/T-TEST", "localvoice:\\terminfindung\\T-TEST",
        ] {
            assert!(parse(raw).is_none(), "invalid synthetic target accepted");
        }
        assert!(parse(&"x".repeat(100_000)).is_none());
    }

    #[test]
    fn only_last_valid_target_waits_for_ready() {
        let mut inbox = Inbox::default();
        assert!(inbox.accept("localvoice://terminfindung/T-AAAA"));
        assert!(inbox.accept("localvoice://terminfindung/T-BBBB"));
        assert!(!inbox.accept("localvoice://terminfindung/T-BBBB?x"));
        assert_eq!(inbox.ignored, 1);
        inbox.flush(|_| panic!("frontend is not ready"));
        inbox.ready = true;
        inbox.flush(|target| {
            assert_eq!(target.id, "T-BBBB");
            true
        });
        inbox.flush(|_| panic!("target delivered twice"));
        assert!(inbox.last.is_none());
    }

    #[test]
    fn failed_delivery_keeps_one_target_for_retry() {
        let mut inbox = Inbox { ready: true, ..Inbox::default() };
        inbox.accept("localvoice://terminfindung/T-TEST");
        inbox.flush(|_| false);
        assert!(inbox.last.is_some());
        inbox.flush(|_| true);
        assert!(inbox.last.is_none());
    }

    #[test]
    fn url_launch_cannot_trigger_cli_actions_or_print_clap_errors() {
        assert!(is_url_launch(&["localvoice://invalid".into(), "--toggle-transcription".into()]));
        assert!(is_url_launch(&["https://invalid".into()]));
        assert!(is_url_launch(&["LOCALVOICE:invalid".into()]));
        assert!(!is_url_launch(&["--calendar-dump".into(), "https://invalid".into()]));
        assert!(!is_url_launch(&[]));
    }

    #[test]
    fn pipe_split_and_extra_cli_arguments_are_rejected_as_a_whole() {
        let mut inbox = Inbox::default();
        assert!(!inbox.accept_args(&["localvoice://terminfindung/T-TEST".into(), "suffix".into()]));
        assert!(inbox.last.is_none());
        assert_eq!(inbox.ignored, 1);
        assert!(inbox.accept_args(&["localvoice://terminfindung/T-TEST".into()]));
    }
}
