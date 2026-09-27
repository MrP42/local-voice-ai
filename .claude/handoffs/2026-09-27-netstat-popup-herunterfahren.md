# Handoff – Local Voice AI: NETSTAT-Popup beim Herunterfahren (27.09.2026)

## Stand in einem Satz
Der Fix ist als **PR #57** gegen `main` offen (<https://github.com/MrP42/local-voice-ai/pull/57>, Commit `f47051d`); der Merge per `gh pr merge` wurde vom Auto-Mode-Klassifikator blockiert und liegt bei Patrick. Der **Installer 0.20.3** (Fix + #56) ist gebaut: `apps/local-voice/src-tauri/target/release/bundle/nsis/Local Voice AI_0.20.3_x64-setup.exe` (+ MSI, beide signiert), aus dem lokalen, **nicht gepushten** Zweig `chore/0.20.3-abnahme` (`bafa473` = `f47051d` + Merge von `origin/fix/tag-vorschlaege` + Version 0.20.3; `cargo test --lib` 684 grün).

## Hintergrund (nicht wiederholen, dort nachlesen)
- Diagnose der PC-Sitzung: `C:\Users\wolff\shutdown-diag\.claude\handoffs\2026-09-27-pc-diagnose-ethernet-0x9f.md`, Abschnitt „Nächster Schritt: NETSTAT-Popup“. Kurz: System-Log „Application Popup“ Id 26 zeigte NETSTAT.EXE mit 0xc0000142 bei praktisch jedem Herunterfahren seit ≥ 18.09.; Quelle war `RunEvent::Exit` in `apps/local-voice/src-tauri/src/lib.rs` → `tts.stop_server()` → `listening_pid()` (startete **immer** `netstat`, auch ohne laufenden Server).
- Projektregeln: `AGENTS.md` (Wurzel) und `apps/local-voice/AGENTS.md` – eigener Zweig, PR, keine Formatierläufe über fremde Dateien, lange Läufe ansagen.

## Was geändert wurde (Diff: `git diff` auf dem Zweig, 3 Dateien, +51/−7)
- `apps/local-voice/src-tauri/src/process_guard.rs`: neue `pub fn session_ending()` über `GetSystemMetrics(SM_SHUTTINGDOWN)` (vorhandene `windows`-Crate, Feature `Win32_UI_WindowsAndMessaging`), dazu Test `session_is_not_ending_during_a_test_run`.
- `apps/local-voice/src-tauri/src/managers/tts/mod.rs`: `stop_server()` überspringt Port-Suche/`taskkill` beim Sitzungsende; `kill_owned_child()` überspringt `taskkill` beim Sitzungsende (Job-Objekt mit KILL_ON_JOB_CLOSE beendet den Baum ohnehin).
- `apps/local-voice/src-tauri/src/managers/llm/server.rs`: `stop()` ohne `taskkill` beim Sitzungsende (`Running._guard` beendet den Baum).
- Normales Beenden der App unverändert; `stop_server_any`/`kill_server_hard` (Nutzeraktionen) unverändert.

## Zweig-Lage
- `fix/netstat-beim-herunterfahren` = ein Commit `f47051d` auf `origin/main` (55036b2, Version 0.20.1), gepusht, PR #57. Vor dem Push auf `main` umgesetzt, weil #56 keine der drei Dateien berührt; Tests danach erneut grün.
- Unabhängig davon ist PR #56 (`fix/tag-vorschlaege`, 0.20.2 = installierte App) offen. #57 enthält bewusst keine Versionsänderung, damit es nicht mit 0.20.2 kollidiert.
- Der Haupt-Checkout steht auf `fix/netstat-beim-herunterfahren`. Der alte Handoff `2026-09-21-skript-editor-werkstatt-pakete.md` ist beim Release-Stand überholt (0.20.0 getaggt, danach #55, #56, #57).

## Nächste Schritte
1. Patrick installiert 0.20.3 (Update-Angebot der App aus dem lokalen Bundle-Ordner oder die Setup-EXE direkt) und startet Windows neu. Abnahme:
   `Get-WinEvent -FilterHashtable @{LogName='System'; ProviderName='Application Popup'; StartTime=(Get-Date).AddHours(-1)}` → keine NETSTAT.EXE/TASKKILL.EXE-Zeile.
2. Patrick merged #56 und #57 selbst (Klassifikator blockt `gh pr merge` für Claude), in #57 vorher die „Human Written Description“ ausfüllen.
3. Danach 0.20.3 nach `main`: `chore/0.20.3-abnahme` pushen und als PR öffnen – nach beiden Merges enthält sein Diff gegen `main` nur noch die vier Versionsdateien. Kein Tag `app-v0.20.3` ohne Ansage (Release ist ein Ereignis für alle Zugänge).
4. Falls NETSTAT trotzdem erscheint: SM_SHUTTINGDOWN war beim Exit noch nicht gesetzt → Plan B: Sitzungsende über `WM_QUERYENDSESSION`/`WM_ENDSESSION` erkennen oder `netstat`/`taskkill` durch IP-Helper-API (`GetExtendedTcpTable`) plus `OpenProcess`/`TerminateProcess` ersetzen.
6. PR-Vorlage: `apps/local-voice/.github/PULL_REQUEST_TEMPLATE.md` (Handy-Upstream) ist laut `apps/local-voice/AGENTS.md` Pflicht; #57 folgt ihr, #56 nicht.

## Werkzeug-Fallen
- `rustfmt`/`cargo` liegen nicht im PATH der pwsh-Sitzung: `$env:USERPROFILE\.cargo\bin\…` nutzen. `cargo test --lib` dauert warm ~1 min.
- rustfmt, clippy und prettier sind im Altcode vorbestehend rot (siehe `AGENTS.md`) – nur die eigenen Zeilen prüfen; meine Zeilen sind rustfmt-sauber, keine neuen Warnungen.
- Das Edit-Tool hat `process_guard.rs` im Arbeitsbaum auf LF gestellt (Git-Warnung „LF will be replaced by CRLF“); `core.autocrlf=true` normalisiert beim Commit, kein Handlungsbedarf.

## Empfohlene Skills
- `superpowers:verification-before-completion` – vor jeder Erfolgsmeldung Tests/Abnahme wirklich laufen lassen.
- `superpowers:finishing-a-development-branch` – für Commit, PR-Basis und Merge-Reihenfolge mit #56.
- `handoff` – am Ende der Session.
