# Dokumentenablage mit Sync und lokalem Abrufindex (Epic „Ablage“)

Stand 08.11.2026. Entscheidungen: wai-sdk ADR-0029. Baut auf Bibliothek-Sync (`2026-11-02-bibliothek-sync.md`),
Projekten (`2026-11-07-buch-ordner-und-projekte.md`, P4-Plan), globalem Sprachassistenten (lva#325) und dem
Assistenten (`2026-11-08-assistent-chat-und-sprache.md`, ADR-0028) auf.

## 1 Ziel

Eine Seite **„Ablage“** in Local Voice (Desktop und iPhone) für allgemeine Dokumente, bedienbar wie OneDrive:
Ordner, Hochladen per Drag & Drop, Vorschau, Fassungen, Papierkorb, Suche. Alle Geräte sehen dieselbe Ablage über
den eigenen Hub (Ende-zu-Ende verschlüsselt). Die KI greift gezielt auf **Auszüge** zu, nur in freigegebenen Ordnern
und nur über den Kontextvertrag (ADR-0028 E6).

Abgrenzung: Bücher bleiben in der Bibliothek, Aufnahmen in Aufnahmen, Notizbuch bleibt eigenes Modul. Projekte
verweisen auf Ablage-Einträge, sie kopieren sie nicht.

## 2 Datenordner (Desktop)

```
<Datenordner>/ablage/
  ablage.json            Journal + Baum (Version, Epoche)
  dateien/<datei-id>/<fassung>.<ext>    Originale, unveränderlich
  text/<datei-id>/<fassung>.txt         Textfassung (UTF-8, Marken siehe §4)
  papierkorb/            30 Tage, Wiederherstellen
  index.db               abgeleitet (wai-retrieval), löschbar, nie exportiert
  verknuepft.json        gerätelokal: verknüpfte Ordner (§8)
```

IDs: `ord_<ULID>`, `dat_<ULID>`. Namen nach P4 §4.4 (Windows-reservierte Namen, Groß/Klein, `..`, Verknüpfungen).
Ablauf jeder Ablage wie P4a `dateien.rs`: `.teil-<id>` (create_new) → Größe + SHA-256 → ohne Ersetzen umbenennen →
Journal `abgelegt` → Baum. Wiederanlauf, „Bestand neu aufbauen“, Lesefehler ≠ leer.

### 2.1 Datensatz `ablage_datei`

```json
{
  "id": "dat_…", "ordner": "ord_…", "name": "Angebot 2026.pdf", "typ": "pdf",
  "fassungen": [
    {"n": 1, "blob": "<64 hex>", "size": 812345, "sha256": "<hex>", "geraet": "<id>", "zeit": 1762600000000,
     "text": {"blob": "<64 hex>", "size": 40211, "stand": "ok|kein_text|fehler", "extraktor": "pdf-extract@0.12"}}
  ],
  "aktuell": 1, "konflikt": [], "geloescht": null, "updated": 1762600000000
}
```

### 2.2 Datensatz `ablage_ordner`

```json
{"id": "ord_…", "eltern": "ord_…|null", "name": "Verträge", "datenklasse": "internal|public|confidential|personal|erbt",
 "nur_geraet": false, "ki": "erbt|an|aus", "zielordner_ki": false, "geloescht": null, "updated": 1762600000000}
```

`ki` ist **gerätelokal** in `ablage/freigabe.json` (wie `kontext.json` in P4a), nicht im Sync, bis P6 eine
geräteübergreifende Widerrufsgarantie liefert. Fehlender Stand = aus. Ein Unterordner mit `aus` gewinnt.

## 3 Oberfläche (D1, HTML-Vorschau an Codex vor dem Bau)

- Leiste: neuer Eintrag „Ablage“ unter Bibliothek (Symbol Ordner-Wolke).
- Kopf über `Seitenkopf`: Pfad als Brotkrumen, Suche, „Hochladen“, „Neuer Ordner“, Ansicht Liste/Kacheln.
- Liste: Name, Geändert, Größe, Sync-Zustand (Wolke/Häkchen/Pfeil/Konflikt), KI-Freigabe (Schild), Datenklasse.
- Rechte Spalte (≥ 1024 px) bzw. Blatt (≤ 768 px): Vorschau (PDF über vorhandenen Viewer, Text/MD, Bild),
  Fassungen, „Zu Projekt hinzufügen“, „Für KI freigeben“, Details.
- Drag & Drop aus dem Explorer in Ordner und auf die Liste; Ordner hochladen erhält die Struktur.
- Leerer Zustand: ein Satz und „Dateien hierher ziehen“. Keine Vorschlagsblasen.
- Kontingent im Fuß: „4,2 GB von 30 GB“.
- 390/768/1440/2560 px, hell/dunkel, Ziele ≥ 44 px, Tastatur (Entf = Papierkorb mit Rückgängig, F2 Umbenennen).

## 4 Textfassung und Index (D2)

- Extraktoren: PDF `pdf-extract` (vorhanden), DOCX/PPTX/ODT/ODP per `zip` + `quick-xml`, XLSX/ODS `calamine`
  (MIT/Apache, neu), TXT/MD/CSV/HTML (Tags entfernt), EML (Kopf + Text, Anhänge als eigene Dateien angeboten).
- Marken in der Textfassung: Zeilen `\f<seite>` für Seitenwechsel, Absätze mit `\n\n`; Offsets UTF-16 wie im Reader.
- Extraktion im Hintergrund (Warteschlange, ein Kern, abbrechbar), Fehler sichtbar am Eintrag, nie Inhalt im Log.
- `index.db` über vendortes `wai-retrieval`: ein Eintrag je (Datei, aktuelle Fassung, Stück ≈ 220 Zeichen mit
  Überlappung), Felder für Ordnerpfad (als IDs), Datenklasse, Freigabe-Epoche. Filter in SQL vor dem Ranking.
- Suche in der Ablage (ohne KI): Treffer mit Auszug und Sprung an die Stelle.
- Neuaufbau, wenn Extraktor- oder Embedder-Kennung wechselt; Fortschritt sichtbar.

## 5 KI-Zugriff (D3, nach #365 P4c)

- **Projekt:** „Zu Projekt hinzufügen“ legt Verweis `wai://lva/ablage/<dat|ord>` an. `projekt_kontext` kennt die Form
  `auszuege` (Vorgabe für Ablage-Mitglieder): je Frage Top-k (Standard 8) Auszüge aus den eingeschalteten Mitgliedern,
  mit `[Q<k>]`, Titelmaske und Deep-Link. `volltext` nur ausdrücklich und nur im Token-Budget (sichtbarer Stopp).
- **Global:** Bereich „Ablage“ in der Freigabe des Assistenten (lva#325), Standard aus, Ordnerbaum mit Schaltern.
- `stand` = Hash über Freigabe-Epoche/Generation, Fassungen, Datenklassen und die gelieferten Auszüge; vor Versand neu
  geprüft, verspätete Ergebnisse mit altem Stand verworfen.
- Datenklasse gegen `datenklasse_max` des Modells; `personal`/`confidential` nur lokal, außer ausdrückliche Freigabe.
- Auszüge sind unvertrauenswürdig (`als_text` grenzt ab, Herkunftsmarke `dokument:<dat>@<n>`).
- KI-Entwurf: Werkzeug `ablage_entwurf_anlegen(ordner, name, inhalt_md)` nur in Ordnern mit `zielordner_ki` oder
  „KI-Entwürfe“; kein Werkzeug zum Überschreiben, Verschieben oder Löschen.
- Beispiele, die gehen müssen: „Was steht im Angebot von Firma X zur Laufzeit?“ (Beleg mit Seite), „Fasse den Ordner
  Verträge zusammen“ (Auszüge, Budgetstopp sichtbar), „Lege eine Zusammenfassung als Notiz ab“ (Entwurf).

## 6 Sync (D4 Desktop, D5 iPhone)

- Hub: Sammlungen `ablage_ordner`, `ablage_datei` (wai-portal-PR analog #19). Blobs wie Bibliothek (3-MiB-Stücke,
  Blob-Kennung über Kontoschlüssel, AAD mit Index). Reihenfolge: Original-Blob, Text-Blob, dann Objekt.
- Ordner mit `nur_geraet` und verknüpfte Ordner werden nie übertragen.
- Empfang: Ordner feldweise nach `updated`; Dateien: Fassungen vereinigt (nach `blob`), `aktuell` vom neueren
  `updated`; zwei neue Fassungen mit gleichem Vorgänger → `konflikt` (beide sichtbar, Nutzer wählt).
- Grabstein nur durch „Endgültig löschen“ in der App. Danach Blob-Löschung am Hub, wenn kein anderer Verweis.
- iPhone: Textfassung sofort, Original bei Bedarf (Einstellung „nur im WLAN“), lexikalischer Index in VoiceCore
  über die Textfassungen; Assistent-Bereich „Ablage“ wie Desktop.
- Kontingent: Anzeige aus `GET /api/v1/voice/blobs`; Hochladen stoppt sichtbar bei Überschreitung.

## 7 Semantische Suche (D6)

`multilingual-e5-small` (MIT) als ONNX über das vorhandene `ort`, Download bei Bedarf mit Hash-Prüfung, Hybrid
per RRF in `wai-retrieval`. Eval-Tor auf synthetischem Korpus (`tools/korpus` bzw. eigener Generator, nur
`example.*`-Daten): Recall@5 ≥ 0,9; sonst bleibt lexikalisch Standard.

## 8 Verknüpfte Ordner (D7)

Gerätelokal, nur lesen, nie hochladen. Auswahl über Ordnerdialog, Ausschlüsse (`~$*`, `.tmp`, Systemordner),
Verknüpfungen außerhalb werden nicht verfolgt, Änderungen per Dateibeobachter mit Entprellung. Datenklasse
Standard `confidential`. Dazu lokale OCR (Windows.Media.Ocr / Apple Vision) für Bild-PDFs.

## 9 Tests (je Stufe)

- D1: Abbruch nach Zwischendatei/Hash → kein Eintrag; `CON.txt`, `..\x`, Groß/Klein-Kollision; Papierkorb +
  Wiederherstellen; Journal defekt → Lage „unklar“ und „Bestand neu aufbauen“.
- D2: Extraktor-Fixtures (PDF, DOCX, PPTX, XLSX, MD, EML); Suchtreffer mit korrekter Seite; Neuaufbau idempotent.
- D3: Kontrolltext in nicht freigegebenem Ordner → 0 Treffer; Freigabe aus mitten im Gespräch → nächster Stand ohne;
  Injektion im Dokument → 0 ausgeführte Aufrufe; `personal` an Cloud-Modell → abgewiesen.
- D4: Zwei-Geräte-Konflikt → zwei Fassungen; Grabstein → Index leer auf beiden Geräten; Blob am Hub gelöscht;
  `nur_geraet` erscheint nie im Hub (Mitschnitt).
- D5: Swift-Testvektor gegen Rust für Blob-Kennung/AAD; Index-Treffer gleich Desktop für Musterfragen.

## 10 Datenschutz

E2E (Hub ohne Schlüssel), keine Namen/Inhalte in Log/Audit, Datenklasse je Ordner, lokale Modelle als Standard für
`personal`/`confidential`, Papierkorb- und Fassungsfristen, endgültiges Löschen mit Nachweis (IDs), Hinweis beim
Einrichten: dienstliche personenbezogene Daten erst nach DSFA. Grenze: lokaler Index liegt im App-Datenordner,
Schutz über Geräteverschlüsselung.

## 11 Abstimmung

- Assistent (ADR-0028, lva#384): Ablage ist eine Quelle hinter `projekt_kontext`; Kontextleiste ＋ → „Aus Ablage“.
- Projekte (lva#365): Verweisart `ablage`, Form `auszuege`; P4c ist Voraussetzung für D3.
- Notizbuch-KI: Notizen bleiben im Notizbuch; „Als Datei ablegen“ ist ein späterer Export, keine Doppelablage.
