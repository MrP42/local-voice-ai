# ChatGPT Desktop (Windows) – Funktions- und UI-Analyse

Stand: 08.11.2026 · Zweck: Inspiration und Funktionsvergleich für die Chat-Startseite von
Local Voice AI. Es wurden **keine Assets und kein Code übernommen**; zitiert werden nur kurze
UI-Beschriftungen, um Funktionen eindeutig zu benennen.

## 1. Untersuchungsgegenstand und Methode

| Punkt | Befund |
|---|---|
| Paket | `OpenAI.ChatGPT-Desktop` 1.2026.190.0 (MSIX), Produktname „ChatGPT Classic“ |
| Technik | Electron-Hülle (`app.asar`, 9 MB JS, Vite-Build), lädt die Weboberfläche von `chatgpt.com` |
| Native Ergänzung | `windowsaddon.node` (Fensterinhalte anderer Apps lesen, siehe 3.3) |
| Weitere Pakete | `OpenAI.Codex` (eigene App, nicht Gegenstand) |

Belegarten (jede Aussage unten ist einer davon zugeordnet):

- **[S] statisch** – REA `analyze_javascript_application` über `app.asar`
  (Evidence `ev_b32364fd…ae67`, Abdeckung *partial*, nichts ausgeführt), ergänzt durch
  gezieltes Lesen der entpackten Hülle (`preload.mjs`, `app-bootstrap-*.js`, Locale-Dateien).
- **[C] Cache** – Text-Konstanten aus dem lokalen V8-Code-Cache der App
  (`LocalCache\Roaming\ChatGPT\Code Cache\js`, 9 Bündel von `cdn.oaistatic.com`, darunter der
  deutsche Meldungskatalog). Belegt, *dass* die Oberfläche diese Texte kennt, nicht, dass sie
  im aktuellen Konto sichtbar sind. Persönliche Daten (Chats, IndexedDB) wurden nicht gelesen.
- **[L] Laufzeit** – Weboberfläche `chatgpt.com` im abgemeldeten Zustand, 1280×860,
  angesehen und Menüs geöffnet, nichts abgeschickt. Die Bedienung der installierten App per
  Bildschirmsteuerung wurde nicht freigegeben; da die App dieselbe Weboberfläche lädt, ist die
  Laufzeitsicht inhaltlich gleich, nur ohne Anmeldung.
- **[V] Vorwissen** – öffentlich bekannte Bedienung ohne eigene Beobachtung in diesem Lauf.

## 2. Architektur in einem Satz

Die Desktop-App ist ein **Rahmen um die Web-App**: Hülle für Fenster, Tray, globales
Tastenkürzel, Kamera/Screenshot und „Mit Apps arbeiten“; alles Fachliche (Chat, Modelle,
Projekte, Voice, Dateien) kommt per Web-Bundle vom Server und wird per Feature-Gates
(Statsig) zugeschaltet. [S]

Folge für uns: Die App ändert ihre Oberfläche ohne App-Update. Eine Analyse ist eine
Momentaufnahme; Feature-Gates in der Hülle: `chatgpt-sidetron-companion-arch`,
`chatgpt-sidetron-work-with-client`. [S]

## 3. Hülle (Desktop-spezifisch)

### 3.1 Fenster und Einstieg

- **Hauptfenster** mit der Web-App. [S]
- **Companion-Fenster** („Chatbar“): kleines schwebendes Eingabefenster, per globalem
  Tastenkürzel ein- und ausblendbar; Standard `Option+Space` (Windows: Alt+Leertaste),
  in den Einstellungen änderbar; Start wahlweise direkt ins Companion-Fenster
  (Startparameter, „startupWindow = COMPANION“). [S]
- **Tray-Menü**: Hauptfenster öffnen, Companion-Fenster öffnen, Neu laden, App-Info kopieren,
  Abmelden, Beenden. **Sprungliste** (Taskleiste): Companion öffnen, Neu laden, Beenden. [S]
- **Ladefehler-Seite** mit genau einer Aktion „ChatGPT neu starten“. [S]
- Eigener Kontextmenü-Satz (Kopieren, Einfügen, Bild speichern, Wörterbuch, Emoji). [S]
- Einstellungen der Hülle werden verschlüsselt (`safeStorage`) in `config.json` gehalten;
  einziger Standardwert ist das Companion-Kürzel. [S]

### 3.2 Brücke Web ↔ Desktop (`DesktopHostGateway`)

Die Weboberfläche erhält über die Preload-Brücke genau diese Fähigkeiten: [S]

| Fähigkeit | Bedeutung für die Bedienung |
|---|---|
| `requestSystemPermission` | Mikrofon/Kamera/Bildschirm beim Betriebssystem anfragen |
| `takePhoto` | Foto mit Webcam als Anhang |
| `takeScreenshot` | Bildschirmfoto als Anhang |
| `getSettingValue` | Hülleneinstellungen (z. B. Kürzel) lesen |
| `checkForUpdates` | Update anstoßen |
| `pairing.getApps / getAppHeader / getContentForPairedApps / getScreenshotForApp` | „Mit Apps arbeiten“ |
| `publish` / `registerHostEventListener` | Ereignisse in beide Richtungen |

### 3.3 „Mit Apps arbeiten“ (Pairing)

ChatGPT liest auf Wunsch Inhalt oder Screenshot **eines anderen geöffneten Fensters** als
Kontext (Hinweise im Bundle u. a. auf „Visual Studio Code“ und `powershell.exe`). Gesteuert
über das Gate `work-with-client`. [S] Die UI-Texte bestätigen den Zweck: „Verwende
Tastaturkürzel, um Fragen zu stellen, Dateien hochzuladen oder Screenshots am Computer zu
machen.“ [C]

### 3.4 Telemetrie

Sentry (Fehler) und Datadog-Dienstname; Ereignisse u. a. „Window Shown“, „Login Changed“,
„Companion Window Shortcut Launch“, „Keyboard Shortcut Pressed“. [S]

## 4. Chat-Startseite (leerer Zustand)

Beobachtet [L], abgemeldet, dunkles Schema:

- **Seitenleiste links** (einklappbar über Symbol oben rechts der Leiste): Neuer Chat, Bilder,
  Plugins/Apps, Deep Research, Einstellungen; angemeldet zusätzlich Suche, Bibliothek,
  Projekte und die Chatliste [C/V].
- **Kopfzeile**: links Produkt- bzw. Modellmenü („ChatGPT ▾“), rechts Konto/Anmelden.
- **Mitte**: eine einzige Begrüßungsfrage, zentriert über der Eingabezeile. Varianten im
  Katalog: „Wo sollen wir anfangen?“, „Wo sollen wir anfangen, {name}?“, „Woran arbeitest
  du?“ [C/L]. Darunter höchstens ein bis wenige **Vorschlagschips** („Was kannst du tun?“).
- **Eingabezeile mittig**, nicht am unteren Rand; nach der ersten Nachricht wandert sie nach
  unten und der Verlauf füllt die Fläche [V].
- **Fußzeile**: ein Satz Rechtshinweis („ChatGPT ist eine KI …“) in kleiner Schrift. [L]
- Keine Kacheln, kein Dashboard: die Startseite *ist* der leere Chat.

**Kernidee für uns:** Startseite = neuer Chat. Begrüßung + Eingabe + wenige Chips, alles
andere in die Seitenleiste.

## 5. Eingabezeile (Composer)

Beobachtet [L]: Pille mit voller Rundung, Platzhalter „ChatGPT fragen“, links „+“, rechts
Mikrofon und ein runder Primärknopf.

| Element | Verhalten |
|---|---|
| „+“ links | öffnet das Werkzeug-/Anhangsmenü (Kap. 6) |
| Platzhalter | kurz, imperativ; wächst mehrzeilig mit dem Text [V] |
| Mikrofon | **Diktieren**: Sprache → Text *in* die Eingabezeile; Zustände „Diktat wird transkribiert“, „Diktat beenden“, „Diktat absenden“ [C] |
| Primärknopf rechts | **leer = Audiomodus starten** (Wellenform-Symbol), **mit Text = Senden** (Pfeil) [L]; während der Antwort = Stopp [V] |
| Gewählte Werkzeuge | erscheinen als entfernbare Chips in der Zeile (z. B. Websuche, Deep Research) [V] |
| Anhänge | Vorschau-Kacheln über/in der Zeile; Ziehen-und-Ablegen auf die ganze Fläche [V/C] |
| Kürzel | Enter senden, Umschalt+Enter Zeilenumbruch [V]; Liste „Tastaturkürzel anzeigen“ [C] |

**Wichtigstes Muster:** *ein* Knopf mit zwei Bedeutungen – Diktat (Text) und Audiomodus
(Gespräch) sind getrennte Einstiege, der Gesprächsmodus liegt auf dem Primärknopf, solange
nichts getippt ist.

## 6. „+“-Menü

Abgemeldet beobachtet [L]: „Fotos hinzufügen“, „Websuche“; ausgegraut mit Hinweis „Anmelden,
um Folgendes zu nutzen …“: „Bild erstellen“, „Deep Research“, „Dateien hinzufügen“.

Angemeldet laut Katalog [C] (Reihenfolge nicht belegt): Fotos und Dateien hinzufügen, Foto
aufnehmen (Desktop: Kamera/Screenshot über die Brücke), Bild erstellen, Deep Research,
Agentenmodus, Websuche, Canvas, Nachdenken, Apps/Konnektoren (z. B. Google Drive, OneDrive),
„Mehr“ als Untermenü für Seltenes [V].

Muster:

- Menü **gemischt**: Anhänge oben, „Modi“ (Werkzeuge, die den nächsten Auftrag prägen)
  darunter. Ein gewählter Modus wird zum Chip in der Eingabezeile.
- Nicht verfügbare Funktionen bleiben **sichtbar, aber gesperrt**, mit Grund
  (Anmeldung, Admin-Sperre, Plan). Das zeigt den Funktionsumfang, ohne zu verstecken.

## 7. Voice

Zwei klar getrennte Modi [C]:

1. **Diktieren** (Mikrofon in der Zeile): reine Spracheingabe, Ergebnis editierbar, dann
   senden.
2. **Audiomodus / „ChatGPT Voice“** (Primärknopf): Gespräch in Echtzeit. Im Katalog:
   „Audiomodus starten/beenden“, „Im Audiomodus weiterchatten“, Mikrofon ein/aus,
   Stimmenauswahl („Mehrere Stimmen“), **Bildschirm- und Videofreigabe** im Audiomodus,
   Hinweis zu Fehlern im Audiomodus.
   - **Integriert** im Chat (Transkript und Bilder laufen im Verlauf mit) **oder**
     **separater Vollbildmodus** ohne Live-Transkript – als Einstellung wählbar
     („IntegratedVoiceMode … separateModeLabel“).
   - Mikrofonstatus wird ehrlich gemeldet („Mikrofon wurde in den Systemeinstellungen oder
     per Hardware-Schalter stumm geschaltet“).
3. Audiomodus ist auch in Projekten verfügbar. [C]

## 8. Modellwahl

- Menü oben links in der Kopfzeile („ChatGPT ▾“), ARIA-Text „Modellauswahl, aktuelles
  Modell: {modelLabel}“. [C/L]
- Angebot gestuft nach **Absicht statt Modellname**: „Auto“, schnelle Antworten („Instant“),
  „Thinking“ (länger nachdenken), „Pro“; dazu **Denkaufwand** als eigene Stufe („Kurzes /
  Standardmäßiges / Längeres / Intensives Nachdenken“). [C]
- Ältere Modelle hinter „Weitere Modelle anzeigen“ bzw. Einstellung „Weitere Modelle
  aktivieren“. [C]
- Wenn ein Modell eine Funktion nicht kann, **erklärter Rückfall**: „Das Modell {modelName}
  unterstützt keine Projektdateien oder Hinweise. Dein Gespräch wird mit … fortgesetzt.“ [C]
- Hinweis „Die Modellauswahl wurde verschoben“ deutet auf eine kürzliche Umplatzierung. [C]

## 9. Dateien und Kontext

Kontextquellen, die ChatGPT pro Antwort heranziehen kann [C]:

| Quelle | Steuerung |
|---|---|
| Anhänge (Fotos, Dateien, Screenshot, Kamera) | „+“-Menü, Ziehen-und-Ablegen |
| Projektdateien und Projekt-Hinweise | im Projekt (Kap. 10) |
| Gespeicherte Erinnerungen | Einstellungen › Personalisierung; verwalten, löschen, Versionen zurücksetzen; voll-Meldung |
| Früherer Chatverlauf | eigener Schalter („frühere Chats als Referenz“) |
| Websuche | Werkzeug oder automatisch |
| Apps/Konnektoren | verbinden, dann „Fragen zu deinen Dateien stellen“ |
| Andere Fenster (Desktop) | „Mit Apps arbeiten“ (3.3) |
| Bibliothek | gesammelte Dateien/Bilder über Chats hinweg |

Datenschutz-Schalter sichtbar am Chat: **Temporärer Chat** („erscheint nicht in deinem
Verlauf“, nutzt keine Erinnerung). Gruppenchats nutzen nie die persönliche Erinnerung. [C]

## 10. Projekte

Laut Katalog [C]:

- Anlegen: „Neues Projekt“ → Name; Projekte in der Seitenleiste; „In Projekt verschieben“
  für bestehende Chats; „Neuer Chat in diesem Projekt“.
- **Projektdateien** („Füge Dokumente, Code-Dateien, Bilder und mehr hinzu …“), Ablegen per
  Ziehen, Text als Datei einfügen; Obergrenze für Dateizahl mit Warnung bei vielen Dateien.
- **Projekt-Hinweise** („Lege fest, wie ChatGPT in diesem Projekt antworten soll“).
- **Erinnerung je Projekt**: entweder projekt-eigen und abgeschottet oder mit globaler
  Erinnerung geteilt – bei Anlage festgelegt, später nicht änderbar.
- Teilen und Zusammenarbeit: einladen, Rollen „nur chatten“ / „Inhalte bearbeiten“,
  Projekt verlassen mit Option, eigene Chats als Kopie zu behalten.
- Funktionen in Projekten: Deep Research, Audiomodus; Aufgaben (Tasks) haben *keinen*
  Zugriff auf Projektdateien (wird angezeigt).
- Löschen mit klarer Folge („alle Projektdateien und Chats dauerhaft gelöscht“) und Tipp,
  wichtige Chats vorher herauszuverschieben.

## 11. Onboarding

Laut Katalog [C], abgemeldet zusätzlich beobachtet [L]:

- „Willkommen bei ChatGPT“, kurze Fragen: Name („Wie soll ChatGPT dich nennen?“ [V]),
  „Was machst du beruflich?“, „Wofür möchtest du ChatGPT nutzen?“ → speist Personalisierung
  und Startvorschläge.
- Feature-Einführungen als **kleine Karten/Banner** beim ersten Kontakt („Der Sprachmodus ist
  da“, „Jetzt kannst du noch mehr in Projekten machen“), nicht als Assistent über mehrere
  Seiten.
- Abgemeldet: Seitenleisten-Karte „Erhalte Antworten, die auf dich zugeschnitten sind“ mit
  einem Anmelden-Knopf; Upsell-Popover „Erweiterte Funktionen kostenlos testen“. [L]
- Cookie-Banner mit gleichwertigem „Nicht unbedingt erforderliche ablehnen“. [L]

## 12. Übertrag auf Local Voice AI (Vorschläge)

Für den Thread „Chat als Startseite wie ChatGPT gestalten“. Nach Nutzen sortiert:

1. **Startseite = leerer Chat.** Begrüßung („Wo sollen wir anfangen, {Name}?“-Muster),
   mittige Eingabezeile, 1–4 Vorschlagschips aus eigenen Funktionen (Besprechung
   zusammenfassen, Diktat, Recherche). Nach erster Nachricht Eingabe nach unten.
2. **Zwei Sprachwege in der Eingabezeile**, wie oben: Mikrofon = Diktat in die Zeile
   (unsere Stärke, lokal), Primärknopf leer = Gesprächsmodus, mit Text = Senden. Das bildet
   Local Voice AI besser ab als ChatGPT selbst, weil beides lokal läuft.
3. **„+“-Menü mit zwei Gruppen**: Kontext (Datei, Foto, Screenshot, Projekt-/Besprechungs-
   material) und Modi (Recherche, Websuche falls erlaubt, Vorlesen/Übersetzen). Gewählter
   Modus als Chip; gesperrte Modi sichtbar mit Grund (z. B. „Modell nicht geladen“).
4. **Modellwahl nach Absicht** („Schnell“, „Gründlich“) statt Dateinamen lokaler Modelle;
   echter Modellname im Untermenü. Erklärter Rückfall, wenn ein lokales Modell eine
   Funktion nicht kann.
5. **Projekte** mit Dateien, Hinweisen und wählbarer Erinnerungs-Abschottung passen zu
   unseren vorhandenen Projekten/Besprechungen; „In Projekt verschieben“ aus dem Chat.
6. **Temporärer Chat** als sichtbarer Schalter – passt zum Datenschutzversprechen.
7. **Companion-Fenster per globalem Kürzel** als späterer Schritt (Diktat/Frage von überall).
8. Onboarding schlank: drei Fragen + Karten beim ersten Kontakt statt Assistent.

Bewusst **nicht** übernehmen: Upsell-Karten, Cloud-Konnektoren als Standard,
Telemetrie-Muster.

## 13. Offene Punkte und Grenzen

| Frage | Stand |
|---|---|
| Angemeldete Oberfläche live (Reihenfolge „+“-Menü, Modellmenü, Projekte-Seite) | **teilweise** – nur Texte [C]; Live-Blick bräuchte Freigabe der App-Steuerung oder Anmeldung |
| Audiomodus-Ablauf (Animation, Unterbrechen, Latenz) | **offen** – nicht ausgeführt |
| Pairing: welche Apps genau, welche Inhalte | **teilweise** – nur Bundle-Hinweise [S] |
| Gültigkeit | Momentaufnahme; Web-Bundle und Gates ändern sich serverseitig |

REA-Abdeckung: 72 relevante Dateien, 70 JS-Dateien geparst, 0 Parsefehler; semantischer Graph
*partial* (Knotenobergrenze 100 000), dynamische IPC-Kanäle ungeklärt. Statische Funde
belegen keine Ausführung.
