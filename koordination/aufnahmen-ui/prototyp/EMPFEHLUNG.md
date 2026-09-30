# Empfehlung nach dem Klick-Prototyp (U1/M1, #64)

Prototyp: `aufnahmen-prototyp.html` (im Browser öffnen; oben Variante A/B, Fenster 1920/1366/900/480/frei,
Hell/Dunkel; die grüne Marke prüft laufend „Seite scrollt nicht, nichts verschachtelt“). Belege:
`screens/*.png` und `screens/messwerte.json` (Playwright-Durchlauf, 0 Skriptfehler). Audit: `../UI-AUDIT.md`.

## Empfehlung: Variante B (Transkript rechts), mit zwei Teilen aus A

**B: Sessions | Arbeitsfläche (Notizen · KI-Notizen · Protokoll) | rechts oben Bedienung, darunter Transkript/Fragen.**

Warum:
1. **Live-Mitschreiben ist der Kernfall, und B zeigt Notiz und Transkript gleichzeitig.** Das gilt in jeder Breite,
   auch bei 480×800 (Eingabezeile bei y = 341, Transkript bei 435–675, ohne Scrollen, `B-480-live.png`). Bei A muss man
   zwischen den Reitern Notizen und Transkript wechseln (`A-480-live.png`). Damit wäre AK6 nur mit einem Sonderfall
   erfüllbar.
2. **Quellsprünge ohne Reiterwechsel.** Ein Klick auf eine Quelle in den KI-Notizen markiert den Satz rechts im
   Transkript, und die KI-Notizen bleiben stehen (`B-1366-ki-notizen-quelle.png`). Bei A verschwinden sie hinter dem
   Reiter Transkript.
3. **Bei A bleibt die Bedienspalte meist leer.** Sie belegt 300 px Breite, trägt aber nur ~200 px Inhalt
   (`A-1366.png`), außer der Chat ist offen. Bei B füllt das Transkript diesen Platz.
4. **Wie Granola, aber mit Transkript immer im Blick.** Notizen stehen im Mittelpunkt, das Transkript ist Beleg.
   Der Aufbau passt zum Vorlesen-Modul: Liste links, Arbeitsfläche Mitte, Bedienung rechts oben.

Aus A übernehmen:
- **Reiter „Fragen“ bleibt ein Reiter** neben dem Transkript (B), nicht der Klappbereich aus A. Der Klappbereich
  verdrängt in A die Bedienung.
- **A als Schalter ist nicht nötig.** Wer das Transkript groß braucht (Import, Nachbearbeitung), klappt Sessions
  ein und zieht die rechte Spalte bis 640 px breit. Das spart den zweiten Layoutpfad.

Gemessen im Prototyp (1366×768): Detailkopf 112 px (AK5 ≤ 120). In allen vier Breiten und in beiden Varianten
scrollt die Seite nicht, und keine Scrollfläche steckt in einer anderen. Die Pfeiltaste am Griff ändert die
Breite um 16 px (248 → 280 nach zweimal →). Breiten, Klappzustand, Session, Besprechung, Reiter und Variante
überstehen Neuladen (localStorage). Bei 900 px bricht der Chip-Streifen auf 2 Zeilen um (140 px). In M4 zeigt
deshalb das Quellen-Chip unter 1000 px nur das Symbol.

## Offene Fragen an Patrick (max. 5)

1. **R1 – Variante:** B wie empfohlen, oder doch A (Transkript Mitte)?
2. **Mehrfachzuordnung:** Eine Besprechung kann heute in mehreren M4-Ordnern liegen. Soll die Sessions-Spalte sie
   in jeder dieser Sessions zeigen? Vorschlag: ja, in jeder. Ziehen verschiebt, Strg+Ziehen fügt hinzu. Das Chip
   im Kopf zeigt die erste Session.
3. **Eine Ebene:** Sessions nur als oberste Ebene, ohne Unterordner? Themen liefen dann über Suche oder Tags.
   Vorschlag: eine Ebene in M3, Unterordner erst bei Bedarf.
4. **Startdialog:** Titel, Session, Vorlage, System-Audio und „Mehrere Personen“ wandern in den Einwilligungsdialog
   (ein Klick mehr, dafür ist die Bedienung 160 px kürzer). Einverstanden?
5. **Kalender:** „Als Nächstes“ als kleiner fester Abschnitt unten in der Sessions-Spalte (ein Termin, Knopf
   „Aufnehmen“) statt der heutigen Karte „Anstehende Termine“? Weitere Termine stünden im Menü.

## Paketschnitt M2–M6 (Rahmen Goal ~2,5 MTok, U1 verbraucht ~0,3 MTok)

| M | Paket | Scope | Akzeptanztest (Befehl → Ergebnis) | kTok |
|---|---|---|---|---|
| M2 | Spaltengerüst | `MeetingsSettings` als `PageShell fill` + `workspace-main--fill` für `meetings`; drei Bereiche `rec-sessions`/`rec-content`/`rec-controls`; `ResizeHandle` + Klappleisten; vorhandene Komponenten nur umhängen (Liste links, Detail Mitte, Bedienung/Transkript rechts); Transkript `flex-1 min-h-0` statt `max-h-96`; `usePersistentState` für Breiten/Klappzustand/Auswahl/Reiter; Ist-Spec nach `tests/` | `pnpm exec playwright test meeting-layout` → AK2, AK4 (4 Viewports), AK8 grün | 450 |
| M3 | Sessions | Sessions-Spalte aus `meeting_folders_*` (oberste Ebene, Reihenfolge über `sort`); anlegen, umbenennen, löschen mit Rückfrage, sortieren; Ziehen + Menü „Verschieben“; Suche/Filter innerhalb der Session; Filterchips → Filter-Popover; Migrationstest (bestehende Ordner = Sessions, keine Dubletten) | `cargo test --lib meetings::folders` + `playwright test meeting-sessions` → AK3 | 450 |
| M4 | Kompakter Kopf | Detailkopf (Titel inline, Status-Chips, Details-Dialog), `TabList` (Notizen · KI-Notizen · Protokoll), Symbolzeile aus `IconAction`, Menü „☰“ (`ActionMenu`) mit Neu-Transkription-Dialog (B17-Hinweis), Vorlage, Verschieben, Löschen; Fortschritt P8a als Zeile mit Symbolknöpfen | `playwright test meeting-header` → AK5 (≤ 120 px bei 1366, Tooltips Maus + Tastatur), AK9-Teil | 400 |
| M5 | Live + Import + schmal | Laufende Aufnahme = gewählte Besprechung (Notizblock und Live-Transkript in ihren Spalten, `LiveChatRow` in den Reiter Fragen); Aufnahmezeile mit Uhr, Pegel, Pause/Stopp, Hinweis kopieren; Startdialog; Import-Symbol + Ablage auf der Inhaltsspalte in die gewählte Session; schmaler Modus < 620 px (geteilt, Griff); Kompaktmodus | `playwright test meeting-live-narrow` → AK6 (480×800 ohne Scrollen), AK7 | 550 |
| M6 | Abschluss | axe-Prüfung, Viewport-Matrix in der Gesamtsuite, Hilfe-Text `aufnahmen`, `docs/BESPRECHUNGEN.md`, Nachher-Screenshots (Spec aus M2), Installer Patch +1, Abnahme | `npx tsc --noEmit`, gesamte Playwright-Suite, i18n-Check → grün; Installer von Patrick abgenommen (AK9, AK10) | 300 |
| | **Summe M2–M6** | | | **2150** (+ U1 300 = 2450) |

Reihenfolge strikt M2 → M3 → M4 → M5 → M6. M3 und M4 berühren verschiedene Dateien und können parallel laufen,
sobald M2 gemergt ist. Größtes Risiko ist M5: Live-Zustand und Detail laufen heute getrennt (`LiveTranscript` hört
selbst auf Ereignisse, `MeetingDetail` lädt Segmente). Wer beide zusammenführt, fasst den Echtzeitpfad an.
Vorschlag deshalb: M5 an `lv-coder-xhigh`, Meldung bei 50 %/80 % des Goal-Rahmens wie im GOAL.md.

## Anschlusspunkte YouTube-Integration (#65, #66 Bündel 1)

- **Player in der Inhaltsspalte (#66 AK3):** In B oben in der Arbeitsfläche, über den Reitern, als einklappbarer
  16:9-Bereich. Die Höhe begrenzt ein vertikaler Griff, damit die Notizen nicht verschwinden. Das Transkript rechts
  läuft mit, ein Klick auf eine Zeitmarke setzt die Videoposition. Der Zeitmarken-Klick (`data-act="seek"`) ist in
  M2/M4 so zu bauen, dass er einen austauschbaren Player anspricht (Audio heute, YouTube-Player später). Die
  CSS-Klasse `.yt` im Prototyp reserviert die Fläche.
- **Link einfügen:** Symbol „Link“ in der Aufnahmezeile neben Import, heute ausgegraut. Ziehen oder Einfügen einer
  URL auf die Inhaltsspalte nutzt dieselbe Ablagezone wie Dateien (M5). Die neue Quelle landet in der gewählten
  Session.
- **Fassungen und Diff (#66 AK4/AK5):** Kopf des Transkript-Reiters rechts: Chip „Fassung v2 · eigene Transkription
  ▾“ mit Liste (Untertitel, eigene, zusammengeführt) und „Vergleichen“. Der Vergleich öffnet sich in der
  **Arbeitsfläche** als eigener Reiter „Vergleich“ (Wort-Diff, breit genug). Rechts bleibt die aktive Fassung
  stehen. Die Details-Dialoge zeigen heute schon „Fassungen“ (B17: Tausch erst am Ende). Dieselbe Stelle trägt
  später die Quelle je Fassung.
- **Herkunft (#66 AK6):** Rechtsklick auf Transkript, KI-Notizen und Protokoll → „Herkunft“. Das Kontextmenü
  gibt es im Prototyp schon für Sessions und Besprechungen. In M4 wird das Menü als gemeinsamer Baustein gebaut,
  damit #66 nur Einträge ergänzt.
- **Quelle-Chip:** Der Kopf zeigt die Quelle als Chip (Aufnahme/Import). „YouTube · Kanal“ ist dort ein weiterer
  Wert, ohne neues Layout.
