//! Tests des CLI-Rohtransports (lva#604, Teilpaket 3): Fake-CLIs, kein echter Aufruf.
//!
//! Diese Tests standen bis Teilpaket 3 in `managers::llm::cli` und riefen `call`/
//! `call_with_options` auf. Mit den Einstiegen sind sie hierher gewandert; Inhalt unverändert.

#![allow(unused_imports)]

use super::*;
use crate::managers::llm::cli::tests::{stream_with, GOOD_CLAUDE};
use crate::managers::llm::cli::{find_node, locate};
use crate::managers::llm::preview::{
    self, binding_for, fixtures, PreviewCheck, PreviewSwitch, Tri,
};
use std::path::PathBuf;

/// Gegen die echten CLIs mit dem Login des Nutzers (nur von Hand):
/// `cargo test --lib real_cli_calls -- --ignored --nocapture`
#[test]
#[ignore]
fn real_cli_calls() {
    for (cli, model) in [(Cli::Claude, "sonnet@low"), (Cli::Codex, "gpt-6-astra@low")] {
        let bin = locate(cli).expect("CLI installiert");
        let t = std::time::Instant::now();
        let r = call(
                cli,
                &bin,
                model,
                Some("Du schreibst knappe deutsche Protokolle. Antworte nur mit dem Ergebnis."),
                "Fasse in einem Satz zusammen: Das Release wird auf Freitag verschoben, Anna repariert die Tests.",
            );
        println!("{cli:?} {} in {:?}: {r:?}", bin.display(), t.elapsed());
        let r = r.expect("Antwort");
        assert!(r.text.contains("Freitag"), "{}", r.text);
        assert!(r.prompt_tokens > 0);
    }
}

/// Sicherheitsbefund 06.10.2026: kein Start ueber `cmd /C`, keine fremden
/// Modellnamen auf der Kommandozeile.
#[test]
fn never_launches_through_cmd_and_rejects_unknown_models() {
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("codex.cmd");
    std::fs::write(&shim, b"@echo off").unwrap();
    let (program, args) = launcher(&shim);
    assert_ne!(program.file_name().and_then(|n| n.to_str()), Some("cmd"));
    assert!(!args.iter().any(|a| a.eq_ignore_ascii_case("/C")));

    // Mit codex.js daneben: node + Skript (sofern Node im PATH liegt).
    let js = dir.path().join("node_modules/@openai/codex/bin");
    std::fs::create_dir_all(&js).unwrap();
    std::fs::write(js.join("codex.js"), b"").unwrap();
    let (program, args) = launcher(&shim);
    if find_node().is_some() {
        assert!(
            program
                .to_string_lossy()
                .to_lowercase()
                .ends_with("node.exe")
                || program.ends_with("node")
        );
        assert!(args[0].ends_with("codex.js"));
    }

    let err = call(Cli::Codex, &shim, "gpt-6 & calc", None, "x").unwrap_err();
    assert!(err.starts_with("cli_model_not_in_plan"), "{err}");
}

#[test]
fn names_and_efforts_are_checked_before_the_command_line() {
    for ok in ["gpt-6.1-sol", "claude-haiku-4-5-20251001", "fable"] {
        assert!(valid_model_name(ok), "{ok}");
    }
    for bad in ["", "gpt-6 & calc", "a\"b", "x;y", "../x", &"a".repeat(65)] {
        assert!(!valid_model_name(bad), "{bad}");
    }
    assert!(valid_effort("ultra") && !valid_effort("high; rm"));
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("claude.exe");
    std::fs::write(&shim, b"").unwrap();
    let err = call(Cli::Claude, &shim, "opus@turbo", None, "x").unwrap_err();
    assert!(err.contains("Effort"), "{err}");
}

/// Alles, was ein Test zur Fake-CLI braucht.
#[cfg(unix)]
struct Fake {
    dir: tempfile::TempDir,
    bin: PathBuf,
}

#[cfg(unix)]
impl Fake {
    /// Fake-Programm: antwortet auf `--version`, `exec --help` und
    /// `mcp list`; sonst protokolliert es Argumente, Umgebung, Arbeitsordner
    /// und `CODEX_HOME` in `log.txt`, wartet `sleep` Sekunden, liest stdin,
    /// legt `finished` an und gibt `reply.txt` aus (Exit-Code aus `exit`).
    fn new(reply: &str, help: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        preview::TEST_ALLOW_SCRIPT.with(|f| f.set(true));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("reply.txt"), reply).unwrap();
        std::fs::write(dir.path().join("help.txt"), help).unwrap();
        let bin = dir.path().join("fakecli");
        // Der Ordner steht fest im Skript: die Vorschau startet eine KOPIE
        // an anderer Stelle, die ihre Dateien trotzdem hier findet.
        let body = r#"#!/bin/sh
D="@@DIR@@"
case "$1" in --version) echo "9.9.9 (Fake)"; exit 0 ;; esac
case "$1 $2" in
  "exec --help") cat "$D/help.txt"; exit 0 ;;
  "mcp list") echo "[]"; exit 0 ;;
esac
{
  for a in "$@"; do printf 'ARG:%s\n' "$a"; done
  printf 'SELF:%s\n' "$0"
  printf 'ENV_MDS:%s\n' "$CLAUDE_CODE_DISABLE_CLAUDE_MDS"
  printf 'ENV_MEM:%s\n' "$CLAUDE_CODE_DISABLE_AUTO_MEMORY"
  printf 'PWD:%s\n' "$(pwd)"
  printf 'FILES:%s\n' "$(ls -A | wc -l | tr -d ' ')"
  env | cut -d= -f1 | sort | while read -r n; do printf 'VAR:%s\n' "$n"; done
} > "$D/log.txt"
[ -f "$D/spawn_child" ] && { sleep 60 & echo $! > "$D/child.pid"; }
[ -f "$D/sleep" ] && sleep "$(cat "$D/sleep")"
if [ -f "$D/capture_stdin" ]; then cat > "$D/stdin.txt"; else cat > /dev/null; fi
touch "$D/finished"
cat "$D/reply.txt"
exit "$(cat "$D/exit" 2>/dev/null || echo 0)"
"#;
        std::fs::write(&bin, body.replace("@@DIR@@", &dir.path().to_string_lossy())).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir, bin }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("log.txt")).unwrap_or_default()
    }

    fn started(&self) -> bool {
        self.dir.path().join("log.txt").exists()
    }

    fn set(&self, name: &str, content: &str) {
        std::fs::write(self.dir.path().join(name), content).unwrap();
    }

    fn binding(&self, cli: Cli, model: &str) -> preview::PreviewBinding {
        binding_for(cli, model, &preview::identity(&self.bin).unwrap())
    }

    fn check(&self, cli: Cli, model: &str, reported: &str, context: u32) -> PreviewCheck {
        fixtures::check(self.binding(cli, model), reported, context)
    }

    fn call(
        &self,
        cli: Cli,
        model: &str,
        check: PreviewCheck,
        switch: Option<PreviewSwitch>,
    ) -> Result<CliReply, String> {
        call_with_options(
            cli,
            &self.bin,
            model,
            Some("Systemtext"),
            "user: Frage",
            Duration::from_secs(30),
            &CallOptions {
                max_tokens: Some(600),
                preview: Some(check),
                preview_switch: switch,
                ..CallOptions::default()
            },
        )
    }
}

fn on() -> Option<PreviewSwitch> {
    Some(PreviewSwitch::new(|| true))
}

#[cfg(unix)]
fn claude_fake(reply: &str) -> Fake {
    Fake::new(reply, "")
}

#[cfg(unix)]
fn claude_call(f: &Fake, reply_model: &str) -> Result<CliReply, String> {
    f.call(
        Cli::Claude,
        "sonnet",
        f.check(Cli::Claude, "sonnet", reply_model, 200_000),
        on(),
    )
}

#[cfg(unix)]
#[test]
fn the_claude_preview_streams_hardens_and_accepts_a_clean_reply() {
    let f = claude_fake(GOOD_CLAUDE);
    let r = claude_call(&f, "claude-sonnet-5-5").expect("saubere Antwort");
    assert_eq!(r.text, "Antwort");
    // Claude: input + cache_creation + cache_read.
    assert_eq!(r.prompt_tokens, 33_000);
    assert_eq!((r.compacted, r.truncated), (Tri::No, Tri::No));
    let log = f.log();
    for want in [
        "ARG:--output-format\nARG:stream-json",
        "ARG:--verbose",
        "ARG:--tools\nARG:\n",
        "ARG:--no-session-persistence",
        "ARG:--setting-sources\nARG:\n",
        "ARG:--strict-mcp-config",
        r#"ARG:{"mcpServers":{}}"#,
        "ARG:--disable-slash-commands",
        "ARG:--settings\nARG:{\"disableAllHooks\":true}",
        "ENV_MDS:1",
        "ENV_MEM:1",
        "FILES:0",
    ] {
        assert!(log.contains(want), "{want:?} fehlt:\n{log}");
    }
    assert!(
        !log.contains("ARG:json"),
        "kein Ergebnisobjekt in der Vorschau"
    );
    assert!(!log.contains("ARG:--bare"), "kein --bare (OAuth-Anmeldung)");
    // Gestartet wurde das installierte Original (keine Kopie, kein Cache).
    let me = log.lines().find_map(|l| l.strip_prefix("SELF:")).unwrap();
    assert_eq!(Path::new(me), std::fs::canonicalize(&f.bin).unwrap());
    let pwd = log.lines().find_map(|l| l.strip_prefix("PWD:")).unwrap();
    assert!(!Path::new(pwd).starts_with(f.dir.path()), "frischer Ordner");
    assert!(!Path::new(pwd).exists(), "Arbeitsordner danach geloescht");
}

#[cfg(unix)]
#[test]
fn other_callers_keep_the_old_isolation_and_json_output() {
    let f = claude_fake(
        r#"{"type":"result","is_error":false,"result":"A","usage":{"input_tokens":5,"output_tokens":3}}"#,
    );
    call_with_options(
        Cli::Claude,
        &f.bin,
        "sonnet",
        Some("S"),
        "u",
        Duration::from_secs(30),
        &CallOptions::default(),
    )
    .unwrap();
    let log = f.log();
    assert!(!log.contains("ARG:--settings\n"), "{log}");
    assert!(log.contains("ARG:--output-format\nARG:json"), "{log}");
    assert!(!log.contains("ENV_MDS:1"), "{log}");
}

#[cfg(unix)]
#[test]
fn a_cancel_set_before_the_preview_call_sends_nothing_to_stdin() {
    let f = claude_fake(GOOD_CLAUDE);
    f.set("capture_stdin", "1");
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let err = call_with_options(
        Cli::Claude,
        &f.bin,
        "sonnet",
        Some("Systemtext"),
        "user: VERTRAULICHE Frage",
        Duration::from_secs(30),
        &CallOptions {
            max_tokens: Some(600),
            preview: Some(f.check(Cli::Claude, "sonnet", "claude-sonnet-5-5", 200_000)),
            preview_switch: on(),
            cancel: Some(cancel),
            ..CallOptions::default()
        },
    )
    .unwrap_err();
    assert!(err.contains("cancel"), "{err}");
    std::thread::sleep(Duration::from_millis(300));
    let got = std::fs::read(f.dir.path().join("stdin.txt")).unwrap_or_default();
    assert!(
        got.is_empty(),
        "stdin blieb nicht leer: {} Bytes",
        got.len()
    );
}

#[cfg(unix)]
#[test]
fn a_claude_preview_reply_is_dropped_unless_everything_checks_out() {
    let compact = format!(
        "{}\n{GOOD_CLAUDE}",
        r#"{"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"auto","pre_tokens":150000}}"#
    );
    let metadata_only = format!("{}\n{GOOD_CLAUDE}", r#"{"type":"x","compact_metadata":{}}"#);
    let cases: Vec<(&str, String, &str)> = vec![
            (
                "compact_boundary",
                compact,
                "cli_preview_rejected:budget:compaction",
            ),
            (
                "compact_metadata",
                metadata_only,
                "cli_preview_rejected:budget:compaction",
            ),
            (
                "max_tokens im Ergebnis",
                stream_with(&[(
                    r#""stop_reason":"end_turn","usage""#,
                    r#""stop_reason":"max_tokens","usage""#,
                )]),
                "cli_preview_rejected:budget:truncated",
            ),
            (
                "max_tokens in der Nachricht",
                stream_with(&[(
                    r#""stop_reason":"end_turn","content""#,
                    r#""stop_reason":"max_tokens","content""#,
                )]),
                "cli_preview_rejected:budget:truncated",
            ),
            (
                "max_tokens in einer frueheren von zwei Nachrichten, Ergebnis end_turn",
                {
                    let lines: Vec<&str> = GOOD_CLAUDE.lines().collect();
                    let cut = lines[1].replace(
                        r#""stop_reason":"end_turn""#,
                        r#""stop_reason":"max_tokens""#,
                    );
                    format!("{}\n{cut}\n{}\n{}\n", lines[0], lines[1], lines[2])
                },
                "cli_preview_rejected:budget:truncated",
            ),
            (
                "max_tokens an einem Nutzer-Ereignis, Ergebnis end_turn",
                {
                    let lines: Vec<&str> = GOOD_CLAUDE.lines().collect();
                    format!(
                        "{}\n{}\n{}\n{}\n",
                        lines[0],
                        r#"{"type":"user","message":{"stop_reason":"max_tokens","content":"Frage"}}"#,
                        lines[1],
                        lines[2]
                    )
                },
                "cli_preview_rejected:budget:truncated",
            ),
            (
                "aussen end_turn, in der Nachricht max_tokens (keine Prioritaet)",
                stream_with(&[(
                    r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn""#,
                    r#"{"type":"assistant","stop_reason":"end_turn","message":{"role":"assistant","stop_reason":"max_tokens""#,
                )]),
                "cli_preview_rejected:budget:truncated",
            ),
            (
                "unbekannter stop_reason in der Nachricht",
                stream_with(&[(
                    r#""role":"assistant","stop_reason":"end_turn""#,
                    r#""role":"assistant","stop_reason":"refusal""#,
                )]),
                "cli_preview_rejected:budget:truncated",
            ),
            (
                "aborted",
                stream_with(&[(
                    r#""stop_reason":"end_turn","usage""#,
                    r#""stop_reason":"aborted","usage""#,
                )]),
                "cli_preview_rejected:budget:truncated",
            ),
            (
                "Cache-Feld als String",
                stream_with(&[(
                    r#""cache_creation_input_tokens":20000"#,
                    r#""cache_creation_input_tokens":"20000""#,
                )]),
                "cli_preview_rejected:llm_failed:usage_invalid",
            ),
            (
                "Cache-Feld negativ",
                stream_with(&[(
                    r#""cache_read_input_tokens":10000"#,
                    r#""cache_read_input_tokens":-1"#,
                )]),
                "cli_preview_rejected:llm_failed:usage_invalid",
            ),
            (
                "Cache-Feld als Kommazahl",
                stream_with(&[(
                    r#""cache_read_input_tokens":10000"#,
                    r#""cache_read_input_tokens":1.5"#,
                )]),
                "cli_preview_rejected:llm_failed:usage_invalid",
            ),
            (
                "Cache-Feld null",
                stream_with(&[(
                    r#""cache_creation_input_tokens":20000"#,
                    r#""cache_creation_input_tokens":null"#,
                )]),
                "cli_preview_rejected:llm_failed:usage_invalid",
            ),
            (
                "Cache-Feld ueber 2^53-1",
                stream_with(&[(
                    r#""cache_read_input_tokens":10000"#,
                    r#""cache_read_input_tokens":9007199254740992"#,
                )]),
                "cli_preview_rejected:llm_failed:usage_invalid",
            ),
            (
                "Summe ueber 2^53-1 (Ueberlauf)",
                stream_with(&[
                    (r#""input_tokens":3000"#, r#""input_tokens":9007199254740991"#),
                    (
                        r#""cache_read_input_tokens":10000"#,
                        r#""cache_read_input_tokens":9007199254740991"#,
                    ),
                ]),
                "cli_preview_rejected:llm_failed:usage_invalid",
            ),
            (
                "Summe ueber u64 (checked_add)",
                stream_with(&[
                    (r#""input_tokens":3000"#, r#""input_tokens":18446744073709551615"#),
                    (r#""cache_read_input_tokens":10000"#, r#""cache_read_input_tokens":1"#),
                ]),
                "cli_preview_rejected:llm_failed:usage_invalid",
            ),
            (
                "ohne stop_reason: unbekannt sperrt",
                stream_with(&[(r#""stop_reason":"end_turn","usage""#, r#""usage""#)]),
                "cli_preview_rejected:budget:truncated_unknown",
            ),
            (
                "ohne Nutzungsangabe",
                stream_with(&[(r#""output_tokens":200}"#, r#""output_tokens":0}"#)]),
                "cli_preview_rejected:llm_failed:usage_missing",
            ),
            (
                "ohne Modell",
                stream_with(&[
                    (
                        r#","modelUsage":{"claude-sonnet-5-5":{"outputTokens":200}}"#,
                        "",
                    ),
                    (r#""model":"claude-sonnet-5-5","tools""#, r#""tools""#),
                ]),
                "cli_preview_rejected:llm_failed:model_missing",
            ),
            (
                "Modellwechsel in der Nachricht",
                stream_with(&[(
                    r#""role":"assistant","stop_reason""#,
                    r#""role":"assistant","model":"claude-haiku-4-5-20251001","stop_reason""#,
                )]),
                "cli_preview_rejected:llm_failed:model_mismatch",
            ),
            (
                "Modellwechsel im Init",
                stream_with(&[(
                    r#""subtype":"init","model":"claude-sonnet-5-5""#,
                    r#""subtype":"init","model":"claude-opus-5-5""#,
                )]),
                "cli_preview_rejected:llm_failed:model_mismatch",
            ),
            (
                "zwei Modelle in modelUsage (kein meiste Token)",
                stream_with(&[(
                    r#""modelUsage":{"claude-sonnet-5-5":{"outputTokens":200}}"#,
                    r#""modelUsage":{"claude-sonnet-5-5":{"outputTokens":200},"claude-haiku-4-5-20251001":{"outputTokens":1}}"#,
                )]),
                "cli_preview_rejected:llm_failed:model_mismatch",
            ),
            (
                "unbekanntes Ereignis",
                stream_with(&[(
                    r#"{"type":"assistant","message""#,
                    "{\"type\":\"stream_event\"}\n{\"type\":\"assistant\",\"message\"",
                )]),
                "cli_preview_rejected:llm_failed:event_unknown",
            ),
            (
                "unbekannter Untertyp",
                stream_with(&[(
                    r#"{"type":"assistant","message""#,
                    "{\"type\":\"system\",\"subtype\":\"hook_started\"}\n{\"type\":\"assistant\",\"message\"",
                )]),
                "cli_preview_rejected:llm_failed:event_unknown",
            ),
            (
                "Fehlerereignis vor dem Erfolg",
                stream_with(&[(
                    r#"{"type":"assistant","message""#,
                    r#"{"type":"assistant","error":"rate_limit","message""#,
                )]),
                "cli_preview_rejected:llm_failed:error_event",
            ),
            (
                "is_error am Ereignis vor dem Erfolg",
                stream_with(&[(
                    r#"{"type":"assistant","message""#,
                    r#"{"type":"assistant","is_error":true,"message""#,
                )]),
                "cli_preview_rejected:llm_failed:error_event",
            ),
            (
                "tool_use",
                stream_with(&[(
                    r#""content":[{"type":"text","text":"Antwort"}]"#,
                    r#""content":[{"type":"tool_use","name":"Read","input":{}}]"#,
                )]),
                "cli_preview_rejected:llm_failed:tool_use",
            ),
            (
                "tool_result",
                format!(
                    "{}\n{}",
                    GOOD_CLAUDE.lines().take(2).collect::<Vec<_>>().join("\n"),
                    r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"x"}]}}"#
                ) + "\n"
                    + GOOD_CLAUDE.lines().nth(2).unwrap(),
                "cli_preview_rejected:llm_failed:tool_use",
            ),
            (
                "Ereignis nach dem Ergebnis",
                format!(
                    "{GOOD_CLAUDE}{}\n",
                    r#"{"type":"assistant","message":{"content":[{"type":"text","text":"mehr"}]}}"#
                ),
                "cli_preview_rejected:llm_failed:event_after_result",
            ),
            (
                "zweites Ergebnis",
                format!("{GOOD_CLAUDE}{}", GOOD_CLAUDE.lines().nth(2).unwrap().to_string() + "\n"),
                "cli_preview_rejected:llm_failed:event_after_result",
            ),
            (
                "unbekannter Inhaltsblock",
                stream_with(&[(
                    r#""content":[{"type":"text","text":"Antwort"}]"#,
                    r#""content":[{"type":"image"}]"#,
                )]),
                "cli_preview_rejected:llm_failed:content_unknown",
            ),
            (
                "Init nicht zuerst",
                GOOD_CLAUDE.lines().skip(1).collect::<Vec<_>>().join("\n") + "\n",
                "cli_preview_rejected:llm_failed:event_order",
            ),
            (
                "Teilstring-Modell",
                stream_with(&[(
                    "claude-sonnet-5-5\":{\"outputTokens",
                    "claude-sonnet-5-5-preview\":{\"outputTokens",
                )]),
                "cli_preview_rejected:llm_failed:model_mismatch",
            ),
            (
                "ueber dem Kontext",
                stream_with(&[(r#""input_tokens":3000"#, r#""input_tokens":170000"#)]),
                "cli_preview_rejected:budget:context_exceeded",
            ),
        ];
    for (name, reply, expected) in &cases {
        let f = claude_fake(reply);
        let err = claude_call(&f, "claude-sonnet-5-5").expect_err(name);
        assert!(err.starts_with(expected), "{name}: {err}");
    }
}

#[cfg(unix)]
#[test]
fn broken_lines_error_results_and_exit_codes_never_succeed() {
    let broken_line = format!("{GOOD_CLAUDE}{{kaputt\n");
    let err_result = stream_with(&[(
        r#""subtype":"success","is_error":false"#,
        r#""subtype":"error_during_execution","is_error":true"#,
    )]);
    let is_error_only = stream_with(&[(r#""is_error":false"#, r#""is_error":true"#)]);
    let no_result = GOOD_CLAUDE.lines().take(2).collect::<Vec<_>>().join("\n");
    let no_is_error = stream_with(&[(r#""is_error":false,"#, "")]);
    for (name, reply) in [
        ("beschaedigte Zeile", broken_line),
        ("Fehler-Ergebnis", err_result),
        ("is_error", is_error_only),
        ("ohne Ergebnis", no_result),
        ("is_error fehlt", no_is_error),
    ] {
        let f = claude_fake(&reply);
        let err = claude_call(&f, "claude-sonnet-5-5").expect_err(name);
        assert!(
            err.starts_with("cli_bad_output") || err.starts_with("cli_failed"),
            "{name}: {err}"
        );
        assert!(!err.contains("Antwort"), "kein Inhalt im Fehler: {err}");
    }
    // Exit-Code <> 0 trotz sauberem Strom und Text.
    let f = claude_fake(GOOD_CLAUDE);
    f.set("exit", "3");
    let err = claude_call(&f, "claude-sonnet-5-5").unwrap_err();
    assert!(err.starts_with("cli_failed"), "{err}");
}

#[cfg(unix)]
#[test]
fn the_budget_and_the_binding_are_checked_before_the_cli_is_started() {
    // Zu knapp: nie gestartet.
    let f = claude_fake(GOOD_CLAUDE);
    let err = f
        .call(
            Cli::Claude,
            "sonnet",
            f.check(Cli::Claude, "sonnet", "claude-sonnet-5-5", 40_000),
            on(),
        )
        .unwrap_err();
    assert_eq!(err, "cli_preview_rejected:budget:budget_precheck");
    assert!(!f.started());
    // Programm nach der Bindung getauscht (z. B. Auto-Update): nie gestartet.
    let f = claude_fake(GOOD_CLAUDE);
    let check = f.check(Cli::Claude, "sonnet", "claude-sonnet-5-5", 200_000);
    let mut script = std::fs::read_to_string(&f.bin).unwrap();
    script.push_str("\n# Update\n");
    std::fs::write(&f.bin, script).unwrap();
    let err = f.call(Cli::Claude, "sonnet", check, on()).unwrap_err();
    assert_eq!(err, "cli_preview_rejected:llm_failed:binding_changed");
    assert!(!f.started());
    // Programm fehlt: nicht bestimmbar.
    let check = fixtures::check(
        binding_for(
            Cli::Claude,
            "sonnet",
            &preview::Identity {
                version: "9.9.9 (Fake)".into(),
                sha256: "00".repeat(32),
            },
        ),
        "claude-sonnet-5-5",
        200_000,
    );
    let err = call_with_options(
        Cli::Claude,
        Path::new("/nicht/vorhanden/claude"),
        "sonnet",
        Some("S"),
        "u",
        Duration::from_secs(5),
        &CallOptions {
            preview: Some(check),
            ..CallOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(err, "cli_preview_rejected:llm_failed:payload_unknown");
}

#[cfg(unix)]
#[test]
fn an_original_changed_during_the_call_voids_the_reply() {
    // Das Original wird waehrend des Laufs veraendert (gleicher Nutzer, in place).
    let f = claude_fake(GOOD_CLAUDE);
    let check = f.check(Cli::Claude, "sonnet", "claude-sonnet-5-5", 200_000);
    let copy = f.bin.clone();
    let swapper = std::thread::spawn({
        let log = f.dir.path().join("log.txt");
        move || {
            for _ in 0..200 {
                if log.exists() {
                    let mut s = std::fs::read_to_string(&copy).unwrap();
                    s.push_str("\n# getauscht\n");
                    std::fs::write(&copy, s).unwrap();
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    });
    f.set("sleep", "1");
    let err = f.call(Cli::Claude, "sonnet", check, on()).unwrap_err();
    swapper.join().unwrap();
    assert_eq!(err, "cli_preview_rejected:llm_failed:binding_changed");
}

#[cfg(unix)]
#[test]
fn codex_is_always_blocked_in_the_preview() {
    // Auch mit Schalter, Tupel, Belegen und einer Fake-CLI, die Erfolg meldete.
    let f = Fake::new(r#"{"type":"turn.completed"}"#, "--ignore-user-config");
    let b = binding_for(
        Cli::Codex,
        "gpt-6.1-sol",
        &preview::identity(&f.bin).unwrap(),
    );
    let check = fixtures::check(b, "gpt-6.1-sol", 272_000);
    let err = f.call(Cli::Codex, "gpt-6.1-sol", check, on()).unwrap_err();
    assert_eq!(
        err,
        "cli_preview_rejected:llm_failed:codex_preview_not_available"
    );
    assert!(!f.started(), "nie gestartet");
}

#[cfg(unix)]
fn pid_alive(pid: i32) -> bool {
    // SAFETY: Signal 0 prueft nur, ob der Prozess existiert.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(unix)]
fn wait_gone(pid: i32) -> bool {
    for _ in 0..100 {
        let mut status = 0;
        // SAFETY: Eingesammelt wird nur ein Waisenkind, das init uebernommen
        // hat; Fehler heisst „nicht unser Kind“, dann zaehlt `kill`.
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[cfg(unix)]
fn child_pid(f: &Fake) -> i32 {
    for _ in 0..100 {
        if let Ok(t) = std::fs::read_to_string(f.dir.path().join("child.pid")) {
            if let Ok(pid) = t.trim().parse() {
                return pid;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("Kindprozess nicht gestartet");
}

#[cfg(unix)]
#[test]
fn the_process_group_ends_the_child_process_on_timeout_and_revocation() {
    // Zeitablauf: das Fake-Skript startet ein Kind (`sleep 60`) und wartet.
    let f = claude_fake(GOOD_CLAUDE);
    f.set("spawn_child", "1");
    f.set("sleep", "30");
    let check = f.check(Cli::Claude, "sonnet", "claude-sonnet-5-5", 200_000);
    let err = call_with_options(
        Cli::Claude,
        &f.bin,
        "sonnet",
        Some("S"),
        "u",
        Duration::from_secs(2),
        &CallOptions {
            preview: Some(check),
            preview_switch: on(),
            ..CallOptions::default()
        },
    )
    .unwrap_err();
    assert!(err.starts_with("cli_timeout"), "{err}");
    let pid = child_pid(&f);
    assert!(wait_gone(pid), "Kindprozess lebt nach dem Zeitablauf noch");

    // Widerruf (Schalter aus) mitten im Lauf: ebenfalls Kind und Enkel weg.
    let f = claude_fake(GOOD_CLAUDE);
    f.set("spawn_child", "1");
    f.set("sleep", "30");
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let switch = {
        let flag = flag.clone();
        PreviewSwitch::new(move || flag.load(std::sync::atomic::Ordering::SeqCst))
    };
    let dir = f.dir.path().to_path_buf();
    let flipper = std::thread::spawn(move || {
        for _ in 0..400 {
            if dir.join("child.pid").exists() {
                std::thread::sleep(Duration::from_millis(200));
                flag.store(false, std::sync::atomic::Ordering::SeqCst);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let err = f
        .call(
            Cli::Claude,
            "sonnet",
            f.check(Cli::Claude, "sonnet", "claude-sonnet-5-5", 200_000),
            Some(switch),
        )
        .unwrap_err();
    flipper.join().unwrap();
    assert_eq!(err, "cli_preview_rejected:cancelled:switch_off");
    let pid = child_pid(&f);
    assert!(wait_gone(pid), "Kindprozess lebt nach dem Widerruf noch");
}

#[cfg(unix)]
#[test]
fn switching_the_preview_off_stops_the_call_and_drops_the_result() {
    // Vor dem Aufruf: nie gestartet.
    let f = claude_fake(GOOD_CLAUDE);
    let err = f
        .call(
            Cli::Claude,
            "sonnet",
            f.check(Cli::Claude, "sonnet", "claude-sonnet-5-5", 200_000),
            Some(PreviewSwitch::new(|| false)),
        )
        .unwrap_err();
    assert_eq!(err, "cli_preview_rejected:cancelled:switch_off");
    assert!(!f.started());

    // Mitten im Lauf: der Prozess wird beendet, lange vor dem Ende seiner Wartezeit.
    let f = claude_fake(GOOD_CLAUDE);
    f.set("sleep", "20");
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let switch = {
        let flag = flag.clone();
        PreviewSwitch::new(move || flag.load(std::sync::atomic::Ordering::SeqCst))
    };
    let dir = f.dir.path().to_path_buf();
    let flipper = std::thread::spawn(move || {
        for _ in 0..400 {
            if dir.join("log.txt").exists() {
                std::thread::sleep(Duration::from_millis(200));
                flag.store(false, std::sync::atomic::Ordering::SeqCst);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let started = std::time::Instant::now();
    let err = f
        .call(
            Cli::Claude,
            "sonnet",
            f.check(Cli::Claude, "sonnet", "claude-sonnet-5-5", 200_000),
            Some(switch),
        )
        .unwrap_err();
    flipper.join().unwrap();
    assert_eq!(err, "cli_preview_rejected:cancelled:switch_off");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "Prozess nicht beendet"
    );
    assert!(
        !f.dir.path().join("finished").exists(),
        "Prozess lief nicht zu Ende"
    );

    // Am Ende des Laufs, vor der Rueckgabe: die CLI ist fertig, die Antwort
    // ist gut, trotzdem wird sie verworfen.
    let f = claude_fake(GOOD_CLAUDE);
    let marker = f.dir.path().join("finished");
    let err = f
        .call(
            Cli::Claude,
            "sonnet",
            f.check(Cli::Claude, "sonnet", "claude-sonnet-5-5", 200_000),
            Some(PreviewSwitch::new(move || !marker.exists())),
        )
        .unwrap_err();
    assert_eq!(err, "cli_preview_rejected:cancelled:switch_off");
    assert!(f.dir.path().join("finished").exists());
}
