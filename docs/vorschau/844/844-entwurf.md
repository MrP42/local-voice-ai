# lva#844 – Designentwurf „Planung“, Revision 2 (Begründung)

Vorschau: `844-entwurf.html` (eigenständig, keine externen Ressourcen, nur example.org-Daten, Lucide-Symbole inline).
Bedienung: Register der Planung per Klick oder Pfeiltasten; Strg+N öffnet die Vorlagenwahl; Strg+F springt in die
Listensuche; „…“ öffnet das Überlaufmenü; Thema oben rechts.

Zustände per Adresse (kombinierbar):

| Parameter | Werte |
|---|---|
| `thema` | `hell`, `dunkel` |
| `ansicht` | `planen`, `termin`, `durchfuehren`, `protokoll` |
| `stand` | `vorher` (Termin steht, verschoben), `laeuft`, `nachher` – Vorgabe ergibt sich aus der Ansicht |
| `zustand` | `leer`, `archivleer`, `laden`, `keinezusagen`, `fehler-outlook`, `fehler-webex`, `fehler-uebergabe`, `fehler-protokoll` |
| sonst | `dialog=vorlagen`, `suche=…`, `menue=1`, `combo=schae`, `bestaetigen=freigeben\|beenden`, `offen=einwilligungen`, `insel=aus`, `kompakt=1`, `drawer=1`, `auswahl=liste`, `lang=en` |

Die Bilder entstanden unter Linux ohne Segoe UI (Ersatzschrift Inter, in Windows laufen Texte etwas schmaler),
Playwright Chromium, deviceScaleFactor 1. Prüfskripte: `tools/shots.js` (Bilder, meldet Skriptfehler, Seitwärts-
Überlauf und leere Symbole – Lauf ohne Befund), `tools/tasten.js` (Tastatur- und Suchabnahme, 29 von 29 OK),
`tools/kontrast.py` (WCAG-Formel).

## 0. Bilder

Alle unter `/tmp/claude-0/-home-user/ea04c13c-5d5e-5296-bda5-d105a97c1932/scratchpad/`.

| Gruppe | Dateien |
|---|---|
| Hauptansichten | `844-{planen,termin,durchfuehren,protokoll,vorlagen}-{1440,2560}-{hell,dunkel}.png` (20) |
| Bedienung | `844-vorlagen-suche-1440-dunkel`, `844-combobox-1440-hell`, `844-menue-1440-dunkel`, `844-planen-lang-1440-hell`, `844-einwilligungen-1440-hell` |
| Breiten | `844-planen-1280-hell`, `844-termin-1280-dunkel`, `844-planen-900-hell`, `844-termin-900-dunkel`, `844-drawer-900-hell`, `844-mobil-liste-390-hell`, `844-mobil-termin-390-dunkel` |
| Leer / Laden | `844-leer-1440-{hell,dunkel}`, `844-leer-archiv-1440-hell`, `844-leer-zusagen-1440-dunkel`, `844-laden-1440-{hell,dunkel}` |
| Fehler | `844-fehler-outlook-1440-{hell,dunkel}`, `844-fehler-webex-1440-{hell,dunkel}`, `844-fehler-uebergabe-1440-hell`, `844-fehler-protokoll-1440-dunkel` |
| Bestätigung | `844-bestaetigen-freigeben-1440-{hell,dunkel}`, `844-bestaetigen-beenden-1440-{hell,dunkel}` |
| Insel-Varianten | `844-kompakt-1440-dunkel` (eine Zeile beim Scrollen), `844-insel-aus-1440-hell` (abgeschaltet), `844-kontrastdesign-1440` (forced-colors), `844-druck-1440` (Druck) |
| Textlänge | `844-en-1440-hell` (englische Rahmentexte) |

## 1. Struktur

```
Seitenkopf: Planung + Beschreibung · [Mit Kalender abgleichen] [Neue Planung aus Vorlage | ▾]   (beide sekundär)
┌ Liste (300–400 px) ─────────┬ Planung ────────────────────────────────────────────────────────────┐
│ Suche (Strg+F)              │ INSEL: Titel · Zustandschips · Aktionen (eine primäre) · „…“         │
│ Aktiv · Archiv · Alle       │        Fakten: Zeit · Dauer · Teilnehmende · Online · Klasse · …    │
│ ─ Diese Woche ─             │        Register ① Planen ② Termin & Zusagen ③ Durchführen ④ Protokoll│
│ Titel                       │        (je ein abgeleiteter Satz zum Stand, s. §4)                  │
│ Datum ▫ Webex  4/9 [Chip]   ├─────────────────────────────────────────────────────────────────────┤
│ ─ Nächste Woche ─ …         │ Meldungen des Registers (MessageBar) · Inhalt (Karten, Container-Q.)│
└─────────────────────────────┴─────────────────────────────────────────────────────────────────────┘
```

Breiten (Container-Queries am Planungsbereich, Media-Queries am Fenster):

| Fenster | Verhalten |
|---|---|
| ≥ 1500 px Planungsbreite | „Planen“ dreispaltig; „Termin“: Zusagen höchstens 1100 px, daneben Termin und Entwürfe (Bild 2560) |
| 1000–1499 px | Liste links, Planung zweispaltig |
| < 1100 px Karte | Spalte „Rolle“ in der Zusagentabelle aus; < 1000 px Spalte „Erinnerung“ aus, Angabe unter dem Namen |
| < 1000 px | Liste als Schublade (Knopf links neben „Planung“, Strg+F öffnet sie), Planung volle Breite |
| < 900 px | Registerleiste kompakt: Ziffern, Name und Stand nur beim gewählten Register |
| < 640 px | Liste ODER Planung mit „Zurück zu Planungen“, Navigationsschiene hinter ≡, Befehle nur Symbol (mit Namen), Ziele 44 px (auch bei `pointer: coarse`), Dialoge vollflächig |

**Mindestfenster: 800 × 600 logische Pixel** (`tauri.conf.json` → `minWidth`/`minHeight`, heute nicht gesetzt).
Begründung: Titelleiste 48 + Statusleiste 24 + Seitenkopf 52 + kompakte Insel ≈ 56 lassen bei 600 px Höhe gut
420 px Inhalt; bei 800 px Breite greifen Schublade und kompakte Register, die Zusagentabelle zeigt zwei Spalten ohne
Abschneiden (Bild 900 zeigt die Stufe darüber). Kleiner bringt am Desktop keinen Nutzen, aber mehr Testfälle. Das
Layout selbst bleibt bis 320 CSS-px benutzbar (WCAG 1.4.10 bei 400 % Zoom); der vorhandene Test „Schmal“ (390 px)
bleibt gültig.

## 2. Komponenten und Fluent-Entsprechung

| Entwurf | Fluent 2 / bestehend | Anmerkung |
|---|---|---|
| Seitenkopf | `PageShell` (vorhanden) | zwei sekundäre Befehle; Primär nur im Inhalt |
| „Neue Planung aus Vorlage“ | `SplitButton` + `Menu` | sekundär (Auflage 1) |
| Listensuche | `SearchBox` | Strg+F (s. §6) |
| Aktiv/Archiv/Alle | `TabList size="small"` (vorhanden `plg-filter`) | Zähler im Label |
| Listenzeile | eigene Zeile, `aria-current` (vorhanden) | Chip zeigt Zustand, kein Phasenring mehr |
| Zustandschip | `Badge appearance="tint" shape="rounded"` (`Statusmarke`) | immer Symbol + Text; „Verschoben“ in der Insel ist ein Knopf |
| Insel | eigene Fläche, Tokens lokal überschrieben | abschaltbar, s. §5 |
| Register | `TabList` + `Tab` mit eigenem Inhalt, Panels `role="tabpanel"` + `aria-labelledby` | gesteuertes `selectedValue` |
| Überlaufmenü „…“ | `Menu`/`MenuList`/`MenuItem` | Pfeile, Pos1/Ende, Esc gibt Fokus zurück; kein Tastenkürzel für Löschen |
| Meldungen | `MessageBar intent=warning\|error` + `MessageBarActions` | „Erneut versuchen“ bei jedem Fehler |
| Bestätigung | `Dialog modalType="alert"` | Fokus auf „Abbrechen“ (unumkehrbare Folge) |
| Karte / Aufklappkarte | `Card`, `Accordion` | Zusammenfassung mit Zählern |
| Felder | `Field` + `Input`/`Textarea`/`Dropdown` | Label oben, Hilfe unten |
| Rollen, Verantwortlich | `Combobox` mit `OptionGroup` (#839) | `aria-activedescendant`, Pfeile, Enter |
| Dauer-Schnellwahl | `ToggleButton` | **25 / 50** aus `EMPFOHLENE_DAUERN` (`logik.ts:63`) |
| Entscheidungspunkt | `ToggleButton` (Symbol Waage, `aria-pressed`) | ersetzt den Chip |
| Agenda-Reihenfolge | Griff-Knopf + Alt+Pfeil hoch/runter | Fokus wandert mit |
| Online-Segment, Ziel erreicht | `RadioGroup layout="horizontal"` | |
| Zeitband, Zeitbox-Spur | eigenes Flex-Band, nur Ziffern | Titel per `title` und `sr-only`, Werte in Tabelle |
| Zusagen-Filter | `ToggleButton shape="circular"` in `Toolbar` | inkl. „Unbekannt“ |
| Zusagentabelle | `Table` mit `table-layout: fixed` | Quelle + Abrufzeit über der Tabelle |
| Vorlagenwahl | `Dialog` + `Combobox`-Suche + `Listbox` | eine Auswahl, Gruppen ohne Doppel |
| Vorlagen-Symbolkachel | getönt `colorBrandBackground2`/`colorBrandForeground2` | „Eigene“ grün getönt |
| Leerzustände | eigener Baustein (Symbol, Satz, Aktion) | |
| Laden | `Skeleton`/`SkeletonItem` | `aria-busy`, Statusmeldung für Screenreader |
| Symbole | lucide-react (vorhanden) | 12–20 px |

Die Vorschau baut Combobox, Listbox, Menü und Dialog von Hand nach, nur um Verhalten zu zeigen. In der App kommen
dafür ausschließlich die Fluent-Bausteine der Tabelle zum Einsatz; deren Tastaturverhalten ist das Vorbild der
Vorschau, nicht umgekehrt.

## 3. Kontrast (WCAG 2.2 AA, `tools/kontrast.py`)

Text 4,5:1, Bedienrahmen und Grafik 3:1. Fluent-Werte aus `webLightTheme`/`webDarkTheme`.

| Paar | Hell | Dunkel |
|---|---|---|
| Text fg1 / fg2 auf Karte | 15,52 / 10,05 | 14,55 / 10,01 |
| Hilfetext fg3 auf Karte / Basis | 6,19 / 5,68 | 6,48 / 8,21 |
| Platzhalter fg4 | 4,95 | 5,11 |
| Weiß auf Primär | 5,38 | 6,66 |
| Link auf Karte | 5,38 | 5,18 |
| Chip info / ok / warn / bad | 5,95 / 5,89 / 4,85 / 6,55 | 6,62 / 10,05 / 6,97 / 6,20 |
| Text auf MessageBar warn | 14,87 | 14,20 |
| Veralteter Stand (fg2 auf bg2, statt Abblenden) | 9,63 | 11,34 |
| Feldkante unten (strokeAccessible) | 6,19 | 6,48 |
| Auswahl-Pill auf gewählter Zeile | 4,52 | 4,18 |
| Balken zugesagt / abgesagt / Vorbehalt | 5,37 / 6,07 / 3,96 | 5,34 / 4,16 / 9,02 |

Insel (hell: Verlauf #13263C → #15161A; dunkel: #203450 → #1C2D42):

| Paar | Hell | Dunkel |
|---|---|---|
| Titel Weiß | 15,33 | 12,59 |
| Fakten #C9CED6 | 9,69 | 8,84 |
| Phaseninfo #A3A9B3 | 6,48 | 5,32 |
| Phaseninfo auf gewähltem Register (+8 % Weiß) | 6,15 | 4,62 |
| Akzent #62ABF5 | 6,32 | 5,19 |
| Warnung #FBB48A | 8,76 | 7,98 |
| Insel gegen Basis | 16,58 | **1,32** (vorher 1,02) |

Bewusste Grenzfälle: Feldrand ringsum (`stroke1`) 1,53 / 2,53 – erkennbar über die untere Kante, wie Fluent.
Verteilungsbalken trägt nie allein: dieselben Zahlen stehen mit Symbol und Text in den Filtern. Mit `insel=aus`
gelten die normalen Karten-Werte. Im Windows-Kontrastdesign (`forced-colors`) fällt die Insel auf `Canvas`/
`CanvasText` zurück, Auswahl auf `Highlight` (Bild `844-kontrastdesign-1440`).

## 4. Zustand ableiten statt behaupten (Regel)

Alles, was die Insel und die Register über den Stand sagen, wird aus dem Datenstand berechnet, nie aus der gewählten
Ansicht. Ein Registerwechsel ändert nichts am Stand: Lauf und Zeitmessung laufen weiter, „Aufnahme läuft“ und
„Besprechung beenden …“ bleiben in der Insel, egal welches Register offen ist (`tasten.js` prüft das).

| Stand | Chips | Planen | Termin & Zusagen | Durchführen | Protokoll |
|---|---|---|---|---|---|
| vorher, verschoben | Verschoben (Knopf), In Outlook | Vollständig, 5 Punkte | 4 von 9 zugesagt + Warnsymbol (Achtung) | Datum (offen) | nach der Besprechung |
| vorher, keine Antworten | In Outlook | wie oben | 0 von 9 geantwortet | Datum | nach der Besprechung |
| läuft | In Outlook | fertig | 6 anwesend (fertig) | Punkt 3 von 5, 22:36 | nach der Besprechung |
| nachher | Durchgeführt | fertig | 6 anwesend | Beendet 10:52, 50 Min. (fertig) | Version 2, Freigabe offen |

Fakten: „Transkription geplant“ nur vorher, „Transkript lokal, 50 Min.“ nur nachher. Die Datenquelle dafür ist
die Planung selbst (`statusMarken`, Lauf, Protokoll) – keine eigene Statusvariable für die Oberfläche.

## 5. Insel: abschaltbar, Kontrastdesign, Druck

- Abschalten: Klasse `insel--aus` (Vorschau `insel=aus`) ergibt einen normalen Fluent-Kopf auf Background1; weil die
  Insel nur Tokens lokal überschreibt, ist sonst nichts zu ändern. Vorschlag: Einstellung „Darstellung → Kopfzeile
  betont“ (Vorgabe an); ohne Einstellung reicht die Klasse.
- Kompakt: Ab 24 px Scrolltiefe wird die Insel einzeilig (Titel, Ziffern-Register, Aktionen; ~48 px statt ~150 px).
  Der Chip „Verschoben“ bleibt sichtbar, „In Outlook“ entfällt dort.
- Kontrastdesign: eigener `forced-colors`-Block, keine Verläufe, Ränder in `CanvasText`.
- Druck: nur die gewählte Planung ohne App-Rahmen, Register und Aktionen; Insel weiß mit schwarzem Rand, Chips
  schwarz umrandet (Bild `844-druck-1440`).
- „Einzige Abweichung“ gilt jetzt wörtlich: die Vorlagenkacheln sind auch im hellen Thema getönt (Brand-Tönung),
  nicht mehr dunkel.

## 6. Tastatur und Fokus

Abnahme ohne Maus (in `tools/tasten.js` automatisiert, alle OK):

1. Strg+N → Dialog offen, Fokus in der Suche, App dahinter `inert`.
2. „fix“ tippen → Liste filtert; Pfeil runter → `aria-activedescendant` wechselt, Option `aria-selected`.
3. Tab ×12 → Fokus bleibt im Dialog. Enter → Planung angelegt, Dialog zu, Meldung nennt die Vorlage, Fokus zurück
   auf den Auslöser.
4. „…“ mit Enter → erster Eintrag fokussiert; Pfeile, Ende = „Löschen …“, Esc → Fokus zurück auf „…“.
5. Register: Pfeil rechts → „Termin & Zusagen“; Panel trägt `aria-labelledby="tab-termin"`.
6. Rollen-Combobox: „schae“ → Pfeil → Enter übernimmt die Person.
7. „Protokoll freigeben …“ → Fokus auf „Abbrechen“; Esc → Fokus zurück auf den Auslöser.
8. Agenda: Alt+Pfeil hoch/runter im Thema-Feld verschiebt den Punkt, Fokus folgt.

Strg+F: in der Vorschau verdrahtet (Listensuche, unter 1000 px öffnet es die Schublade). In WebView2 vor dem
Einbau prüfen, dass `preventDefault` die eingebaute Suchleiste unterdrückt; sonst Kürzel und Hinweis „Strg+F“
streichen statt doppelt belegen. Kein Kürzel für Löschen.

## 7. Datenwahrheit (Zusagen, Webex)

- Quelle und Abrufzeit stehen sichtbar über der Tabelle: „Quelle Outlook-Kalender des Organisators, abgerufen
  25.11., 14:02 · Abgleichen“. Genau einmal; Statusleiste und Termin-Karte zeigen keinen Abgleich mehr.
- „Unbekannt“ ist eine eigene Antwort (Person im Kalender nicht gefunden, z. B. Weiterleitung), getrennt von
  „Keine Antwort“; eigener Filter, gestrichelter Balkenteil, Erklärung im Tooltip.
- Spalte „Erinnerung“ zeigt nur, was die App selbst getan hat: „Entwurf 25.11.“ = Entwurf geöffnet. Die App sieht
  nicht, ob er verschickt wurde, und sagt das in „Entwürfe und Übergaben“. Kein „zuletzt angeschrieben“.
- Webex: nur Hinweis („folgt womöglich nicht“), bis #842 per ADR klärt, was die App über Webex wirklich weiß. Daher
  auch kein Etikett „Webex aktuell“.
- Fehler Outlook: letzter Stand bleibt lesbar (kein Abblenden), markiert mit Randlinie und „Letzter Stand … Neuere
  Antworten fehlen.“

## 8. Suche (#839)

ä/ö/ü/ß werden zu ae/oe/ue/ss aufgelöst, ein Treffer darf aber nicht innerhalb eines aufgelösten Zeichens beginnen
oder enden. Geprüft: „schae“ und „schä“ → Schäfer; „strasse“ → Straße; „ae“ → Bär; „er“ und „e“ → nicht Bär.
Nebenwirkung: Beim Tippen von „Bär“ trifft „ba“ noch nicht (erst „bä“ oder „bae“). Das ist der Preis der strengen
Regel; Alternative wäre, nur das Ende innerhalb des Zeichens zu erlauben.

## 9. Kein Funktionsverlust

| Heute | Entwurf |
|---|---|
| Einwilligung je Person dokumentieren (zustimmen, ablehnen) | Karte „Einwilligungen“ in Planen, je Person Menü Zustimmung / Ablehnung / Widerruf |
| Widerruf, Verlauf | Widerruf setzt auf offen, Nachweis bleibt; aufklappbarer „Verlauf“ (`offen=einwilligungen`) |
| Zusammenfassung „1 zugestimmt, 1 offen, 1 abgelehnt“ | Reihenfolge unverändert (Test `planung.spec.ts:384`) |
| Lauf, Zeitboxen, Weiter/Zurück/Anhalten | Durchführen; läuft über Registerwechsel hinweg |
| Verschiebung bestätigen | MessageBar in „Termin & Zusagen“ |
| Löschen mit Rückfrage | „…“ → „Löschen …“ → bestehender Dialog |
| Dauer-Empfehlung | 25 / 50 wie `EMPFOHLENE_DAUERN` |
| Schmal: Liste, dann Blatt mit Zurück | unverändert, Knopf „Zurück zu Planungen“ (`planung-zurueck`) |

Die Vorschau sagte in Revision 1 „Nur Anzeige“ bei den Einwilligungen – das war falsch und ist entfernt.

## 10. Tests: so bedienen sie die neue Struktur

Alle `data-testid` bleiben. Die Specs dürfen die Struktur nicht umgehen, sondern bedienen sie wie ein Mensch:

- **`planung.spec.ts:342` Verschiebung:** nach der Zeilenwahl zuerst
  `page.getByRole("tab", { name: /Termin & Zusagen/ }).click()`, dann wie bisher `planung-termin`,
  `planung-status-verschoben`, `planung-verschiebung-bestaetigen`. Zusätzlich: Klick auf den Insel-Chip
  „Verschoben“ öffnet dasselbe Register und fokussiert die MessageBar.
- **`planung.spec.ts:384` Einwilligungen:** `page.getByRole("button", { name: /Einwilligungen/ }).click()`
  (Accordion-Kopf, `aria-expanded` wird `true`), danach `planung-einwilligungen` wie bisher.
- **`planung.spec.ts:431` Löschen:** `page.getByRole("button", { name: "Weitere Aktionen zur Planung" }).click()`,
  dann `page.getByRole("menuitem", { name: /Löschen/ }).click()`; `planung-loeschen` sitzt auf dem Menüeintrag,
  der Rückfragedialog bleibt.
- **„Schmal“ (390 px):** unverändert; Liste und Planung werden wie heute bedingt gerendert (nicht nur per CSS
  versteckt), sonst scheitert `toHaveCount(0)`.
- **Neu:** Tastaturabnahme aus §6 als Spec; Lauf bleibt bei Registerwechsel (`lauf-chip` sichtbar in „Planen“);
  Bestätigungsdialoge für Freigabe und Beenden; Suchgrenzen aus §8 als Unit-Test zu `auswahlSuche`.

Akzeptanz je Paket: `pnpm test:playwright tests/planung` → grün, dazu Bilder hell/dunkel bei 1440 und 2560.

## 11. Pakete

| Paket | Scope | Akzeptanz | Abhängig |
|---|---|---|---|
| **U1 Rahmen** | Insel + Register (`PlanungDetail.tsx`, `planung.css`), Inhalte nur verteilen, Aktionen je Register, „…“-Menü, Bestätigung „Beenden“, Tokens statt `--plg-*`-Farben, Insel-Schalter, forced-colors, Druck; für Protokoll nur Platz reservieren (Register zeigt heutige `ProtokollKarte`) | Specs :342/:384/:431 nach §10 angepasst und grün | – |
| **U2 Liste** | Suche, Gruppen, Zustandschip, Zähler, Schublade < 1000 px, Leer-/Ladezustand | `pnpm test:playwright tests/planung` grün, Bild 900 | U1 |
| **U3 Planen** | Raster, Accordion, Zeitband, 25/50, Entscheidungs-Umschalter, Agenda per Tastatur, Einwilligungen | wie oben + Tastatur-Spec | U1, #839 |
| **U4 Durchführen** | Restzeit-Ring, Zeitbox-Spur (Darstellung zu `AgendaZeitboxen`/`LaufPanel`) | wie oben | U1 |
| **U5 Protokoll** | Freigabe-Schritte, Bestätigung „Freigeben und Audio löschen“, Fehlerzustand | wie oben | U1, **#738** |
| Schwester-Issues | #839 Combobox → #840 Vorlagenwahl → #843 Zusagen/Erinnerung/Vorab-Info → #841 Designprofil → #842 (nach ADR) | je Issue | U1 |

## 12. Nachweis je Auflage

### Review Opus

| # | Auflage | Umsetzung | Beleg |
|---|---|---|---|
| 1 | eine Primäre je Ansicht | Kopfbefehle sekundär; Planen: „Speichern“; Termin: „Webex nachziehen“; Durchführen: „Weiter zu Punkt 4“; Protokoll: „Protokoll freigeben …“ | Hauptbilder 1440 |
| 2 | jeder Zustand einmal, dort wo die Aktion ist | „Verschoben“: Chip (Knopf → Termin, Fokus MessageBar), Phaseninfo nur Symbol, Termin-Karte nur gestrichene Altzeit, in Planen nichts; „Aufnahme läuft“ nur Insel; Abgleich nur in der Zusagen-Karte | `844-planen/termin-1440-*` |
| 3 | Leer, Laden, Fehler | erste Planung, leeres Archiv, keine Antworten; Skeleton Liste + Zusagen; MessageBar mit „Erneut versuchen“ für Outlook, Webex, Übergabe, Protokoll | Gruppe Leer/Laden/Fehler |
| 4 | responsiv | `table-layout: fixed`, Spalten fallen weg, Erinnerung unter Namen; kompakte Register < 900; Schublade < 1000; Mindestfenster 800 × 600 (§1) | `844-*-1280-*`, `*-900-*`, `*-390-*` |
| 5 | Tastatur | Vorlagenliste und Combobox mit `aria-activedescendant`, Menü mit Pfeilen; Abnahme §6 | `tasten.js` 29/29 |
| 6 | Insel zu schwer, dunkel unsichtbar | einzeilig beim Scrollen; dunkel Navy-Verlauf 1,32:1; Schalter bleibt | `844-kompakt-1440-dunkel`, §3 |
| 7 | „Alle“ ohne Doppel, eine Auswahl | 22 Einträge, 22 verschieden, genau einer `aria-selected` | `844-vorlagen-*` |
| 8 | Kacheln getönt | Brand-Tönung, „Eigene“ grün | `844-vorlagen-1440-hell` |
| 9 | Infos aus echtem Stand | Ableitungstabelle §4 | `844-durchfuehren/protokoll-*` |
| 10 | Bestätigungen | „Protokoll freigeben und Audio löschen?“, „Besprechung beenden?“; Beenden in der Insel; kein Entf | `844-bestaetigen-*` |
| 11 | 2560: Tabelle begrenzen, Platz nutzen | Zusagen ≤ 1100 px, Termin und Entwürfe daneben | `844-termin-2560-*` |
| 12 | InfoBar-Rand dunkel gedämpft | Rand 45 % gemischt | `844-termin-1440-dunkel` |
| 13 | Phasenring in der Liste | entfernt | Liste in allen Bildern |
| 14 | schärfere Etiketten | „In Outlook“, „Wartet auf Termin“; „Webex aktuell“ bewusst nicht (§7) | Liste, Insel |
| 15 | Abschneidungen | Ziel einzeilig ohne Kürzung, „Erinnerung an 3 Personen“ ganz, Platzhalter „Name tippen, z. B. „schae““, Zeitbox-Spur nur Ziffern; englische Rahmentexte passen | `844-planen-lang`, `844-en-1440-hell` |
| 16 | Zeitband-Titel als Tooltip | Ziffern + `title` + `sr-only` | Planen, Durchführen |
| 17 | Strg+F | verdrahtet, WebView2-Prüfung als Bedingung (§6) | `tasten.js` |

### Auflagen Codex

| Auflage | Umsetzung | Beleg |
|---|---|---|
| (a) Zuschnitt U1 → U2 → U3 → U4/U5; U5 auf #738; U1 reserviert nur Platz für Protokoll | übernommen | §11 |
| (b) Insel nur mit Schalter, forced-colors, Druck; dunkle Kacheln widersprechen „einzige Abweichung“ | alle drei umgesetzt, Kacheln getönt | §5, Bilder Insel-Varianten |
| Tests bedienen Register, Accordion, Überlauf (:342, :384, :431) | Rezepte je Test | §10 |
| kein Funktionsverlust (Einwilligung bearbeiten, Widerruf, Verlauf; Lauf über Registerwechsel; Agenda per Tastatur) | umgesetzt, „Nur Anzeige“ entfernt | §9, `844-einwilligungen-1440-hell` |
| Dauer 25/50 | `ToggleButton` 25/50 | Planen-Bilder |
| Namen für Tabpanels, Vorlagenliste per Tastatur, Fokus im Dialog | `aria-labelledby`, Listbox-Tasten, Fokusfalle, Rückgabe | §6 |
| 390 px: Liste und Planung mit Zurück, 44-px-Ziele | umgesetzt | `844-mobil-*` |
| Quelle und Abrufzeit sichtbar, „unbekannt“ getrennt, „zuletzt angeschrieben“ nur aus echtem Ereignis, Webex nur Hinweis | umgesetzt | §7, `844-termin-*` |
| Suche ohne Teiltreffer in aufgelösten Zeichen | Grenzprüfung | §8 |

## 13. Bewusst nicht umgesetzt

- Listen-Chip „Verschoben“ bleibt: Die Liste ist die Übersicht über alle Planungen, dort ist es der einzige Hinweis
  ohne Öffnen. In der gewählten Planung steht der Zustand einmal (Insel).
- Etikett „Webex aktuell“: Die App weiß das vor #842 nicht; ein grünes Etikett wäre eine Behauptung.
- Mitlaufende Seitenspalte (`sticky`) in Planen: nur sinnvoll, wenn sie kürzer als der Inhalt bleibt; offen.
- Nicht gezeichnet: Entwurfsdialoge Erinnerung/Vorab-Info (#843), Zuordnung beim HTML-Import (#841),
  Vorlagenverwaltung (#840). Sie folgen dem Dialogmuster der Vorlagenwahl.
- Agenda bei 1440 px: Das längste Thema („Offene Punkte aus dem letzten Termin“) wird im Eingabefeld knapp
  angeschnitten (Feld scrollt, `title` trägt den vollen Text), damit Namen in „Verantwortlich“ ganz bleiben.
- Englisch: Die Vorschau übersetzt nur Rahmentexte (Längenprobe). Die ~306 fehlenden `tts.*`-Schlüssel sind
  vorbestehend und nicht Teil von #844.
