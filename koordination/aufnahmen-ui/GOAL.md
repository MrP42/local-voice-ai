---
thema: aufnahmen-ui
titel: Aufnahmen-Oberflaeche: Spalten, Sessions, eine Scrollbar
state: READY
vorzustand: -
pausengrund: -
issue: 64
repo: MrP42/local-voice-ai
branch: feat/aufnahmen-ui
iteration: 0
erstellt: 2026-09-30
aktualisiert: 2026-09-30T20:00
---

# Goal: Aufnahmen-Oberflaeche: Spalten, Sessions, eine Scrollbar

## Zielzustand
Die Seite „Aufnahmen“ arbeitet wie das Vorlesen-Modul in Spalten: links Sessions (Ordner wie „Privat“, „Geschäftlich“, Themen) mit ihren Besprechungen, in der Mitte der Inhalt (Transkript, Notizen, KI-Notizen, Protokoll), rechts eine kompakte Bedienspalte (Aufnahme/Import, Wiedergabe, Symbol-Zeile mit Tooltips, Menü „☰“, Fortschritt) – jede Spalte scrollt für sich, die Seite selbst nie. Erkennbar im Installer: auch im kleinen Fenster neben einem Meeting-Client lässt sich live mitschreiben, Details stehen kompakt bzw. im Dialog statt in großen Statusblöcken, und Breiten, Sessions und Ansicht bleiben über Neustarts erhalten.

## Scope
- Ist-Analyse und UI-Audit der Seite Aufnahmen (Liste, Aufnahmekarte, Detail mit Status/Neu-Transkription, Reiter Notizen/Transkript/Protokoll, Chat, Export, Follow-up, Fortschritt aus P8a) gegen das Vorlesen-Modul (#61/#62) und gängige Muster (Granola, Otter, Notion AI Meeting Notes, Apple Notizen) → `koordination/aufnahmen-ui/UI-AUDIT.md`, Entwurf mit Varianten (Klick-Prototyp/Screenshots) vor dem Bau.
- Spaltenlogik: Sessions-Spalte | Inhalt | Bedienspalte; Breiten per Ziehgriff, ein-/ausklappbar; Anordnung der Inhalte (z. B. Transkript rechts, Notizblock Mitte) als Entscheidung nach Prototyp.
- Sessions: anlegen, umbenennen, sortieren, löschen; Besprechung in Session verschieben (Ziehen und Menü); Verhältnis zu den M4-Ordnern klären (Sessions = Ordner der obersten Ebene, keine zweite Struktur).
- Kompakte Details: Status, Quelle, Dauer, Einwilligung, Modell als Chips/Symbolzeile; Vollinfo im Dialog; „Neu transkribieren“ und seltene Aktionen ins Menü „☰“.
- Eine Scrollbar je Bereich: keine verschachtelten Scrollflächen, die Seite scrollt nicht; Bedienelemente bleiben stehen.
- Live und Import gleich komfortabel: Aufnahme starten, Notizen tippen, Live-Transkript mitlaufen lassen, Datei hineinziehen – alles ohne Seitenwechsel.
- Responsive: unter einer Mindestbreite stapeln sich die Spalten (Reiter oder Klappbereiche), Kompaktmodus für ein schmales Fenster neben dem Meeting.
- Persistenz: Breiten, eingeklappte Spalten, gewählte Session, letzter Reiter, Layout – dauerhaft und je Nutzer über alle Sessions hinweg individualisierbar.
- Tests (Playwright gegen die Tauri-Attrappe inkl. Viewport-Matrix), Hilfe-Text der Seite, Abnahme-Installer.

## Non-Scope
- Neue Besprechungsfunktionen (KI, STT, Sprecher, Kalender) – nur ihre Anordnung; Fehler dort als Befund ins Granola-Folge-Goal.
- Vorlesen-, Verlauf-, Modelle-, Einstellungsseite (außer gemeinsam genutzte Komponenten wie Spaltengriff, Tooltip, Menü).
- Neuer Eintrag in der Hauptnavigation oder neuer Einstellungsreiter.
- Mobile Apps.

## Akzeptanzkriterien
- [x] AK1 — Audit + Entwurf: `koordination/aufnahmen-ui/UI-AUDIT.md` (Element · Ist · Soll · Fundstelle · Schwere) und ein Entwurf mit mindestens zwei Spaltenvarianten als Screenshot/Artefakt; Patrick hat eine Variante gewählt (Entscheidung in GOAL.md).
- [ ] AK2 — Spalten: Playwright → Seite Aufnahmen hat drei Bereiche (`data-testid="rec-sessions"`, `rec-content`, `rec-controls`); Griffe (`role="separator"`, Pfeiltasten, Doppelklick = Standard) ändern die Breite in Grenzen; ein-/ausklappbar; Zustand übersteht Neuladen.
- [ ] AK3 — Sessions: anlegen, umbenennen, löschen (mit Rückfrage, Besprechungen bleiben erhalten), Besprechung per Ziehen und per Menü verschieben; Filter/Suche wirken innerhalb der gewählten Session; Datenmodell nutzt die vorhandenen Ordner (Migrationstest).
- [ ] AK4 — Eine Scrollbar: Viewports 1920×1050, 1366×768, 900×700, 480×800 → `document.scrollingElement.scrollHeight <= innerHeight + 1`; innerhalb jeder Spalte höchstens eine scrollbare Fläche entlang eines Wegs (Playwright prüft verschachtelte `overflow:auto` mit Überlauf).
- [ ] AK5 — Kompakt: Detailkopf (Titel, Status-Chips, Symbolzeile) ≤ 120 px hoch bei 1366×768; Vollinfo per Dialog; „Neu transkribieren“ im Menü; Tooltips mit Name + Kurzerklärung (Maus und Tastatur).
- [ ] AK6 — Live im kleinen Fenster: bei 480×800 lässt sich Aufnahme starten, eine Notiz tippen und das Live-Transkript sehen, ohne zu scrollen oder die Seite zu wechseln (Playwright-Ablauf).
- [ ] AK7 — Import: Datei in die Inhaltsspalte ziehen oder über die Bedienspalte wählen → Import startet in der aktuellen Session, Fortschritt (P8a) sichtbar an der erwarteten Stelle.
- [ ] AK8 — Persistenz: Breiten, Klappzustand, gewählte Session, Reiter und Layout überstehen Neuladen, Seitenwechsel und App-Neustart (localStorage bzw. Settings) – Playwright.
- [ ] AK9 — Barrierefreiheit: Tastaturbedienung aller Spaltenaktionen, sichtbarer Fokus, `aria`-Rollen; axe-Prüfung ohne kritische Befunde.
- [ ] AK10 — Anfassbar: Vorher/Nachher-Screenshots (breit, schmal, gestapelt, Dialog, Menü) als Artefakt; Installer mit Patch-Version +1 gebaut und von Patrick abgenommen.

## Quality Gates
- [ ] QG1 — Typen: `cd apps/local-voice && npx tsc --noEmit` → Exit 0.
- [ ] QG2 — Gesamte Playwright-Suite grün (Port je Checkout automatisch).
- [ ] QG3 — Rust unverändert grün, falls Backend berührt (`cargo test --lib`).
- [ ] QG4 — i18n: neue Schlüssel in de und en, echte Umlaute (`tools/check_i18n_meetings.py` bzw. Pendant).
- [ ] QG5 — Lint/Format nur berührte Dateien (eslint 0 Fehler, prettier grün); vorbestehendes Rot bleibt.
- [ ] QG6 — Doku: Hilfe-Text der Seite und `docs/BESPRECHUNGEN.md` angepasst; Handoff geschrieben.
- [ ] QG7 — Budget: Schätzung 2,5 MTok (Spanne 2–3,5), Meldung bei 50 % und 80 %, harter Stopp bei 150 %.

## Constraints
- Baut auf `feat/granola-besprechungen` (inkl. #62 und Runde 3: P8a Fortschritt, P1k Protokoll-Vorlagen) auf; Start erst nach Merge von Runde 3 in diesen Branch.
- Einstellungen nur an vorhandenen Stellen; keine zweite Ordnerstruktur neben M4-Ordnern.
- Abnahme nur per Installer, Patch-Version +1; keine Installation durch Agenten über Patricks App.
- Parallele Arbeit anderer Zugänge (AGENTS.md): Branch + PR, keine Formatierläufe über fremde Dateien.

## Architekturprinzipien
- Gemeinsame Bausteine mit dem Vorlesen-Modul wiederverwenden (Spaltengriff, Klappleiste, Symbol-Knöpfe, Tooltip, Menü „☰“) statt neu zu bauen; Abweichungen begründen.
- Scrollen nur im Inhalt; Kopf- und Bedienleisten sind feste Flächen (CSS-Grid mit `min-height:0`).
- Zustand der Oberfläche lokal (localStorage), Daten (Sessions/Ordner) im Store.

## Dependencies
- Granola-Goal Runde 3 (P8a, P1k) gemergt; Vorlesen-Bausteine aus #62.

## Risiken / Owner-Entscheidungen
- R1 Anordnung Transkript/Notizen (Mitte oder rechts) – Vorschlag: zwei Varianten als Klick-Prototyp, Patrick wählt (AK1). Owner: Patrick.
- R2 Sessions = oberste Ordnerebene der M4-Ordner (keine zweite Struktur) – Vorschlag: ja. Owner: Patrick.
- R3 Budget 2,5 MTok (Spanne 2–3,5) – Vorschlag: so. Owner: Patrick.

## Meilensteine
| M | Ergebnis (anfassbar) | Status |
|---|---|---|
| M1 | UI-Audit + Klick-Prototyp mit 2 Varianten (Artefakt) | offen |
| M2 | Spaltengerüst mit einer Scrollbar, Griffe, Persistenz | offen |
| M3 | Sessions (Ordner) links, Verschieben, Filter | offen |
| M4 | Kompakter Detailkopf, Symbolzeile, Menü, Dialog | offen |
| M5 | Live/Import im kleinen Fenster, responsive Stapeln | offen |
| M6 | Tests, Hilfe, Installer, Abnahme | offen |

## Evidence
- 2026-09-30T20:00 AK1 erfüllt — U1 c3e6e4ec: UI-AUDIT.md (34 Zeilen), Prototyp A/B https://claude.ai/artifact/3RaBLbuyHDX1WnQtAvMQJS, Patrick waehlt B (30.09.)

## Blocker
-

## Entscheidungen
- 2026-09-30 Patrick am Prototyp (https://claude.ai/artifact/3RaBLbuyHDX1WnQtAvMQJS): R1 = Variante B (Sessions | Notizen/KI-Notizen/Protokoll | rechts Bedienung + Transkript/Fragen); Besprechung in mehreren Sessions (Ziehen verschiebt, Strg+Ziehen fügt hinzu); Sessions eine Ebene; Aufnahme-Optionen in den Einwilligungs-Startdialog; Kalender als „Als Nächstes“ unten in der Sessions-Spalte.
- 2026-09-30 Patrick: R2 ja (Sessions = oberste Ebene der M4-Ordner), R3 Budget – gemeinsam mit YouTube-Bündel 1 aus #66 zusammen 3,5 MTok (Aufnahmen-UI ~2,5); R1 (Anordnung) wählt Patrick am Klick-Prototyp (AK1). Zusätzlich B17 aus #59: Neu-Transkription tauscht die Fassung erst am Ende.
- 2026-09-30 Patrick: nächstes Goal nach Granola = Aufnahmen-Oberfläche nach Vorbild Vorlesen (Spalten, Sessions/Ordner, kompakte Details, eine Scrollbar, responsive, persistent).

## Nächste empfohlene Aktion
Nach Merge von Granola-Runde 3: Branch auf `feat/granola-besprechungen` nachziehen, R1–R3 von Patrick bestätigen lassen, `goal.py set --state READY`, dann M1 (Audit + Prototyp).

## Verlauf
- 2026-09-30T16:41 DISCOVERY — Goal State angelegt
- 2026-09-30T16:42 DISCOVERY (Runde 0) — Metadaten: issue=64
- 2026-09-30T19:27 READY (Runde 0) — Owner-Entscheidungen 30.09. eingetragen, Rahmen 3,5 MTok gemeinsam

