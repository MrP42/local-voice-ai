# Inhalte-Modul: Vorlesen wird Arbeitsfläche für Inhalte (Epic, Stand 12.11.2026, Fassung 1)

Epic #530. Auftrag Patrick, 12.11.2026 (diktiert): Das Modul „Vorlesen“ soll mehr als vorlesen und KI-Dokumente erzeugen. Es
soll Inhalte verwalten, öffnen, bearbeiten und erstellen, angebunden an Projekte, Ablage (RAG) und Aufnahmen, mit
Baumansicht links, formatierter Darstellung, bereinigtem Import (Text, E-Mail, Dokument, URL), KI-Auswertung
(„das für mich Relevante, ohne alles zu lesen“) und Routinen (z. B. täglich 6 Uhr neue Intranetbeiträge prüfen,
relevante als Seite anlegen und sofort zusammenfassen). Vorschau: `apps/local-voice/docs/vorschau/inhalte-i0/`.

## 1 Bestand (wird wiederverwendet, nicht neu gebaut)

| Baustein | Ort | Nutzen für das Inhalte-Modul |
|---|---|---|
| Seiten des Vorlesens (`projects/<seite>/state.json`, Dateien, KI-Dokumente) | `commands/pages.rs`, `settings/tts/*` | bleiben die Dokumente; bekommen eine formatierte Fassung (§3) |
| Ordnerleiste, Buch-Ordner | `ReaderFolders.tsx`, `useReaderFolders.ts` | wird zum Baum (§2) |
| Bereinigungsregeln (Zeitstempel, Fragmente, Leerzeichen, Absätze) | `lib/reader/textCleanup.ts` | Grundlage der Bereinigung beim Einfügen (§4.1) |
| Kopf-/Fußzeilen, Seitenzahlen, Silbentrennung bei PDF | `ebook.rs`, `ablage/textlauf.rs` (lva#476) | gleiche Regeln für eingefügten Text und importierte PDFs |
| Extraktoren HTML (`scraper`), E-Mail (`mail-parser`), DOCX/PDF | `ablage/extrakt/*` | Import per Drag-and-drop (§4.2); bekommen eine strukturierte Ausgabe |
| Projekte mit Einträgen und Gruppen (Texte, Bücher, Aufnahmen, Dateien, Recherchen) | `projekte/*`, `lib/projekteKontext.ts` | Baum gruppiert danach (§2), Einträge bleiben Verweise |
| Ablage mit Index (wai-retrieval) und KI-Freigabe je Ordner | `ablage/*` | Ablage-Dokumente erscheinen im Baum; Suche über den Index |
| Workflow-Engine: Zeitplan, Ordner, YouTube-RSS, `page.create`, `text.summarize`, `agent.route`, `agent.extract`, `notify.local`, Trockenlauf | `managers/workflows/*` | Routinen (§6) brauchen nur einen Feed-Auslöser und `content.import_url` |
| Lokales Modell (llama-server) und Anbieter | `managers/llm/*`, Thread „App ohne Claude-Installation lauffähig machen“ | alle KI-Schritte laufen auch ohne Anbieter-CLI (§5.3) |

## 2 Oberfläche und Navigation

- **Name:** Der Menüpunkt heißt „Inhalte“. Vorlesen bleibt Funktion jeder Seite (Wiedergabeleiste unten), nicht
  Modulname. Begründung: Patrick beschreibt ein Inhalte-Modul; „Vorlesen“ trägt Import, KI und Routinen nicht.
- **Spalte links: Baum** (Fluent-2-Tree, Tastatur: Pfeile, Pos1/Ende, `*` alle öffnen, Typeahead):
  1. *Neu für dich* (Eingang aus Routinen und Importen, ungelesen fett, Zähler),
  2. *Projekte* → je Projekt die vorhandenen Gruppen aus `gruppiere()` (Seiten, Bücher, Aufnahmen, Dateien aus der
     Ablage, Recherchen, KI-Dokumente),
  3. *Ohne Projekt* (heutige Seiten- und Buch-Ordner),
  4. *Routinen* (Liste der Abläufe mit Auslöser `content.*`, Sprung in den Editor der Automationen).
  Der Baum zeigt Verweise; das Original bleibt im besitzenden Modul (Aufnahmen, Ablage, Bibliothek). Öffnen eines
  fremden Eintrags zeigt ihn lesend im Inhalte-Modul mit Knopf „In Aufnahmen öffnen“ usw.
- **Mitte: Dokument** mit Reitern oben in der Reader-Leiste (bleibt wie lva#440): *Kurzfassung* (neu, §5),
  *Original* (formatiert, §3), *Übersetzung*, *KI-Dokumente*. Schalter Lesen/Bearbeiten wie heute.
- **Rechts (ab 1280 px, sonst Blatt):** Quelle und Herkunft (URL, Abrufzeit, Absender, Datei, Routine), Projekt,
  Datenklasse, Notizen und Anhänge.
- **Listenansicht** als Alternative zum Baum für einen Knoten (Name, Art, Quelle, Geändert, Relevanz), sortier- und
  filterbar; Umschalter Baum/Liste merkt sich je Gerät.
- Responsiv nach Projektregel: 390 px Baum als eigene Ebene (zurück-Pfeil), 768 px Baum einklappbar, 1440/2560 px
  drei Spalten mit fluider Mitte und Lesespalte ≤ 70ch. Keine doppelten Scrollleisten.

## 3 Formatierte Fassung

- Neues Feld der Seite: `inhalt.md` (Markdown-Untermenge: Überschriften 1–4, Absatz, fett/kursiv, Listen, Zitat,
  Tabelle GFM, Link, Bild als Verweis auf Seiten-Datei, Code). Kein rohes HTML, keine Skripte; Links öffnen nur
  `http(s)`/`mailto` extern mit Rückfrage.
- `state.json` behält den Sprechtext. Ist `inhalt.md` vorhanden, wird der Sprechtext daraus abgeleitet
  (Markdown → Absätze, Überschriften als eigene Sätze, Tabellen zeilenweise „Spalte: Wert“) und die Satzmarken
  verweisen auf Blöcke des Markdown, damit die Hervorhebung beim Vorlesen in der formatierten Ansicht läuft.
- Alte Seiten ohne `inhalt.md` bleiben unverändert (reiner Text = Markdown ohne Auszeichnung). Keine Migration.
- Bearbeiten: Ansicht *Bearbeiten* bleibt der Chip-Editor für den Sprechtext; für die formatierte Fassung kommt ein
  einfacher Blockeditor (Überschrift, Liste, fett, Link), der `inhalt.md` schreibt. Konflikt Sprechtext ↔ Inhalt:
  Wer zuletzt bearbeitet, gewinnt; der andere wird neu abgeleitet, Rückgängig hält beide.

## 4 Import

### 4.1 Einfügen (Strg+V, Kontextmenü, „Neue Seite aus Zwischenablage“)

- Liegt `text/html` in der Zwischenablage: Bereinigen nach Erlaubnisliste (§3) → Markdown. Sonst `text/plain`.
- **Automatische Bereinigung** (ohne Rückfrage, mit Meldung „7 Bereinigungen – Rückgängig“ und Liste):
  Seitenzahlen allein auf einer Zeile („12“, „- 12 -“, „Seite 12 von 30“), wiederholte Kopf-/Fußzeilen (gleiche Zeile
  ≥ 3× im Abstand einer Seite), Silbentrennung am Zeilenende („Verwal-\ntung“ → „Verwaltung“, nicht bei
  „Bund- und Länder“), harte Zeilenumbrüche mitten im Satz, Zeitstempel, doppelte Leerzeichen, Steuerzeichen,
  Cookie-/Navigations-Reste aus Webseiten („Zum Inhalt springen“, „Drucken“, „Teilen“).
- Regeln sind deterministisch und getestet (≥ 20 Vorlagen aus Intranet, Newsletter, PDF-Kopie, Outlook, Word).
- Einstellungen: Schalter „Beim Einfügen automatisch bereinigen“ (Standard an), Regeln einzeln abwählbar.

### 4.2 Drag-and-drop und Datei

- E-Mail (`.eml`; `.msg` über Outlook-Export), HTML, DOCX, PDF, TXT/MD: Extraktor der Ablage liefert neu eine
  strukturierte Ausgabe (Markdown) neben dem Markenformat. Original wird als Seiten-Datei aufbewahrt (`quelle/`).
- E-Mail: Kopf (Von, Datum, Betreff) als Herkunft, Text-Teil oder HTML-Teil bereinigt, Anhänge als Seiten-Dateien.
- Mehrere Dateien → mehrere Seiten im gewählten Projekt/Ordner.

### 4.3 URL

- Feld „Von Adresse importieren“ und Ablegen eines Links. Abruf im Rust-Kern: nur `http(s)`, 10 s, ≤ 5 MiB,
  ≤ 3 Umleitungen, jede Umleitung erneut geprüft, keine Cookies außer gerätelokal hinterlegter Anmeldung (§6.3),
  private Netze nur nach Freigabe je Host (Intranet), sonst abgelehnt (SSRF-Schutz).
- Hauptinhalt bestimmen (Lesbarkeits-Heuristik: `article`/`main`, Textdichte, Navigation/Fuß/Kommentare weg),
  Überschriften, Listen, Tabellen und Links bleiben erhalten, Bilder optional als Seiten-Dateien.
- Herkunft `quelle.json`: URL, kanonische URL, Abrufzeit, HTTP-Datum, SHA-256 des Rohtexts, Titel, Autor/Datum
  falls vorhanden. Roh-HTML gesichert in `quelle/original.html` (nicht ausgeführt, nur für „Original ansehen“).

## 5 KI: „Das Wichtigste für mich“

### 5.1 Kurzfassung als feste Struktur

Ausgabe als JSON nach Schema, gerendert als Karte (Reiter *Kurzfassung*), Belege je Aussage auf Blöcke des Originals:

```json
{"version": 1, "relevanz": {"stufe": "hoch|mittel|gering", "warum": "…", "themen": ["KI"]},
 "in_einem_satz": "…",
 "kernaussagen": [{"text": "…", "beleg": ["b12"]}],
 "fuer_mich": [{"text": "…", "art": "aufgabe|frist|termin|entscheidung|info", "datum": "2026-12-01", "beleg": ["b7"]}],
 "zahlen": [{"wert": "…", "bedeutung": "…", "beleg": ["b3"]}],
 "betroffen": ["…"], "quelle": {"titel": "…", "url": "…", "datum": "…"}}
```

Aussagen ohne Beleg werden nicht angezeigt (Belegregel wie wai-assistent). Klick auf einen Beleg springt in die
formatierte Fassung und hebt den Block hervor.

### 5.2 Relevanzprofil

Einstellungen → Inhalte → „Was ist für mich relevant“: Rolle (frei), Themen mit Gewicht (z. B. KI hoch,
Digitalisierung mittel), Ausschlüsse (z. B. Kantine, Parkplätze), Organisationseinheiten. Wirkt auf Kurzfassung,
Routinen-Filter (§6) und Sortierung „Neu für dich“. Gerätelokal und mit Profil-Sync wie andere Einstellungen.

### 5.3 Modelle und Daten

Standard lokales Modell; Cloud nur, wenn die Datenklasse es erlaubt (Intranet = INTERNAL: lokal oder freigegebener
EU-Anbieter). Ohne Anbieter-CLI lauffähig. Lange Texte: abschnittsweise verdichten, dann zusammenführen
(vorhandenes Kurzbuch-Verfahren `book_ai/kurzbuch.rs`). Weitere Erzeugnisse über die vorhandenen KI-Dokumente
(Protokoll, E-Mail-Entwurf, Vermerk, Präsentation) mit Quelle als Kontext.

## 6 Routinen

### 6.1 Auslöser

- `feed.new_item` (neu, nach Muster `youtube_channel`): RSS/Atom einer Adresse, Ledger je Eintrag, erster Abruf
  markiert nur, `backfill` 0–20, Abruf nur bei eingeschaltetem Ablauf.
- `web.new_links` (neu): HTML-Listenseite (z. B. Intranet-Startseite „Neuigkeiten“) mit CSS-Auswahl für
  Beitragslinks, sonst gleiche Regeln wie `feed.new_item`.
- Alternativ ohne neuen Abruf: vorhandene E-Mail-Auslöser (Newsletter-Mail) bzw. M365 SharePoint-News über Graph,
  je nach Antwort von Patrick (§8).
- Zeitplan „täglich 6:00“ = vorhandener Zeitplan-Auslöser; Feed-Auslöser bekommen zusätzlich „nur zu Zeitpunkt X
  prüfen“ statt Intervall.

### 6.2 Aktionen

- `content.import_url` (neu): wie §4.3, legt Seite im Ziel-Projekt/-Ordner an, markiert „Neu für dich“.
- `content.relevance` (neu, dünne Hülle um `agent.route` + Relevanzprofil): stuft ein, Bedingung im Ablauf
  „nur bei hoch/mittel“ und optional Stichwortfilter (muss enthalten / darf nicht enthalten).
- `content.brief` (neu): erzeugt die Kurzfassung §5.1 und hängt sie an die Seite.
- Vorhanden: `notify.local`, `mail.draft`, `task.create_from`.
- Vorlage „Intranet: neue Beiträge zu einem Thema“ und 10–20 weitere Vorlagen (Newsletter, Pressemitteilungen,
  Amtsblatt, Ausschreibungen, Fachportale, Gesetzesänderungen, Förderaufrufe …), alle im Trockenlauf angelegt.

### 6.3 Anmeldung an Intranet-Seiten

Offen bis Patricks Antwort. Vorschlag in dieser Reihenfolge: (1) Feed/Seite ohne Anmeldung im Behördennetz;
(2) Windows-integrierte Anmeldung (Kerberos/NTLM über das System, kein gespeichertes Passwort); (3) M365 über die
vorhandene Integration; (4) Cookie aus einer einmaligen Anmeldung im eingebetteten Fenster, gerätelokal und
verschlüsselt. Keine Passwörter im Klartext, keine Inhalte im Log.

## 7 Ausbaustufen (je eigener PR)

| Stufe | Inhalt | hängt ab von |
|---|---|---|
| I0 | dieses Konzept + HTML-Vorschau, Codex-Abnahme | – |
| I1 | Einfügen mit automatischer Bereinigung + Rückgängig + Einstellung (§4.1, nur Text) | – |
| I2 | formatierte Fassung `inhalt.md`, Darstellung, Hervorhebung beim Vorlesen, HTML-Einfügen (§3, §4.1) | I0 |
| I3 | Import Datei/E-Mail/URL mit Herkunft (§4.2, §4.3) | I2 |
| I4 | Baum, Liste, „Neu für dich“, Umbenennung „Inhalte“ (§2) | I0-Vorschau |
| I5 | Kurzfassung „Das Wichtigste für mich“ + Relevanzprofil (§5) | I2 |
| I6 | Routinen: `feed.new_item`, `web.new_links`, `content.*`, Vorlagen (§6) | I3, I5, Patricks Antwort |
| I7 | iOS nachziehen (Lesen, Kurzfassung, „Neu für dich“) | I4, I5 |

## 8 Offene Fragen an Patrick

Intranet-System, Feed oder Newsletter-Mail, Netz/VPN und Anmeldeart, Gerät und Modell (im Thread gestellt am
12.11.2026). Antworten entscheiden nur I6; I1–I5 sind unabhängig.

## 9 Entscheidungen zur Abnahme (Codex)

- **E1** Menüpunkt „Inhalte“ ersetzt „Vorlesen“ (Empfehlung) – oder Vorlesen bleibt und bekommt nur den Baum.
- **E2** Baum zeigt Verweise auf Inhalte anderer Module, Original bleibt dort (Empfehlung) – oder Kopie.
- **E3** Formatierte Fassung als Markdown-Untermenge `inhalt.md` neben dem Sprechtext (Empfehlung) – oder HTML.
- **E4** Bereinigung beim Einfügen automatisch mit Rückgängig (Empfehlung) – oder Vorschau mit Bestätigung.
- **E5** Kurzfassung als festes JSON-Schema mit Belegen (Empfehlung) – oder freier Text.
- **E6** Routinen über die vorhandene Workflow-Engine mit neuen Auslösern/Aktionen (Empfehlung) – oder eigener
  Planer im Inhalte-Modul.

## 10 Tests (je Stufe)

Rust-Einheitstests je Regel/Abruf/Auslöser (Ledger, SSRF, Größen, Umleitungen, Zeitlimit), Vitest für
Bereinigung und Markdown-Ableitung (≥ 20 Vorlagen), Playwright für Baum, Einfügen, Kurzfassung bei 390/768/1440/2560
hell/dunkel, Tastaturbedienung und Bildschirmleser-Namen. Keine echten Intranetinhalte in Fixtures, nur
synthetische (`example.org`).
