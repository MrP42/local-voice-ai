# Diktat-Startlatenz: Befund, Fix, Messung (15.09.2026)

Anlass: Nach dem Diktat-Hotkey fehlen die ersten ein bis zwei Wörter, wenn man sofort
spricht. Auslöser war ein Video von Everlast AI, in dem ein ähnliches Werkzeug ~600 ms
durch eine PowerShell-Abfrage der Mikrofon-Berechtigung verlor.

## Befund

Eine PowerShell- oder Berechtigungsprüfung gibt es in Local Voice AI nicht. Die Latenz
sitzt woanders, und sie war schon im App-Log sichtbar (`%LOCALAPPDATA%\de.wolffappliedai.localvoiceai\logs\handy.log`):

| Diktat 15.09. | Stream öffnen | erstes Audio nach Stream-Start | Ergebnis |
|---|---:|---:|---|
| 08:12:12 (Kaltstart) | 140 ms | 628 ms | – |
| 08:17:49 (Kaltstart) | 29 ms | 557 ms | **0 Samples** bei 1 s Aufnahme |
| 08:17:59 (10 s später) | 21 ms | 187 ms | – |

Ursache in zwei Teilen:

1. **Jedes Diktat war ein Kaltstart.** Im On-Demand-Modus (Standard) schloss die App den
   Mikrofon-Stream nach jedem Stopp sofort, weil das Setting „Mikrofon offen halten"
   (`lazy_stream_close`) standardmäßig aus war. Ein frisch geöffneter WASAPI-Stream
   liefert am USB-Mikrofon (Insta360 Link 2) erst 190 bis 630 ms nach `play()` das
   erste Sample. Das ist Geräte-/Treiber-Aufwärmzeit, nicht Code.
2. **Das Bereit-Signal log.** Overlay und Startton kamen 30 bis 140 ms nach dem Hotkey,
   also bevor Audio floss. Der Nutzer sprach in die Lücke.

Was nicht die Ursache ist: Geräte-Enumeration (gecacht, < 20 µs), VAD-Laden (nur beim
allerersten Mal 50 ms), Overlay/Tray (je < 10 ms), VAD-Onset (Prefill 15 Frames = 450 ms
werden beim Onset nachgeliefert).

Zweitanalyse durch Codex (nur lesend, unabhängig): gleicher Befund, gleiche Empfehlung
(„Lazy Close aktivieren und verlängern; ein echter Kaltstart ist ohne Vorwärmsignal nicht
auf null zu bringen"). Vollständiger Text unten.

## Fix (Zweig `fix/diktat-startlatenz`)

1. **Stream nach dem Diktat offen halten, standardmäßig.** `lazy_stream_close` ist jetzt
   `true`; Settings-Schema 3 setzt es bei bestehenden Installationen einmalig, danach
   gilt wieder die Nutzerwahl. Idle-Fenster von 30 s auf 5 min. Innerhalb des Fensters
   ist jedes Diktat ein Warmstart.
2. **Ehrliches Bereit-Signal.** Der Recorder meldet, wenn der erste Audio-Block wirklich
   erfasst ist (`with_capture_ready_callback`). Im On-Demand-Modus erscheinen Overlay und
   Startton erst dann (Obergrenze 1,5 s). Beim unvermeidbaren Kaltstart sieht der Nutzer
   also, wann er sprechen kann, statt in die Lücke zu sprechen.
3. **Kennzahl im Log.** Neue Zeile `capture ready N ms after request`: Zeit vom Hotkey
   bis zum ersten erfassten Audio. Damit ist die Latenz künftig direkt ablesbar.

Bewusst nicht gemacht: Vorwärmen beim ersten Modifier-Druck (würde bei jedem Strg+C das
Mikrofon öffnen und die Windows-Mikrofonanzeige flackern lassen); dauerhaft offenes
Mikrofon als Standard (Datenschutz, das gibt es weiter als Option „Always-on").

## Messung vorher/nachher

Hardware-Test am echten Standardmikrofon (`cargo test capture_latency -- --ignored --nocapture`,
`audio_toolkit/audio/recorder.rs`). Gemessen: Aufnahme-Anforderung bis erstes erfasstes Audio.

| Betriebsart | Lauf 1 | Lauf 2 | Lauf 3 | Lauf 4 |
|---|---:|---:|---:|---:|
| **Vorher**: Stream je Diktat neu geöffnet (Kaltstart) | 580 ms | 523 ms | 428 ms | 523 ms |
| **Nachher**: Stream offen (Warmstart) | 9 ms | 10 ms | 9 ms | 10 ms |

Der erste Start nach mehr als 5 Minuten Pause bleibt ein Kaltstart (Gerät schläft); dort
zeigt das Overlay jetzt erst „Aufnahme", wenn Audio fließt.

Prüfung im Betrieb: nach zwei Diktaten hintereinander im Log nach `capture ready` suchen.
Erwartung: erstes Diktat je nach Gerät 200–600 ms, jedes weitere < 40 ms.

## Anhang: Codex-Zweitanalyse (unverändert)

# Review: Hotkey → Audioaufnahme

**Der Verdacht ist plausibel:** Beim Kaltstart liegen Geräteöffnung und mehrere vorbereitende Aufrufe vor der Aufnahme. Das VAD-Prefill kann diese Lücke nicht schließen. Ein tatsächlicher Verlust von ein bis zwei Wörtern ist ohne Laufzeitmessung jedoch nicht bewiesen.

Nur lesende Codeanalyse; keine Änderungen oder Hardwaretests. Zeitspannen unten sind **grobe, ungemessene Arbeitsannahmen für Windows/WASAPI**, keine Benchmarks. Die HAL-/macOS-Zahlen in Codekommentaren sind keine Windows-Messwerte.

## 1. Vollständiger Startpfad und Zeitbudget

Abkürzungen: `A` = `actions.rs`, `M` = `managers/audio.rs`, `R` = `audio_toolkit/audio/recorder.rs`; Zeilen beziehen sich auf den vorliegenden Stand.

**Voraussetzungen:**

- `default_always_on_microphone()` ist `false` ([settings.rs:824](/C:/Users/wolff/local-voice-project/apps/local-voice/src-tauri/src/settings.rs:824)).
- **Auch `lazy_stream_close` ist standardmäßig `false`** (`settings.rs:511,1390`). Dann schließt OnDemand nach jedem Stop; ein Start innerhalb von 30 Sekunden ist deshalb nicht automatisch warm (`M:763`).
- Mit aktiviertem Lazy Close bleibt der Stream 30 Sekunden offen (`M:20,449`). Der nächste Start entwertet den Schließauftrag; State-Lock und Generation schützen vor gleichzeitigem Schließen (`M:455,663`).
- AlwaysOn öffnet bei Manager-Konstruktion (`M:375`). Ein geschlossener Stream bedeutet dagegen nicht zwingend kaltes VAD oder leere Geräte-/Konfigurations-Caches: Diese überleben das normale Schließen.

| Schritt in Aufrufreihenfolge | Kalt: Stream geschlossen | Warm: Stream offen |
|---|---:|---:|
| Betriebssystem-Hotkey → `TranscribeAction::start` | Unbekannt; vorgelagerter Dispatcher außerhalb des Prüfbereichs | Ebenso |
| Startzeit/Log, Meeting-Prüfung, Manager-Zugriffe (`A:516–534`) | ca. <1–2 ms | gleich |
| ASR-Laden anstoßen; Thread für `preload_vad` starten (`A:537–544`) | ca. 0,1–5 ms Aufrufkosten; Hintergrundarbeit überlappt | gleich |
| Tray auf Recording setzen (`A:548–550`) | ca. <1–20 ms; UI-Stau darüber möglich | gleich |
| Settings, Modellinfo, VAD-Policy; ggf. `tm.start_stream()` (`A:553–578`) | ca. <1–10 ms ohne Lock-Stau; aufgerufene Interna hier nicht untersucht | gleich |
| Overlay anzeigen bzw. überspringen, Timing-Logs (`A:583–598`) | None: nahezu 0; sonst ca. 1–30 ms, Ausreißer möglich | gleich |
| AlwaysOn: Feedback-Thread starten; OnDemand: direkt weiter (`A:601–622`) | OnDemand: nahezu 0 | ca. 0–2 ms |
| `try_start_recording`: State-/Mode-Locks, Idle-Prüfung, Close-Generation (`M:663–670`) | typ. <1 ms; laufendes Schließen kann blockieren | typ. <1 ms |
| `start_microphone_stream`: Open-Lock, ggf. alte Ausgangsstummschaltung restaurieren, Settings (`M:537–562`) | typ. <1–3 ms; Restaurierung nur Sonderfall | sofortiger Return bei `is_open`, typ. <1 ms |
| Gerätewahl (`M:564`, `device.rs:10`) | Default/Cache: ca. <1–2 ms; benanntes Gerät ohne Cache: grob 5–100 ms | entfällt |
| VAD sicherstellen (`M:569`) | vorhanden: <1 ms; sonst verbleibender Anteil der Initialisierung, grob 10–200+ ms | entfällt |
| `open`: Kanäle, Host, ggf. Default-Gerät, Klone, Worker starten (`R:139–164`) | grob 1–15 ms | entfällt |
| Worker: Gerätename, Konfigurations-Cache bzw. Default-/Supported-Configs, Geräte-Log (`R:168–193,403`) | Cache: ca. <1–5 ms; Miss: grob 5–100 ms | entfällt |
| `build_input_stream`, danach `play()` (`R:195–246,395`) | Build grob 10–150+ ms; Play ca. <1–20 ms | entfällt |
| Init-Antwort; `open` wartet darauf, Manager setzt Open-Flag (`R:266,294`; `M:593`) | ca. <1–3 ms zusätzlich; Wartezeit enthält vorstehende Worker-Schritte | entfällt |
| Recorder-Lock, `Cmd::Start` senden, Manager setzt Recording (`M:677`; `R:319`) | ca. <1–2 ms; **keine Consumer-Bestätigung** | gleich |
| Parallel nach Play: Gerät füllt ersten Block → CPAL-Callback (`R:359`) | grob 5–30 ms bei unkompliziertem Gerät; USB/BT/Aufwachen ggf. 50–200+ ms | Samples fließen bereits; nächster Block nach 0–einer Pufferperiode |
| Callback: Mono-/Float-Konvertierung, Kopie, Channel-Send (`R:369–388`) | typ. <1 ms pro kleinem Block | gleich |
| Consumer initialisiert Resampler/Visualizer, empfängt Block (`R:527–601`) | Initialisierung grob <1–5 ms, überlappt Callback-Anlauf | bereits initialisiert |
| Commands **vor** aktuellem Block: Recording aktivieren, Puffer/Resampler/VAD zurücksetzen (`R:607–628`) | typ. <1–3 ms; Wartezeit auf Block bereits oben enthalten | ebenso, ggf. bis nächste Pufferperiode |
| Visualizer/Level-Callback, Resampling, erste 30-ms-Frames (`R:717–734`) | ca. <1–5 ms Rechenarbeit plus Frame-Füllung; genaue Resampler-Latenz hier offen | gleich |
| VAD-Onset, danach `processed_samples` und Audio-Callback (`R:577–594`) | zwei positive 30-ms-Frames; bei sofort positiver Klassifikation etwa 60 ms Audio plus Verarbeitung | gleich |

**Nicht alles addieren:** VAD-Preload, Consumer-Anlauf und Geräte-Callbacks überlappen. Beim Warmstart existieren physische Samples schon vor dem Hotkey; entscheidend ist, welche davon in der Diktataufnahme landen.

Grobe Größenordnung bis zum ersten für die Aufnahme verwendeten Rohblock: warm wenige bis einige zehn Millisekunden; kalt einige zehn bis mehrere hundert Millisekunden, bei langsamen Geräten darüber. Die nachgelagerten **100 ms Feedback-Sleep blockieren den Aufnahmestart nicht** (`A:635–641`).

Bei fehlgeschlagener Öffnung folgen Cache-Invalidierung, erneute Geräteauflösung und ein zweiter `open`-Versuch (`M:575–583`): zusätzlicher, nicht fest begrenzter Fehlerpfad.

## 2. Die drei wahrscheinlich größten Latenztreiber

1. **WASAPI-/Geräteanlauf einschließlich Konfigurationsabfrage und erstem Callback.**  
   `open()` wartet auf Worker-Initialisierung bis nach `play()`, aber nicht auf Samples. Build-/Play-Kosten bleiben auch bei Cache-Hits bestehen. Belege: [recorder.rs:168](/C:/Users/wolff/local-voice-project/apps/local-voice/src-tauri/src/audio_toolkit/audio/recorder.rs:168), `:243`, `:294`, `:359`.

2. **Erstinitialisierung des Silero-VAD als Aufnahmevoraussetzung.**  
   Der Hintergrund-Preload hält den Recorder-Mutex während der Konstruktion. Der synchrone `preload_vad()` im Öffnungspfad wartet gegebenenfalls darauf. „Parallel gestartet“ bedeutet deshalb nicht „außerhalb des kritischen Pfads“. Auch bei `VadPolicy::Disabled` wird der Recorder mit Silero konstruiert. Belege: [audio.rs:515](/C:/Users/wolff/local-voice-project/apps/local-voice/src-tauri/src/managers/audio.rs:515), `:569`, `:278`.

3. **Serielle Vorarbeit vor `try_start_recording`: Tray, Settings/Streaming-Plan und Overlay.**  
   Diese Aufrufe verschieben sowohl kalte als auch warme Aufnahmeaktivierung. Insbesondere UI-/Lock-Ausreißer können relevant sein. Beleg: [actions.rs:548](/C:/Users/wolff/local-voice-project/apps/local-voice/src-tauri/src/actions.rs:548).

Die Rangfolge bleibt eine Messhypothese. Eine ungecachte Geräteenumeration (`M:428`; `device.rs:12–17`) kann Platz 2 oder 3 überholen. Nach der ersten Recorder-Konstruktion entfällt normalerweise die VAD-Ladezeit.

## 3. Verliert SmoothedVad den Wortanfang?

**Beim erfolgreichen Onset werden vorherige Frames nachgeliefert.**

- Jeder Frame wird **vor der Klassifikation** gepuffert; maximal `prefill_frames + 1` Frames bleiben erhalten.
- Die Konstanten sind **15 Prefill-Frames und 2 Onset-Frames**; ein Frame umfasst 30 ms.
- Der erste positive Frame liefert zunächst `Noise`, bleibt aber im Puffer. Beim zweiten aufeinanderfolgenden positiven Frame wird der gesamte Puffer als `Speech` ausgegeben.
- Das sind maximal **450 ms vor dem auslösenden Frame plus dessen 30 ms**, insgesamt 480 ms. Belege: [smoothed.rs:42](/C:/Users/wolff/local-voice-project/apps/local-voice/src-tauri/src/audio_toolkit/vad/smoothed.rs:42), `:53–69`; `vad/mod.rs:3–6`.
- Bei direkt aufeinanderfolgenden positiven Frames stehen damit maximal **420 ms vor dem ersten positiven Frame** zur Verfügung.
- Ein leiser Anfang passt hinein, wenn er bei bestätigtem Onset noch im Puffer liegt. Bei länger verzögerter Erkennung werden ältere Frames verdrängt; ohne zwei aufeinanderfolgende positive Frames wird die Passage überhaupt nicht ausgegeben. Standard-Schwelle: 0,3 (`settings.rs:927–941`; `silero.rs:46`).

**Die wesentliche Grenze:** Bei `recording == false` kehrt `handle_frame` vor dem VAD zurück (`R:573`). `Cmd::Start` leert außerdem Resampler- und VAD-Zustand (`R:620–628`). Es gibt daher **keinen fortlaufenden Aufnahme-Pre-Roll**, auch nicht bei AlwaysOn. Das vorhandene Prefill schützt ausschließlich bereits in die aktive Aufnahme eingespeiste Frames.

## 4. Lösungsmöglichkeiten mit kleinem Umfang

**Ein echter Kaltstart lässt sich unter diesen Bedingungen nicht praktisch auf null reduzieren:** Wenn das Mikrofon bis zum vollständigen Hotkey geschlossen bleibt, fehlen die während der Öffnung gesprochenen Samples unwiederbringlich. Dafür braucht es ein früheres Öffnungssignal oder eine bereits laufende Erfassung.

| Variante | Vorteil | Nachteil / Umfang |
|---|---|---|
| Aufnahme in `actions.rs` vor Tray/Overlay aktivieren | Kleiner Diff; reduziert die tatsächliche Kaltstartlücke | Geräteanlauf bleibt; Streaming-Router muss vor Audioausgabe bereit sein |
| 300–500 ms Ringpuffer im Recorder vor dem Recording-Gate | Rettet bei offenem Stream während der Vorarbeit verworfene Samples | Rettet keine Samples vor Streamöffnung; Änderungen an `Cmd::Start`, Zeitzuordnung und Reset nötig |
| Vorwärmen beim Modifier-Druck | Kann Geräteanlauf vor den vollständigen Hotkey verlagern | Kein entsprechender Eingang im geprüften Pfad; nötiger Shortcut-Hook wäre außerhalb des erlaubten Umfangs; Vorlauf nicht garantiert |
| Lazy Close aktivieren, Timeout etwa 30 → 120 s | Sehr kleiner Umfang; Wiederholungsdiktate häufiger warm; weiterhin endliches Schließen | Erster Start bleibt kalt; Mikrofon bleibt nach Stop länger aktiv |
| Stream während sichtbarer App-Fenster offen halten | Aufnahme bei sichtbarer App kann warm sein | Kein vorhandener Sichtbarkeits-Lebenszyklus im geprüften Pfad; bei dauerhaft sichtbarem Fenster dauerhaft offen |

**Meine Empfehlung für den minimalen Diff: Lazy Close aktivieren und auf 120 Sekunden verlängern.** Das ist eine klar begrenzte Verbesserung häufiger Folgestarts, **keine Beseitigung echter Kaltstarts**.

Konkret: `M:20` ändern und das vorhandene `lazy_stream_close` nutzen. Falls neue Installationen das standardmäßig erhalten sollen, müssen Feld-Default und Settings-Konstruktor konsistent angepasst werden; gespeichertes `false` bleibt andernfalls unverändert. Generation-/State-Sicherung beibehalten. Eine ehrliche Kaltstartgarantie ist innerhalb dieses Prüfbereichs und ohne früheres Öffnungssignal nicht möglich.

## 5. Vorher/Nachher sauber messen

**Vorhandene Logs gemeinsam auswerten:**

- `TranscribeAction::start called`, `start-path pre-recording steps`, `TranscribeAction::start completed` — Vorarbeit/Gesamtdauer (`A:517,592,680`).
- `device resolve: enumerate/cache hit`, `mic stream breakdown`, `mic worker init` — Auflösung, VAD-Warten, Config/Build/Play (`M:421,439,587`; `R:248`).
- `Microphone stream initialized`, `Recording started in` — Initialisierung bzw. Start-Aufruf, **kein Nachweis erster Samples** (`M:600`; `A:624`).
- `Cmd::Start processed`, `first audio chunk arrived`, `first captured chunk ... processed` (`R:611,711,739`).
- `Closing idle microphone stream` — tatsächlichen Kaltzustand feststellen (`M:465`).

**Messlücken:** „first audio chunk arrived“ wird erst im Consumer geloggt, nach Initialisierung und Command-Verarbeitung; sein Startzeitpunkt liegt nach dem Init-Send, nicht exakt bei Play. „first captured chunk“ beweist weder einen vollständigen 30-ms-Frame noch eine VAD-Ausgabe.

Zusätzlich wären nötig:

1. Gemeinsame Session-ID und monotone Zeitmarke ab `A:516`, mitgeführt bis Callback/Consumer; für den tatsächlichen Tastendruck eine externe synchronisierte Referenz.
2. Im CPAL-Callback Zeitpunkt und Capture-Zeitstempel aus `InputCallbackInfo` erfassen — dieser wird derzeit ignoriert (`R:359`). Messdaten gepuffert weiterreichen, keine blockierenden Callback-Logs.
3. Erstes behaltenes Rohsample, erster Resampler-Frame und erste VAD-Ausgabe getrennt markieren; Sample-Positionen vor/nach VAD vergleichen.
4. Identische Testphrase mit definiertem leisen Anlaut bei 0/50/100/200 ms nach Hotkey abspielen; Rohsignal und Ergebnis auf fehlende Anfangsdauer prüfen.
5. Je Variante mindestens 30 Wiederholungen: erster Prozessstart, geschlossener Stream mit warmen Caches, offener Stream; Gerät, VAD-Policy und Lazy-Close-Zustand festhalten. Median, p95 und verlorene Anfangs-Millisekunden berichten.

So lassen sich **verspätete Erfassung**, **Verwerfen vor `Cmd::Start`** und **VAD-bedingtes Abschneiden** getrennt nachweisen.