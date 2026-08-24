# Probe: fishaudio/s1-mini als schnelle Zweit-Engine

Kurze, einmalige Probe fuer die Frage: Läuft das kleine TTS-Modell `fishaudio/s1-mini`
(0,5 B Parameter, HF-gated, Lizenz CC-BY-NC-SA) mit der lokal installierten
fish-speech-Codebase (S2-Branch), und ist es schnell/verständlich genug, um später als
zweite, schnellere Engine neben S2-Pro in die App zu kommen?

Das Skript dafür: `apps/local-voice/scripts/probe-s1-mini.ps1`. Es ändert **keinen
App-Code** — es ist ein eigenständiges Diagnose-Skript, das den fish-speech-Server aus
`C:\AI\fish-speech` standalone startet, testet und wieder beendet.

## Erfolgs-/Misserfolgskriterien

**GO für den App-Einbau**, wenn beide zutreffen:

- Der Server wird innerhalb von 240 s `healthy` (Report: „Server geladen: ja“).
- Der TTS-Testsatz erzeugt eine hörbare, verständliche deutsche WAV-Datei.

**Vermutlich NO-GO / weitere Arbeit nötig**, wenn:

- Der Server beim Laden abbricht oder das Zeitfenster reißt. Das Skript druckt in
  diesem Fall automatisch die letzten 40 Logzeilen — **das ist der eigentliche
  Erkenntnisgewinn der Probe**, nicht das PASS/FAIL an sich. Ein typischer Verdacht:
  s1-mini braucht einen älteren fish-speech-Stand (Pre-S2-Checkout) als der aktuell
  installierte S2-Branch. Was in den Logzeilen stand, bitte 1:1 mit zurückgeben
  (siehe unten).

## Vorbereitung: Modell einmalig herunterladen (manuell, HF-gated)

Das Skript lädt selbst **nichts** aus dem Netz herunter — dafür ist ein Hugging-Face-Konto
mit akzeptierten Modellbedingungen nötig, das nur Patrick machen kann:

1. Auf https://huggingface.co/fishaudio/s1-mini einloggen und die Modellbedingungen
   akzeptieren (gated repo).
2. Herunterladen in das erwartete Verzeichnis, mit der im fish-speech-venv bereits
   installierten `hf`-CLI:

   ```
   C:\AI\fish-speech\.venv\Scripts\hf.exe download fishaudio/s1-mini --local-dir C:\AI\models\fish-audio\s1-mini
   ```

   Bei einer älteren `huggingface_hub`-Installation heißt der Befehl stattdessen:

   ```
   C:\AI\fish-speech\.venv\Scripts\huggingface-cli.exe download fishaudio/s1-mini --local-dir C:\AI\models\fish-audio\s1-mini
   ```

Beide Varianten laden in denselben Zielordner, den das Skript per Default erwartet
(`-ModelDir`, überschreibbar).

## Wichtige Warnung: S2-Pro-Server darf währenddessen NICHT laufen

Der App-eigene S2-Pro-Server und die s1-mini-Probe konkurrieren um denselben VRAM. Läuft
S2-Pro parallel, sind die gemessenen Ladezeiten und die TTS-Dauer wertlos (siehe
`how-to-fish.md`: dieselbe Synthese war schon einmal 10-20x langsamer, weil ein zweites
Modell im VRAM lag und Windows auslagern musste). Vor der Probe also:

- Local Voice AI schließen, falls die App gerade eine eigene TTS-Engine geladen hat.
- Prüfen, dass auf Port 8080 (S2-Pro-Standard) kein Server mehr lauscht.

Das Probe-Skript nutzt bewusst einen anderen Port (Default 8081), damit es nicht
versehentlich gegen einen bereits laufenden S2-Pro-Server testet.

**Lizenzhinweis:** CC-BY-NC-SA erlaubt private Nutzung, Forschung und Evaluation — genau
das ist diese Probe. Für eine spätere kommerzielle/geschäftliche Nutzung (auch intern bei
Wolff Applied AI) wäre vorab eine gesonderte Klärung nötig, wie schon für S2-Pro in
`how-to-fish.md` vermerkt.

## Ausführen

```powershell
cd apps\local-voice\scripts
.\probe-s1-mini.ps1                 # voller Lauf: Start, Health-Poll, TTS-Test, Report
.\probe-s1-mini.ps1 -CheckOnly      # Trockenlauf: nur Vorbedingungen pruefen, kein Serverstart
```

Parameter (alle optional):

| Parameter   | Default                              | Bedeutung |
|---|---|---|
| `-FishDir`  | `C:\AI\fish-speech`                  | Lokaler fish-speech-Checkout (S2-Branch, uv-venv) |
| `-ModelDir` | `C:\AI\models\fish-audio\s1-mini`     | Heruntergeladener s1-mini-Checkpoint |
| `-Port`     | `8081`                                | Port des Probe-Servers (bewusst nicht 8080) |
| `-CheckOnly`| aus                                   | Nur Vorbedingungen pruefen, dann beenden |

Der volle Lauf startet den Server im Hintergrund, wartet bis zu 240 s auf
`http://127.0.0.1:<Port>/v1/health`, schickt einen deutschen Testsatz mit einem
`(excited)`-Emotions-Tag über den vorhandenen `tools/api_client.py`, misst VRAM vorher/
nachher per `nvidia-smi` (falls installiert) und beendet den Serverprozess danach wieder
sauber — inklusive aller Kindprozesse, da der Interpreter-Prozess unter Windows manchmal
einen weiteren Kindprozess mitbringt, der beim einfachen Beenden zurückbleiben würde.

## Wie das Ergebnis zurückfließt

Das Skript schreibt am Ende einen Report gleichzeitig auf die Konsole und als Textdatei
unter `%TEMP%\s1-mini-probe-<Zeitstempel>\report.txt`. Diese Report-Datei (Inhalt reicht,
kein Screenshot nötig) bitte an Claude zurückgeben — sie enthält alles Nötige, um zu
entscheiden, ob s1-mini als Zweit-Engine sinnvoll ist: geladen ja/nein, Ladezeit,
VRAM-Delta, TTS-Dauer, WAV-Pfad + Größe, und im Fehlerfall die letzten 40 Logzeilen.
