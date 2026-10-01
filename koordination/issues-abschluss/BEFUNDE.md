# Befunde des Planers — Offene Issues abschliessen, neue Features, Version 0.21.0

(Je Befund: Überschrift `## B<n> — <Paket>: <Titel>`, Zeilen Beobachtung / Beleg / Konsequenz und
eine Zeile `- Status: offen` bzw. `- Status: erledigt (<Paket/Commit>)`. Offene Befunde verhindern COMPLETE.)

## B20 — QG5: Sandbox-TOCTOU Ordner/Anhang (hoch)
- Beobachtung: folder.rs:345/385, attachments.rs:255: Pfadprüfung und Zugriff getrennt; Junction-Tausch dazwischen erlaubt Lesen/Überschreiben außerhalb.
- Beleg: Review koordination/issues-abschluss/review-qg5-codex.md
- Konsequenz: S1: handle-basierter Zugriff
- Status: offen

## B21 — QG5: Anhang nach Freigabe austauschbar (mittel)
- Beobachtung: integration_actions.rs:605/620: gate_view prüft, run liest neu.
- Beleg: dto.
- Konsequenz: S1: Bytes einmal lesen, SHA-256 gegen Freigabe, genau diese senden
- Status: offen

## B22 — QG5: M365-Abmeldung durch laufendes Refresh rückgängig (mittel)
- Beobachtung: m365/service.rs:228/282
- Beleg: dto.
- Konsequenz: S1: Kontogeneration/Widerruf vor Token-Ablage prüfen
- Status: offen

## B23 — QG5: Agentenbrücke durch blockierte Schreibvorgänge lahmlegbar (mittel)
- Beobachtung: agent_bridge/server.rs:111/463, protocol.rs:94
- Beleg: dto.
- Konsequenz: S1: Schreib-/Flush-Fristen, ID-/Antwortgrößen begrenzen
- Status: offen
