# lva#844 – Designentwurf „Planung“, Revision 3 (Begründung)

Vorschau: `844-entwurf.html` (eigenständig, keine externen Ressourcen, nur example.org-Daten, Lucide-Symbole inline).
Bedienung: Register der Planung per Klick oder Pfeiltasten; Strg+N öffnet die Vorlagenwahl; „Besprechung starten …“
in der Kopfzeile öffnet den Startdialog; „…“ öffnet das Überlaufmenü; Thema oben rechts.

Zustände per Adresse (kombinierbar):

| Parameter | Werte |
|---|---|
| `thema` | `hell`, `dunkel` |
| `ansicht` | `planen`, `termin`, `durchfuehren`, `protokoll` |
| `stand` | `vorher` (Termin steht, verschoben), `laeuft`, `nachher` – Vorgabe ergibt sich aus der Ansicht |
| `stufe` | `u1` – Protokoll-Register als nicht verfügbarer Platzhalter (Stand nach Paket U1, vor #738) |
| `zustand` | `leer`, `archivleer`, `laden`, `keinezusagen`, `fehler-outlook`, `fehler-uebergabe`, `fehler-protokoll` |
| sonst | `dialog=vorlagen\|start`, `suche=…`, `menue=1`, `combo=schae`, `bestaetigen=freigeben\|beenden`, `offen=einwilligungen`, `insel=aus`, `kompakt=1`, `drawer=1`, `auswahl=liste`, `lang=en` |

Die Bilder entstanden unter Linux ohne Segoe UI (Ersatzschrift Inter; unter Windows laufen Texte etwas schmaler),
Playwright Chromium, deviceScaleFactor 1. Prüfskripte unter `tools/`: `shots.js` (Bilder; meldet Skriptfehler,
Seitwärts-Überlauf und leere Symbole – letzter Lauf ohne Befund), `tasten.js` (Tastatur-, Start- und Suchabnahme,
39 von 39 OK), `kontrast.py` und `kontrast_zustaende.py` (WCAG-Formel), `medien.js` (Kontrastdesign, Druck).

## Änderungen gegenüber Revision 2

| Auflage (Codex, Rev. 2) | Umsetzung | Beleg |
|---|---|---|
| HOCH 1: `ProtokollKarte` gibt es auf main nicht | U1 zeigt im Register „Protokoll“ einen nicht verfügbaren Platzhalter („kommt mit #738“); Registerinfo ebenso; U5 hängt an #738. Vor der Besprechung zeigt das Register einen Leerzustand statt eines fertigen Protokolls | `844-protokoll-u1-1440-hell`, `844-protokoll-vorher-1440-dunkel`, §11 |
| HOCH 2: Startvertrag `planung-durchfuehren-start.spec.ts:24–25, 37` | „Besprechung starten …“ steht in der Kopfzeile jedes Registers, sobald gestartet werden kann; kein Registerwechsel nötig; Startdialog wie heute | `844-start-1440-*`, `844-bereit-1440-*`, §10 |
| HOCH 10: Webex nicht zusagen | Kein Webex-Anlegen, kein „Webex nachziehen“, kein Webex-Fehlerzustand (Bilder gelöscht). Einladung: „Beitrittslink kommt aus dem Outlook-Termin“ + Platzhalter „Auswahl Webex oder Teams – kommt mit #842“. Verschiebung: nur Hinweis, „Verschiebung bestätigen“ ist die Primäraktion | `844-planen-*`, `844-termin-*`, §7 |
| 6: Kontrast Hover, Auswahl, Fokus | gerechnet, gedrückter Filter mit Rand `brand-fg` (vorher 1,51 → 5,38/5,18) | §3 |
| 7: Vorlagengruppen nur per Klick | Pfeil links/rechts, Pos1/Ende in den Gruppen-Registern | §6, `tasten.js` |
| 8: DOM-Reihenfolge und 36-px-Ziele | DOM jetzt Ziel → Agenda → Rollen → Unterlagen, keine CSS-`order`-Umsortierung mehr; ab 1500 px zeilenweises Zwei-Spalten-Raster in derselben Reihenfolge. Aufklapp-Köpfe `max(36px, --ctl-h)`, also 44 px bei Touch | §1, `tasten.js` |
| 9: Abrufzeit ohne Jahr und Zeitzone | „abgerufen 25.11.2026, 14:02 MEZ“, ebenso „Letzter Stand …“ | `844-termin-*`, `844-fehler-outlook-*` |
| Entscheidung: 800 × 600 nur Zielgröße | korrigiert: App-Minimum bleibt 480 × 420 (`lib.rs`); Bild bei 480 × 420 ergänzt; Prüfung bei 390 px bleibt | §1, `844-planen-480-hell` |
| Entscheidung: Strg+F | aus Vorschau und Entwurf gestrichen (kein Kürzel, kein Hinweis im Suchfeld); als Option vermerkt | §6 |
| Entscheidung: „ba“ ↛ „Bär“ | in den Suchvertrag aufgenommen | §8 |

Nebenbei: „Einladung aktualisieren“ steht jetzt in der Karte „Einladung“ statt in der Kopfzeile. So bleibt die
Kopfzeile in „Planen“ bei „Besprechung starten …“, „Speichern“ (primär) und „…“, und die Insel wird nicht höher.

## 0. Bilder

Alle unter `/tmp/claude-0/-home-user/ea04c13c-5d5e-5296-bda5-d105a97c1932/scratchpad/` (58 Dateien, alle in Revision 3 neu erzeugt).

| Gruppe | Dateien |
|---|---|
| Hauptansichten | `844-{planen,termin,durchfuehren,protokoll,vorlagen}-{1440,2560}-{hell,dunkel}.png` (20) |
| Start | `844-start-1440-{hell,dunkel}` (Startdialog), `844-bereit-1440-{hell,dunkel}` (Durchführen vor dem Start) |
| Protokoll vor #738 | `844-protokoll-u1-1440-hell` (U1-Platzhalter), `844-protokoll-vorher-1440-dunkel` (vor der Besprechung) |
| Bedienung | `844-vorlagen-suche-1440-dunkel`, `844-combobox-1440-hell`, `844-menue-1440-dunkel`, `844-planen-lang-1440-hell`, `844-einwilligungen-1440-hell` |
| Breiten | `844-planen-1280-hell`, `844-termin-1280-dunkel`, `844-planen-900-hell`, `844-termin-900-dunkel`, `844-drawer-900-hell`, `844-planen-480-hell` (480 × 420), `844-mobil-liste-390-hell`, `844-mobil-termin-390-dunkel` |
| Leer / Laden | `844-leer-1440-{hell,dunkel}`, `844-leer-archiv-1440-hell`, `844-leer-zusagen-1440-dunkel`, `844-laden-1440-{hell,dunkel}` |
| Fehler | `844-fehler-outlook-1440-{hell,dunkel}`, `844-fehler-uebergabe-1440-hell`, `844-fehler-protokoll-1440-dunkel` |
| Bestätigung | `844-bestaetigen-freigeben-1440-{hell,dunkel}`, `844-bestaetigen-beenden-1440-{hell,dunkel}` |
| Insel-Varianten | `844-kompakt-1440-dunkel`, `844-insel-aus-1440-hell`, `844-kontrastdesign-1440` (forced-colors), `844-druck-1440` (Druck) |
| Textlänge | `844-en-1440-hell` (englische Rahmentexte) |

## 1. Struktur

```
Seitenkopf: Planung + Beschreibung · [Mit Kalender abgleichen] [Neue Planung aus Vorlage | ▾]   (beide sekundär)
┌ Liste (300–400 px) ─────────┬ Planung ────────────────────────────────────────────────────────────┐
│ Suche                       │ INSEL: Titel · Zustandschips · [Besprechung starten …] [Primär] [ … ]│
│ Aktiv · Archiv · Alle       │        Fakten: Zeit · Dauer · Teilnehmende · Online-Beitritt · …    │
│ ─ Diese Woche ─             │        Register ① Planen ② Termin & Zusagen ③ Durchführen ④ Protokoll│
│ Titel                       │        (je ein abgeleiteter Satz zum Stand, s. §4)                  │
│ Datum ▫ Online  4/9 [Chip]  ├─────────────────────────────────────────────────────────────────────┤
│ ─ Nächste Woche ─ …         │ Meldungen des Registers (MessageBar) · Inhalt (Karten, Container-Q.)│
└─────────────────────────────┴─────────────────────────────────────────────────────────────────────┘
```

Reihenfolge in „Planen“: Das DOM folgt der Lesereihenfolge Ziel → Agenda → Rollen → Unterlagen, danach die
Seitenspalte (Prüfung, Einladung, Einwilligungen). Bis 1499 px steht das einspaltig untereinander; ab 1500 px
Planungsbreite liegt es in einem Zwei-Spalten-Raster, zeilenweise in derselben Reihenfolge (Ziel | Agenda,
Rollen | Unterlagen). Keine CSS-`order`-Umsortierung, also stimmen Tab-Folge und Bild überein (WCAG 1.3.2, 2.4.3).

Breiten (Container-Queries am Planungsbereich, Media-Queries am Fenster):

| Fenster | Verhalten |
|---|---|
| ≥ 1500 px Planungsbreite | „Planen“ dreispaltig; „Termin“: Zusagen höchstens 1100 px, daneben Termin und Entwürfe |
| 1000–1499 px | Liste links, Planung zweispaltig |
| < 1100 px Karte | Spalte „Rolle“ in der Zusagentabelle aus; < 1000 px Spalte „Erinnerung“ aus, Angabe unter dem Namen |
| < 1000 px | Liste als Schublade (Knopf links neben „Planung“), Planung volle Breite |
| < 900 px | Registerleiste kompakt: Ziffern, Name und Stand nur beim gewählten Register |
| < 640 px oder < 560 px hoch | Liste ODER Planung mit „Zurück zu Planungen“; Navigationsschiene hinter ≡; Befehle nur als Symbol (mit Namen); Ziele 44 px (auch bei `pointer: coarse`); Dialoge vollflächig; Insel ohne Faktenzeile (Fakten stehen in „Termin & Zusagen“) |

**Fenstergröße.** Das App-Minimum bleibt, wie in `src-tauri/src/lib.rs` gesetzt, bei 480 × 420 logischen Pixeln
(`min_inner_size`); die Startgröße ist 800 × 600 (`inner_size`). 800 × 600 ist für diesen Entwurf die **Zielgröße**, bei
der alles ohne Scrollen der Kopfzeile bequem passt. Ein neues globales Minimum ist nicht vorgeschlagen. Bei 480 × 420
(Bild `844-planen-480-hell`) bleiben die Kopfzeile ohne Fakten, die kompakte Registerleiste und der Anfang des
Inhalts sichtbar; beim Scrollen wird die Insel einzeilig. Die Prüfung bei 390 px (Test „Schmal“) bleibt.

## 2. Komponenten und Fluent-Entsprechung

| Entwurf | Fluent 2 / bestehend | Anmerkung |
|---|---|---|
| Seitenkopf | `PageShell` (vorhanden) | zwei sekundäre Befehle; Primär nur in Insel oder Inhalt |
| „Neue Planung aus Vorlage“ | `SplitButton` + `Menu` | sekundär |
| Listensuche | `SearchBox` | kein Tastenkürzel (s. §6) |
| Aktiv/Archiv/Alle | `TabList size="small"` (vorhanden `plg-filter`) | Zähler im Label |
| Listenzeile | eigene Zeile, `aria-current` (vorhanden) | Chip zeigt Zustand |
| Zustandschip | `Badge appearance="tint" shape="rounded"` (`Statusmarke`) | immer Symbol + Text; „Verschoben“ in der Insel ist ein Knopf |
| Insel | eigene Fläche, Tokens lokal überschrieben | abschaltbar, s. §5 |
| „Besprechung starten …“ | `Button` (primär nur im Register „Durchführen“) | `data-testid="planung-durchfuehren-start"`, öffnet den bestehenden Startdialog (`StartPanel`) |
| Register | `TabList` + `Tab`, Panels `role="tabpanel"` + `aria-labelledby` | Panels bleiben gemountet (`hidden`), s. §10 |
| Überlaufmenü „…“ | `Menu`/`MenuList`/`MenuItem` | Pfeile, Pos1/Ende, Esc gibt Fokus zurück; kein Tastenkürzel für Löschen |
| Meldungen | `MessageBar intent=warning\|error` + `MessageBarActions` | „Erneut versuchen“ bei jedem Fehler |
| Bestätigung | `Dialog modalType="alert"` | Fokus auf „Abbrechen“ |
| Karte / Aufklappkarte | `Card`, `Accordion` | Zusammenfassung mit Zählern; Inhalt bleibt im DOM (s. §10) |
| Felder | `Field` + `Input`/`Textarea`/`Dropdown` | Label oben, Hilfe unten |
| Rollen, Verantwortlich | `Combobox` mit `OptionGroup` (#839) | `aria-activedescendant`, Pfeile, Enter |
| Dauer-Schnellwahl | `ToggleButton` | 25 / 50 aus `EMPFOHLENE_DAUERN` (`logik.ts:63`) |
| Entscheidungspunkt | `ToggleButton` (Symbol Waage, `aria-pressed`) | |
| Agenda-Reihenfolge | Griff-Knopf + Alt+Pfeil hoch/runter | Fokus wandert mit |
| Online-Besprechung | Text + gestrichelter Platzhalter | Auswahl Webex/Teams erst mit #842 |
| Zeitband, Zeitbox-Spur | eigenes Flex-Band, nur Ziffern | Titel per `title` und `sr-only` |
| Zusagen-Filter | `ToggleButton shape="circular"` in `Toolbar` | gedrückt: Rand `brand-fg` |
| Zusagentabelle | `Table` mit `table-layout: fixed` | Quelle + Abrufzeit mit Jahr und Zone über der Tabelle |
| Vorlagenwahl | `Dialog` + `Combobox`-Suche + `TabList` (Gruppen) + `Listbox` | eine Auswahl, Gruppen ohne Doppel |
| Vorlagen-Symbolkachel | getönt `colorBrandBackground2`/`colorBrandForeground2` | „Eigene“ grün getönt |
| Platzhalter „kommt mit #…“ | gestrichelter Rahmen, `aria-disabled` | nur im Entwurf; in der App erscheint die Stelle erst mit dem Issue |
| Leerzustände | eigener Baustein (Symbol, Satz, Aktion) | |
| Laden | `Skeleton`/`SkeletonItem` | `aria-busy`, Statusmeldung für Screenreader |
| Symbole | lucide-react (vorhanden) | 12–20 px |

Die Vorschau baut Combobox, Listbox, Menü und Dialog von Hand nach, nur um das Verhalten zu zeigen. In der App kommen
dafür ausschließlich die Fluent-Bausteine der Tabelle zum Einsatz.

## 3. Kontrast (WCAG 2.2 AA)

Text 4,5:1, Bedienrahmen und Grafik 3:1. Fluent-Werte aus `webLightTheme`/`webDarkTheme`.

**Ruhezustand**

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
| Balken zugesagt / abgesagt / Vorbehalt | 5,37 / 6,07 / 3,96 | 5,34 / 4,16 / 9,02 |

**Hover, Auswahl, Fokus** (`tools/kontrast_zustaende.py`)

| Paar | Hell | Dunkel |
|---|---|---|
| Text fg1 auf Hover (Zeile, Knopf, Menüeintrag) | 14,24 | 10,86 |
| Hilfetext fg3 auf Hover | 5,68 | 4,84 |
| Text fg1 auf Auswahl (Listenzeile, Vorlage) | 13,02 | 11,73 |
| Hilfetext fg3 auf Auswahl | 5,20 | 5,23 |
| Auswahl-Balken links (brand-fg) gegen Auswahlfläche | 4,52 | 4,18 |
| Register-Unterstrich (brand-fg) gegen Karte | 5,38 | 5,18 |
| Gedrückter Filter: Text auf brand-bg2 | 5,95 | 6,62 |
| Gedrückter Filter: Rand brand-fg gegen Karte (neu, vorher 1,51 / 1,85) | 5,38 | 5,18 |
| Weiß auf Primär-Hover | 6,66 | 5,38 |
| Fokusring fg1 (2 px) gegen Karte / Basis / Auswahl | 15,52 / 14,24 / 13,02 | 14,55 / 18,42 / 11,73 |
| Fokusring am Primärknopf | Ring sitzt mit 1 px Abstand auf der Fläche dahinter (≥ 14:1); gegen den Knopf selbst 2,88 | 6,66 |

**Insel** (hell: Verlauf #13263C → #15161A; dunkel: #203450 → #1C2D42)

| Paar | Hell | Dunkel |
|---|---|---|
| Titel Weiß | 15,33 | 12,59 |
| Fakten #C9CED6 | 9,69 | 8,84 |
| Phaseninfo #A3A9B3 / auf Hover / auf gewähltem Register | 6,48 / 5,44 / 5,13 | 5,32 / 4,91 / 4,62 |
| Akzent #62ABF5 / Unterstrich gegen gewähltes Register | 6,32 / 5,00 | 5,19 / 4,50 |
| Warnung #FBB48A | 8,76 | 7,98 |
| Weiß auf Knopf-Hover (+12 % Weiß) | 10,61 | 9,62 |
| Fokusring Weiß gegen Insel | 15,33 | 13,97 |
| Insel gegen Basis | 16,58 | 1,32 |

Bewusste Grenzfälle: Den Feldrand ringsum (`stroke1`, 1,53 / 2,53) erkennt man nicht, das Feld aber an der unteren
Kante (≥ 6:1), wie in Fluent. Der Verteilungsbalken trägt nie allein, dieselben Zahlen stehen mit Symbol und Text in den
Filtern. Mit `insel=aus` gelten die normalen Karten-Werte. Im Windows-Kontrastdesign fällt die Insel auf
`Canvas`/`CanvasText` zurück, die Auswahl auf `Highlight`.

## 4. Zustand ableiten statt behaupten (Regel)

Alles, was die Insel und die Register über den Stand sagen, wird aus dem Datenstand berechnet, nie aus der gewählten
Ansicht. Ein Registerwechsel ändert nichts am Stand: Lauf und Zeitmessung laufen weiter, „Aufnahme läuft“ und
„Besprechung beenden …“ bleiben in der Insel, egal welches Register offen ist.

| Stand | Chips | Aktion in der Insel | Termin & Zusagen | Durchführen | Protokoll |
|---|---|---|---|---|---|
| vorher, verschoben | Verschoben (Knopf), In Outlook | Besprechung starten … | 4 von 9 zugesagt + Warnsymbol | Datum; Inhalt „Bereit zum Start“ | nach der Besprechung; Leerzustand |
| vorher, keine Antworten | In Outlook | Besprechung starten … | 0 von 9 geantwortet | wie oben | wie oben |
| läuft | In Outlook | Aufnahme läuft · Besprechung beenden … | 6 anwesend | Punkt 3 von 5, 22:36; Lauf | nach der Besprechung |
| nachher | Durchgeführt | Exportieren (in „Protokoll“) | 6 anwesend | Beendet 10:52, 50 Min.; Zusammenfassung | Version 2, Freigabe offen |
| U1 (vor #738) | wie oben | wie oben | wie oben | wie oben | „kommt mit #738“; Platzhalter |

Fakten: „Transkription geplant“ nur vorher, „Transkript lokal, 50 Min.“ nur nachher; „Online-Beitritt“, wenn der
Outlook-Termin einen Beitrittslink hat. Die Daten dafür liefert die Planung selbst (`statusMarken`, Lauf, Protokoll);
eine eigene Statusvariable für die Oberfläche gibt es nicht.

## 5. Insel: abschaltbar, Kontrastdesign, Druck

- Abschalten: Klasse `insel--aus` (Vorschau `insel=aus`) ergibt einen normalen Fluent-Kopf auf Background1.
- Kompakt: ab 24 px Scrolltiefe einzeilig (Titel, Ziffern-Register, Aktionen). „Verschoben“ bleibt sichtbar.
- Kontrastdesign: eigener `forced-colors`-Block, keine Verläufe, Ränder in `CanvasText`.
- Druck: nur die gewählte Planung ohne App-Rahmen, Register und Aktionen; Insel weiß, Chips schwarz umrandet.
- Die Vorlagenkacheln sind getönt; die Insel ist die einzige Abweichung von reinem Fluent.

## 6. Tastatur und Fokus

Abnahme ohne Maus (in `tools/tasten.js` automatisiert, 39 Prüfungen, alle OK):

1. Strg+N → Dialog offen, Fokus in der Suche, App dahinter `inert`.
2. „fix“ tippen → Liste filtert; Pfeil runter → `aria-activedescendant` wechselt, Option `aria-selected`.
3. Vorlagengruppen (Alle, Zuletzt verwendet, Eigene, Mitgeliefert): Pfeil links/rechts, Pos1/Ende wechseln und
   fokussieren die Gruppe (roving tabindex).
4. Tab ×12 → Fokus bleibt im Dialog. Enter → Planung angelegt, Dialog zu, Fokus zurück auf den Auslöser.
5. „…“ mit Enter → erster Eintrag fokussiert; Pfeile, Ende = „Löschen …“, Esc → Fokus zurück auf „…“.
6. Register: Pfeil rechts → „Termin & Zusagen“; Panel trägt `aria-labelledby="tab-termin"`.
7. Rollen-Combobox: „schae“ → Pfeil → Enter übernimmt die Person.
8. Agenda: Alt+Pfeil runter im Thema-Feld verschiebt den Punkt, Fokus folgt.
9. „Besprechung starten …“ → Startdialog, Fokus auf der ersten offenen Zustimmung; nach jeder Zustimmung springt
   der Fokus zur nächsten, zuletzt auf „Mit Transkription starten“.
10. „Protokoll freigeben …“ → Fokus auf „Abbrechen“; Esc → Fokus zurück auf den Auslöser.

**Strg+F ist gestrichen**: kein Kürzel, kein Hinweis im Suchfeld. Option für später: nur nach einem echten
WebView2-Test (unterdrückt `preventDefault` die eingebaute Suchleiste?) wieder aufnehmen. Strg+N bleibt im Entwurf, wird
aber im selben WebView2-Test mitgeprüft und bei Kollision ebenso gestrichen. Kein Kürzel
für Löschen.

## 7. Datenwahrheit (Zusagen, Online-Besprechung)

- Quelle und Abrufzeit stehen sichtbar über der Tabelle: „Quelle Outlook-Kalender des Organisators, abgerufen
  25.11.2026, 14:02 MEZ · Abgleichen“. Die Zeitzone ist die des Geräts (Kurzname, z. B. MEZ/MESZ). Statusleiste und
  Termin-Karte zeigen keinen Abgleich.
- „Unbekannt“ ist eine eigene Antwort (Person im Kalender nicht gefunden), getrennt von „Keine Antwort“.
- Spalte „Erinnerung“ zeigt nur, was die App selbst getan hat („Entwurf 25.11.“ = Entwurf geöffnet).
- **Online-Besprechung vor #842:** Die App legt keine Webex- oder Teams-Besprechung an, zieht keine nach und meldet
  dazu keinen Fehler. Sie zeigt den Beitrittslink aus dem Outlook-Termin. Bei einer Verschiebung sagt die Meldung nur:
  „Ob die Online-Besprechung mitgewandert ist, sieht die App nicht; prüfen Sie den Beitrittslink im Outlook-Termin.“
  Aktionen: „Verschiebung bestätigen“ (primär) und „In Outlook öffnen“. Die Auswahl Webex/Teams steht im Entwurf nur
  als gestrichelter Platzhalter „kommt mit #842“; ihr Fehlerzustand wird mit #842 entworfen.
- Fehler Outlook: letzter Stand bleibt lesbar (kein Abblenden), markiert mit Randlinie und „Letzter Stand 25.11.2026,
  14:02 MEZ … Neuere Antworten fehlen.“

## 8. Suchvertrag (#839)

ä/ö/ü/ß werden zu ae/oe/ue/ss aufgelöst; ein Treffer darf nicht innerhalb eines aufgelösten Zeichens beginnen oder
enden. Verbindliche Beispiele (in `tasten.js` geprüft, als Unit-Test zu `auswahlSuche` übernehmen):

| Eingabe | Text | Treffer |
|---|---|---|
| schae, schä | Lea Schäfer | ja |
| strasse | Hauptstraße 1 | ja |
| ae | Bär | ja |
| er, e | Bär | nein |
| ba | Bär | **nein** – akzeptiert: Treffer erst mit „bä“ oder „bae“ |

## 9. Kein Funktionsverlust

| Heute | Entwurf |
|---|---|
| Einwilligung je Person dokumentieren | Karte „Einwilligungen“ in Planen, je Person Menü Zustimmung / Ablehnung / Widerruf |
| Widerruf, Verlauf | Widerruf setzt auf offen, Nachweis bleibt; Verlauf in der Karte |
| Zusammenfassung „1 zugestimmt, 1 offen, 1 abgelehnt“ | Reihenfolge unverändert (`planung.spec.ts:384`) |
| Start mit Zustimmung je Person, Notizenstart | „Besprechung starten …“ → bestehender Startdialog, unverändert |
| Lauf, Zeitboxen, Weiter/Zurück/Anhalten | Durchführen; läuft über Registerwechsel hinweg |
| Verschiebung bestätigen | MessageBar in „Termin & Zusagen“ |
| Löschen mit Rückfrage | „…“ → „Löschen …“ → bestehender Dialog |
| Dauer-Empfehlung | 25 / 50 wie `EMPFOHLENE_DAUERN` |
| Schmal: Liste, dann Blatt mit Zurück | unverändert, `planung-zurueck` |
| Beitrittslink aus Outlook (`TerminKarte`) | Termin-Karte, Zeile „Beitritt“ mit Kopieren |

## 10. Tests: so bedienen sie die neue Struktur

Alle `data-testid` bleiben. Zwei Regeln für die Umsetzung halten die bestehenden Zählungen gültig:
**(R1)** Register-Panels bleiben gemountet und werden mit `hidden` ausgeblendet. **(R2)** Die Karte „Einwilligungen“
behält ihren Inhalt samt Verlauf im DOM, auch zugeklappt (eigener Aufklapper mit `hidden` statt eines
Fluent-`AccordionPanel`, das zugeklappt nichts rendert). `toHaveCount` zählt DOM-Knoten, nicht Sichtbarkeit.

**Startvertrag `planung-durchfuehren-start.spec.ts`** – Registerweg: keiner nötig.

- Nach der Auswahl (`:24`) öffnet die Planung im abgeleiteten Register (vor dem Start: „Planen“). In der Kopfzeile
  steht in jedem Register „Besprechung starten …“ mit `data-testid="planung-durchfuehren-start"`; aktiv nach denselben Regeln wie
  heute (Logik unverändert, nur der Ort wechselt). **`:25` bleibt unverändert** (`toBeEnabled()` direkt nach der Auswahl).
- **`:37` bleibt unverändert**: Klick öffnet den Startdialog (`role="dialog"`); darin wie heute `planung-start-person`,
  `planung-start-zustimmen`, `planung-start-notizen`, `planung-start-los`. Der Dialog ist `StartPanel`, nur der
  Auslöser zieht in die Kopfzeile.
- `:57` (`planung-verlauf-eintrag` = 2 direkt nach der Auswahl) bleibt dank R1 + R2 unverändert.
- Nach „los“ wechselt der Stand auf „läuft“; die Ansicht springt einmal auf „Durchführen“ (Folge der eigenen Aktion,
  kein späteres Zurückspringen). `planung-lauf-modus` ist sichtbar – Assertion unverändert. Die Verlaufszählung = 4
  und die Reihenfolge c, b, a (`:98–102`) bleiben dank R1 + R2 unverändert.
- Notizenstart-Tests (`:104` ff.) unverändert; sie nutzen denselben Startknopf.
- Neu: Die Vorschau prüft, dass der Startknopf in „Planen“ und „Termin & Zusagen“ sichtbar und aktiv ist und dass
  „Mit Transkription starten“ erst nach allen Zustimmungen aktiv wird (`tasten.js`).

**Weitere Specs**

- `planung.spec.ts:342` Verschiebung: nach der Zeilenwahl `page.getByRole("tab", { name: /Termin & Zusagen/ }).click()`,
  dann wie bisher `planung-termin`, `planung-status-verschoben`, `planung-verschiebung-bestaetigen`. Zusätzlich:
  Klick auf den Insel-Chip „Verschoben“ öffnet dasselbe Register und fokussiert die Meldung.
- `planung.spec.ts:384` Einwilligungen: Zählungen gelten dank R2 ohne Klick; für Sichtbarkeitsprüfungen vorher
  `page.getByRole("button", { name: /Einwilligungen/ }).click()` (`aria-expanded` → `true`).
- `planung.spec.ts:431` Löschen: `page.getByRole("button", { name: "Weitere Aktionen zur Planung" }).click()`, dann
  `page.getByRole("menuitem", { name: /Löschen/ }).click()`; `planung-loeschen` sitzt auf dem Menüeintrag.
- „Schmal“ (390 px): unverändert; Liste und Planung werden wie heute bedingt gerendert (R1 gilt nur für die Register
  innerhalb der Planung).
- Neu: Tastaturabnahme aus §6, Suchvertrag aus §8, Bestätigungsdialoge Freigabe/Beenden, Lauf bleibt bei
  Registerwechsel.

Akzeptanz je Paket: `pnpm test:playwright tests/planung` (alle `tests/planung*.spec.ts`) → grün, dazu Bilder hell/dunkel
bei 1440 und 2560.

## 11. Pakete

| Paket | Scope | Akzeptanz | Abhängig |
|---|---|---|---|
| **U1 Rahmen** | Insel + Register (`PlanungDetail.tsx`, `planung.css`); vorhandene Inhalte nur verteilen (Planen: `PlanungFormular`, `Regelhinweise`, `EinladungsVorschau`, `Einwilligungen`/`EinwilligungsVerlauf`; Termin: `TerminKarte`; Durchführen: `StartPanel`, `LaufPanel`, `AgendaZeitboxen`); „Besprechung starten …“ in die Insel; Aktionen je Register; „…“-Menü; Bestätigung „Beenden“; Tokens statt `--plg-*`-Farben; Insel-Schalter, forced-colors, Druck; R1 + R2. **Register „Protokoll“ ist ein nicht verfügbarer Platzhalter** („kommt mit #738“) | `pnpm test:playwright tests/planung` grün mit den Anpassungen aus §10; `planung-durchfuehren-start.spec.ts` ohne Änderung grün | – |
| **U2 Liste** | Suche, Gruppen, Zustandschip, Zähler, Schublade < 1000 px, Leer-/Ladezustand | wie oben, Bilder 900 und 480 | U1 |
| **U3 Planen** | DOM-Reihenfolge Ziel → Agenda → Rollen → Unterlagen ohne `order`; Raster; Aufklapper; Zeitband; 25/50; Entscheidungs-Umschalter; Agenda per Tastatur; Ziele ≥ 36 px Desktop / 44 px Touch | wie oben + Tastatur-Spec | U1, #839 |
| **U4 Durchführen** | „Bereit zum Start“, Restzeit-Ring, Zeitbox-Spur (Darstellung zu `AgendaZeitboxen`/`LaufPanel`) | wie oben | U1 |
| **U5 Protokoll** | Protokoll-Register mit der Karte aus #738: Freigabe-Schritte, Bestätigung „Freigeben und Audio löschen“, Fehlerzustand | wie oben | U1, **#738** (gebunden: erst wenn #738 auf main ist) |
| Schwester-Issues | #839 Combobox → #840 Vorlagenwahl → #843 Zusagen/Erinnerung/Vorab-Info → #841 Designprofil → #842 Online-Besprechung (nach ADR; ersetzt den Platzhalter) | je Issue | U1 |

## 12. Nachweis je Auflage (frühere Runden)

### Review Opus (Revision 2, weiter gültig)

| # | Auflage | Stand in Revision 3 | Beleg |
|---|---|---|---|
| 1 | eine Primäre je Ansicht | Planen „Speichern“; Termin „Verschiebung bestätigen“; Durchführen vorher „Besprechung starten …“, im Lauf „Weiter zu Punkt 4“; Protokoll „Protokoll freigeben …“ | Hauptbilder, `844-bereit-*` |
| 2 | jeder Zustand einmal | unverändert; der Webex-Teil der Verschiebungsmeldung ist nur noch ein Hinweis | `844-termin-*` |
| 3 | Leer, Laden, Fehler | unverändert; Webex-Fehler entfällt bis #842 | Gruppe Leer/Laden/Fehler |
| 4 | responsiv | wie Rev. 2; Mindestfenster korrigiert (§1) | Breiten-Bilder |
| 5 | Tastatur | erweitert um Vorlagengruppen und Start | `tasten.js` 39/39 |
| 6 | Insel leichter, dunkel sichtbar | unverändert | `844-kompakt-1440-dunkel` |
| 7 | „Alle“ ohne Doppel | 22 Einträge, 22 verschieden, einer gewählt | `844-vorlagen-*` |
| 8 | Kacheln getönt | unverändert | `844-vorlagen-1440-hell` |
| 9 | Infos aus echtem Stand | erweitert: Durchführen und Protokoll vor/nach der Besprechung | §4 |
| 10 | Bestätigungen | unverändert | `844-bestaetigen-*` |
| 11 | 2560 | unverändert | `844-termin-2560-*` |
| 12–17 | InfoBar-Rand, Phasenring, Etiketten, Abschneidungen, Tooltips, Strg+F | Strg+F jetzt gestrichen (§6), Rest unverändert | wie Rev. 2 |

### Codex (Revision 2, weiter gültig)

Paketzuschnitt (§11), Insel-Schalter/forced-colors/Druck (§5), Testrezepte (§10), kein Funktionsverlust (§9),
25/50, Tabpanel-Namen und Fokus (§6), 390 px mit Zurück und 44-px-Zielen, Datenwahrheit (§7), Suchgrenzen (§8).

## 13. Bewusst nicht umgesetzt

- Listen-Chip „Verschoben“ bleibt: Die Liste ist die Übersicht; ohne Öffnen ist er der einzige Hinweis.
- Keine Webex-/Teams-Funktion und kein Etikett „Webex aktuell“ vor #842.
- Keine mitlaufende Seitenspalte (`sticky`) in Planen.
- Nicht gezeichnet: Entwurfsdialoge Erinnerung/Vorab-Info (#843), Zuordnung beim HTML-Import (#841),
  Vorlagenverwaltung (#840), Online-Auswahl und ihr Fehlerzustand (#842).
- Agenda bei 1440 px: Das längste Thema wird im Eingabefeld knapp angeschnitten (Feld scrollt, `title` trägt den
  vollen Text), damit die Namen ganz bleiben.
- Bei 480 × 420 sieht man ohne Scrollen nur den Anfang des Inhalts; das ist das App-Minimum, nicht die Zielgröße.
- Englisch: nur Rahmentexte als Längenprobe; die fehlenden `tts.*`-Schlüssel sind vorbestehend.
