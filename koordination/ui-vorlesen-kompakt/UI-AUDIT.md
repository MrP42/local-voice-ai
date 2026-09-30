# UI-Audit Vorlesen-Seite (P1, Goal ui-vorlesen-kompakt, #61)

Stand 29.09.2026, Basis `dd22dd8`. Messung: Playwright/Chromium gegen die Tauri-Attrappe, Theme hell,
1920×1050 und 1366×768, Spec `apps/local-voice/tests/readaloud-screens.spec.ts`. Belege: PNGs und
Messwerte (JSON mit Box, Schriftgröße, Symbolgröße je Bedienelement) unter `screens/vorher/`.

## Standard (aus Code und Vergleich mit Verlauf, Modelle, Einstellungen)

- **Schrift:** Wurzel 15 px (`App.css:21`) → `text-xs` 11,25 px, `text-sm` 13,125 px, Seitentitel 18,75 px. Verlauf, Modelle, Einstellungen: 0× `text-[10px]`/`text-[11px]`.
- **Knöpfe:** `Button` md = 31 px hoch, 13,125 px/500, Symbol 15–16 px (Einstellungen: 31 px, 15 px); `sm` = 25 px/11,25 px nur für Nebenaktionen. Transport = eigene Familie `.mbtn` (Bedienspalte 34 px), genau ein gelber Hauptschalter.
- **Symbol-Knöpfe:** Seitenkopf 31 px/20-px-Symbol (`PageShell`); Leisten-Kopf `p-1` 24 px/16 px; Zeilenaktionen im Verlauf 44×44 px (`HistorySettings.tsx:30`, `min-h-11 min-w-11`).
- **Reiter:** Einstellungen (`AppSettings.tsx:112–144`): 41 px, 13,125 px/500, Unterstrich `border-logo-primary`, inaktiv `text/60`, `role="tab"` + Pfeiltasten.
- **Klappbereich:** `.workspace-disclosure` (`App.css:658`): 44 px, 14 px/500. **Leisten-Kopf:** `text-xs font-semibold uppercase tracking-wide text-text/50` (`PageShell.tsx:83`).
- **Abstände:** `gap-4` (15 px) zwischen Spalten, `gap-2` in Gruppen. **Tooltip:** `ui/Tooltip.tsx` vorhanden (nur `SettingContainer`); Vorlesen nutzt ausschließlich `title`.

## Befunde

Kontrast gerechnet für `text-text/NN` (rgb 31,41,55) auf Hintergrund `#f8f9fb`.

| ID | Element | Ist (Messwert) | Standard | Fundstelle (Datei:Zeile) | Schwere | Paket | Status |
|---|---|---|---|---|---|---|---|
| A01 | Ausdruck & Sprechstil, Filter „Alle“ | Klappbereich 757 px hoch, `scrollHeight` = `clientHeight` (scrollt nicht); summary bei y=214 liegt im Textfeld (y=119–279) → **65 px Überlagerung**; Textfeld auf `min-height` 160 px gedrückt, läuft aus seinem Behälter | Klappbereich begrenzt + scrollt, Editor ≥ 160 px ohne Überschneidung (AK10) | `TtsSettings.tsx:1599–1613`, `App.css:694–697` | hoch | P3 | behoben (P3, 534f66c) |
| A02 | Dateiliste, Dateiname | sichtbar 19 von 51 Zeichen („CASE-GESPRÄCH-IE2S-…“), alle 6 Einträge identisch, Zeitstempel nie sichtbar; beim Hover weniger (Größe weicht Stift+Papierkorb) | Anfang + „…“ + Zeitstempel, voller Name im Tooltip (AK9); Name kurz (AK8) | `WorkspaceSidebars.tsx:572–574`, `src/lib/utils/exportName.ts` | hoch | P4 | behoben (P4, c0da046) |
| A03 | Auto-Tagging-Knopf | 266×**25** px, 11,25 px, Symbol 14 px | Nachbarn 266×31 px, 13,125 px, 16 px | `AutoTagBar.tsx:343–356` (`size="sm"`) | mittel | P2 | behoben (P2, 2c9f6fd) |
| A04 | Symbol `Sparkles` in der Bedienspalte | 3× (Vorab erzeugen, Text aufbereiten, Auto-Tagging) | je Aktion ein eigenes Symbol (AK2) | `TtsSettings.tsx:1854`, `:2005`, `AutoTagBar.tsx:353` | mittel | P2 | behoben (P2, 2c9f6fd) |
| A05 | Symbol `Sparkles` im Palette-Filter „Spezial“ | 4. Vorkommen auf derselben Seite (13 px) | eindeutige Symbole je Bedeutung | `src/lib/tags/registry.ts:29` | niedrig | neu – nach P2 ein Symbol wählen, das keine Aktion der Bedienspalte trägt (z. B. `Shapes`) | behoben (P7, 5bd3775) |
| A06 | Spaltenbreiten | fest: Seiten 195 px (`w-52`), Bedienung 270 px (`w-72`), Dateien 225 px (`w-60`); Editor 1920: 959 px = 57 % der Arbeitsfläche, **1366: 405 px = 36 %** (~45 Zeichen/Zeile) | ziehbar, gespeichert (AK7), eine rechte Spalte (AK5) | `WorkspaceSidebars.tsx:131`, `:465`, `TtsSettings.tsx:1620` | mittel | P5 | behoben (P5, cb9501d) |
| A07 | Höhe Bedienblock | 463 px (y 79–542) = **60 %** von 768 px bei 1366 | ≤ 50 % `innerHeight` (AK5) | `TtsSettings.tsx:1619–2218` | mittel | P5 (mit P2) | behoben (P5 cb9501d + P2 2c9f6fd; AK5-Test P8 c8176e0) |
| A08 | Seitentitel in der Seitenliste | „CASE-GESPRÄCH IE2S · 28.09…“: 26 von 31 Zeichen, das unterscheidende Datum angeschnitten (195 px Spalte) | Titel lesbar oder per Spaltenbreite lesbar zu machen | `WorkspaceSidebars.tsx:200` | mittel | P5 | behoben (P5, cb9501d – Seitenliste ziehbar) |
| A09 | Editor-Reiter Original/Übersetzung/Zusammenfassung | 32 px hoch, 13,125 px/**400**, inaktiv `text/50`; keine `role="tab"`/`aria-selected`, keine Pfeiltasten | 41 px, 500, `text/60`, `role="tab"` + Pfeiltasten | `TtsSettings.tsx:1490–1524` | mittel | neu – Klassen und Tastaturführung der Einstellungen-Reiter übernehmen (`min-h-11 font-medium text-text/60`, `role="tablist"/"tab"`) | behoben (P6, fd979ef) |
| A10 | Reiter-Stile auf einer Seite | 3 Stile: Unterstrich 32 px (Editor), graue Großbuchstaben-Pillen 19 px (Dateien/Hilfe), gelbe Pillen 23 px (Palette) | ein Reiter-Stil (Unterstrich) | `TtsSettings.tsx:1494`, `WorkspaceSidebars.tsx:476`, `TagPalette.tsx:98` | mittel | neu – Dateien/Hilfe als Unterstrich-Reiter; Palette-Filter als Filterchips klar von Reitern absetzen (neutral statt gelb, s. A11) | behoben (P6 fd979ef, P7 5bd3775) |
| A11 | Aktiver Palette-Filter | Vollgelb `bg-logo-primary` – zweite gelbe Fläche neben dem Abspielknopf | Gelb nur für die eine Hauptaktion (GOAL Constraints) | `TagPalette.tsx:100` | mittel | neu – aktiv als `bg-mid-gray/20 text-text` wie die Leisten-Reiter | behoben (P7, 5bd3775) |
| A12 | Klickziele der Zeilenaktionen | Umbenennen/Löschen 17×17 px (13-px-Symbol, Seiten- und Dateizeile), Anhören 20×20 px (12 px), Favoriten-Stern 16×16 px (10 px) | Verlauf 44×44 px; mindestens 24×24 px (WCAG 2.5.8) | `WorkspaceSidebars.tsx:222–275`, `:579–629`, `TagPalette.tsx:432–436` | mittel | neu – `p-1` + 16-px-Symbol wie die Leisten-Köpfe (24 px) | behoben (P6 fd979ef, P7 5bd3775) |
| A13 | Symbolgrößen | 9 Größen auf einer Seite: 10, 12, 13, 14, 15, 16, 17, 20, 23 px; Leisten-Kopf Dateien 15/15/14/16 px, Hinzufügen-Menü 15 px | 16 px in Knöpfen/Köpfen, 20 px Seitenkopf, `.mbtn` eigene Familie | `WorkspaceSidebars.tsx:501,510,519,530`, `TtsSettings.tsx:1948,1959,1970` | niedrig | neu – außerhalb `.mbtn` nur 16 px (Köpfe, Knöpfe, Menü) und 20 px (Seitenkopf) | behoben (P6, fd979ef; Menüs bereits 16 px) |
| A14 | Schriftgrößen außerhalb der Skala | `text-[11px]` 13× und `text-[10px]` 3× im Vorlesen-Bereich (Seitenliste, Dateiliste, Legende), summary fest 14 px, Tempo 12,75 px (0,85rem) – 6 Größen zwischen 10 und 14 px | nur `text-xs` 11,25 / `text-sm` 13,125 px | `WorkspaceSidebars.tsx:207,575,652,671,672,686,693`, `TagPalette.tsx:376`, `App.css:662`, `App.css:736` | mittel | neu – auf `text-xs`/`text-sm` abbilden, summary `0.875rem` | behoben (P6 fd979ef, P7 5bd3775, P8 c8176e0) |
| A15 | Zeilenhöhe der `text-[10/11px]`-Texte | Beliebigwerte setzen keine Zeilenhöhe → 11 px Schrift mit **24 px** Zeilenhöhe (2,2×): Palette-Legende 2 Zeilen = 48 px, Satzliste der Aufnahme 24 px je Zeile, „vor 20 Minuten“ | `text-xs` setzt 15 px Zeilenhöhe | `TagPalette.tsx:376`, `WorkspaceSidebars.tsx:207,671–672` | mittel | neu – mit A14 erledigt (`text-xs` bringt die Zeilenhöhe mit) | behoben (P6 fd979ef, P7 5bd3775) |
| A16 | Kontrast Kleinschrift | Dateigröße 10 px `text/35` → **2,1:1**; „vor 20 Minuten“ 11 px `text/40` → 2,3:1; Seitenvorschau `text/45` → 2,6:1; Stimmenhinweis `text/50` → 3,0:1 | Metatext andernorts `text/50–60`; WCAG AA 4,5:1 | `WorkspaceSidebars.tsx:203,207,575,652`, `TtsSettings.tsx:1785` | mittel | neu – Metatext mindestens `text/60` (4,0:1), kleiner als `text-xs` nie | behoben (P6 fd979ef; Stimmenhinweis als Tooltip P2) |
| A17 | Klappbereich „Schreibregeln: Sprecher & Tags“ | zweiter Klappbereich-Stil: summary 15 px hoch, 11,25 px/400, `text/50` | `.workspace-disclosure` 44 px, 14 px/500 | `TtsSettings.tsx:2187–2188` | niedrig | neu – Inhalt in den Hilfe-Reiter der Dateileiste verschieben (steht dort ohnehin) oder `.workspace-disclosure` verwenden | behoben (P8, c8176e0 – Klappbereich-Stil) |
| A18 | Höhen in der Bedienspalte | 5 Höhen: Transport 34, Stimmenwahl 44 (Select `minHeight` 40), Knöpfe 31, Auto-Tagging 25, Schreibregeln 15 px | eine Knopfhöhe je Zeile | `TtsSettings.tsx:1632–2212`, `Select.tsx:54` | niedrig | P2 | teilweise (P2: Aktionen 36 px einheitlich); Transport 34 px / Select 44 px → Folge-Goal |
| A19 | Stimmenhinweis unter der Stimmenwahl | dauerhaft 2 Zeilen, 11,25 px, `text/50` (~30 px Höhe in der knappen Spalte) | Erklärung im Tooltip (AK4) | `TtsSettings.tsx:1784–1791` | niedrig | neu – Hinweis in den Tooltip der Stimmenwahl, sichtbar nur beim Wechsel der Betriebsart | behoben (P2, 2c9f6fd) |
| A20 | „Als Audio speichern…“ | Kommentar „Nur das Symbol“, gebaut als 266-px-Knopf mit Text | Symbol-Knopf + Tooltip (AK2/AK4) | `TtsSettings.tsx:1793–1806` | niedrig | P2 | behoben (P2, 2c9f6fd) |
| A21 | Abspielen in der Dateiliste | Zustand „läuft“ nur per `aria-pressed`, Symbol bleibt ▷, keine Hervorhebung; darunter nativer `<audio>`-Player = dritte Player-Optik neben `.mediabar` und `AudioPlayer` | `.mbtn[aria-pressed]`-Zustand, eine Player-Familie | `WorkspaceSidebars.tsx:579–602`, `:640–648` | niedrig | neu – Knopf mit Pressed-Stil (`bg-logo-primary/15`) und Stopp-Symbol, solange der Player offen ist | behoben (P6, fd979ef) |
| A22 | Zeitstempel der Aufnahme | `toLocaleString()` ohne Sprache: Attrappe zeigt „9/29/2026, 11:45:36 AM“ bei deutscher Oberfläche; die Seitenliste nutzt dagegen `i18n.language` | Sprache der Oberfläche | `WorkspaceSidebars.tsx:654` | niedrig | neu – `toLocaleString(i18n.language)` | behoben (P6, fd979ef) |
| A23 | Palette-Tags (Klickfläche) | sichtbare Pille 22 px, Klickfläche 43 px (`p-3`), Zeilenabstand 47 px → „Alle“ (95 Tags) = 11 Zeilen ≈ 520 px | kompakt, im begrenzten Klappbereich scrollend | `TagPalette.tsx:413` | niedrig | neu – mit P3 abstimmen: `p-1.5` (Zeilenabstand ~30 px); die Touch-Begründung im Kommentar gilt am Desktop nicht | behoben (P7, 5bd3775) |
| A24 | Palette-Filterleiste | 10 Filter brechen bei 1366 in 3 Zeilen um (y 748/774/801), plus 2 Zeilen Legende ≈ 110 px vor dem ersten Tag | einzeilig, scrollt waagrecht (wie Einstellungen-Reiter) | `TagPalette.tsx:342`, `:375–385` | niedrig | neu – `flex-nowrap overflow-x-auto` statt `flex-wrap` | behoben (P7, 5bd3775) |

Summe: 24 Befunde – 2 hoch, 12 mittel, 10 niedrig; 9 in P2–P5, 15 „neu“ (davon mittel: A09–A12, A14–A16).

## Folge-Goal (andere Seiten, nur notiert)

- **Einstellungen:** Knöpfe „Satz 1–4“, „Feld aktivieren“, „Leeren“ 25 px, 11,25 px/**400** (nicht `Button`); Eingabe „Wort hinzufügen“ 28 px statt 36 px (`Input`).
- **Modelle:** Suchfeld 883×**19** px – ohne `Input`-Polster, deutlich niedriger als jedes andere Eingabefeld.
- **Verlauf:** „Aufnahmeordner öffnen“ im Seitenkopf als `sm` (25 px, 11,25 px) neben 31-px-Symbolknöpfen.
- Messwerte: `screens/vorher/vergleich-andere-seiten.json` (Attrappe mit leeren Listen).
