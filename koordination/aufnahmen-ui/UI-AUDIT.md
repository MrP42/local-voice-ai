# UI-Audit Seite „Aufnahmen“ (U1/M1, Goal aufnahmen-ui, #64)

Stand 30.09.2026, Basis `400814d` (0.20.9, inkl. #62 und Besprechungs-Runde 3). Messung: Playwright/Chromium
gegen die Tauri-Attrappe, Theme hell, 1920×1050, 1366×768, 480×800. Spec:
`screens/ist-screens.spec.ts` (liegt unter `koordination/`, da U1 keinen Produktcode anfasst; M2 übernimmt sie
nach `apps/local-voice/tests/` und erzeugt damit die Nachher-Bilder). Belege: `screens/ist-*.png` und je eine
Messdatei `ist-*.json` (Höhe des Hauptbereichs, jede Fläche mit Überlauf, Lage von Reitern, Fortschritt,
Transkript und Notizblock).

## Messwerte (Kern)

| Ansicht | Viewport | Hauptbereich `scrollHeight` / sichtbar | Weitere Scrollflächen | Reiter beginnen bei |
|---|---|---|---|---|
| Liste | 1366×768 | 836 / 716 → Seite scrollt | – | – |
| Liste | 480×800 | 1024 / 694 → Seite scrollt | – | – |
| Live (Aufnahme läuft) | 1366×768 | **1231** / 716 → Seite scrollt | Live-Transkript `max-h-80` (465 / 300) | – |
| Live | 480×800 | **1681** / 694 → Seite scrollt | Live-Transkript beginnt bei **y = 685** von 694 | – |
| Detail (Transkript) | 1366×768 | 921 / 716 → Seite scrollt | Transkript `max-h-96` (1733 / 360) | **y = 454** |
| Detail | 1920×1050 | 998 / 998 | Transkript 360 px hoch, darunter ~100 px leer | y = 454 |
| Detail + Fortschritt P8a | 1366×768 | 889 / 716 → Seite scrollt | Transkript (1733 / 360) | y = 395 (Fortschrittsblock 105 px) |
| Detail | 480×800 | 970 / 694 → Seite scrollt | Transkript (2693 / 360) | y = 491 |

Kurz: In jeder Ansicht unter 1920 scrollen die Seite **und** das Transkript bzw. das Live-Transkript,
also zwei Scrollbalken übereinander. Genau das hat Patrick bemängelt. Vor dem ersten Transkriptsatz stehen bei 1366 px
**438 px Kopf**.

## Vergleich mit dem Vorlesen-Modul und gängigen Mustern

| Muster | Vorlesen (#61/#62) | Granola | Otter | Notion AI Meeting Notes | Apple Notizen | Aufnahmen heute |
|---|---|---|---|---|---|---|
| Spalten | Seiten · Editor · Bedienung, ziehbar, einklappbar | Liste · Notizblock, Transkript auf Abruf | Liste · Transkript, Player unten angedockt | Block in der Seite, Reiter Zusammenfassung/Notizen/Transkript | Ordner · Liste · Notiz, einklappbar | Liste **oder** Detail (eins ersetzt das andere) |
| Scrollen | Seite steht, Spalten scrollen einzeln | Notiz scrollt, Kopf steht | Transkript scrollt, Player steht | Seite scrollt (Dokument) | jede Spalte für sich | Seite + innere Fläche |
| Bedienung | Transport + Symbolzeile + Menü „☰“, Tooltips | Aufnahmeknopf unten mit Pegel, sonst fast nichts | „Aufnahme“-Knopf + Zeit, „Zum Live-Ende“ | Start/Stopp im Block | – | Formular + Textknöpfe + Statusraster |
| Ordner | Seitenliste links | Ordner in der Leiste | Ordner/Kanäle links | Seitenbaum | Ordner links mit Zählern | Filterchips über der Liste |
| Details | im Tooltip/Menü | in der Kopfzeile (Datum, Teilnehmende) | Kopf: Datum, Dauer, Sprecher | Eigenschaften der Seite | – | 7-zeiliges Raster immer offen |

## Befunde

Schwere: **hoch** = verfehlt ein AK des Goals oder Patricks Kernwunsch; **mittel** = Standardabweichung mit
spürbarer Wirkung; **klein** = Politur. „Soll“ verweist auf die Muster oben und die Bausteine aus #62
(`ResizeHandle`, `IconAction`, `TabList`, `ActionMenu`, `usePersistentState`, `PageShell fill`).

| ID | Element | Ist (Messwert) | Soll | Fundstelle (Datei:Zeile) | Schwere |
|---|---|---|---|---|---|
| H01 | Scrollen der Seite | Hauptbereich scrollt (`overflow-y:auto`), Transkript darin zusätzlich (`max-h-96 overflow-y-auto`): 1366 → 921/716 **und** 1733/360; Live 1366 → 1231/716 **und** 465/300 | Seite scrollt nie; je Spalte genau eine Scrollfläche (AK4). Aufnahmen bekommt `PageShell fill` + `workspace-main--fill` wie Vorlesen | `App.tsx:374`, `App.css:778–781`, `MeetingDetail.tsx:984`, `LiveTranscript.tsx:93`, `notes/LiveNotesPad.tsx:75`, `MeetingsSettings.tsx:267` | hoch |
| H02 | Liste ↔ Detail | Detail **ersetzt** Liste und Seitenkopf; Wechsel zur nächsten Besprechung nur über „Zurück zur Liste“, Filter/Scrollposition der Liste gehen dabei verloren | Liste (Sessions) bleibt links stehen, Auswahl wechselt den Inhalt daneben (Apple Notizen, Otter) | `MeetingsSettings.tsx:248–287` | hoch |
| H03 | Live im kleinen Fenster | 480×800: Aufnahmekarte samt Chat-Hinweis ~255 px + Notizblock 260 px; Live-Transkript beginnt bei y = 685 (sichtbar bis 694) → Notiz tippen **und** Transkript sehen geht nur mit Scrollen | Aufnahmezeile ≤ 56 px oben fest, darunter Notizen und Transkript geteilt, beide sichtbar (AK6) | `MeetingsSettings.tsx:272–279`, `RecorderCard.tsx:560–582`, `LiveTranscript.tsx:89–116` | hoch |
| H04 | Detailkopf | 438 px bis zu den Reitern bei 1366 (Zurück + 3 Textknöpfe 40, Titel + Datei 50, zwei Player-Zeilen 60, **Statusraster 7 Zeilen 175**, Neu-Transkription 74 px); bei 480 px 491 px | Titel + Status-Chips + Symbolzeile ≤ 120 px (AK5); Vollinfo per Dialog „Details“ | `MeetingDetail.tsx:566–821` (Raster `:755–815`) | hoch |
| H05 | Laufende Aufnahme vs. Besprechung | Notizblock und Live-Transkript stehen **über** der Liste, nicht in der Besprechung; wer die laufende Besprechung öffnet, verlässt den Notizblock | die laufende Aufnahme ist die gewählte Besprechung; Notizen/Transkript sind ihre Reiter bzw. Spalten | `MeetingsSettings.tsx:272–279`, `notes/LiveNotesPad.tsx:59–88` | hoch |
| M01 | Ordner | 4 Ordner als Filterchips plus zweite Chipreihe (Zeitraum/Quelle/Notizen) über der Liste; bei vielen Ordnern Umbruch; keine Zähler-Spalte | Sessions-Spalte links (Name, Zähler, Anlegen/Umbenennen/Löschen, Ziehen); Zeitraum/Quelle als Filter-Popover | `MeetingList.tsx:487–493`, `search/FolderChips.tsx` | mittel |
| M02 | Beschriftung „Besprechungen“ | dreimal übereinander: Kartentitel Aufnahme, Kartentitel Liste, Absatz in der Liste; beim Aufnehmen zeigt der grüne Chip „Besprechungen“ statt „Aufnahme läuft“ | ein Titel je Bereich; Laufzustand eindeutig („Aufnahme läuft“ + roter Punkt) | `RecorderCard.tsx:365`, `:467–469`, `MeetingList.tsx:426`, `:435` | mittel |
| M03 | Aufnahmedauer | während der Aufnahme keine Uhr, nur Chip + zwei Pegelbalken über die volle Breite | Laufzeit mm:ss neben dem roten Punkt, Pegel als kleine Anzeige am Knopf | `RecorderCard.tsx:460–484`, `:570–580` | mittel |
| M04 | Aktionen im Detailkopf | „Exportieren“, „Follow-up-Mail“, „Fragen“ als Textknöpfe `sm` (25 px, 11,25 px Schrift, 14-px-Symbol) | Symbolzeile aus `IconAction` (36 px, Tooltip Name + Kurzerklärung), Seltenes im Menü „☰“ | `MeetingDetail.tsx:578–611` | mittel |
| M05 | Neu transkribieren | dauerhafter Block 74 px (Erklärsatz, Modell-Dropdown über volle Breite, Knopf) für eine seltene Aktion | Eintrag im Menü „☰“ → Dialog mit Modellwahl und Hinweis „Fassung wird erst am Ende getauscht“ (B17) | `MeetingDetail.tsx:817–821`, `RetranscribeControl.tsx:68–106` | mittel |
| M06 | Reiter Notizen/Transkript/Protokoll | eigener Stil 33 px, keine `role="tab"`, keine Pfeiltasten | `TabList` aus #62 (41 px bzw. compact 34 px, Rollen, Pfeiltasten) | `MeetingDetail.tsx:823–857` | mittel |
| M07 | KI-Notizen | zweite Ebene im Reiter Notizen (Pillenumschalter), aktive Pille gelb hinterlegt | KI-Notizen als eigener Reiter; Gelb nur für die eine Hauptaktion | `MeetingDetail.tsx:936–956` | mittel |
| M08 | Vorlage | Auswahl + „Vorlagen verwalten …“ an drei Stellen (Aufnahmekarte, Reiter Notizen, Reiter Protokoll), je 44 px Select | einmal je Besprechung (Chip im Kopf oder Menü), Verwalten im Menü | `RecorderCard.tsx:449–458`, `MeetingDetail.tsx:957`, `MinutesView.tsx` | mittel |
| M09 | Transkripthöhe | fest `max-h-96` (360 px): bei 1920 bleibt unter dem Transkript ~100 px leer, bei 1366 scrollt stattdessen die Seite | Transkript füllt die Resthöhe seiner Spalte (`flex-1 min-h-0`) | `MeetingDetail.tsx:984` | mittel |
| M10 | Chat | eigene 384-px-Spalte (sticky), die nur ~330 px hoch ist (1920: darunter ~650 px leer); zwei Chat-Orte (Besprechung, global) mit eigener Spalte | Chat als Reiter/Klappbereich in der rechten Spalte, füllt deren Höhe; global = derselbe Ort mit Session-Umfang | `MeetingDetail.tsx:1092–1100`, `MeetingsSettings.tsx:292–303`, `chat/ChatPanel.tsx:512` | mittel |
| M11 | „Frage zur laufenden Besprechung“ | eigene Karte zwischen Transkript und Liste | im Chat der rechten Spalte (Recipes als Vorschläge) | `MeetingsSettings.tsx:58–155`, `:279` | mittel |
| M12 | Hinweis für den Meeting-Chat | beim Aufnehmen dauerhaft ~90 px (1366) bzw. ~115 px (480) | Symbolknopf „Hinweis kopieren“ mit Tooltip-Vorschau | `RecorderCard.tsx:582`, `MeetingChatNotice.tsx` | mittel |
| M13 | Aufnahmeformular | Titel, 2 Schalter, Vorlage, Kalender-Chip und Start stehen immer über der Liste (1366: ~160 px) | in der Bedienung: gelber Start + Import; Titel/Optionen im Startdialog (Einwilligung liegt dort ohnehin) | `RecorderCard.tsx:384–484` | mittel |
| M14 | Import | nur Textknopf in der Listenkopfzeile; Ziehen wirkt auf das ganze Fenster, Hinweis erscheint in der Liste; Fortschritt steht je nach Ansicht in der Listenzeile (`JobBar`) oder im Detail (`JobPanel`) | Import-Symbol in der Bedienung, Ablage auf die Inhaltsspalte, Import landet in der gewählten Session, Fortschritt an einer festen Stelle (AK7) | `MeetingList.tsx:331–353`, `:459–467`, `:523–527`, `:616–626` | mittel |
| M15 | Fortschritt P8a | Block 105 px im Kopf mit Textknöpfen „Pausieren“/„Stoppen“ | eine Zeile: Phase + Balken + Prozent, Pause/Stopp als Symbolknöpfe, Laufzeit/Rest im Tooltip | `JobProgress.tsx:177–262`, `MeetingDetail.tsx:685` | mittel |
| M16 | Wiedergabe | bis zu zwei Player untereinander (Ich/Gegenseite), der Dateiname steht doppelt (unter dem Titel und über dem Player); bei 480 bricht die Transportzeile um | ein Player (`.mediabar`) in der Bedienung, Spur-Umschalter im Menü | `MeetingDetail.tsx:656–663`, `:720–753` | mittel |
| M17 | Persistenz | gemerkt wird nur „Automatisch mitscrollen“; gewählte Besprechung, Reiter (`useState("transcript")`), Ordnerfilter, Chat offen gehen beim Seitenwechsel verloren | Breiten, Klappzustand, Session, Besprechung, Reiter, Layout über Neustart (AK8) | `MeetingDetail.tsx:112`, `:136–140`, `MeetingsSettings.tsx:159`, `MeetingList.tsx` (Ordnerfilter) | mittel |
| M18 | Hilfe im Detail | Seitenkopf mit Hilfeknopf verschwindet in der Detailansicht | Seitenkopf bleibt stehen (eine Zeile), Hilfe immer erreichbar | `MeetingsSettings.tsx:248–265` | mittel |
| M19 | Kontrast Zeitstempel | Transkript-Zeitmarken `text-text/40` (≈ 2,3:1, vgl. Vorlesen-Audit A16) | Metatext mindestens `text/60` | `MeetingDetail.tsx:1006`, `:1011`, `LiveTranscript.tsx:103` | mittel |
| K01 | Listenzeilen | jede Zeile trägt Chip „Fertig“ und zwei Symbolknöpfe (Verschieben, Löschen) dauerhaft | Status nur bei Abweichung (läuft, wird verarbeitet, Fehler); Aktionen im Kontextmenü/beim Überfahren | `MeetingList.tsx:615–650` | klein |
| K02 | Listenkopf | „Alle Besprechungen fragen“, „Auswählen“, „Importieren …“ als Textknöpfe `sm` | Symbole im Spaltenkopf der Sessions | `MeetingList.tsx:436–468` | klein |
| K03 | Datumsformat | Liste „25. Sept. 2026, 16:13“, Detail „25.09.2026, 16:13“ | ein Format (kurz, relativ in der Liste: „Mo 28.09.“) | `MeetingList.tsx:553–559`, `MeetingDetail.tsx:774–779` | klein |
| K04 | Umbenennen | Stift rechts außen, bei 1366 ~1100 px vom Titel entfernt | Titel direkt anklickbar (Inline-Bearbeitung) + Menüeintrag | `MeetingDetail.tsx:639–654` | klein |
| K05 | Segment bearbeiten | Stift nur bei Hover (`opacity-0`), per Tastatur unsichtbar | sichtbar bei `:focus-within` oder per Doppelklick/Kontextmenü | `MeetingDetail.tsx:1068–1075` | klein |
| K06 | Kanal im Transkript | live als graue Pille, im Detail als Text: zwei Darstellungen | eine Darstellung (Name als Text, farbiger Punkt je Sprecher) | `LiveTranscript.tsx:107`, `MeetingDetail.tsx:1015–1036` | klein |
| K07 | Mitscrollen | Checkbox-Zeile über dem Transkript | Symbolknopf „Zum Live-Ende“, erscheint nur, wenn man hochgeblättert hat (Otter) | `MeetingDetail.tsx:859–883` | klein |
| K08 | Kopieren | drei Knöpfe (mit Metadaten, nur Text, Export-Symbol) über dem Transkript | ein Kopieren-Symbol in der Symbolzeile, Varianten im Menü | `MeetingDetail.tsx:884–915` | klein |
| K09 | Notizblock-Überschrift | `text-mid-gray` statt Leisten-Kopf-Standard `text-text/50`; Hinweissatz dauerhaft | Leisten-Kopf-Stil, Hinweis als Platzhalter | `notes/LiveNotesPad.tsx:66–73` | klein |
| K10 | Symbolgrößen | 10, 12, 14, 16, 20 px gemischt (Recipe-Sparkles 10, Filter-X 10, Knöpfe 14) | 16 px in Knöpfen/Köpfen, 18 px in `IconAction`, 20 px im Seitenkopf | `MeetingsSettings.tsx:139`, `MeetingList.tsx:517`, `MeetingDetail.tsx:586–608` | klein |

**Summe:** 5 × hoch, 19 × mittel, 10 × klein.

## Was bleibt, wie es ist

- Einwilligungsdialog vor Aufnahme und Import (`RecorderCard.tsx:585–611`, `MeetingList.tsx:709–742`): fachlich
  richtig, nur der Auslöser wandert in die Bedienung.
- Suche mit Treffer-Schnipsel (`SearchBar`, `SearchSnippet`) und Personen-Popover: bleiben, wandern in die
  Sessions-Spalte bzw. den Detailkopf.
- `JobBar` in der Listenzeile: gutes Muster für die Sessions-Spalte (kompakt, mit Prozent).
