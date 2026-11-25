# Dokumentenablage mit Sync und lokalem Abrufindex (Epic #395)

Stand 08.11.2026, Fassung 5, von Codex abgenommen (Runde 5; D1-Vorschau Runde 4). Entscheidungen: wai-sdk ADR-0029 (Abschnitte
E1–E11 werden hier nur umgesetzt, nicht wiederholt). Baut auf Bibliothek-Sync (`2026-11-02-bibliothek-sync.md`),
Projekten (`2026-11-07-buch-ordner-und-projekte.md`, P4-Plan), globalem Sprachassistenten (lva#325) und dem
Assistenten (`2026-11-08-assistent-chat-und-sprache.md`, ADR-0028) auf.

## 1 Ziel

Eine Seite **„Ablage“** in Local Voice (Desktop und iPhone) für allgemeine Dokumente, bedienbar wie OneDrive:
Ordner, Hochladen per Drag & Drop, Vorschau, Fassungen, Papierkorb, Suche. Alle Geräte sehen dieselbe Ablage über
den eigenen Hub (Ende-zu-Ende verschlüsselt). Die KI greift gezielt auf **Auszüge** zu, nur über den Kontextvertrag
(ADR-0028 E6) und nur, soweit Freigabe, Datenklasse und Berechtigungsstand es erlauben.

Abgrenzung: Bücher bleiben in der Bibliothek, Aufnahmen in Aufnahmen, das Notizbuch bleibt eigenes Modul. Projekte
verweisen auf Ablage-Einträge, sie kopieren sie nicht. Vorschau: `apps/local-voice/docs/vorschau/ablage-d1/`.

## 2 Datenordner (Desktop)

```
<Datenordner>/ablage/
  ablage.json            Journal + Baum (Version, Epoche)
  dateien/<dat>/<fas>.<ext>             Originale, unveränderlich
  text/<dat>/<textrev>.json             Textfassung (Markenformat v1, §4.2)
  papierkorb/            30 Tage, Wiederherstellen
  index.db               abgeleitet (wai-retrieval), löschbar, nie exportiert, nie synchronisiert
  freigabe.json          gerätelokal: KI-Freigaben, Cloud-Ausnahmen (§5.1)
  verknuepft.json        gerätelokal: verknüpfte Ordner (§8)
```

IDs: `ord_<ULID>`, `dat_<ULID>`, `fas_<ULID>`. Namen nach P4 §4.4 (Windows-reservierte Namen, Groß/Klein, `..`).
Jede Ablage wie P4a `dateien.rs`: `.teil-<id>` (create_new) → Größe + SHA-256 → ohne Ersetzen umbenennen → Journal
`abgelegt` → Baum. Wiederanlauf, „Bestand neu aufbauen“, Lesefehler ≠ leer. Öffnen ohne Folgen von Verknüpfungen
(ADR-0029 E9), Ziel immer unter `ablage/`.

### 2.1 Datensatz `ablage_datei` (synchronisiert)

```json
{
  "id": "dat_…", "ordner": {"wert": "ord_…", "zaehler": 7, "geraet": "g_…"},
  "name": {"wert": "Angebot 2026.pdf", "zaehler": 3, "geraet": "g_…"},
  "klasse_min": {"wert": "confidential", "zaehler": 1, "geraet": "g_…"},
  "nur_geraet_min": false,
  "fassungen": [
    {"fas": "fas_…", "vorgaenger": null, "blob": "<64 hex>", "size": 812345, "original_sha256": "<hex>",
     "typ": "pdf", "geraet": "g_…", "zeit": 1762600000000,
     "texte": [{"textrev": "<hex>", "blob": "<64 hex>", "size": 40211, "extraktor": "pdf-extract",
                "extraktor_version": "0.12.0", "marken_version": 1, "stand": "ok|kein_text|fehler"}]}
  ],
  "wahlen": [{"id": "wahl_…", "gewaehlt": "fas_…", "gegen": ["fas_…", "fas_…"], "ersetzt": ["wahl_…"],
              "zaehler": 4, "geraet": "g_…"}],
  "entfernt": ["fas_…"],
  "blob_refs": ["<64 hex>"],
  "papierkorb": {"wert": null, "zaehler": 0, "geraet": "g_…"}
}
```

Zusammenführen (ADR-0029 E4): Register nach höherem `(zaehler, geraet)`; `fassungen` vereinigt nach `fas`, ohne die
IDs in `entfernt`; `entfernt`, `wahlen` und `texte` vereinigt (nie verkleinert). Aktive Wahl = in keinem `ersetzt`
genannt. Abgelöst ist F, wenn eine aktive Wahl F in `gegen` hat und nicht gewählt hat und keine aktive Wahl mit
höherem `(zaehler, geraet)` F gewählt hat. Köpfe = ohne Nachfolger und nicht abgelöst; mehr als ein Kopf = Konflikt.
Gegenwahl ergibt nach der Vorrangregel genau einen Kopf (Hinweis „gleichzeitig anders gewählt“); eine erneute Wahl
nennt beide Ereignisse in `ersetzt`; ihr `gegen` ist die Vereinigung der offenen Köpfe und
der `gegen` aller ersetzten Wahlen (verdrängte Fassungen bleiben verdrängt). Die Rechnung ist unabhängig von der Reihenfolge des Zusammenführens.
`zaehler` = größter am Gerät bekannter Wert + 1 (Lamport). `blob_refs` ist sichtbares Steuerfeld (AAD-gebunden) für
die Referenzprüfung am Hub (§6).

### 2.2 Datensatz `ablage_ordner` (synchronisiert)

```json
{"id": "ord_…", "eltern": {"wert": "ord_…|null", "zaehler": 2, "geraet": "g_…"},
 "name": {"wert": "Verträge", "zaehler": 1, "geraet": "g_…"},
 "datenklasse": {"wert": "erbt|public|internal|confidential|personal", "zaehler": 1, "geraet": "g_…"},
 "nur_geraet": {"wert": false, "zaehler": 1, "geraet": "g_…"},
 "zielordner_ki": {"wert": false, "zaehler": 1, "geraet": "g_…"},
 "papierkorb": {"wert": null, "zaehler": 0, "geraet": "g_…"}}
```

Kein `ki`-Feld: Freigaben sind gerätelokal (§5.1). Zyklen: Register bleiben unverändert; die wirksame Elternzuordnung
wird je Stand abgeleitet (Ordner absteigend nach `(zaehler, geraet)` von `eltern`, Kante nur ohne Zyklus übernommen,
sonst vorläufig „Wiederhergestellt“). Elternordner begraben: ebenso abgeleitet unter „Wiederhergestellt“. Unbekannter
Klassenwert: wie `gesperrt` bis zur Neuwahl. Unterordner und Dateien dürfen nur gleich oder höher einstufen; eine
niedrigere gespeicherte Klasse wird auf die geerbte angehoben (nie umgekehrt).

## 3 Oberfläche (D1)

Siehe Vorschau `ablage-d1` (32 Bilder, Maßbelege). Kern:
- Leiste „Ablage“; Kopf über `Seitenkopf`; Ordnerbaum mit eigenen Ordnern, verknüpften Ordnern (nur lesen), Papierkorb.
- Liste: Name, Geändert, Größe, KI-Freigabe/Klasse, Sync-Zustand (synchronisiert, lädt hoch, nur Cloud, nur Gerät,
  Konflikt, „Text wird am PC erstellt“).
- Details: Vorschau, „Für die KI“ (Zustand an/aus/gesperrt, geerbt von), Klasse, Textfassung, Fassungen, Projekte.
- Drag & Drop, Ordner hochladen mit Struktur, Namenskollision → „neue Fassung“ oder „beide behalten“.
- Konflikt: beide Fassungen sichtbar, Wahl; KI nutzt bis dahin die jüngste gemeinsame Vorgängerfassung, ohne
  gemeinsame Fassung die Datei gar nicht.
- Leer: ein Satz, keine Vorschlagsblasen. Kontingent im Fuß. 390/768/1440/2560 px, hell/dunkel, Ziele ≥ 44 px.
- Bestätigungsdialoge: sync → nur dieses Gerät („Kopien auf anderen Geräten und in der Cloud werden entfernt“),
  nur dieses Gerät → sync, Cloud-Ausnahme für `confidential`, endgültig löschen.

## 4 Textfassung und Index (D2)

### 4.1 Extraktion (ADR-0029 E9)

- Extraktoren: PDF `pdf-extract`, DOCX/PPTX/ODT/ODP per `zip` + `quick-xml` (DOCTYPE abgewiesen, keine Entitäten),
  XLSX/ODS `calamine` (MIT/Apache, neu), TXT/MD/CSV, HTML (ohne Skripte/Nachladen), EML (Kopf + Textteile; Anhänge
  werden als eigene Dateien angeboten, nie still aufgenommen).
- Eigener Unterprozess `lva-extrakt` über `process_guard` (Windows Job-Objekt mit Speichergrenze, Unix Prozessgruppe +
  `RLIMIT_AS`): 60 s, 1 GiB, ZIP ≤ 10 000 Einträge, ≤ 200 MiB entpackt, Verhältnis ≤ 100:1, kein Netz.
- Eingabe ist ausschließlich die abgelegte Fassung; Prozess prüft `original_sha256` vor dem Lesen und gibt ihn mit aus.
- Warteschlange im Hintergrund (ein Kern, abbrechbar). Fehler sichtbar am Eintrag; nie Inhalt im Log.

### 4.2 Markenformat v1

```json
{"marken_version": 1, "textrev": "<hex>", "original_sha256": "<hex>", "extraktor": "…", "extraktor_version": "…",
 "abschnitte": [{"anker": "seite=7", "pfad": ["§ 4 Laufzeit"], "text": "…"}]}
```

Anker je Format: PDF `seite=<n>`; DOCX/ODT `absatz=<n>` + `pfad` (Überschriften); PPTX/ODP `folie=<n>`;
XLSX/ODS `blatt=<name>&zeilen=<a>-<b>` (Zeilen als `Spalte: Wert`-Text); EML `teil=kopf|text|<n>`; TXT/MD/CSV
`zeile=<n>`. Neue Extraktorversion = neue `textrev`; alte bleibt bis zum Neuaufbau, Belege darauf zeigen
„Text wurde neu erstellt“.

### 4.3 Index

- `index.db` über vendortes `wai-retrieval`: Stücke je (`dat`, `fas`, `textrev`, Abschnitt), Startwert 220 Zeichen
  mit Überlappung (Eval entscheidet). Felder für Ordnerkette (IDs), Klasse, `nur_geraet`, Papierkorb, Stand.
- Filter in SQL vor dem Ranking: Papierkorb raus, Freigabe nach §5.1, Klasse nach §5.2.
- Suche in der Ablage (ohne KI): Treffer mit Auszug und Sprung zum Anker. Neuaufbau bei Extraktor-/Embedder-Wechsel.

## 5 KI-Zugriff (D3, nach #365 P4c)

### 5.1 Freigaben und Berechtigungsstand

- `freigabe.json` gerätelokal (Temp + Umbenennen, Sperre, Epoche/Generation wie P4a `kontext.json`): je Ordner
  `an|aus|gesperrt`, fehlend = aus. Cloud-Ausnahmen je (Ordner, Anbieter) nur für `confidential`.
- Vor jedem Schreiben wird die letzte gültige Fassung als `freigabe.json.vorher` abgelegt. **Defekt/unlesbar/neuer** =
  „Freigabe unklar“: die ganze Ablage ist auf **beiden** Wegen gesperrt (auch Projektmitgliedschaften, die an sind).
  Wiederherstellung automatisch aus gültiger `.vorher` (mit allen `gesperrt`), sonst nur über „Freigaben neu
  festlegen“ (alle Ordner `aus`, Bestätigung). Defekte Datei wird beiseitegelegt.
- Globaler Weg (Assistent, lva#325 fragt den D2-Index ab): nur Unterbäume mit `an`.
- Projektweg: Mitgliedschaftsschalter (P4a); `gesperrt` schließt auf beiden Wegen aus.
- Berechtigungsstand nach ADR-0029 E6; vor jeder Modellübergabe neu berechnet.

### 5.2 Datenklasse

`public`/`internal`: lokal und freigegebene Cloud-Modelle. `confidential`: lokal, Cloud nur mit Ausnahme + passender
`datenklasse_max`. `personal`: nur lokal. Unbekannt: kein Abruf.

### 5.3 Form `auszuege`

Je Frage Top-k (Start 8) aus eingeschalteten Mitgliedern bzw. freigegebenen Unterbäumen, mit `[Q<k>]`, maskiertem
Titel, Anker und Deep-Link `wai://lva/ablage/<dat>?fas=<fas>&textrev=<rev>#<anker>`. Dazu eine **Abdeckungsmeldung**:
Anzahl Dateien im Bereich, durchsucht, ohne Text, im Konflikt; „kein Beleg“, wenn nichts trägt. `volltext` nur
ausdrücklich und im Budget, sonst sichtbarer Stopp.

### 5.4 Widerruf im Gesamtprompt

Jeder Auszug trägt `dokument:<dat>@<fas>#<textrev>`. Vor jeder Modellübergabe prüft das Tor alle Auszüge in Verlauf,
Zusammenfassungen und Auftragszwischenständen; nicht mehr berechtigte werden durch „[Auszug entzogen]“ ersetzt,
abgeleitete Zusammenfassungen verworfen; geht das nicht, neue Runde ohne alten Verlauf; Anbietersitzung zurücksetzen.

### 5.5 KI-Entwürfe

Einziges schreibendes Werkzeug `ablage_entwurf_anlegen(ordner, name, inhalt_md, quellen)`: neue Datei in
„KI-Entwürfe“ oder Zielordner mit `zielordner_ki`; Klasse = Maximum aus Zielordner und allen Quellen; `nur_geraet`, wenn
eine Quelle `nur_geraet` ist (dann Ordner „KI-Entwürfe (nur dieses Gerät)“). Kein Werkzeug zum Überschreiben,
Verschieben oder Löschen.

### 5.6 Beispiele

„Was steht im Wartungsvertrag zur Laufzeit?“ (Beleg Seite 7) · „Fasse den Ordner Verträge zusammen“ (Auszüge,
Abdeckung, Budgetstopp sichtbar) · „Lege die Zusammenfassung als Notiz ab“ (Entwurf, Klasse vertraulich).

## 6 Sync (D4a, D4b, D5)

- Hub: Sammlungen `ablage_ordner`, `ablage_datei`; Blobs wie Bibliothek; neu (wai-portal-PR):
  `POST /api/v1/voice/blobs/{id}/reserve` (24 h je Gerät), Push von `ablage_datei` prüft und setzt `blob_refs` in einer
  Transaktion (fehlender Blob → 422), Löschen nur ohne Referenz und ohne gültige Reservierung (sonst 409), Grabsteine
  geben Referenzen frei, Schreibvorgänge auf begrabene IDs → abgewiesen.
- Reihenfolge je Fassung: reservieren → Original-Blob hochladen (oder vorhandenen nutzen) → Text-Blob(s) (D4b) →
  Objekt-Push (gibt Reservierungen frei). 422 beim Push → neu hochladen und erneut pushen.
- `nur_geraet`-Unterbäume nie übertragen. Wechsel sync → lokal: lokale Kopie prüfen → Grabstein `lokalisiert` →
  andere Geräte entfernen Kopien, Texte, Index → Blob-Löschung (nur ohne Referenz).
- Rückweg lokal → sync: neue IDs für den ganzen Unterbaum, Journal `ablage/uebergang-<ULID>.json` (`begonnen` mit
  Zuordnung alt → neu → `hochgeladen` → Verweise, Freigaben, `kontext.json`, Index umgeschrieben → `fertig`),
  Wiederanlauf am letzten Schritt; alte IDs bleiben lokal als Weiterleitung für Deep-Links.
- Endgültig löschen: Grabstein; jedes Gerät entfernt Original, Texte, Index, Caches, Miniaturen; Blob-Löschung nur,
  wenn im vollständig gezogenen Bestand keine lebende Fassung den Blob nutzt, und nur bedingt.
- Aufbewahrung: Fassung bleibt, wenn sie Kopf ist, von einer aktiven Wahl gewählt ist, jüngste gemeinsame
  Vorgängerin offener Köpfe ist, zu den 10 jüngsten gehört oder jünger als 90 Tage ist; sonst `entfernt`.
- iPhone (D5): Textfassung sofort, Original bei Bedarf („nur im WLAN“); eigene Importe zeigen „Text wird am PC
  erstellt“ und sind bis dahin nicht im Abruf; lexikalischer Index in VoiceCore; Assistent-Bereich wie Desktop.
- Kontingent aus `GET /api/v1/voice/blobs`; Hochladen stoppt sichtbar bei Überschreitung.

### 6.1 Eigene Phonon-Stimmen (lva#898, S1 Variante B)

Der private Stimmtransport verwendet unverändert `ablage_ordner`, `ablage_datei` und die bestehenden
Blob-Routen; kein Portal-Deploy und keine zusätzliche Sammlung. Nur Stimmen aus `stimmen.json` mit
Herkunft `import` oder `gradium-clone`, niemals Modellstimmen oder Modellgewichte. Der Transport läuft
im eingerichteten Geräte-Sync. Im Desktop-Stand vom 25.11.2026 gibt es keinen eigenen Ablage-Sync-Schalter:
ohne gespeichertes, aktives Sync-Konto läuft auch dieser Transport nicht. Import, Gradium-Klon und
Entfernen stoßen den bestehenden Sync-Takt an; der erste Lauf trägt vorhandene Stimmen nach.

**Kennungen, Generationen und CAS.** Die Funktion `ablage::system::kennung` bildet
`SHA-256(key || "lv-phonon-sync-v1" || art || generation_be_u32 || salt_utf8)`;
`art` ist ein Byte (`1` Ordner, `0` Datei). Die 16 ULID-Bytes bestehen aus vier Domänenbytes
(`LVSO` beim Ordner, `LVSF` bei der Datei) und den ersten zwölf Hash-Bytes. Davor steht `ord_` bzw. `dat_`.
Normale neue Ablage-ULIDs verwenden den Zeitstempelraum, die reservierten IDs diese feste Domäne.
Der Systemordner heißt `.lv-stimmen`, hat keine Eltern und Klasse `internal`; sein `salt` ist leer.
Dateien verwenden `salt = sha256_hex + ":" + nonce_dezimal` (anfangs `0`). Die Fassungskennung
ist dieselbe ULID mit `fas_` statt `dat_`. Beide Geräte erzeugen deshalb bei gleichem Inhalt dieselbe
Datei und dieselbe Fassung, auch bei gleichzeitigem Erstimport. Die bestehende `superseded`-Behandlung
vereinigt den Stand über CAS, ohne einen zweiten Ordner oder einen zweiten Fassungskopf anzulegen.
Die Kennungen sind kontoschlüsselgebunden und enthalten keinen Stimmnamen; der gekürzte Hash hat
96 Bit Kollisionswiderstand. Vorhandene Datensätze werden stets zusammengeführt, nie blind ersetzt.

Grabsteine bleiben endgültig. Ist die Ordnerkennung einer Generation am Hub oder lokal begraben,
wird die erste noch nicht begrabene Generation (`0`, `1`, …) verwendet. Eine begrabene Datei bekommt
bei erneuter Anlage die erste freie `nonce` und damit eine neue ID. Alte Apps zeigen den Ordner als
gewöhnlichen Ordner. Nach dessen endgültigem Löschen wird seine ID niemals neu angelegt: vorhandene
lokale Stimmen ziehen in die Folgegeneration; erst nach der dauerhaften Ablage dort werden ihre alten
Dateikennungen begraben. Auch ein vom Hub bereits begrabener, lokal wegen lebender Kinder noch
gehaltener Ordner zählt als begraben. Nicht durch denselben Namen einen neuen Nutzerordner erkennen!

**Nutzlast und iOS-Vertrag.** Eine Stimme ist eine normale `Datei` mit genau einer Fassung
(`typ: "safetensors"`, `blob == original_sha256 == sha256`, unveränderliche Bytes, keine Texte,
Vorgänger oder Wahlen). `blob_refs` und AAD entsprechen §6, einschließlich Schlüsselkennung und
Grabsteinflag. Der sichtbare Dateiname innerhalb der verschlüsselten Nutzlast ist `voice.safetensors`.
Die zusätzliche Eigenschaft `phonon_voice` ist ein Array:

```json
["00000001795603200000", {
  "id": "Probe", "name": "Probe", "language": "de", "origin": "import",
  "consent_at": null, "sha256": "<64 kleine Hexzeichen>", "generation": 0, "nonce": 0
}]
```

Das erste Element ist `updated` als nichtnegative Unix-Millisekunden, mit Nullen auf genau 20 Stellen
aufgefüllt. So gewinnt beim bestehenden kanonischen Merge unbekannter Zusatzfelder zuerst die Zeit,
bei gleicher Zeit deterministisch der restliche Wert. Zwischen verschiedenen Dateien gleichen
Stimmnamens gewinnt `(updated, Datei-ID)`; der Namensvergleich ignoriert ASCII-Groß/Kleinschreibung.
Erstimporte alter `stimmen.json` verwenden `created_at`. `language` ist `de`, `en`, `fr`, `es`, `pt`
oder `und` für einen Dateiimport ohne bekannte Aufnahmesprache. `und` wird lokal wieder als leerer
Sprachwert geführt. `consent_at` ist der optionale übernommene Einwilligungszeitpunkt; Herkunft,
Einwilligung und Lizenz werden dadurch nicht verifiziert.

Desktop-Reihenfolge: lokale Löschbelege anwenden → lokale Stimmen in die Ablage einlegen → normaler
Ablage-Lauf (Ordner-Pull, Datei-Pull, Blobs, Push) → entfernte Stimmen im VoiceStore übernehmen.
Ein nach dem Pull erforderlicher Push folgt spätestens im nächsten Takt. Die iOS-Seite muss zuerst
die Metadaten, danach diese kleinen Original-Blobs automatisch ziehen (nicht erst beim Öffnen eines
Dokuments) und vor dem Import dieselben Prüfungen anwenden. S2 und die Modell-Injektion folgen nach S0.

Vor Pfadbildung: ID und Anzeigename wie beim Desktop-Import prüfen (ID aus `[A-Za-z0-9_-]`, maximal
40 Zeichen, keine Windows-Gerätenamen, kein `default`, keine Pfadzeichen; gefalteter Anzeigename muss
die ID ohne Beachtung der ASCII-Groß/Kleinschreibung ergeben), bekannte Sprache/Herkunft, erwartete Konto-/Generations-/Dateikennung,
16 KiB bis 4 MiB, SHA-256, gültiger safetensors-Kopf mit lückenlosen Tensor-Offsets und
`speaker_wavs` genau `F32 [1,512,125]`. Nie eine Modellstimme überschreiben. Ohne installiertes Modell
trotzdem nach `eigene-stimmen/` importieren; der vorhandene Modellabgleich kopiert später.
Unbrauchbare Metadaten oder Bytes werden je Fingerabdruck des unveränderlichen Datensatzes lokal
vermerkt, nicht endlos heruntergeladen/importiert und niemals deswegen begraben. Netzfehler bleiben
wiederholbar; der übrige Ablage-Lauf läuft weiter.

`stimmen.json` führt pro Stimme optional `sync_file` (`konto`, `file`, `sha256`, `voice`, `updated`)
und `updated`. Der Herkunftsbeleg wird beim Import zusammen mit dem Stimmeintrag atomar geschrieben.
`sync_pending` bewahrt lokale Löschbelege; sie begraben auch später eintreffende ältere Namenskonflikte.
Ein Grabstein entfernt nur die Stimme mit genau dieser Dateikennung und unveränderten Bytes.
Ein lokaler neuer Import setzt den Beleg zurück und erhält einen höheren Zeitwert; gleiche Namen
allein reichen nie zum Entfernen. Datei-vor-Index-Abbrüche werden anhand der bereits bekannten
entfernten Prüfsumme fertiggestellt und erzeugen keinen künstlich neuen Upload.

**Abschirmung.** `Ablage::lage` projiziert den Benutzerbestand zentral ohne Systemkennungen und deren
Unterbäume; beim Sichern bleiben die ausgeblendeten Datensätze erhalten. Liste, Volltext-/Semantik-
Quellen, KI-Bestand, Export des Originals, Papierkorb und direkte Änderungsbefehle sehen sie deshalb
nicht. Verwaiste Systemdateien bleiben an ihrer ID erkennbar und werden auch nach Journalverlust nicht
als Dokumentenfunde angeboten. Dokumentzähler und lokale Dokument-Speicheranzeige schließen sie aus;
das Hub-Kontingent zählt ihre verschlüsselten Blob-Bytes weiterhin. Sync-Snapshots und Wiederanlauf
verwenden den vollständigen internen Stand. Stimmbytes und Stimmnamen gelangen niemals in Logs.

**Prüfstand.** `sync::ablage::tests::phonon_tests` verwendet ausschließlich synthetische Stimmen und
den vorhandenen Hub-Nachbau: Hochladen/Ziehen/Entfernen, gleichzeitiger Erstimport, Namens-LWW,
lokale Neuanlage, kaputte Bytes bei weiterlaufenden Dokumenten, Pfad-/Windows-/Sprachprüfung,
Modellschutz, Folgegeneration und Wiederanlauf, auch nach einem Namenswechsel bei voller Stimmenliste
und bei offenem System-Löschjournal. Rust-Ausführung einschließlich Rot-Gegenproben ist
im schreibenden Codex-Auftrag ausdrücklich an den anschließenden Thread-Claude-Lauf delegiert.
Gegenproben: Projektion umgehen → Abschirmungsasserts rot; Inhalts-ID durch Zufall ersetzen →
Deduplizierung rot; SHA/Tensor-Prüfung entfernen → Importprüfung rot; Herkunfts-/Hashprüfung des
Grabsteins entfernen → Neuanlage/Wiederanlauf rot; Generationsauswahl beim Ordnergrab überspringen →
Ordnergrabtest rot; nachgelagertes Namens-Cleanup auslassen → Duplikat-Wiederanlauf rot; Kapazitätsersatz
auslassen → Grenztest rot; Journal erst nach der Projektion abarbeiten → Löschjournaltest rot.

## 7 Semantische Suche (D6, nach D2 parallel)

`multilingual-e5-small` (MIT) als ONNX über das vorhandene `ort`, Download bei Bedarf mit Hash-Prüfung, Hybrid per RRF.
Eval-Tor auf synthetischem Korpus (nur `example.*`): Recall@5 ≥ 0,9, Belegkorrektheit ≥ 0,95, unbeantwortbare Fragen
≥ 0,9 als „kein Beleg“; sonst bleibt lexikalisch Standard.

## 8 Verknüpfte Ordner und OCR (D7)

Gerätelokal, nur lesen, nie hochladen. Auswahl über Ordnerdialog, Ausschlüsse (`~$*`, `.tmp`, Systemordner), keine
Verknüpfungen nach außen, Dateibeobachter mit Entprellung. Klasse Vorgabe `confidential`. Extraktion über eine
geprüfte Momentkopie. Lokale OCR (Windows.Media.Ocr / Apple Vision) für Bild-PDFs und Bilder.

## 9 Tests (je Stufe)

- D1: Abbruch nach Zwischendatei/Hash → kein Eintrag; `CON.txt`, `..\x`, Groß/Klein-Kollision; Verknüpfung/Junction
  als Ziel oder Quelle → abgewiesen; Papierkorb + Wiederherstellen; Journal defekt → „Bestand neu aufbauen“.
- D2: Fixtures je Format mit Ankern; ZIP-Bombe, XXE, übergroßes XLSX, Endlos-PDF → Abbruch an der Grenze, App stabil;
  Extraktorwechsel → neue `textrev`; Suchtreffer mit korrektem Anker; Neuaufbau idempotent.
- D4a: Zwei-Geräte-Konflikt → zwei Köpfe, `aktuell` nicht per Zeit; Wahl-Rennen (Lamport) deterministisch; Ordner-
  Verschieberennen ohne Zyklus; Grabstein → Offline-Gerät belebt nichts; Löschen ∥ Upload desselben Blobs → keine
  lebende Fassung ohne Blob (reservieren → löschen 409 → pushen; Reservierung abgelaufen → Push 422 → neu hochladen);
  `nur_geraet` erscheint nie im Hub (Mitschnitt), Wechsel entfernt Cloud-Kopien; Wahl B, dann B → C: ein Kopf;
  Offline A → D: neuer Konflikt; Gegenwahl → genau ein Kopf (Vorrang) → erneut wählen → genau ein Kopf →
  Weiterbearbeitung konfliktfrei, in allen Zusammenführ-Reihenfolgen; W1 wählt A, W2 (höher) wählt B, W3 wählt B und
  ersetzt beide → genau ein Kopf B; Ordner A→B (1), B→A (2), B→C (3) in beiden Merge-Abfolgen → A unter B unter C; Aufbewahrung hält die gemeinsame Vorgängerin
  offener Köpfe auch mit 12 neueren Fassungen und Alter 120 Tage; Rückweg lokal → sync mit Abbruch nach jedem
  Journalschritt; Projektverweis zeigt danach auf die neue ID.
- D4b: Textfassung kommt mit passender `textrev`; fehlende Textfassung → Wartezustand.
- D3: Kontrolltext in `aus`/`gesperrt` → 0 Treffer (gesperrt auch bei Projektmitgliedschaft an); Widerruf mitten im
  Gespräch → 0 widerrufene Auszüge im nächsten Gesamtprompt (Verlauf, Zusammenfassung); Papierkorb → 0 Treffer;
  defekte `freigabe.json` → 0 Auszüge auf beiden Wegen, Wiederherstellung aus `.vorher` erhält `gesperrt`;
  Konflikt ohne gemeinsame Fassung → Datei nicht genutzt, in der Abdeckung gemeldet;
  `personal` an Cloud → 0 Übermittlungen; `confidential` ohne Ausnahme → abgewiesen; Injektion → 0 Aufrufe;
  Entwurf aus `personal`-Quelle in `internal`-Ordner → Klasse `personal`.
- D5: Swift-Testvektor gegen Rust (Blob-Kennung, AAD, `textrev`); Index-Treffer gleich Desktop für Musterfragen.

## 10 Datenschutz

E2E (Hub ohne Schlüssel), keine Namen/Inhalte in Log/Audit, Datenklasse je Ordner, `personal` nie Cloud, Papierkorb-
und Fassungsfristen, endgültiges Löschen mit Nachweis (IDs) auf allen Geräten, Hinweis beim Einrichten: dienstliche
personenbezogene Daten erst nach DSFA. Grenze: lokaler Index im App-Datenordner, Schutz über Geräteverschlüsselung.

## 11 Abstimmung

- Assistent (ADR-0028, lva#384): Ablage ist eine Quelle hinter `projekt_kontext`; Kontextleiste ＋ → „Aus Ablage“;
  Tor prüft Herkunftsmarken im Gesamtprompt.
- Projekte (lva#365): Verweisart `ablage`, Form `auszuege`; P4c ist Voraussetzung für D3.
- lva#325: Bereich „Ablage“ fragt den D2-Index ab, kein zweiter RAM-Index.
- Notizbuch-KI: Notizen bleiben im Notizbuch; „Als Datei ablegen“ ist ein späterer Export, keine Doppelablage.
