# Local Voice AI — Weiterentwicklungsplan (Stand 2026-08-24, ab v0.13.0)

Analyse und Etappenplan für Features, Design und die Rückkopplung ins
WAI-Design-System. Jede Etappe endet mit einem anfassbaren Artefakt und
EINEM gebündelten Release (Tag `app-v*`).

## 1. Standortbestimmung

**Was die App heute kann** (Tauri 2, Rust + React, vollständig lokal):

| Bereich | Stand |
|---|---|
| Diktat | Global-Shortcut, VAD, Whisper (ggml) + ONNX-Modelle (Parakeet u. a.), Einfügen per Clipboard/Paste, Post-Processing per LLM |
| Meetings (M8) | Mikrofon + System-Audio (Loopback), Streaming-WAV, Protokolle, Re-Transkription, Export/Import, Aufbewahrung |
| Vorlesen (TTS) | Quellen-Menü, Reiter-Sitzungen, Player |
| Modelle | Katalog, Download, HF-Cache-Scan, Modell-Optionen je Karte |
| Plattformen | Windows (Vulkan, NSIS, portable), macOS NEU (Metal, DMG aarch64 + x86_64), Linux teilweise |
| Sonstiges | Updater (latest.json), 24 Sprachen, Tray, Overlay, Onboarding, CLI-Flags |

**Plattform-Lücken macOS** (frisch portiert, v0.12.1–v0.13.0):
- System-Audio-Mitschnitt fehlt (Loopback ist WASAPI/Windows-only; Meetings laufen mic-only).
- DMG ist nur ad-hoc signiert: erster Start per Rechtsklick → Öffnen, Gatekeeper warnt.
- Intel-Build (x86_64) ist Cross-Compile und noch auf keinem echten Intel-Mac verifiziert.
- Kein CoreML/ANE-Pfad für die ONNX-Modelle; Whisper nutzt Metal (ggml), das ist gut,
  aber die ONNX-Familie läuft CPU.

**Technische Schulden:**
- Upstream-Subtree (Handy) driftet; UPSTREAM.md pflegt die Grenze, aber jede
  Portierung (siehe cpal-!Send-Fix in `mic_capture.rs`) ist Handarbeit.
- `docs/KNOWN-LIMITATIONS.md` und die m*-Evidence-Ordner wachsen schneller als
  sie konsolidiert werden.
- App-`theme.css` spiegelt WAI-Tokens von Hand — keine Drift-Prüfung gegen den
  DS-Master (Abschnitt 4 behebt das).

## 2. Leitplanken

- Einstellungen wandern in bestehende Tabs, nie in neue Oberflächen (AGENTS.md).
- Releases bündeln: erst Etappe fertig, dann EIN Tag.
- Aufwand je Etappe vorab in kTok schätzen; bei 150 % harter Stopp und Lagebericht.
- MLX & Co.: neue Inferenz-Backends nur mit messbarem Nutzen gegen die Baseline
  (Metal-ggml). Liegt die Baseline gleichauf → abbrechen, dokumentieren, weiter.

## 3. Etappen

### E1 — macOS erstklassig (v0.14.x) · Schätzung ~150 kTok
1. **System-Audio für Meetings auf macOS** über ScreenCaptureKit
   (`SCStream` Audio-Tap, ab macOS 13): `LoopbackCapture`-Gegenstück hinter dem
   bestehenden `#[cfg]`-Schnitt in `audio_toolkit/audio/loopback.rs`.
2. **Signierung + Notarisierung**: Apple-Developer-Zertifikat einrichten
   (Secrets `APPLE_CERTIFICATE`, `APPLE_ID`, … — die Schritte stehen fertig im
   geerbten `build.yml`), dann entfällt der Rechtsklick-Start. *Entscheidung
   Patrick: Developer-Account (99 $/Jahr) ja/nein.*
3. **Intel-Verifikation**: x86_64-DMG auf dem MacBook Pro testen (Diktat,
   Meeting mic-only, TTS); Ergebnis in KNOWN-LIMITATIONS.md.
4. **Prüfauftrag CoreML** (klein): ONNX-Runtime CoreML-Execution-Provider für
   Parakeet messen. Nutzenkriterium: ≥ 1,5× Realtime-Faktor vs. CPU, sonst verwerfen.
   MLX bleibt außen vor, solange Metal-ggml nicht der Engpass ist.

**Artefakt:** notarisierte DMGs (arm + Intel) + Vergleichstabelle CoreML/CPU.

### E2 — Meetings vertiefen (v0.15.x) · ~200 kTok
1. **Sprecher-Trennung (Diarisierung)**: ONNX-Diarisierungsmodell (z. B.
   pyannote-Community-ONNX) als optionaler Nachbearbeitungsschritt; Sprecher-Labels
   in Protokoll und Untertitel-Export.
2. **Live-Transkript** während der Aufnahme (Chunker existiert) als Seitenpanel.
3. **Protokoll-Qualität**: Abschnitts-Zusammenfassungen + Aufgabenliste über den
   vorhandenen Post-Processing-LLM-Pfad, Prompt-Vorlagen je Meeting-Typ.

**Artefakt:** ein echtes Meeting-Protokoll mit Sprecherlabels als Screenshot/Export.

### E3 — Vorlesen ausbauen (v0.16.x) · ~150 kTok
1. Stimmen-Verwaltung (Fish-Speech-Stimmen importieren, benennen, Standard je Sprache).
2. Satz-Synchronisation: mitlaufende Hervorhebung im Text, Klick springt zum Satz.
3. Export als Audiodatei (WAV/MP3) inkl. Kapitelmarken aus Überschriften.

**Artefakt:** vorgelesenes Dokument mit Highlight-Video/GIF + exportierte MP3.

### E4 — Design & UX nach WAI-Sprache (v0.17.x) · ~180 kTok
Grundlage existiert (`src/styles/theme.css`, Gelb auf Ink, Logo-Regeln). Vertiefen:
1. **Akzent-Inseln** konsequent: CTAs, Featured-Icons und Kennzahlen-Kacheln als
   dunkle Inseln mit gelbem Inhalt auf neutralem Grund; kein Gelb-auf-Hell ohne Ink.
2. **Media-Controls**: der Vorlese-Player übernimmt das `.mediabar`/`.mbtn`-Muster
   des DS (ein Primär-Schalter, Glyph-Wechsel Play/Pause) statt generischer Buttons.
3. **Icon-Konsolidierung**: durchgängig Lucide (`lucide-react` ist schon da),
   Eigenbau-Icons (`components/icons/*`) nur wo semantisch nötig (Logo, Aufnahme).
4. **A11y-Pass** nach ui-ux-pro-max-Checkliste: Fokus-Ringe, 4,5:1-Kontraste in
   beiden Themes, `prefers-reduced-motion`, Touch-Ziele ≥ 44 px (Overlay!),
   `cursor-pointer` auf allen klickbaren Karten.
5. **Onboarding** als geführte drei Schritte (Mikrofon → Modell → Probediktat)
   im Akzent-Insel-Stil.

**Artefakt:** Vorher/Nachher-Screenshots aller Hauptansichten in Light + Dark.

### E5 — Rückkopplung ins Design-System (parallel zu E4) · ~80 kTok
Das DS ist Quelle der Wahrheit; die App ist sein erster Desktop-Konsument.
1. **App als Downstream-Kopie registrieren**: `apps/local-voice/src/styles/theme.css`
   in `check_token_drift.py` aufnehmen — Token-Abweichungen fallen dann mechanisch auf.
2. **Neuer DS-Layer „Desktop-App (Tauri)"** in `references/`: Tray-/Overlay-Muster,
   Fensterchrome, Einstellungs-Listen (SettingsGroup/SettingContainer), Verhalten
   bei Systemthemen — als `references/desktop-app.md` + Katalog-Sektion
   (`wai-portal/config/designsystem.php` + Partial + Testzahl, wie im Skill beschrieben).
3. **Neue Bausteine zurückspielen statt forken**: Wellenform-/Pegel-Anzeige und
   Aufnahme-Status-Badge entstehen zuerst als DS-Komponente (portal.css + Katalog),
   die App übernimmt Markup/Klassen.
4. Nach jeder DS-Änderung: `check_token_drift.py`, `check_catalog_coverage.py`,
   Katalog-Sichtprüfung — erst dann „fertig".

**Artefakt:** Katalog-Sektion „Desktop-App" live unter `/admin/designsystem`.

## 4. Reihenfolge und nächste Schritte

Empfohlene Reihenfolge: **E1 → E4+E5 → E2 → E3.** Begründung: E1 macht die neue
Plattform belastbar, solange der Portierungskontext frisch ist; E4/E5 sind
sichtbar und tragen jede Demo; E2/E3 bauen auf stabilem Fundament.

**Sofort (vor E1):**
1. v0.13.0-Release abnehmen: DMG auf M-Mac (Icon!) und Intel-Mac testen,
   Windows-Update von 0.12.x prüfen.
2. Entscheidung Apple-Developer-Account (blockiert E1.2).
3. `theme.css` in den Token-Drift-Check aufnehmen (E5.1, 30 Minuten, sofort möglich).
