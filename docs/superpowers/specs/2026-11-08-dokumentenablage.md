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

### 6.1 Eigene Phonon-Stimmen (lva#898, S1a: eine Datei je Stimme)

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
Dateien verwenden ausschließlich `salt = uid`: 16 zufällige Bytes, als 32 kleine Hexzeichen,
beim Anlegen dauerhaft in `stimmen.json` gespeichert. Genau eine `ablage_datei` gehört zu dieser uid
innerhalb einer Konto-/Ordnergeneration. Weder Name noch Inhalt, Änderungszeit oder nonce gehen in
die Dateikennung ein. Die Fassungskennungen sind normale neue `fas_`-ULIDs. Die bestehende
`superseded`-Behandlung vereinigt den Stand über CAS und erhält konkurrierende Fassungen.
Die Kennungen sind kontoschlüsselgebunden und enthalten keinen Stimmnamen; der gekürzte Hash hat
96 Bit Kollisionswiderstand. Vorhandene Datensätze werden stets zusammengeführt, nie blind ersetzt.

**S1a synchronisiert Anlegen, Ändern und Umbenennen. Entfernen wirkt nur lokal.**
Keine Löschweitergabe in S1a; eine spätere Löschweitergabe benötigt ein Folgeissue mit eigenem
Vertrag. Weder Ordner- noch Dateigrabsteine bedeuten einen Löschwunsch für lokale Stimmen.

Grabsteine bleiben für Transportkennungen endgültig. Ist ein Systemordner oder eine Stimmdatei
durch eine alte App begraben, bleibt die lokale Stimme samt uid erhalten. Für jede Stimme wird
ab der höchsten bekannten Generation die nächste Generation mit lebendem Ordner und lebender
oder noch nicht vorhandener Dateikennung verwendet. Ordner- und Dateikennung werden weiterhin
deterministisch aus Kontoschlüssel, uid und Generation abgeleitet; zwei Geräte verwenden dieselbe
Kennung. Jede Generation wird pro Stimme und Abgleich höchstens einmal geprüft, pro Stimme wird
höchstens einmal bereitgestellt. Weitere am Hub gelernte Gräber werden im nächsten Takt behandelt.
Dateien verschiedener lebender Generationen bleiben lesbar; je uid gewinnt die höchste Generation.
Alte Clients dürfen Ordner und Dateien in beliebiger Reihenfolge begraben. Es gibt keine vorgemerkten
Gräber, Doppel-Pull-Bestätigung oder Grabaufträge aus der Stimmenbrücke. Eine lokal als entfernt
gemerkte uid wird weder importiert noch erneut bereitgestellt.

**iOS-Testvektor.** Schlüsselbytes `00 01 … 1f`, Generation `0`, uid
`00112233445566778899aabbccddeeff` (UTF-8-Hextext, nicht die dekodierten 16 Bytes):
`dat_2CAS9MD268N1T7K9V0N4GCDVT8`. Derselbe Schlüssel und leerer salt ergeben
`ord_2CAS9MY90GTRQEBVKFKP5TXZAB` für Generation `0` und
`ord_2CAS9MYNV00X4X0TY6MQE0ECCH` für Generation `1`. Crockford-Base32 kodiert die
16 Bytes in 26 Zeichen mit führenden Nullbits. Der Rust-Vertragstest enthält dieselben Vektoren.

**Nutzlast und iOS-Vertrag.** Eine Stimme ist eine normale `Datei` mit einer oder mehreren Fassungen
(`typ: "lvstimme"`, `blob == original_sha256`, unveränderliche Bytes, keine Texte).
Neue Bytes werden als neue Fassung mit der vorherigen Fassung als `vorgaenger` angelegt;
reine Metadaten- und Namensänderungen benötigen keinen neuen Blob und keine neue Datei.
Die normale Fassungsaufbewahrung (zehn jüngste bzw. 90 Tage, Köpfe und aktive Wahlen) begrenzt
die gespeicherten Bytes auch für Systemdateien; `entfernt` und `blob_refs` reisen wie üblich mit.
Bei mehreren Köpfen wird deterministisch die größte Fassung nach `(zeit, fas)` gewählt;
eine normale Ablage-Wahl löst die übrigen Köpfe ab. Ein paralleler Namenswechsel bleibt erhalten.
`blob_refs` und AAD entsprechen §6, einschließlich Schlüsselkennung und
Grabsteinflag. Der sichtbare Dateiname innerhalb der verschlüsselten Nutzlast ist `voice.safetensors`.
Die zusätzliche Eigenschaft `phonon_voice` ist ein Array:

```json
["00000001795603200000", {
  "id": "Probe", "uid": "00112233445566778899aabbccddeeff", "name": "Probe",
  "language": "de", "origin": "import", "consent_at": null,
  "voice_id": "", "source_note": null,
  "created": "2026-11-25T00:00:00Z", "sha256": "<64 kleine Hexzeichen>",
  "fas": "fas_<26 ULID-Zeichen>", "generation": 0, "linked": false, "merged": []
}]
```

Das erste Element ist `updated` als nichtnegative Unix-Millisekunden, mit Nullen auf genau 20 Stellen
aufgefüllt. So gewinnt beim bestehenden kanonischen Merge unbekannter Zusatzfelder zuerst die Zeit,
bei gleicher Zeit deterministisch der restliche Wert. `sha256` und `fas` zeigen auf die ausgewählte
Fassung; nach einer konkurrierenden Zusammenführung werden sie an die Fassungswahl angepasst.
Der Name besitzt zusätzlich ein eigenes Extra-Register `phonon_name`:
`["00000001795603200000", "Probe", "Probe"]` = Rang, gefaltete ID, gewünschter Anzeigename.
So kann eine parallele Byteänderung einen Namenswechsel nicht zurücksetzen. Beide Register müssen
valide Namen enthalten; für die Anzeige gilt `phonon_name`. `created` bleibt bei Änderungen erhalten
und ist der RFC3339-Zeitpunkt der Anlage. Erstimporte bestehender lokaler Stimmen verwenden
`created_at` auch als anfänglichen `updated`-Wert. `language` ist `de`, `en`, `fr`, `es`, `pt`
oder `und` für einen Dateiimport ohne bekannte Aufnahmesprache. `und` wird lokal wieder als leerer
Sprachwert geführt. `consent_at` ist der optionale übernommene Einwilligungszeitpunkt; Herkunft,
Einwilligung und Lizenz werden dadurch nicht verifiziert.

Desktop-Reihenfolge: normaler Ablage-Lauf (vollständiger Ordner-/Datei-Pull, Blobs, Push) →
Generation je Stimme bestimmen → lokale Änderungen in die Ablage einlegen →
Erstsync-Zusammenführungen und Namenskonflikte bestimmen → entfernte Stimmen im VoiceStore übernehmen.
Ein nach dem Pull erforderlicher Push folgt spätestens im nächsten Takt. Die iOS-Seite muss zuerst
die Metadaten, danach diese kleinen Original-Blobs automatisch ziehen (nicht erst beim Öffnen eines
Dokuments) und vor dem Import dieselben Prüfungen anwenden. S2 und die Modell-Injektion folgen nach S0.

Vor Pfadbildung: ID und Anzeigename wie beim Desktop-Import prüfen (ID aus `[A-Za-z0-9_-]`, maximal
40 Zeichen, keine Windows-Gerätenamen, kein `default`, keine Pfadzeichen; gefalteter Anzeigename muss
die ID ohne Beachtung der ASCII-Groß/Kleinschreibung ergeben), bekannte Sprache/Herkunft, erwartete Konto-/Generations-/Dateikennung,
16 KiB bis 4 MiB, SHA-256, gültiger safetensors-Kopf mit lückenlosen Tensor-Offsets und
`speaker_wavs` genau `F32 [1,512,125]`. Nie eine Modellstimme überschreiben. Ohne installiertes Modell
trotzdem nach `eigene-stimmen/` importieren; der vorhandene Modellabgleich kopiert später.
Unbrauchbare Metadaten oder Bytes werden je Fingerabdruck des Datensatzes lokal
vermerkt, nicht endlos heruntergeladen/importiert und niemals deswegen begraben. Netzfehler bleiben
wiederholbar; der übrige Ablage-Lauf läuft weiter.
Auch der feste Clientfehler `Stückantwort zu groß` wird einmalig je Fingerabdruck verworfen.

**Konflikte und Erstsync.** Verschiedene uids bleiben auch bei gleichem Namen beide sichtbar.
Der Namensvergleich ignoriert ASCII-Groß/Kleinschreibung. Je gewünschter Namens-ID behält die kleinste
`(created, uid)` den Basisnamen (`created` als Unix-Millisekunden des RFC3339-Zeitpunkts verglichen).
Alle echten Basisnamen und Katalog-Modellstimmenkennungen werden
vorab reserviert. Übrige uids werden aufsteigend sortiert und bekommen den ersten freien Namen
`<Basis-ID>-2`, `-3`, …; die Basis wird vor dem Suffix auf insgesamt 40 Zeichen gekürzt und der
fertige Name durch die normale Namensprüfung geschickt. Bekannte Modellnamen werden niemals belegt.
Die Projektion wird vor jeder Änderung als Gesamtplan einschließlich Kapazität, verbleibender lokaler
Namen und Foreign-Prüfung berechnet. Bei einem fremden Modellnamen erhält die eigene Stimme `<Basis-ID>-eigen`, dann
`-eigen-2`, `-eigen-3` usw. Auch diese Suffixe werden vorab auf 40 Zeichen begrenzt und
vollständig validiert. Ein belegter Suffix wird übersprungen, niemals
erst während der Ausführung eine einzelne UID ausgelassen. Entdeckte fremde Modellnamen werden als
monotone Extra-Felder `phonon_blocked_<kleine gefaltete ID>: true` an der betroffenen Datei geteilt.
Der bestehende Extra-Merge vereinigt diese einzelnen Sperren auch bei paralleler Entdeckung.
Alle Geräte reservieren die bekannten Sperren; sie bleiben für die Lebensdauer der tragenden Datei
auch bei späterem Entfernen der fremden Modelldatei bestehen. So konvergieren Geräte mit verschiedenen
installierten Modellen auf dieselben Namen. Fremde Bytes werden nicht ersetzt. Der gewünschte
Name bleibt im Register, der Konfliktname ist eine berechnete lokale Projektion. Dadurch erzeugen
Geräte keine wechselnden Umbenennungsoperationen. Entfällt eine Kollision, wird neu projiziert.

Nur zwei noch nie auf einem anderen Gerät empfangene Erstimporte (`linked: false`) mit gleichem
Namen **und** SHA-256 werden zusammengeführt. Kleinste uid gewinnt. Der Gewinner führt die sortierten,
eindeutigen Verlierer-uids in `merged` (jede größer als die Gewinner-uid); das ist eine Umleitung zur
selben Stimme, kein namens- oder hashbezogener Löschbeleg. Die Verliererdatei bleibt unbenutzt
am Hub liegen und wird niemals wegen der Zusammenführung begraben. Jedes Gerät merkt die
Zuordnung zusätzlich lokal in `sync_aliases` (Verlierer-uid → Gewinner-uid), bevor die lokale
Kopie auf den Gewinner umgebunden wird. Die Merkliste verhindert spätere Verliererimporte,
auch nach lokalem Entfernen des Gewinners.
Ein Empfang auf einem weiteren Gerät oder eine lokale Änderung beendet das Erstsync-Fenster
(`linked: true`). Bereits verknüpfte Stimmen und Neuanlagen werden nicht durch gleiche Bytes verschmolzen.

**Lokaler Stand, Löschen und Wiederanlauf.** `stimmen.json` führt die zufällige `uid`, `updated` und
optional `sync_file` (`konto`, `file`, `sha256`, `voice` = uid, `updated`, `generation`, `snapshot`,
`name` = gewünschter Name, `display` = projizierter Name). `snapshot` ist der SHA-256 des kanonischen
JSON-Arrays aus lokaler ID, uid, Name, Sprache, Herkunft, voice_id, source_note, Einwilligung,
Anlagezeit, updated und Byte-SHA. `sync_linked` merkt lokal das beendete Erstsync-Fenster,
auch nach Verlust des Links. Damit sind auch Metadatenänderungen seit dem letzten Abgleich erkennbar.
Explizites Entfernen merkt **immer**, auch ohne `sync_file`, die exakte uid im optionalen Feld
`sync_removed`, bevor lokale Bytes entfernt werden. Diese Merkliste gilt nur auf diesem Gerät
und bleibt erhalten, solange `stimmen.json` nicht als Ganzes zurückgesetzt wird. Eine solche uid
wird nie importiert oder bereitgestellt. Die Datei am Hub bleibt erhalten; andere Geräte behalten
ihre Stimme. Ein Weg zum Wiederholen ist in S1a nicht vorgesehen. Erneutes Anlegen oder Importieren
gleichen Namens erzeugt eine neue uid, auch bei gleichem Inhalt.

Entfernen während des Erstsyncs hält zusätzlich nur lokal in `sync_removed_initial` die eigene
`uid`, die gewünschte gefaltete Namens-ID, den Byte-SHA und den Entfernzeitpunkt (`updated` des
Vermerks, mindestens das lokale Stimm-`updated`) fest, sofern `sync_linked` noch falsch ist.
Damit zählt eine schon entfernte eigene Transportkopie bei der Erstsync-Zuordnung mit, auch wenn
sie nicht mehr in den Importkandidaten steht. Vor jedem Import wird ein ebenfalls noch nie
verknüpfter Kandidat mit gleichem Namen (ASCII-insensitiv) und SHA als dieselbe hier entfernte
Stimme behandelt: auch seine UID kommt dauerhaft in `sync_removed`; er wird nicht importiert.
Dies gilt nur, wenn sowohl `created` (RFC3339 als Unix-Millisekunden) als auch das Transport-`updated`
höchstens dem Entfernzeitpunkt entsprechen. Eine später angelegte oder geänderte Stimme fällt
nicht darunter. Bewusstes lokales Anlegen/Importieren entfernt nur den passenden Name+SHA-Vermerk,
niemals bereits entfernte UIDs. Die Vermerke werden nicht zum Hub übertragen.

Bereitstellen schreibt zuerst die Ablagedatei samt Original, dann den Link in `stimmen.json`.
Bei Abbruch vor dem Link findet der Neustart dieselbe deterministische Dateikennung, ergänzt
gegebenenfalls das Original und stellt den Link fertig, ohne weitere Datei, Fassung oder Zeitänderung.
Eine seit der Erstsync-Zusammenführung lokal geänderte Verliererkopie bekommt vor dem Umbinden eine
neue uid, damit ihre Änderungen erhalten bleiben. Dies ist keine Auswertung von Grabsteinen.

Für Namenswechsel und mehrere gleichzeitige Importe werden zuerst alle Zielbytes atomar unter
`.sync-<uid>.safetensors` bereitgestellt. Danach hält `sync_receive` den vollständigen Zielindex,
die zu installierenden Datensätze und die alten Namen dauerhaft fest. Erst dann werden Zielnamen und
Masterdateien geschrieben; nur nicht mehr verwendete alte eigene Namen werden entfernt. Pflichtteil
sind ausschließlich `eigene-stimmen/` und `stimmen.json`. Der fertige Index entfernt `sync_receive`
atomar und merkt optionale Modellarbeiten als `sync_models_open`, alte Modellnamen samt bekanntem
SHA in `sync_model_removals`; danach können die Zwischenkopien verschwinden. Modellkopien und ihre
`SOURCE.json` sind optional: Fehler lassen den Pflichtabschluss, Lesen, Import und lokales Entfernen
weiterlaufen, bleiben als offen vermerkt und werden ohne Namen, Pfade oder Inhalte protokolliert.
Der vorhandene Abgleich beim Modellstart/-Installieren (`VoiceStore::sync`) sowie erneutes Lesen
versuchen die Arbeiten wieder; sie verwenden den aktuellen lokalen Bestand, damit veraltete
Modellaufträge keine entfernten Stimmen wieder anlegen. Fremde Modellbytes bleiben geschützt.
Ein Neustart versucht das Pflichtjournal vor Änderungen am Index abzuschließen. Gelingt das noch
nicht, liefert `list()` trotzdem den bestmöglichen nutzbaren lokalen Bestand: bereits geschriebene
Zielbytes mit passendem SHA, danach noch nutzbare alte Einträge, ohne doppelte Namen, plus eine
inhaltfreie Fehlermeldung im Log. Das offene Pflichtjournal bleibt erhalten.
Ein Bytewechsel zwischen Datei und Index
wird anhand des bekannten entfernten SHA fertiggestellt. Nach endlich vielen fairen Abgleichen
bleiben Hub, Blobs und Index unverändert. Es gibt keine ausgelieferten Daten des verworfenen
Transportobjekt-Modells und deshalb keine Migration oder parallel aktive Altlogik.

**Abschirmung.** `Ablage::lage` und `lage_intern` liefern den vollständigen internen Bestand;
Lesen verändert weder Baum noch Journale. Die Filterung geschieht in den jeweiligen Benutzersichten
und Zugriffswegen: Liste, Volltext-/Semantik-Quellen, KI-Bestand, Export des Originals, Papierkorb
und direkte Änderungsbefehle blenden Systemkennungen und deren Unterbäume aus beziehungsweise
weisen ihren Zugriff ab. Neue Leser müssen diese Filter ausdrücklich übernehmen. Beim Sichern
bleiben die Systemdatensätze erhalten. Verwaiste Systemdateien bleiben an ihrer ID erkennbar und werden auch nach Journalverlust nicht
als Dokumentenfunde angeboten. Dokumentzähler und lokale Dokument-Speicheranzeige schließen sie aus;
das Hub-Kontingent zählt ihre verschlüsselten Blob-Bytes weiterhin. Sync-Snapshots und Wiederanlauf
verwenden den vollständigen internen Stand. Stimmbytes und Stimmnamen gelangen niemals in Logs.

**Prüfstand.** `sync::ablage::tests::phonon_tests` verwendet ausschließlich synthetische Stimmen und
den vorhandenen Hub-Nachbau: feste Saaten `7`, `29`, `898`, zwei und drei Geräte, zufällige
Verschränkung und abschließende faire Ruhephase. Geprüft werden unabhängige gleichnamige Importe,
Löschen jeder einzelnen uid, Neuanlage, paralleler Namens-/Bytewechsel, rekursive alte Clients
einschließlich Dateigräbern, Wiederanlauf vor/nach Dateischritten, Fassungsaufbewahrung und
unveränderte fremde Dokumente. Ruhe bedeutet keine weiteren Hub-, Blob- oder Index-Schreibvorgänge.
Die bestehenden sonstigen Ablage-Tests bleiben unverändert. Der verworfene Test
`benutzerlesen_beendet_system_loeschjournal_vor_der_projektion` ist entfernt.
Rust-Ausführung einschließlich Rot-Gegenproben erfolgt im anschließenden Thread-Claude-Lauf.
`python apps/local-voice/scripts/test_phonon_sync_spec.py` prüft den Sichtvertrag mit Gegenprobe
gegen `c4da4e85`, die drei iOS-Kennungsvektoren und LF in den geänderten Dateien ohne Installation.
Zusätzliche Regressionen in `phonon_regression_tests.rs`: alle sechs Reihenfolgen eines alten
Ordnergrabs mit zwei Dateigräbern, auch zwischen den Sammlungs-Pulls; Foreign-Konfliktname
einschließlich `-eigen-2` und 40-Zeichen-Grenze; Wiederanlauf nach Ablagedatei vor Original bzw.
vor Link; fehlender Link und lokale Entfernung mit zwei/drei Geräten und Saaten `29`/`898`;
später eintreffende hier entfernte UID und übergroße Transportantwort.
Review-Regressionsfälle: defekte Modell-`SOURCE.json` beim Empfang, weiterer lokaler Import und
Entfernen, anschließendes Heilen über den Modellabgleich; nutzbare Liste bei blockiertem
Pflichtjournal; Entfernen einer Erstsync-Dublette vor Empfang bei beiden UID-Reihenfolgen,
spätere Neuanlage/Änderung anhand jeweils `created` oder `updated` und bewusste lokale Neuanlage.
Gegenproben auf `548fe0ec`: Modellfehler wieder aus `finish_receive` durchreichen →
`defekte_optionale_modellkopie_blockiert_keinen_pflichtabschluss` rot; Fehler beim Journalabschluss
wieder als leere Liste behandeln → `empfangsjournal_heilt_abbruch_vor_und_nach_einer_datei` rot;
Name+SHA-Vermerk vor Kandidatenaufnahme auslassen →
`entfernte_erstsync_dublette_bleibt_lokal_entfernt_aber_neuanlagen_kommen_an` rot.
Jeweils die `created`- oder `updated`-Schranke aus `RemovedInitial::matches` entfernen → derselbe
Test rot. Diese Rust-Gegenproben sind durch Thread-Claude auszuführen; die Python-Vertragsprüfung
prüft nur die strukturellen Voraussetzungen mit dem alten HEAD als Gegenprobe.
Die neue Testgruppe soll nach dem Build zusammen höchstens etwa 20 Sekunden benötigen;
diese Laufzeit ist vor dem Rust-Lauf noch nicht gemessen.

Gegenproben für S1a: alte Löschweitergabe einsetzen → lokale Entfernung und Zufallsmodell rot;
Dateigräber wieder als Löschwunsch auswerten oder Wiederbereitstellung überspringen →
`alte_clients_begraben_ordner_und_dateien_in_jeder_reihenfolge_ohne_stimmverlust` rot;
UID-Merkliste entfernen → später Import nach Entfernen ohne Link rot; Alias-Merkliste
entfernen oder Verliererdatei begraben → Erstsync-Test rot; numerischen Foreign-Suffix
verwenden → Foreign-Tests rot; Wiederanlaufzweig entfernen →
`bereitstellen_heilt_absturz_vor_original_und_vor_link_ohne_neues_schreiben` rot.
Weitere bestehende Gegenproben: updated in Dateikennung → Fassungstest rot; Namens-LWW statt
uid-Projektion → unabhängige Importtests rot; Namensregister entfernen → paralleler Namens-/Bytewechsel
rot; uid-Staging/Write-ahead entfernen → Empfangsjournaltest rot; System-Aufbewahrung über
Dokumentfilter führen → Aufbewahrungstest rot; `Stückantwort zu groß` nicht verwerfen →
Transportantworttest rot. Die Offline-Vertragsprüfung prüft zusätzlich das Entfernen der
Löschlogik gegen HEAD `972b5e13`.

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
