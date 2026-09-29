# Local Voice AI starten und benutzen

Kurzanleitung für den stabilisierten Stand vom 17.08.2026.
Die Voice-AI-Funktionen vom 18.08.2026 (Vorlesen, Stimmen klonen, Übersetzung,
Stimmwechsler) sind am Ende dieses Dokuments beschrieben.
Die Besprechungsfunktionen (Notizen, KI-Notizen, Sprecher, Chat, Kalender, Export)
stehen in [BESPRECHUNGEN.md](BESPRECHUNGEN.md).

Seit dem 18.08.2026 heißt die App **Local Voice AI** (vormals „Sprechstift").
Seit dem 19.08.2026 heißt auch die Programmdatei `local-voice-ai.exe`, und der
Datenordner wurde umbenannt — **Einstellungen, Verlauf und Modelle zieht die App
beim ersten Start automatisch um**, es geht nichts verloren.

## Starten

Doppelklick auf:

```
C:\Users\wolff\local-voice-project\apps\local-voice\src-tauri\target\release\local-voice-ai.exe
```

Das Fenster erscheint, danach genügt das Symbol im Infobereich. **Das Fenster darf
geschlossen werden** — die Anwendung läuft im Infobereich weiter. Sie muss laufen,
sonst reagiert der Hotkey nicht.

## Diktieren

| Taste | Wirkung |
|---|---|
| **Strg-links + Windows-links** | Aufnahme starten, nochmal drücken zum Beenden |
| **Esc** | laufende Aufnahme abbrechen (kein Text) |

Ablauf: Hotkey drücken → sprechen → Hotkey drücken. Nach rund zwei Sekunden steht der
Text an der Einfügemarke des Fensters, in dem Sie den Hotkey zum Beenden gedrückt haben.

Das Modell ist **Parakeet TDT 0.6B v3** und läuft vollständig lokal auf der CPU. Kein
Netzwerk, kein Docker, kein Ollama, kein Konto.

## Wenn der Text nicht erscheint

Dann erscheint stattdessen für neun Sekunden eine Meldung am Bildschirmrand — mit dem
Grund und dem Hinweis, was zu tun ist. In aller Regel lautet er:

> Der vollständige Text liegt in der Zwischenablage — mit Strg+V einfügen.

Das ist die Zusage dieses Stands: **entweder der Text wird eingefügt, oder er liegt
vollständig in der Zwischenablage und Sie werden darauf hingewiesen.** Verloren geht er
nicht. Zusätzlich steht jedes Diktat im Verlauf im Hauptfenster.

## Was nicht funktioniert

**In Fenstern mit Administratorrechten reagiert der Hotkey nicht.** Windows liefert
Tastendrücke nicht an ein Programm niedrigerer Rechtestufe aus, solange ein solches
Fenster im Vordergrund ist (z. B. der Task-Manager). Es passiert dann schlicht nichts —
die Aufnahmeanzeige bleibt aus, und Sie merken es. Wer dort diktieren will, muss
Local Voice AI selbst als Administrator starten.

Wechselt der Fokus erst **nach** dem Beenden der Aufnahme in ein solches Fenster,
greift der oben beschriebene Weg über die Zwischenablage.

**In Fenstern ohne Eingabefeld** (etwa dem Explorer) meldet Windows uns keinen Fehler.
Die Anwendung hält den Einfügeversuch dann für erfolgreich, der Text landet aber
nirgends. Er steht weiterhin im Verlauf.

## Live-Einfügung während des Sprechens (Streaming)

Der Text erscheint dabei schon **während** Sie sprechen, nicht erst nach dem
Beenden. Verifiziert am 17.08.2026 — vollständige Sätze, Sprechpausen und Umlaute
kamen korrekt an, ohne Zeichenwiederholung und ohne Duplikate.

Dafür sind zwei Dinge nötig:

1. **Ein streaming-fähiges Modell.** Für Deutsch ist das **Nemotron Streaming
   3.5**; es ist heruntergeladen und geprüft. Parakeet V3 kann kein Streaming und
   ignoriert den Schalter.
2. **Die Einstellung „Live-Einfügung"** (`stream_injection`) einschalten.

**Zwei Dinge, die Sie dabei wissen sollten:**

- **Der Fokus wird während des Streamings nicht überwacht.** Wechseln Sie
  mitten im Sprechen das Fenster, schreiben die folgenden Bruchstücke in das
  neue Fenster. Der Schutzmechanismus des Standardpfads greift hier nicht.
- **Nemotron normalisiert Zahlen nicht.** Sie erhalten „dritten Februar um
  vierzehn Uhr dreißig" statt „3. Februar um 14.30 Uhr". Wer die normalisierte
  Form braucht, bleibt bei Parakeet V3 ohne Streaming.

Umschalten der Modelle im Hauptfenster unter „Modelle".

## Einstellungen, die bewusst so stehen

| Einstellung | Wert | Grund |
|---|---|---|
| Modell | Parakeet V3 | verifiziert, rund 23-fache Echtzeit, normalisiert Zahlen |
| Live-Einfügung während des Sprechens | aus | funktioniert, aber ohne Fokusüberwachung — siehe oben |
| KI-Nachbearbeitung (Ollama) | aus | gehört nicht in den stabilen Pfad |
| Debug-Modus | aus | — |

## Falls etwas klemmt

Protokoll (enthält **keine** Diktatinhalte):

```
%LOCALAPPDATA%\de.wolffappliedai.localvoiceai\logs\handy.log
```

Die Sicherung Ihrer vorherigen Einstellungen liegt unter
`%APPDATA%\de.wolffappliedai.localvoiceai\settings_store.json.vor-stabilisierung-2026-08-17.bak`.

Die alte Protokolldatei von vor der Bereinigung enthält noch vollständige Diktate im
Klartext und heißt `handy.log.vor-m3-2026-08-17`. Sie darf gelöscht werden (Issue #6).

## Voice AI: Vorlesen, Stimmen, Übersetzung, Stimmwechsler (Stand 18.08.2026)

Alles davon läuft lokal: die Sprachsynthese über den Fish-Speech-Server aus
`C:\AI\fish-speech` (die App startet und stoppt ihn selbst), die Spracherkennung
über die vorhandenen Modelle, die Übersetzung über den konfigurierten
Nachbearbeitungs-Provider (für lokal: Custom → Ollama).

Alle vier Funktionen liegen im Hauptfenster unter **„Vorlesen"**:

| Funktion | Bedienung |
|---|---|
| **Vorlesen** | Text ins Feld tippen → „Vorlesen"; oder Text kopieren und **Strg+Alt+Leertaste** drücken (zweiter Druck stoppt). Beim ersten Mal startet der Server (~2 Minuten, Status-Anzeige); danach kommt der erste Satz nach rund 2 Sekunden. Nach 15 Minuten Leerlauf stoppt der Server von selbst und gibt den Grafikspeicher frei. |
| **Stimmen** | „Neue Stimme aufnehmen" → 10–30 Sekunden natürlich sprechen → Stopp. Das Transkript entsteht automatisch und ist korrigierbar. Name vergeben, speichern — die Stimme ist sofort aktiv. Alternativ eine WAV-Datei importieren (beste Qualität). Aufnahmen bleiben auf diesem Rechner. |
| **Audio-Übersetzung** | Zielsprache wählen, dann tippen oder „Aufnehmen & übersetzen". Die Übersetzung erscheint als Text und wird in der gewählten Stimme gesprochen. Voraussetzung: ein Nachbearbeitungs-Provider mit Modell (für lokal: Ollama mit einem **kleinen** Modell — ein großes passt nicht neben die Sprachsynthese in den Grafikspeicher). |
| **Stimmwechsler** | Aufnehmen oder WAV-Datei wählen → der Inhalt wird in der gewählten Stimme nachgesprochen und lässt sich als WAV exportieren. Kein Live-Effekt: erst erkennen, dann synthetisieren (ein 10-Sekunden-Ergebnis dauert etwa 7 Sekunden). |

Ein Hinweis zur GPU: Während der Fish-Server geladen ist, belegt er rund 20 GB
Grafikspeicher. Andere GPU-Programme (ComfyUI, große Ollama-Modelle) gleichzeitig
laufen zu lassen macht alles um ein Vielfaches langsamer — die App zeigt beim
Serverstart einen Hinweis, wenn genau das passiert.

### Erweiterungen vom 19.08.2026

| Funktion | Bedienung |
|---|---|
| **Bücher & Dokumente (Hörbuch)** | Karte „Bücher & Dokumente": TXT/MD/PDF/DOCX öffnen → Abspielen. Fortschritt wird **je Dokument gespeichert** (Bibliothek mit Fortsetzen, Zurücksetzen, Entfernen), satzweises Springen mit ◀/▶, Pause/Weiter. Das Vorlesen startet nach dem ersten Satz (~2 s), der Rest entsteht währenddessen. |
| **Player-Steuerung im Textfeld** | Vorlesen, Pause, Weiter (setzt am letzten vollständigen Satz fort), Satzanzeige. Bereits erzeugte, unveränderte Sätze kommen aus einem Cache und werden nicht neu synthetisiert. |
| **Lautstärke / Geschwindigkeit / Format** | In den Vorlesen-Einstellungen: Lautstärke-Regler, Tempo 0,5×–2×, Exportformat WAV/MP3/Opus für gespeicherte Audiodateien. |
| **KI-Zusammenfassung** | Karte „KI-Zusammenfassung": Text einfügen oder Dokument laden → Umfang/Detailgrad/Zielgruppe wählen → zusammenfassen (LLM über die Nachbearbeitungs-Einstellungen: lokal per Ollama oder API) → vorlesen oder **als Audiodatei speichern**. |
| **Beliebige Medienformate** | Stimmwechsler und Stimmen-Import akzeptieren jetzt auch mp3, m4a, mp4, mov, mkv u. a. (Umwandlung über das installierte ffmpeg). |
| **Explorer-Kontextmenü** | Schalter in den Vorlesen-Einstellungen: „Mit Local Voice AI vorlesen" im Rechtsklick-Menü von Dokumentdateien. Für **markierten Text** bietet Windows kein systemweites Menü — dafür gibt es den Hotkey Strg+Alt+Leertaste. |
