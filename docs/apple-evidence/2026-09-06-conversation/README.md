# Lokaler Gesprächsmodus

Neue getrennte Optionen unter „Gespräch“ auf iPhone und Watch:

- **Antwort vorlesen:** fertige Antworten automatisch lokal abspielen; bisheriges Standardverhalten bleibt eingeschaltet.
- **Freisprechen:** nach explizitem Start auf eine Sprechpause reagieren, speichern, antworten und anschließend wieder zuhören. Während der Sprachausgabe bleibt das Mikrofon geschlossen. Ohne Vorlesen beginnt der nächste Beitrag nach der Textantwort.
- **Neues Gespräch:** getrennten lokalen Gesprächskontext beginnen. Der bestehende Verlauf bleibt erhalten.

Ein Schalter oder App-Neustart startet niemals allein das Mikrofon. „Gespräch beenden“,
Audio-Unterbrechung, fehlende Berechtigung, acht Sekunden ohne erkannte Sprache oder
Verlassen der Eingabe-App beenden/pausieren das automatische Zuhören. Einzelne Beiträge
bleiben auf 30 Sekunden begrenzt. Die Sprechpause wird derzeit über Audiopegel erkannt
(1,2 Sekunden Pause nach erkannter Sprache), nicht über ein zusätzliches Watch-ML-Modell.
Reale Gesprächsqualität bei Hintergrundgeräuschen ist noch nicht abgenommen.

Das iPhone darf während der Verarbeitung einer Watch-Aufnahme im Hintergrund sein.
Der neue Freisprechmodus hält hingegen die **Eingabe-App** nicht unbegrenzt aktiv;
insbesondere ist er kein Ausbau zur Aufnahme direkt aus einer Komplikation.

## Gedächtnis und Wiederherstellung

Gesprächs-UUIDs werden mit Aufnahme, Transfer-Chunks und Verlauf gespeichert. Auch ein
wiederhergestellter Aufnahmeentwurf behält seine Zuordnung. Die lokale Antwort erhält
bis zu sechs vollständige vorherige Wortwechsel desselben Gesprächs, insgesamt maximal
3.400 Zeichen aus diesen Wortwechseln. Andere Gespräche und künftige/offene Beiträge
werden ausgeschlossen. Der Originalverlauf wird durch das Kontextlimit nicht gekürzt.
Apple Foundation Models und der lokale Qwen-Fallback erhalten den Kontext; das CPU-
Antwortfenster wurde passend vergrößert.

## Prüfung

- 84 Core-Tests bestanden: einschließlich Kontext nach Neustart, Gruppentrennung,
  Job-Verarbeitung mit vorherigem Wortwechsel, Aufnahme-Recovery, Chunk-Transport
  Sprechpausenerkennung, kurze Störgeräusche und Kontextobergrenzen.
- iPhone-Bedienungstest: Schalter allein startet nicht, Start/Stop und Stop bleibt
  wirksam trotz nachfolgender Verarbeitung; bestanden im dunklen Modus.
- iPhone-Gesprächszyklus: echte Simulator-Aufnahme und System-Sprachausgabe,
  kontrollierte STT-/LLM-Antwort und simulierter erster Sprechschluss; nächster
  Mikrofonbeitrag startet nach der Antwort, Verlassen pausiert; im hellen und dunklen Modus bestanden.
- Watch-Bedienungstest: beide Schalter vorhanden, bloßes Öffnen startet nicht; bestanden.
- Echtes iPhone, Apple Foundation Models: vorher „Mein Hund heißt Milo.“, danach
  „Wie heißt mein Hund?“ → tatsächlich „Dein Hund heißt Milo.“. Erzeugung 1,94 s,
  System-Sprachausgabe startete nach 14 ms. Synthetisch vorbereitete Transkripte,
  kein vollständiger Mikrofon-/STT-Gesprächslauf. Siehe `device-memory.json`.

Keine gemessene Akkuwirkung oder Zusage unbegrenzten Zuhörens. Hintergrund-
Gerätenachweise stehen separat in `../2026-09-06-background/README.md`.

## Geräteinstallation und Git

Finale signierte Builds für iPhone und Watch erfolgreich, auf beiden echten Geräten installiert. Normaler iPhone-Start bestätigt. Der Start der endgültigen Watch-Fassung wurde ausdrücklich wegen der Watch-Sperre abgewiesen; zum abschließenden Start muss die Watch entsperrt werden. Die Installation selbst ist erfolgreich. Code-Commits: `749328f` (dauerhafter Kontext), `8289a90` (Gesprächssteuerung, lokale Anbieter und Bedienungstests). Eigener Worktree und Branch `codex/watch-conversation-background`; kein Merge/Push nach main. Einzelheiten zur parallelen Entwicklung: `../2026-09-06-background/parallel-development.md`.
