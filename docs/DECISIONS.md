# Entscheidungen

Evidenzklassen: **OBSERVED** (selbst geprüft) · **SOURCE-CLAIM** · **INFERRED** · **UNKNOWN**

## D1 — Fork von `cjpais/Handy` statt Eigenentwicklung
**Datum:** 2026-07-28 · **Status:** entschieden

Nach der gewichteten Bewertung (funktionale Passung 25 %, Lizenz 20 %, Plattform/HW 15 %,
Wartung 15 %, Codequalität 10 %, Datenschutz 10 %, Erweiterbarkeit 5 %) erreichte Handy als
einziger Kandidat gleichzeitig: vollständiger Hotkey→Cursor-Pfad, verifiziertes MIT, aktive
Multi-Contributor-Wartung, keine Telemetrie, kein Konto, deutsche UI bereits vorhanden.

**Lizenz OBSERVED:** LICENSE-Blob `ff8dfab0159b41263ccc3c50da54007ca6752a22` am Commit
`ea3c20a3a67c7401d8b19198723760da9d40ac45` direkt aus dem Repository gelesen — Stock-MIT-Text,
© 2025 CJ Pais, keine Zusatzklauseln.

Ausgeschlossen wurden u. a. `whisper-writer` und `typewhisper-win` (GPL-3.0), `dictto`,
`voicetypr`, `voquill` (AGPL bzw. Lizenz-Stub ohne Volltext). `blurredmachine/VoiceTyping`
existiert nicht (GitHub 404) — OBSERVED.

## D2 — Rebranding ist Pflicht, nicht Kosmetik
**OBSERVED**, Handy-README am Fork-Commit, wörtlich:
> "Handy is open-source software, but the Handy name, logo, icon, and brand assets are not
> open-source. Unofficial forks, rewrites, and redistributions must use their own branding."

Die MIT-Lizenz deckt den Code, nicht die Kennzeichen. Produktname **Sprechstift**, Identifier
`de.wolffappliedai.sprechstift`. Namensprüfung OBSERVED: npm, crates.io, PyPI je HTTP 404,
GitHub-Repo-Suche 0 Treffer (2026-07-28).

## D3 — Kein Claude-Code-CLI- und kein Codex-CLI-Provider
**Status:** entschieden, nicht implementiert

Der Auftrag erlaubt solche Provider ausdrücklich nur, wenn die geltende offizielle Dokumentation
den Einsatz stützt. Sie tut es nicht.

**Anthropic — ausdrücklich untersagt.** `https://code.claude.com/docs/en/legal-and-compliance`
(abgerufen 2026-07-28), wörtlich:
> "Anthropic does not permit third-party developers to offer Claude.ai login or to route requests
> through Free, Pro, or Max plan credentials on behalf of their users."

Ergänzend Consumer Terms §3: Zugriff "through automated or non-human means, whether through a
bot, script, or otherwise" ist außerhalb von API-Keys untersagt. Eine App, die `claude -p`
startet, ist ein solches Skript.

**OpenAI — nicht verifizierbar.** Die Terms of Use lieferten bei drei URLs HTTP 403; das
Web-Archiv war blockiert. Die geforderte Bedingung ist damit nicht belegbar erfüllt.

**Kosten dieser Entscheidung: keine.** Der generische `OpenAiCompatProcessor` erreicht Anthropic
und OpenAI bereits auf dem ausdrücklich gesegneten Weg — per nutzereigenem API-Key mit Base-URL —
über denselben Codepfad wie Ollama.

⚠️ Anthropics Politik änderte sich 2026 etwa quartalsweise. Vor einem Release neu prüfen.

## D4 — Vulkan hinter ein Cargo-Feature statt hart verdrahtet
**OBSERVED:** Upstream setzt für Windows x86_64 fest `features = ["dynamic-backends","vulkan"]`.
Ohne LunarG Vulkan SDK bricht CMake ab: `Could NOT find Vulkan (missing: Vulkan_LIBRARY
Vulkan_INCLUDE_DIR glslc)`. Der NVIDIA-Laufzeittreiber allein genügt nicht.

Geändert zu Opt-in `--features gpu-vulkan`. Ein frischer Checkout baut damit CPU-only.

**Gemessen (OBSERVED, 2026-07-28, i9-13900K, CPU-Backend `ggml-cpu-alderlake`):**
Parakeet V3 int8, 9,15 s deutsches Audio → Modell laden 1035 ms, Inferenz **392 ms** (≈23×
Echtzeit). Die CPU-Leistung ist für Diktatlängen mehr als ausreichend; die Installation des
Vulkan-SDK ist damit **keine Voraussetzung**, sondern eine Optimierung.

## D5 — Updater und Code-Signatur des Upstream entfernt
Der Upstream-Updater zeigte auf `https://github.com/cjpais/Handy/releases/latest/download/latest.json`
(OBSERVED). Ein Fork darf diese Artefakte nicht laden. Plugin, Abhängigkeit, Capabilities und
Konfiguration wurden entfernt — das beseitigt zugleich einen ausgehenden Netzwerkpfad.
`bundle.windows.signCommand` verwies auf das Azure-Trusted-Signing-Konto von CJ Pais und wurde
entfernt.

## D6 — Sicherheitslage der Abhängigkeiten
**OBSERVED, `cargo audit` 2026-07-28:** 8 Vulnerabilities im geforkten Stand.
Behoben durch Update: `rustls-webpki` 0.103.9→0.103.13 (4 Advisories, u. a. fehlerhafte
CRL-/Name-Constraint-Prüfung) und `tar` 0.4.44→0.4.46 (2 Advisories, u. a. `unpack_in` folgt
Symlinks beim chmod). Beide liegen direkt auf unserem Modell-Download-Pfad.

Verbleibend: `quick-xml` RUSTSEC-2026-0194/0195 (DoS-Klasse), erreichbar nur über `plist`
(macOS-Info.plist bzw. Build-Zeit-Codegen). `plist 1.8.0` pinnt `quick-xml "^0.38"`, die
gepatchte 0.41 ist ohne Upstream-Release nicht wählbar. In `deny.toml` dokumentiert ignoriert.

**Lizenzen OBSERVED:** Auf dem Windows-Zielgraphen kein GPL/LGPL/AGPL und keine Crate ohne
Lizenzfeld. MPL-2.0 (17 Crates, u. a. symphonia) ist dateibasiertes schwaches Copyleft und
verträglich. `cargo deny check licenses` → **ok**.

## D7 — Einfügen ist fail-closed, nicht best-effort
**Datum:** 2026-08-17 · **Status:** entschieden und implementiert

Eine Einfügung, deren Erfolg nicht belegbar ist, wird **nicht versucht**. Statt einer
Fallback-Kette aus mehreren Einfügestrategien (Strg+V, dann Umschalt+Einfg, dann Tippen)
gibt es genau **einen** Versuch mit vorgelagerter Prüfung.

Begründung: Jede weitere Strategie nach einem als fehlgeschlagen vermuteten Versuch
riskiert eine **doppelte** Einfügung, denn der erste Versuch kann trotzdem angekommen sein —
`SendInput` sagt es uns nicht. Doppelter Text ist für den Nutzer schlimmer als gar kein Text
plus ein Hinweis, weil er ihn manuell finden und entfernen muss.

Konsequenz: Die App fügt in manchen Fällen nicht ein, obwohl sie es gekonnt hätte
(z. B. bei nicht abfragbarer Rechtelage des Ziels). Dieser Preis ist bewusst gewählt.
Details in `KNOWN-LIMITATIONS.md`.

## D8 — Live-Injektion: erst gesperrt, dann durch Messung freigegeben
**Datum:** 2026-08-17 · **Status:** revidiert am selben Tag

**Erste Entscheidung (verworfen).** Im Store stand `stream_injection: true` bei
`experimental_enabled: false`, dazu ein Nemotron-Streaming-Modell. Weil
`KNOWN-LIMITATIONS.md` die Funktion als defekt führte, verlangte
`stream_injection_active()` zusätzlich `experimental_enabled`.

**Revision.** Die Dokumentation war überholt. Sie beschrieb den Stand von `32ee6d3`;
die beiden Ursachen wurden **danach** behoben — `6b9143e` ersetzte das fehlerhafte
`enigo.text()` durch Ctrl+V, `d223fa8` reparierte die Präfixberechnung über
Sprechpausen. Nachgemessen hatte das niemand, und der Schalter blieb aus.

**OBSERVED 2026-08-17**, Text jeweils **vor** dem Stopp aus Notepad zurückgelesen:
vollständige Sätze, drei Absätze mit Pausen, Umlaute — alles korrekt, keine
Zeichenwiederholung, keine Duplikate.

Die Zusatzsperre ist damit sachlich unbegründet und entfernt; `stream_injection`
ist wieder ein einfacher Opt-in-Schalter. Die Lehre ist nicht „die Sperre war
falsch", sondern: **eine Einschränkung, die als Text weiterlebt, nachdem ihre
Ursache behoben wurde, kostet die Funktion.** Ein „defekt"-Eintrag braucht ein
Ablaufdatum oder einen Test, der ihn widerlegen kann.

Was blieb: Beim Streaming griff der `paste_guard` nicht (der finale Einfügevorgang
ist unterdrückt), es gab keine Fokusprüfung pro Fragment. Diese Lücke ist mit D14
geschlossen.

## D9 — Transkript-Klartext nur in Debug-Builds
**Datum:** 2026-08-17 · **Status:** entschieden und implementiert

**OBSERVED:** `handy.log` enthielt 264 STREAMDIAG-Zeilen mit vollständigen Diktaten des
Nutzers im Klartext, ausgelöst durch `debug_mode: true`.

Eine Laufzeit-Einstellung ist der falsche Ort für diese Entscheidung: Sie ist in der
Oberfläche mit einem Klick erreichbar, und ihr Zweck („mehr Logs") lässt nicht erkennen,
dass damit Diktatinhalte auf die Platte geschrieben werden. Die Inhaltsprotokollierung
hängt daher an `#[cfg(debug_assertions)]` und existiert im ausgelieferten Binary
schlicht nicht. `debug_mode` steuert weiterhin die Ausführlichkeit, aber nicht mehr die
Preisgabe von Inhalten.

Nicht betroffen: `history.db` und die WAV-Aufnahmen. Das ist die vom Nutzer gewollte
Verlaufsfunktion und unterliegt der Aufbewahrungseinstellung.

## D10 — Die Fehlermeldung trägt das Overlay, nicht das Hauptfenster
**Datum:** 2026-08-17 · **Status:** entschieden und implementiert

Der bestehende Weg für Einfügefehler war ein Toast im Hauptfenster (`paste-error` →
`App.tsx`). Beim Diktieren ist das Hauptfenster aber typischerweise verborgen (Tray-Betrieb,
`start_hidden`) — der Nutzer sieht also gar nichts und hält den Text für verloren.

Die Meldung erscheint deshalb im Overlay-Fenster, das ohnehin „immer oben" liegt. Sie ist
außerdem die **einzige** Overlay-Form, die auch bei `overlay_style: none` gezeigt wird:
Wer die Aufnahmeanzeige abschaltet, will keine Statusanzeige — nicht aber den Hinweis
verlieren, dass sein Text nicht angekommen ist. Der Toast bleibt als Zweitkanal für den
Fall, dass das Hauptfenster offen ist.

## D11 — Rebranding zu „Local Voice AI" mit vollem internen Rename und Datenmigration
**Datum:** 2026-08-19 · **Status:** entschieden

Mit dem Ausbau zur lokalen Sprach-KI (M4-M7: Vorlesen, Stimmen klonen, Übersetzung,
Stimmwechsler) beschreibt der Name „Sprechstift" (Diktierstift) das Produkt nicht mehr.
Umbenannt wurde vollständig: productName, Fenstertitel, Wortmarke und Icons
(Wellenform + KI-Funke), Cargo-Paket `local-voice-ai`, Lib `local_voice_ai_lib`,
Binärdatei `local-voice-ai.exe`, Identifier `de.wolffappliedai.localvoiceai`,
CSS-Präfix `lva-`, Harness-Skripte und Doku.

Der Identifier-Wechsel verschiebt die Tauri-Datenpfade (Settings, Verlauf, mehrere GB
Modelle). Deshalb zieht `appdata_migration.rs` beim Start einmalig die alten Ordner
(Roaming + Local) auf den neuen Namen um — nur wenn der neue noch nicht existiert;
vorhandene neue Daten gewinnen immer (OBSERVED: Unit-Test mit Tempdir, beide Fälle).
Historische Evidence-Dokumente (m2-m7) behalten die alten Namen, weil sie reale
Kommandos von damals protokollieren.

## D12 — Ein stiller Ausfall des System-Kanals wird gemeldet, nicht nur protokolliert
**Datum:** 2026-08-20 · **Status:** entschieden und implementiert (Issue #12)

Stirbt der WASAPI-Loopback-Thread **nach** erfolgreichem Start (Endpoint entfernt,
Treiberfehler), verstummte bisher nur der Callback: Die Besprechung lief weiter, das
halbe Protokoll fehlte, und der Fehler stand allein im Log. Das ist der schlimmste
Fehlermodus des Meeting-Features, weil er wie Erfolg aussieht.

`LoopbackCapture` führt jetzt — wie `MeetingMicCapture` — ein Fehlerflag (`had_error()`)
und gibt Stop- und Fehlerflag über `watch_flags()` heraus. Der Recorder hängt daran einen
schlanken Wachthread (500 ms Takt), der einmalig `MeetingEvent::Error { loopback_died }`
sendet und sich beendet; die Oberfläche zeigt dafür `meetings.errors.loopbackDied`.

Bewusst **kein** Abbruch der Aufnahme: Der Mikrofonkanal ist der wichtigere von beiden,
und eine laufende Besprechung wegen eines toten Zweitkanals zu beenden wäre der größere
Schaden. Gemeldet wird, entschieden wird vom Nutzer.

## D13 — Ein misslungener Transkriptblock kostet nicht das ganze Protokoll
**Datum:** 2026-08-20 · **Status:** entschieden und implementiert (Issue #13)

Der Map-Reduce-Lauf in `minutes.rs` brach bisher beim ersten endgültig
fehlgeschlagenen Block ab — bei einer zweistündigen Besprechung waren damit alle
bereits ausgewerteten Blöcke verloren und der Nutzer sah gar nichts.

Jeder Block hat jetzt ein eigenes Retry-Budget (`CHUNK_ATTEMPTS`, zusätzlich zum
Struktur-Retry innerhalb eines Versuchs). Bleibt er trotzdem erfolglos, wird
**degradiert statt abgebrochen**: Der Merge läuft über die übrigen Blöcke, der
Merge-Prompt nennt die fehlenden Teile ausdrücklich (das Modell soll die Lücke kennen,
nicht überspielen), das Dokument trägt einen sichtbaren Hinweis und
`generation_metadata` hält `chunks_total`/`chunks_failed` fest.

Nur wenn **kein einziger** Block durchkommt, schlägt die Erzeugung fehl — dann gäbe es
nichts zu verdichten. Das vollständige Transkript bleibt in jedem Fall erhalten.

## D14 — Mehrfach-Einfügen (Segment-Modus, Live-Injektion) ist fail-closed mit Puffer
**Datum:** 2026-10-01 · **Status:** entschieden und implementiert (Issues #3 und #9)

D7 regelt den **einen** Einfügeversuch nach dem Stopp. Zwei Betriebsarten fügen aber
mehrfach pro Aufnahme ein: der Segment-Modus (`segmenter.rs`, ging über den alten
`clipboard::paste` ohne jede Prüfung) und die Live-Injektion (`stream_injection`, die
Kontextprüfung hing an `RunState::wants_context()` und damit an `refinement_enabled`,
ohne Refinement prüfte niemand etwas). „Genau ein Versuch" ist dort falsch, und eine
Fallback-Meldung pro Satz wäre unbrauchbar. Beide Pfade teilen jetzt ein Modell
(`paste_session.rs`, reine Zustandsmaschine über einen austauschbaren Desktop):

1. **Ziel** ist das Vordergrundfenster beim ersten Einfügen des Laufs. (Batch erfasst beim
   Stopp, damit der Nutzer noch hinwechseln kann; Live-Text erscheint beim Sprechen, das
   erste Einfügen ist der entsprechende Moment.)
2. **Jedes Fragment** durchläuft dieselbe `paste_guard::preflight` wie der Batch-Pfad:
   gleiches Fenster, Ziel nicht erhöht, nicht abfragbare Rechtelage gilt als erhöht. Der
   Segment-Modus geht zusätzlich durch `paste_transcript_guarded` (Prüfung unmittelbar vor
   und nach dem Tastendruck, Zwischenablage-Rücklesen).
3. **Die erste Abweichung sperrt den Lauf dauerhaft.** Dieses und jedes weitere Fragment
   wird gepuffert statt eingefügt. Das ist Absicht: so bleibt die Reihenfolge, nichts wird
   in ein fremdes Fenster getippt, und es wird nicht mitten in eine Arbeit des Nutzers
   hineingefügt, nur weil er kurz zurückgekehrt ist. Wiederaufnahme wäre Raten.
4. **Ein Hinweis am Ende**, nicht je Satz (Overlay + Toast). Der Rest ist exakt der
   gepufferte Text; der Wortlaut sagt „der Rest liegt in der Zwischenablage", weil der
   frühere Teil im Zielfenster steht. Das Transkript im Verlauf bleibt in jedem Fall
   vollständig.
5. **Ausgabe des Rests:** Wurde für den gepufferten Text nie ein Tastendruck versucht
   (Fokus gewechselt, Ziel erhöht, kein Ziel), bekommt er **einen** weiteren abgesicherten
   Versuch ins Ziel — ist der Nutzer zurück, landet er dort, sonst parkt ihn der Guard in
   der Zwischenablage. Ging ein Versuch mit unbekanntem Ausgang voraus (Tastendruck
   fehlgeschlagen, Fokus wechselte währenddessen), wird **nur geparkt, nie wiederholt**
   (D7: doppelter Text ist schlimmer als fehlender Text mit Hinweis).
6. **Abbruch** (Esc) verwirft den Puffer: der ganze Diktatlauf entfällt, so wie bereits
   Eingefügtes auch nicht zurückgenommen wird.
7. **Segment-Modus, Nebenläufigkeit:** Aufnehmen → Transkribieren → Einfügen eines Segments
   ist serialisiert. `finish` wartet auf das gerade laufende Segment, bevor es den Rest
   liest — sonst wäre der Rest unvollständig und der Schluss könnte vor einem Satz landen.
8. **Nicht-Windows:** Fokus und Rechte sind nicht beobachtbar; die Pfade verhalten sich
   wie vor D14 (wie auch der Batch-Pfad dort).

Nicht gemessen: Das Verhalten wurde bisher mit einem simulierten Desktop getestet, nicht
auf einem echten Windows-Fenstersystem (Issue #9 verlangt eine Messung, wie oft der Fall
auftritt). Die Abnahme am Installer steht aus.
