# Übergabe Stufe D5 (iOS): Dokumentenablage in Local Voice

An: Mac/iOS-Thread. Von: Planer-Thread (nur gelesen, nichts gebaut). Stand: `origin/main` @ `281b6f586` (D6 #434), 09.11.2026.

Quellen: `docs/superpowers/specs/2026-11-08-dokumentenablage.md` (Spec, Fassung 5; §6 „iPhone (D5)“, §9 „D5“), Rust-Client unter
`apps/local-voice/src-tauri/src/` und der bestehende iOS-Code unter `apps/apple/`. Alle Pfade unten sind relativ zu
`apps/local-voice/src-tauri/src/` (kurz `src/`), sofern nicht `apps/apple/…` davorsteht. Zeilennummern gelten für den genannten Stand.
Der Hub (wai-portal) selbst wurde nicht gelesen; seine Regeln stehen hier so, wie der Rust-Client und der Hub-Nachbau in
`src/sync/ablage_tests.rs` sie voraussetzen. Wo das wichtig ist, steht es unter „Offene Fragen“.

## 0. Die wichtigsten Fallen vorweg

1. **`blob_refs` gibt es zweimal.** In der Nutzlast (`ablage_datei.blob_refs`) stehen die **Inhalts-Hashes** (SHA-256 des Klartexts, hex).
   Im sichtbaren Steuerfeld des Push/Pull und in der AAD stehen die **Hub-Kennungen** (`SHA256(key ‖ "lv-blob-v1" ‖ sha)`), sortiert, eindeutig.
2. **Grabsteine der Ablage sind versiegelt** (deterministische Nonce, AAD mit `deleted=1`). Ein Grabstein mit leerer Nutzlast, wie ihn
   `LibrarySync.deleteEverywhere` für Bücher sendet (`apps/apple/iPhone/Library/LibrarySync.swift`, `payload: ""`), gilt für `ablage_*` am Desktop als
   „ohne Siegel“, wird **ignoriert** und hält das Objekt an. Nie so senden.
3. **Null nicht entfernen.** `LibrarySyncFormat.canonicalHash` (`apps/apple/VoiceCore/Sources/VoiceCore/LibrarySyncModel.swift`) streicht `null`-Felder.
   Der Desktop schreibt `null` (z. B. `vorgaenger`, `eltern.wert`, `papierkorb.wert`). Für `ablage_*` Nutzlast und Hash ohne Streichen bauen.
4. **Zeichenketten bytegenau vergleichen.** Rust ordnet `String` nach UTF-8-Bytes. Swift-`String <` vergleicht anders. Überall, wo die Ordnung das Ergebnis bestimmt
   (Register `geraet`, `fas`, `textrev`, Sortierung von Listen, `blob_refs`), über `Array(s.utf8).lexicographicallyPrecedes` o. ä. vergleichen.
5. **Unbekannte Felder erhalten** (Ordner und Datei: `#[serde(flatten)] extra`), und ein Schema `v` > 1 hält das Objekt an (nicht überschreiben).
6. **`register.geraet` ist nicht die Hub-`device_id`.** Es ist eine eigene Ablage-Gerätekennung `g_<ULID>` (`src/ablage/speicher.rs:214`, geprüft in `:433`).
   Das iPhone braucht eine eigene, dauerhaft gespeicherte `g_<ULID>`.
7. **Am Desktop liegen `confidential` und `personal` nie im Hub** (`src/sync/ablage.rs:9-11`, `:282-288`). Das iPhone sieht also nur `public`/`internal`.
   Trotzdem: Zustände, die nicht `Sync` sind, nie indexieren und nie an ein Modell geben (siehe §4).
8. **Das bestehende iOS-Sync-Modell kennt die Ablage nicht:** `HubPushObject` und `HubRemoteObject` haben kein `blob_refs`, `HubPushResult` kein `missing`;
   es gibt kein `reserve`, kein bedingtes `delete`, keinen Pull je Sammlung, keine AAD mit `blob_refs`, kein deterministisches Siegel (Details in Anhang A).

## 1. Ziel D5 laut Spec

Spec §6 (Aufzählung „iPhone (D5)“) und §9:

- **Textfassung sofort:** Die Textfassungen (`text/<dat>/<textrev>.json`, Markenformat v1) werden bevorzugt und früh geladen, damit Suche und Assistent sofort arbeiten.
  Der Rust-Client lädt Texte vor Originalen (`src/sync/ablage.rs:884-908`, Sortierung `(0 = Text, 1 = Original)`).
- **Original bei Bedarf, „nur im WLAN“:** Originale (Vorschau, Öffnen, Teilen) werden erst auf Anforderung geladen und folgen der bestehenden Regel
  `LibrarySync.wifiOnly` (Vorgabe `true`, `bulk:`-Anfragen mit `allowsExpensiveNetworkAccess = false`, `apps/apple/iPhone/Library/LibrarySync.swift:59`, `:723-727`).
  Das ist eine bewusste Abweichung vom Desktop, der Originale immer lädt.
- **Eigene Importe zeigen „Text wird am PC erstellt“**, sind bis dahin nicht im Abruf (weder Suche noch Assistent). Das iPhone extrahiert nicht (§4).
  Der Zustand heißt am Desktop `WartetAufText` (`src/sync/ablage.rs:1704-1711`, `:1801-1837`), in der Liste „Text wird am PC erstellt“ (Spec §3).
- **Lexikalischer Index in VoiceCore:** Stücke wie am Desktop, BM25 wie `wai-retrieval`; „Index-Treffer gleich Desktop für Musterfragen“ (Spec §9 D5). Details §2.17.
- **Assistent-Bereich wie am Desktop:** Bereich „Ablage“ des Assistenten fragt den lokalen Index ab, nur über den Kontextvertrag und nur mit Freigabe (Spec §5, §11).
  Der Umfang auf dem iPhone ist offen (Frage 5).
- Abnahme laut Spec §9: „Swift-Testvektor gegen Rust (Blob-Kennung, AAD, `textrev`); Index-Treffer gleich Desktop für Musterfragen.“

## 2. Genaue Datenformate (aus dem Rust-Code)

### 2.1 Sammlungen, Objekt-Hülle, Pull und Push

- Sammlungen: `ablage_ordner`, `ablage_datei` (`src/sync/ablage.rs:51-52`). Eigener Pull je Sammlung mit eigenem Cursor, **Ordner vor Dateien**
  (`:650-711`, `HubClient::pull_collection`, `src/sync/client.rs:513-535`: `GET /api/v1/voice/sync/pull?since=&limit=&collection=`, Seite 200, `PULL_LIMIT` 200).
- Push-Objekt (`src/sync/client.rs:27-39`): `collection`, `object_id`, `base_revision`, `payload` (base64), `key_id`, `deleted`, und **nur bei `ablage_datei` ohne Grabstein**
  `blob_refs` (Hub-Kennungen). Bei allen anderen Sammlungen darf das Feld nicht gesendet werden (`:35-37`, Nachbau: `blob_refs_not_allowed`).
  `POST /api/v1/voice/sync/push` mit `{device_id, objects[]}` (`:316-336`), Stapel am Desktop 40 (`PUSH_STAPEL`, `ablage.rs:58`).
- Pull-Objekt (`:41-59`): wie oben, dazu `revision`, `device_id`, `server_seq` und `blob_refs` (Hub-Sicht) bei `ablage_datei`.
- Push-Ergebnis (`:62-77`): `outcome` ∈ `accepted | superseded | rejected`, `revision`, `current` (bei `superseded`), `reason`, `missing[]` (bei `blob_missing`).
- Fehlerform des Hubs: `{"error":{"code","message"}}`; 401 = abgemeldet, 410 = Cursor abgelaufen (Pull von 0 neu, genau einmal, `ablage.rs:665-669`),
  sonst `code: message` (`src/sync/client.rs:247-273`).
- **Objektstand je Gerät** (`ObjStand`, `src/sync/ablage.rs:153-166`): `revision`, `hash` (Hash des zuletzt abgeglichenen Standes), `deleted`, `refs`
  (Hub-Kennungen, die der Hub für dieses Objekt führt), `angehalten`. Dazu `cursor` je Sammlung, `rueck` (Rückstellung: 1 min · 2^Stufe, höchstens 6 h, `:66-72`),
  `zu_viele`, `hochgeladen` (Menge abgeschlossener Blobs), `blob_loeschen`, `verbrauch`, `kontingent_voll` (`Zustand`, `:199-233`).
  Der Zustand ist an das Konto gebunden (`konto_kennung = "<user_id>:<key_id>:<hub_url ohne /am Ende>"`, `:1841-1848`); anderes Konto = leerer Zustand (`:581-589`).
  Das iPhone braucht dafür ein eigenes Ledger neben `SyncLedger` (z. B. `AblageLedger`).

### 2.2 `ablage_ordner` (Nutzlast)

Struct `Ordner`, `src/ablage/zusammenfuehren.rs:151-163`; Spec §2.2. JSON-Felder (Rust-Namen = Serde-Namen, keine Umbenennungen):

| Feld | Typ | Bemerkung |
|---|---|---|
| `id` | String | `ord_` + ULID (26 Zeichen, gültig laut `ablage::id_gueltig`, `src/ablage/mod.rs:98-101`) |
| `eltern` | Register<String?> | `wert` = Elternordner oder `null` (Wurzel) |
| `name` | Register<String> | |
| `datenklasse` | Register<String> | `erbt | public | internal | confidential | personal`; unbekannter Wert = Klasse „unbekannt“ (wie gesperrt) |
| `nur_geraet` | Register<Bool> | |
| `zielordner_ki` | Register<Bool> | |
| `papierkorb` | Register<Int64?> | `wert` = Zeitstempel ms oder `null` |
| (flatten) | beliebig | alle weiteren Schlüssel der Objektwurzel (`extra`), bleiben erhalten und reisen mit |

Ein Register ist `{"wert": …, "zaehler": u64, "geraet": "g_…"}` (`zusammenfuehren.rs:22-27`). Das höhere `(zaehler, geraet)` gewinnt (`:60-66`, Ordnung `:30-36`:
erst `(zaehler, geraet)`, bei Gleichstand der Wert).
Neuer Ordner (Referenz fürs Anlegen): `src/ablage/speicher.rs:2968-2979` (`datenklasse` = `erbt`, `nur_geraet`/`zielordner_ki` = `false`, `papierkorb` = `null`, alle Zähler `z`).
Beim Empfang prüft der Client: `o.id == object_id` und **`blob_refs` leer** (`src/sync/ablage.rs:839-845`), sonst unlesbar.

### 2.3 `ablage_datei` (Nutzlast)

Struct `Datei`, `zusammenfuehren.rs:128-148`; Spec §2.1. Felder:

| Feld | Typ | Default beim Lesen | Bemerkung |
|---|---|---|---|
| `id` | String | – | `dat_` + ULID |
| `ordner` | Register<String> | – | |
| `name` | Register<String> | – | |
| `klasse_min` | Register<String> | – | `erbt` oder Klassenwert; neue Dateien am Desktop: `"public"` (`speicher.rs:2490`, `:3124`) |
| `nur_geraet_min` | Bool | `false` | |
| `fassungen` | [Fassung] | `[]` | |
| `wahlen` | [Wahl] | `[]` | |
| `entfernt` | [String] | `[]` | Fassungs-IDs, wachsen nur |
| `blob_refs` | [String] | `[]` | **Inhalts-Hashes** (hex) aller Originale und Texte, sortiert, eindeutig (`blob_refs_aus`, `zusammenfuehren.rs:184-194`); Texte ohne `blob` (Stand `fehler`) fehlen |
| `papierkorb` | Register<Int64?> | – | |
| (flatten) | beliebig | – | `extra`, bleibt erhalten |

`Fassung` (`:95-107`): `fas`, `vorgaenger` (String oder `null`), `blob` (hex, bei Fassungen vom Desktop = `original_sha256`, `speicher.rs:2437-2441`), `size` (u64),
`original_sha256`, `typ` (Endung ohne Punkt), `geraet`, `zeit` (i64, ms), `texte` (Standard `[]`).
`Text` (`:68-89`): `textrev`, `blob` (hex SHA-256 **der Textfassungs-JSON-Bytes**; leer bei `fehler`), `size`, `extraktor`, `extraktor_version`, `marken_version` (u32),
`stand` ∈ `ok | kein_text | fehler`, optional `code` (nur bei `fehler`, fehlt sonst), optional `versuche`, `versuche_zeit` (fehlen bei 0).
`Wahl` (`:109-119`): `id` (`wahl_…`), `gewaehlt`, `gegen` ([String], Standard `[]`), `ersetzt` ([String], Standard `[]`), `zaehler`, `geraet`.

Wichtig für Swift-Codable: `Option`-Felder des Desktops fehlen beim Lesen als `nil` gleichwertig zu `null`; beim Schreiben `null` ausgeben (Falle 3).
`Fassung`, `Text`, `Wahl` haben **kein** `flatten`: unbekannte Felder darin verwirft der Desktop beim Neuschreiben ohnehin. Erhalten müssen das iPhone nur Datei und Ordner.
Unterstützte Schema-Marke: `v` (optional, Standard 1); `v` > 1 = unlesbar, Objekt ruht (`src/sync/ablage.rs:65`, `:836-838`).

### 2.4 Kanonische Nutzlast und Hash

- Klartext der Nutzlast = `collect::canonical(wert)` (`src/sync/collect.rs:60-72`): alle Objektschlüssel rekursiv aufsteigend (Bytes), Arrays in gegebener Reihenfolge,
  kompaktes JSON ohne Leerzeichen, `serde_json`-Zeichenketten (Nicht-ASCII bleibt Klartext), `null` bleibt. Hash = SHA-256 (hex) dieses Textes (`:74-77`).
- Für `ablage_datei` wird vor dem Serialisieren `blob_refs` aus den Fassungen neu berechnet (`wert_datei`, `src/sync/ablage.rs:508-512`).
- **Reihenfolge der Listen** nach dem Zusammenführen (damit zwei Geräte denselben Hash erzeugen und nicht gegenseitig neu pushen):
  `fassungen` aufsteigend nach `fas`, `wahlen` nach `id`, `entfernt` sortiert und eindeutig, `texte` je Fassung nach `textrev`, `blob_refs` sortiert
  (`zusammenfuehren_datei`, `:220-259`; `fassung_vereinigen`, `:206-217`). Das iPhone muss genauso ausgeben.
- Zahlen: `zaehler`, `size`, `zeit` sind ganze Zahlen; `PassthroughJSON` (Double) trägt sie bis 2^53 verlustfrei, `Int64` ist sicherer.
- Größenlimit: Der Nachbau lehnt leere Nutzlast ab (`payload`), Grabstein-Siegel über 16 384 Bytes (`ablage_tests.rs:219`). `LibrarySyncFormat.maxPlaintext` (256 KiB)
  gilt am iPhone für Bibliotheksobjekte; die Ablage-Obergrenze ist offen (Frage 7).

### 2.5 AAD-Aufbau

`crypto::aad_ablage` (`src/sync/crypto.rs:64-82`), Bytes (UTF-8):

```
"{user_id}" 1F "{collection}" 1F "{object_id}" 1F "blob_refs" 1F "{blob_refs, mit Komma verbunden}" 1F "key_id" 1F "{key_id}" 1F "deleted" 1F "{0|1}"
```

(`1F` = U+001F.) Der Anfang ist `crypto::aad` (`:56-58`, bereits `SyncCrypto.aad` in Swift). `blob_refs` sind die **Hub-Kennungen**, sortiert, eindeutig; für `ablage_ordner` und Grabsteine leer
(Verbindung ergibt einen leeren Text zwischen den Trennern). `key_id` = erste 4 Byte von SHA-256(Schlüssel), hex (`:51-54`, Swift `SyncCrypto.keyId`). `deleted` = `1` nur für Grabsteine.
Versiegelung sonst unverändert: XChaCha20-Poly1305, Ausgabe `base64(nonce ‖ chiffrat ‖ tag)` mit zufälliger 24-Byte-Nonce (`:106-133`; Swift `SyncCrypto.seal/open`).

Beim **Öffnen** eines Pull-Objekts (`src/sync/ablage.rs:826-854`): AAD mit den vom Hub gelieferten `blob_refs` und der eigenen `key_id`, `deleted=0`; danach prüfen:
`key_id` des Objekts = eigene (sonst „fremder Schlüssel“, Objekt ruht, Pull geht weiter, Lauf gilt als nicht vollständig, `:778-795`); Schema; `id` = `object_id`; bei Datei
`hub_refs(d) == blob_refs` (Ableitung aus der Nutzlast, Listenvergleich in Reihenfolge); bei Ordner `blob_refs` leer. Jede Abweichung = unlesbar, Objekt ruht.
Der Test dazu: `src/sync/ablage_tests.rs:649` (`blob_refs_sind_an_die_aad_gebunden`).

### 2.6 Blob-Kennung `SHA256(key ‖ label ‖ sha)` und Stücke

- `blobs::blob_id` (`src/sync/blobs.rs:61-67`): `hex(SHA-256(key[32] ‖ "lv-blob-v1" ‖ sha[32 Rohbytes]))`, klein geschrieben. In Swift bereits `LibrarySyncFormat.blobId(key:plaintextSha256:)`.
- Ablage: `hub_blob(key, sha256_hex)` (`src/sync/ablage.rs:465-468`) wandelt den Hex-Hash der Nutzlast (`Fassung.blob`, `Text.blob`) in Rohbytes (64 Hex-Zeichen, sonst `None`) und nimmt `blob_id`.
  Für Originale ist `sha` der SHA-256 der Originaldatei, für Texte der SHA-256 der **Bytes der Textfassungs-Datei** (`speicher.rs:2716-2727`).
- `blob_quellen` / `hub_refs` (`ablage.rs:477-501`): alle Originale und alle Texte mit nicht leerem `blob`, in `BTreeMap<String,_>` nach Hub-Kennung (also nach Hex-Text sortiert), eindeutig.
- Stücke wie bei der Bibliothek: 3 MiB Klartext, `ONE_CHUNK_MAX` 3 827 302, `MAX_CHUNKS` 4096 (`blobs.rs:24-35`, `:69-75`), AAD `"{user}" 1F "blob" 1F "{blob_id}" 1F "{index}"` (`:77-79`),
  roh `nonce ‖ chiffrat ‖ tag`; Transport als `{data: base64, sha256: hex(sealed)}` (`client.rs:420-447`, `:485-510`). Download prüft jedes Stück, die Gesamtgröße **und** die Blob-Kennung gegen den Klartext-Hash
  (`blobs.rs:366-423`); das gibt es in Swift schon (`downloadBlob`/`uploadFile` in `LibrarySync.swift`).
- Kontoweiter Standardordner „Allgemein“: `standard_id` = `"ord_" + ULID(SHA-256(key ‖ "lv-ablage-allgemein")[0..16])` (`ablage.rs:389-404`; ULID aus den 16 Bytes, 26 Zeichen Crockford-Base32). Ohne diese Ableitung legt das iPhone einen zweiten „Allgemein“ an.

### 2.7 Versiegelte Grabsteine

`grab_klartext`, `grab_nutzlast`, `grab_echt` (`src/sync/ablage.rs:406-452`), `seal_deterministisch` (`src/sync/crypto.rs:84-103`):

- Klartext = kanonisches JSON `{"collection":"<coll>","deleted":true,"id":"<id>"}` (Schlüssel in dieser Reihenfolge, kompakt).
- AAD = `aad_ablage(user_id, coll, id, [], key_id, deleted = true)`.
- Nonce = die ersten 24 Bytes von `SHA-256("lv-ablage-siegel-nonce" ‖ 0x1F ‖ key ‖ u64_big_endian(aad.len) ‖ aad ‖ klartext)`. Gleiche Eingabe ergibt dasselbe Chiffrat; so nimmt der Hub eine
  Wiederholung nach verlorener Antwort als `accepted` an (Nachbau: gleiche Nutzlast **und** gleiche `key_id`, `ablage_tests.rs:228-236`).
- Push: `deleted: true`, `payload` = `base64(nonce ‖ chiffrat ‖ tag)`, `key_id`, **ohne** `blob_refs`, `base_revision` = bekannte Revision. Siegel höchstens 16 384 Bytes (`ablage_tests.rs:219`, Spec-Fußnote wai-portal#23).
- Empfang: nur gültig, wenn `key_id` = eigene, Öffnen mit AAD (`deleted=1`, leere Refs) gelingt und das JSON `id`, `collection`, `deleted == true` bestätigt (`:432-452`). Sonst: **nicht anwenden**,
  Objektstand `angehalten`, Warnung „Grabstein ohne gültiges Siegel“ (`:755-776`). Ordnergräber werden erst nach dem Datei-Pull angewendet (`:712-728`, sonst hielte eine gleichzeitig begrabene Datei den Ordner).
- Wirkung am Desktop: Original und Texte → Papierkorb (30 Tage), Index weg, Kennung nie wieder belebt (Spec §6). iPhone: Original, Text, Index, Miniaturen, Caches entfernen, Kennung merken und nie wieder aufnehmen.
- Test: `ablage_tests.rs:1302` (`grabstein_wiederholung_traegt_dasselbe_siegel`), `:1236` (gefälschter Grabstein löscht nichts).

### 2.8 `textrev` und Textfassung (Markenformat v1)

- `textrev` = `hex(SHA-256(original_sha256_hex_als_ASCII ‖ 0x00 ‖ extraktor ‖ 0x00 ‖ extraktor_version ‖ 0x00 ‖ dezimal(marken_version)))`
  (`src/ablage/extrakt/mod.rs:114-124`; `MARKEN_VERSION` = 1, `:24`). Eingabe ist der **Hex-Text** des Original-Hashes, nicht die Rohbytes.
- Erwartete `textrev` einer Fassung = `textrev(f.original_sha256, extraktor(format(f.typ)), MARKEN_VERSION)` (`src/ablage/speicher.rs:2637-2646`). Format aus `typ` (`extrakt/mod.rs:86-99`),
  Extraktor und Version je Format in `:103-112` (Stand: `pdf-extract` `0.12.0+lva1`, `lva-office` `1`, `calamine` `0.32.0+lva1`, `lva-text` `1`, `lva-html` `1`, `lva-mail` `1`). Die Tabelle ändert sich mit jeder Extraktorversion:
  **das iPhone darf sie nicht festschreiben**, nur im Testvektor als Eingabe verwenden. „Text fertig“ am iPhone = irgendein `texte`-Eintrag mit `stand` = `ok` (und lokal geladen) oder `kein_text`,
  unabhängig von der Version (Desktop genauso, `ablage.rs:1818-1824`; ältere Textfassungen gelten bis zum Neuaufbau, `speicher.rs:2836-2842`).
- Datei `text/<dat>/<textrev>.json` (`speicher.rs:2661-2666`, `:2716-2727`) = `serde_json` von `Textfassung` (`extrakt/mod.rs:50-57`):
  `{"marken_version":1,"textrev":"…","original_sha256":"…","extraktor":"…","extraktor_version":"…","abschnitte":[{"anker":"…","pfad":["…"],"text":"…"}]}`
  (`pfad` fehlt, wenn leer, `:42-48`). Anker je Format (Spec §4.2): PDF `seite=<n>`, DOCX/ODT `absatz=<n>` + `pfad`, PPTX/ODP `folie=<n>`, XLSX/ODS `blatt=<name>&zeilen=<a>-<b>`, EML `teil=kopf|text|<n>`, TXT/MD/CSV `zeile=<n>`.
- Prüfung beim Einlegen (`src/ablage/speicher_sync.rs:459-489`): SHA-256 der heruntergeladenen Bytes = `Text.blob`, `kopf.textrev` = Eintrag, `kopf.original_sha256` = `Fassung.original_sha256`; sonst verwerfen. Empfehlung für Swift
  zusätzlich: `textrev` aus den Feldern des Eintrags neu berechnen und vergleichen (der Rust-Client prüft nur Gleichheit). Lesegrenze für Textdateien am Desktop 32 MiB (`index.rs:40`).
- Gewähltes Textfassung-Paar für Index und Anzeige: `index_quellen` (`speicher.rs:2821-2882`): aktuelle Fassung = einziger Kopf, sonst die jüngste nach `(zeit, fas)` (`aktuelle_fassung`, `:3023-3032`);
  Text = der mit erwarteter `textrev` (nicht `fehler`), sonst der letzte mit `stand = ok`. Achtung: Die Spec verlangt für die **KI** bei Konflikt die jüngste gemeinsame Vorgängerfassung (Spec §3, D3), der Such-Index nimmt die jüngste.

### 2.9 Lamport-Zähler und Zusammenführen

- Neuer Zähler = **größter bekannter Wert über alle Register + 1** (`speicher.rs:301-304`; `groesster_zaehler`, `zusammenfuehren.rs:634-660`:
  alle sechs Ordner-Register, `ordner`, `name`, `klasse_min`, `papierkorb` jeder Datei und `wahlen[].zaehler`). Nicht gezählt: `nur_geraet_min`, Fassungs-`zeit`.
- Register: höheres `(zaehler, geraet)` gewinnt; `nur_geraet_min` = ODER; `entfernt`, `wahlen`, `texte` nur vereinigen (`zusammenfuehren_datei`, `:220-259`). Ordner: `zusammenfuehren_ordner` `:262-273`.
  Fassungen werden nach `fas` vereinigt und die IDs in `entfernt` herausgenommen. Ein Text je `textrev`: der größere nach der abgeleiteten Ordnung (`:206-217`, Feldreihenfolge `Text` `:68-89`).
- Köpfe/Konflikt (`:295-392`): aktive Wahl = in keinem `ersetzt` genannt; abgelöst = F in `gegen` einer aktiven Wahl W, nicht gewählt von W, und keine aktive Wahl mit höherem Rang hat F gewählt; Kopf = lebende Fassung ohne Nachfolger
  (`vorgaenger`-Verweis) und nicht abgelöst, Ergebnis nach `fas` sortiert; `aktuell` = einziger Kopf, sonst `None` (nie per Zeit); `gemeinsame_vorgaengerin` `:371-382`; neue Wahl `neue_wahl` `:397-434`.
- Ordnerbaum (`:440-532`): wirksame Eltern ableiten (Ordner absteigend nach Rang des `eltern`-Registers, Kante nur ohne Zyklus; fehlender oder begrabener Eltern = „Wiederhergestellt“), nie die Register ändern. Klasse und `nur_geraet`
  laufen über die **strenge Kette** (wirksame Kette plus gespeicherte Elternkette unter „Wiederhergestellt“, `:507-543`); Klasse: Wurzel abwärts, `erbt` übernimmt, ein bekannter Wert nur gleich oder höher, Vorgabe `internal`,
  unbekannt ⇒ `Unbekannt` (`:584-621`).
- Papierkorb: Datei lebt, wenn `papierkorb.wert == null` und kein Ordner der wirksamen Kette im Korb (`speicher.rs:323-325`). Im Korb: nicht anzeigen, nicht indizieren.
- Aufbewahrung (`src/ablage/aufbewahrung.rs`, Spec §6): Fassung bleibt, wenn Kopf, von aktiver Wahl gewählt, jüngste gemeinsame Vorgängerin offener Köpfe, unter den 10 jüngsten (`JUENGSTE`) oder jünger als 90 Tage (`TAGE`) (`:17-18`, Pfadschluss `:7-9`).
  Der Desktop schreibt dafür `entfernt` (`ablage.rs:859-875`). Das iPhone **berechnet das nicht selbst** (nur `entfernt` befolgen: Original und Text der Fassung löschen, nicht anzeigen).
- Nach `sync_einspielen` vereinigt der Desktop frisch mit dem lokalen Stand und nimmt nie ein begrabenes Objekt wieder auf (`src/ablage/speicher_sync.rs:163-167`). Ein Ping-Pong-Risiko entsteht nur, wenn das iPhone anders sortiert oder `null` streicht (Fallen 3/4, §2.4).

### 2.10 Weiterleitung alter IDs

Beim Rückweg lokal → sync vergibt der Desktop dem Unterbaum neue Kennungen; Journal `ablage/uebergang-<ULID>.json` (`src/ablage/speicher_uebergang.rs:6-18`, Schritte `Begonnen → Umgezogen → … → Fertig`, Zuordnung alt → neu), alte Kennungen bleiben **nur gerätelokal**
als `weiterleitung` (`speicher.rs:189`, `speicher_uebergang.rs:204-221`, Kette höchstens 16 Schritte). Lebt eine alte Kennung am Hub, sendet der Desktop ihren Grabstein, Dateien vor Ordnern (`ablage.rs:1094-1115`).
Für das iPhone folgt: Es kennt alte Kennungen normalerweise nie (`nur_geraet`-Unterbäume werden nie übertragen). Relevant nur für Deep Links `wai://lva/ablage/<dat>?fas=<fas>&textrev=<rev>#<anker>` (Spec §5.3) und Projektverweise
(Spec §6, „Projektverweis zeigt danach auf die neue ID“): kommt ein Verweis auf eine unbekannte `dat_`-ID, ist das „Datei nicht verfügbar“, kein Fehler. Eine Weiterleitungstabelle auf dem iPhone ist nicht nötig, solange keine Projektverweise vom PC auf das iPhone synchronisiert werden (Frage 6).

### 2.11 Zustände, auch „angehalten“

- `ObjStand.angehalten` (`ablage.rs:162-165`): Der entfernte Stand ist hier unlesbar (Siegel, JSON, neueres Schema, fremder Schlüssel, Grabstein ohne Siegel, Datei≠`object_id`). **Kein Push, keine Blob-Löschung, bis ein lesbarer Stand eintrifft**
  (`:985-987`, `:1015-1018`, `:1054-1057`). Unlesbare Stände werden nie überschrieben und nicht in den Index aufgenommen.
- `Teilnahme` (`:274-288`): `Sync` (public/internal, nicht `nur_geraet`), `Lokal` (nur Gerät, `personal`, `confidential`), `Halten` (unbekannte Klasse oder unvollständige Kette: weder senden noch begraben; Downloads überspringen es, `:889`).
- Anzeigezustand `AblageSyncZustand` (`:1704-1711`): `synchronisiert`, `ausstehend`, `konflikt`, `wartet_auf_text`, `nur_geraet`, `angehalten`; Berechnung `zustaende` `:1734-1797`
  (Reihenfolge: angehalten zuerst; dann bei `Sync` Konflikt vor `WartetAufText` vor synchronisiert/ausstehend; `Halten` = angehalten; sonst `nur_geraet`).
- `wartet_auf_text` (`:1801-1837`): aktuelle Fassung (sonst jüngste) ohne brauchbaren Text und mit Format, das eine Textfassung hat; `fehler` zählt nicht als Warten; **ohne Original hier** oder bereits eingetragener Text, der noch geholt wird, ⇒ wartet.
  Für das iPhone vereinfacht: eigene Fassung mit `texte == []` und `typ` eines Textformats ⇒ „Text wird am PC erstellt“; alles andere ergibt sich aus `texte`.
- Download-Reihenfolge und Budget (`:879-974`): nur Objekte, die der Hub kennt und nicht begraben sind; Texte (`stand == ok`, `blob` nicht leer) zuerst, dann Originale, kleinere zuerst; Budget 256 MiB je Lauf, mindestens eine Datei; Original nur wenn `Fassung.size` zur Datei passt (`speicher_sync.rs:406-440`).
- Kontingent: `GET /api/v1/voice/blobs` → `{used, quota, blobs}` (`client.rs:81-88`, `:582-594`); `kontingent_voll = quota > 0 && used >= quota` (`ablage.rs:615`).

### 2.12 Hub-Endpunkte und Fehlercodes

Alle mit `Authorization: Bearer <token>`, keine Umleitungen folgen (`NoRedirects`).

| Zweck | Anfrage | Antwort / Fehler | Beleg |
|---|---|---|---|
| Pull je Sammlung | `GET /api/v1/voice/sync/pull?since&limit&collection` | `{objects, cursor, more}`; 410 = Cursor abgelaufen | `client.rs:513-535` |
| Push | `POST /api/v1/voice/sync/push` `{device_id, objects}` | `{results:[…]}` | `client.rs:316-336` |
| Blob-Stand | `GET /api/v1/voice/blobs/{id}` | `{complete, chunks:[{idx}]}`; 404 = unbekannt | `client.rs:402-418` |
| Stück laden/senden | `GET`/`POST …/chunks/{i}` `{data, sha256}` | | `client.rs:420-447`, `:485-510` |
| Abschließen | `POST …/commit` `{chunks}` | 200 oder 409 mit `outcome` `complete | incomplete` + `missing[]` | `client.rs:449-482` |
| **Reservieren** | `POST …/reserve` `{device_id}` | 200; Reservierung 24 h je Gerät, vor Upload **und** vor Wiederverwendung eines vorhandenen Blobs | `client.rs:537-547`, Spec §6 |
| **Bedingt löschen** | `POST …/delete` `{device_id}` | `{deleted}`; **409** `referenced` (noch von einem lebenden `ablage_datei` referenziert) oder `reserved` (von anderem Gerät reserviert) = Ergebnis, kein Fehler | `client.rs:549-579`, `ablage_tests.rs:146-158` |
| Kontingent | `GET /api/v1/voice/blobs` | `{used, quota, blobs}` | `client.rs:582-594` |

Push-Ergebnisse und Codes (`src/sync/ablage.rs:1408-1560`):

- `accepted` → Ledger `{revision, hash, deleted, refs}` übernehmen; frühere `refs`, die das neue Objekt nicht mehr nennt (oder alle bei Grabstein), werden Löschkandidaten.
- `superseded` + `current` → entfernten Stand **zusammenführen (nie überschreiben)** und mit neuer Basis erneut pushen; `current.deleted` → Grabstein nur mit gültigem Siegel anwenden; `current == None` → Revision verwerfen, mit Basis 0 neu.
  Höchstens 4 Runden je Lauf (`MAX_RUNDEN`, `:63`).
- `rejected` + `reason`:
  - `blob_missing` (+ `missing[]`): Reservierung abgelaufen oder Blob nie abgeschlossen → diese Blobs aus `hochgeladen` streichen, neu hochladen (reservieren, Stücke, commit), erneut pushen. Spec nennt dies „422“ (§6).
  - `tombstoned`: Hub hat einen Grabstein, den dieser Lauf nicht kennt → nichts lokal anwenden, Objektstand `revision: 0, deleted: true`; der nächste Pull liest und prüft den Grabstein.
  - alle übrigen (Nachbau: `collection`, `payload_size`, `blob_refs_not_allowed`, `blob_refs`, `payload`): Rückstellung mit wachsendem Abstand (`Rueck`), Warnung „vom Hub abgewiesen“; ohne Grund `ohne Grund`.
- Hub-Fehler beim Hochladen: `too_many_blobs` (Hub-Status 507, Anzahl der Blobs; `ablage.rs:1628-1633`, Test `ablage_tests.rs:1628-1635`) ⇒ Hochladen ruht (`zu_viele`, Rückstellung), unabhängig von `quota_exceeded` (Speicher voll) ⇒ `kontingent_voll`.
  **Nur der Fehlercode vor `": "` zählt**, nie der Meldungstext (`ist_zu_viele`, `:1631-1633`).
- Kontingent-Vorprüfung vor jedem Upload: `used + size > quota` ⇒ erst per `blob_info` prüfen, ob der Blob schon komplett am Hub liegt (dann kein neuer Speicher nötig), sonst „Speicher voll, Hochladen angehalten“ (`ablage.rs:1218-1236`). Spec §6: „Hochladen stoppt sichtbar bei Überschreitung“.
- Reihenfolge je Datei beim Senden (`blobs_bereitstellen`, `:1160-1296`): für jede Hub-Kennung in `blob_quellen`, die das Objekt nicht schon referenziert und die nicht in `hochgeladen` steht: erst frisch prüfen „noch synchron?“ → **reservieren** → Datei lokal vorhanden? sonst `blob_info` (vollständig ⇒ ok, sonst wartet das Objekt) → Kontingent → `upload` (Stücke, commit) → nochmals „noch synchron?“ (ändert sich das während des Uploads, Blob bedingt löschen) → `hochgeladen`.
  Erst danach das Objekt pushen (Stapel). **Ein iPhone, das nur Texte lädt, muss für nicht lokale Originale genauso `reserve` + `blob_info` ausführen**, sonst lehnt der Hub den Push wegen fehlendem Blob ab oder löscht ihn unter dem Objekt weg.
- Bedingte Blob-Löschung (`:1583-1625`) nur nach vollständigem Pull und nur für Kandidaten, die in keinem Bestand mehr lebend sind (`lebende_blobs`, `:1566-1578`). Das iPhone sollte in D5 **keine** Blobs löschen (§4, Frage 4).

### 2.13 Nie übertragen

Unterbäume mit `nur_geraet`, Klasse `personal` und `confidential`, `freigabe.json`, `verknuepft.json`, `index.db`, der Sync-Zustand (`sync.json`) (`src/sync/ablage.rs:9-11`; Spec §6, §2). Die Freigaben der KI sind gerätelokal (Spec §5.1): das iPhone führt **eigene** Freigaben, Vorgabe überall `aus`.

### 2.14 Lexikalischer Index (Paritätsvorgaben)

Referenz ist `src/ablage/index.rs` über das gevendorte `wai-retrieval` (`vendor/wai-retrieval/src/lib.rs`):

- Stücke je Abschnitt: höchstens `STUECK` = 220 Zeichen, `UEBERLAPPUNG` = 40 (`index.rs:35-36`), Funktion `stuecke` (`:128-185`: Schnitt möglichst an Leerraum in der zweiten Hälfte des Fensters, nächster Start 40 Zeichen vor dem Ende am Wortanfang; Zeichen = Unicode-Skalare, nicht Bytes). Stück ↔ Abschnitt/Anker/Pfad über Begleittabelle (`ablage_stueck`).
- Ranking: SQLite FTS5 `tokenize='unicode61 remove_diacritics 2'` (`lib.rs:430`), Anfrage = `content_tokens` (alphanumerische Wörter klein, Stoppwörter `STOP` in `lib.rs:307-312` entfernt), jeder Token in Anführungszeichen, mit `OR` verbunden;
  Sortierung `bm25(chunks_fts)`, dann `c.id` (`lib.rs:1250-1270`); Kandidaten 60 (`KANDIDATEN`), höchstens 3 Stellen je Datei (`STELLEN_JE_DATEI`), Anfrage auf 200 Zeichen gekürzt (`index.rs:738-760`); mit Vektoren RRF (`RRF_K` 60, `lib.rs:25`; D6 ist **nicht** Teil von D5).
  Papierkorb und alles ohne Leser fällt in SQL vor dem Ranking heraus.
- Ein zweiter, vorhandener BM25-Baustein in Swift (`BookTalkSearch.swift`, `BookTalkTokens`) hat **andere** Tokenisierung (Umlautfaltung, Endungskürzung, andere Stoppwörter) und passt nicht für Parität. Entweder FTS5 des System-SQLite (zu prüfen, ob iOS-SQLite FTS5 mit `unicode61 remove_diacritics 2` in allen Zielversionen mitbringt) oder eigener BM25 mit identischer Tokenisierung; Entscheidung siehe Frage 3.
- Filter vor dem Ranking: nur lebende Dateien, Zustand `Sync`, Text vorhanden, Freigabe des Ordners, Klasse (Spec §4.3, §5). Stempel je Datei zum Abgleichen (`index.rs:187-203`).

## 3. Testvektoren, die der Mac-Thread gegen Rust prüfen soll

Keine Zahlenwerte in dieser Übergabe: Die Zahlen kommen aus dem Rust-Code und werden dort ausgegeben. Eingaben wie die vorhandenen: Schlüssel = Bytes `00..1f`, Benutzer `7`, Nonce = Bytes `40..57` (wie `testvektor_fuer_swift`).

**Schon vorhanden (Blob-Kennung, Stück):**
- Rust: `src/sync/blobs.rs:482-497` `testvektor_fuer_swift` mit festen Konstanten `BLOB_ID_KAT` und `SEALED_KAT` (Eingabe „Hallo Buch“). Ausgabe: `cd apps/local-voice/src-tauri && cargo test --lib sync::blobs::tests::testvektor_fuer_swift -- --nocapture` (der Test druckt `blob_id=` und `sealed=`).
- Swift: `DesktopVectorTests` in `apps/apple/VoiceCore/Tests/VoiceCoreTests/LibrarySyncModelTests.swift:85-` prüft dieselben Werte (`swift test --package-path apps/apple/VoiceCore`).
- Zusätzlich für die Ablage fehlt nur der Weg **Hex-Hash → Rohbytes → Kennung** (`hub_blob`, `ablage.rs:465-468`): ein Swift-Test, der aus `sha256_hex` die Kennung bildet und mit `blobId(key:plaintextSha256:)` aus Rohbytes vergleicht.

**Noch nicht vorhanden (Rust-Seite muss die Vektoren erst ausgeben; kleiner Test nach Muster von `blobs.rs:482`, Code ändert nur Tests):**
1. **AAD** (`crypto::aad_ablage`, `crypto.rs:64-82`): Ausgabe der Bytes (hex) für drei Fälle: Datei mit zwei `blob_refs` und `deleted=false`, Ordner (leere Refs, `deleted=false`), Grabstein (leere Refs, `deleted=true`). Heute prüfen nur Eigenschaftstests die Bindung (`ablage_tests.rs:649`); Bytes sind nicht festgeschrieben.
2. **Grabstein-Siegel** (`grab_nutzlast`, `ablage.rs:417-429`): vollständige base64-Nutzlast für feste Eingaben (Schlüssel `00..1f`, Benutzer 7, Sammlung `ablage_datei`, feste `dat_`-ID). Prüft AAD, Klartext-Reihenfolge und die deterministische Nonce. Bezug: `ablage_tests.rs:1302`.
3. **`textrev`** (`extrakt::textrev`, `extrakt/mod.rs:114-124`): ein bis zwei Fälle mit festem `original_sha256`-Hex, Extraktor und Version (z. B. aus der Tabelle `extrakt/mod.rs:103-112`) und `marken_version = 1`. Heute nur Eigenschaftstest `src/ablage/tests_d2.rs:581` (`textrev_haengt_an_extraktorversion`); Verwendung in `tests_d2.rs:677`, `:862`, `:985`, `:1029`.
4. **Textfassung-Blob**: SHA-256 der JSON-Bytes einer festen kleinen Textfassung (Feldreihenfolge `marken_version, textrev, original_sha256, extraktor, extraktor_version, abschnitte`, `extrakt/mod.rs:50-57`) und die daraus abgeleitete Hub-Kennung. Sichert, dass Swift beim Lesen die Bytes nicht neu serialisiert und bei einem eigenen Text-Upload (falls je nötig) bytegleich schreibt.
5. **Kanonische Nutzlast** einer Beispiel-`ablage_datei` und eines `ablage_ordner` mit `null`-Feldern und einem unbekannten Zusatzfeld (`collect::canonical`, `src/sync/collect.rs:60-72`; `hash_datei`, `ablage.rs:518-520`): Text und Hash. Sichert Falle 3 und die Listenordnung (§2.4).
6. **`standard_id`** (`ablage.rs:391-404`) für den Schlüssel `00..1f`.
7. **Zusammenführen:** Die Permutations-Tests in `zusammenfuehren.rs` (Modul `tests`, ab `:662`; `unabhaengig` `:731`) sind die Quelle der Fälle. Für Swift als JSON-Fixture mit Eingabe-Ständen und Ergebnis ausgeben (Köpfe, abgelöst, wirksame Eltern, wirksame Klasse), kleine Matrix aus Spec §9 „D4a“ (Gegenwahl, W1/W2/W3, Ordner A→B/B→A/B→C).

Vorschlag für die Ablage der Vektoren: JSON unter `tooling/fixtures/ablage/` (Muster: `tooling/fixtures/buch-gespraech/relay/golden.json`, geteilt zwischen Rust und iPhone, vgl. `src/sync/booktalk_relay.rs:29-31`; Kopplung hat dasselbe Muster mit `docs/superpowers/specs/2026-11-05-geraete-kopplung-qr-vektoren.json`, `src/sync/pairing_proto.rs:256`). Rust und Swift lesen dieselbe Datei.
Hinweis: Argon2 ist teuer; für die Vektoren den festen Schlüssel `00..1f` verwenden, nicht `derive_key`.

## 4. Was am iPhone nicht passieren darf

1. **Nichts „nur dieses Gerät“ übertragen:** Weder Unterbäume mit `nur_geraet`, noch `personal`/`confidential` (Teilnahme `Lokal`) oder `Halten` (unbekannte Klasse/unvollständige Kette) senden, indexieren oder an ein Modell geben.
   Importiert das iPhone in einen Ordner, dessen wirksame Klasse oder `nur_geraet` das verbietet, bleibt der Import **lokal** (Anzeige „nur Gerät“), bis der Nutzer es ändert (Spec §6). Vor jedem Upload und vor jedem Push frisch prüfen („noch synchron?“, `ablage.rs:1182-1189`, `:1253-1267`).
   Der iPhone-Stand darf Klasse und `nur_geraet` **nie stillschweigend senken**: Klasse wird nur gleich oder höher gesetzt (Spec §2.2).
2. **Keine Extraktion auf dem iPhone** (kein PDFKit-/Vision-/Zip-Parsing in der App, kein OCR): Das ist der Sicherheitsrahmen (Spec §4.1: eigener Unterprozess `lva-extrakt` mit Grenzen). Eigene Importe bekommen keinen `texte`-Eintrag und stehen auf „Text wird am PC erstellt“; nicht im Index, nicht im Abruf.
   Ein Text-Eintrag mit `stand = fehler` ist ein Ergebnis des PC und wird nicht vom iPhone wiederholt.
3. **Fail-closed bei unlesbarem Stand:** Siegel, JSON, Schema > 1, fremde `key_id`, `blob_refs` ≠ Ableitung, Grabstein ohne Siegel ⇒ `angehalten`: nicht überschreiben, nicht pushen, nicht löschen, nicht indizieren, Datei nicht im Abruf. Ein Pull-Lauf mit fremdem Schlüssel gilt als nicht vollständig.
   Ein defekter lokaler Ablagezustand (Journal/Ledger/Freigaben unlesbar) sperrt die ganze Ablage auf beiden Wegen („Freigabe unklar“, Spec §5.1); Wiederherstellung nur aus gültigem `.vorher` (mit allen `gesperrt`) oder durch „Freigaben neu festlegen“ (alle `aus`).
4. **Keine Blob-Löschung am Hub in D5**, kein Papierkorb-Leeren, keine `entfernt`-Berechnung (Aufbewahrung), keine Grabsteine für `lokalisiert`/Rückweg (nur der PC). Bei Löschen im Papierkorb setzt das iPhone allenfalls `papierkorb` (Register) und überlässt endgültiges Löschen dem PC (Frage 4).
5. **Grabstein nur versiegelt:** nie `payload: ""` für `ablage_*` (Falle 2). Empfangene Grabsteine nur mit gültigem Siegel anwenden; Kennung nie wieder aufnehmen (auch nicht vom eigenen Stand).
6. **Kein Überschreiben fremder Arbeit:** Bei `superseded` zusammenführen und neu senden. Nie die Revision „weglassen“ oder `base_revision: 0` auf ein lebendes Objekt.
7. **Kein Log mit Inhalten oder Namen** (nur Kennungen und Codes, Spec §10, `ablage.rs:36`); keine Dateinamen in `status`/`error`-Texten, die in Logs landen können.
8. **Keine Originale eigenmächtig laden:** Nur auf Anforderung und mit „nur im WLAN“; Textfassungen sind klein genug zum Vorausladen (Spec: „Textfassung sofort“; Rust-Budget je Lauf 256 MiB).
9. **Kein KI-Zugriff ohne Freigabe** (Vorgabe `aus`), keine Cloud-Modelle für `confidential` ohne Ausnahme (Spec §5.2); `personal` bleibt lokal.
10. **IDs nur ULID-gültig** (`id_gueltig`): Objekte mit ungültiger `id` verwirft der Desktop (`ablage.rs:693-695`). Dasselbe auf dem iPhone beim Lesen; Namen nach P4 §4.4 (Windows-reservierte Namen, Groß/Klein-Kollision, `..`) beim Anlegen.

## 5. Vorgeschlagene Schritte

### D5a: lesender Teil (kein Push)

- `VoiceCore/Ablage/*`: Typen (`AblageOrdner`, `AblageDatei`, `Fassung`, `Text`, `Wahl`, `Register<T>`) mit **erhaltenem** `extra`; kanonische Serialisierung (§2.4); `aadAblage`, `blobIdFromHex`, `sealDeterministisch` (auch ohne Push zum Prüfen empfangener Grabsteine nötig: `grabEcht`); `Textfassung`-Decoder; `textrev`.
- Reines Zusammenführen (`zusammenfuehren_*`), Köpfe/Konflikt/`aktuell`/`gemeinsame_vorgaengerin`, wirksame Eltern, strenge Kette, wirksame Klasse, `nur_geraet`, Papierkorb, `teilnahmen` (Swift-Port der Rust-Funktionen in `zusammenfuehren.rs`, Permutationstests wie dort).
- Pull je Sammlung mit Cursor (Ordner vor Dateien), Entschlüsseln mit Prüfungen (§2.5), `angehalten`, Grabsteine (Siegel, Ordnergräber nachgelagert), Ledger je Konto; Texte vorab laden (prüfen: Hash, `textrev`, `original_sha256`); Originale nur auf Anforderung mit `wifiOnly`.
- Oberfläche: Seite „Ablage“ (Ordnerbaum, Liste mit Zuständen §2.11, Detail mit Fassungen und Konflikt-Hinweis, Vorschau über geladenes Original, Kontingent im Fuß); Assistent-Bereich „Ablage“ nur lesend.
- Lexikalischer Index (§2.14) und Suche in der Ablage mit Sprung zum Anker.
- **Abnahmekriterien:**
  1. Swift-Tests gegen Rust-Vektoren grün: Blob-Kennung (vorhanden), AAD (3 Fälle), Grabstein-Siegel, `textrev`, Textfassung-Blob, kanonische Nutzlast, `standard_id` (§3), Quelle der Werte jeweils der Rust-Testausgabe.
  2. Zusammenführen: alle Fälle der Rust-Permutationstests als Fixture grün (reihenfolgeunabhängig; Spec §9 D4a: Gegenwahl, Wahl-Rennen, Ordner-Zyklus A→B/B→A/B→C, Offline A→D).
  3. Gegen den Hub-Nachbau (`apps/apple/scripts/mock_voice_hub.py` muss um Sammlungsfilter, `blob_refs`, `reserve` erweitert werden, heute ohne diese Teile) oder gegen den Desktop-Lauf: PC legt Ordner/Dateien an, iPhone zeigt dieselben Namen, Klassen, Konflikt (zwei Köpfe) und Papierkorb-Zustand; Texte laden vor Originalen; Original nur mit WLAN-Regel.
  4. Fehlerpfade: fremde `key_id`, kaputtes Siegel, neueres Schema, Grabstein ohne Siegel, gefälschter Grabstein ⇒ nichts gelöscht, Zustand `angehalten`, kein Index-Eintrag, Warnung ohne Inhalte.
  5. Index: Musterfragen aus dem synthetischen Korpus (`korpus/demo`-Muster, nur `example.*`) liefern dieselben Dateien und Anker wie der Desktop (gleiche Reihenfolge der ersten 5; Rangzahlen dürfen abweichen). Abdeckungsmeldung wie Spec §5.3 (Dateien im Bereich, durchsucht, ohne Text, im Konflikt).
  6. Papierkorb, `entfernt` und begrabene Dateien liefern 0 Treffer; Konflikt ohne gemeinsame Fassung wird in der Abdeckung gemeldet.
  7. Nichts aus `Lokal`/`Halten` im Index oder im Prompt (Test mit Kontrolltext, Spec §9 D3).

### D5b: schreibender Teil (eigene Importe)

- Import aus ShareExtension/Dateien-App (`apps/apple/ShareExtension`): SHA-256 streamend, `fas_`/`dat_`-ULIDs, eigene `g_<ULID>` (persistent), Lamport-Zähler (§2.9), neue `ablage_datei` (`klasse_min = "public"`, `nur_geraet_min = false`, `papierkorb = {null, z, g}`, ein `Fassung` mit `vorgaenger = null`, `texte = []`) und
  Zustand „Text wird am PC erstellt“; Namenskollision im Ordner: „neue Fassung“ oder „beide behalten“ (`freier_name`, `speicher.rs:516-`).
- Senden nach Spec §6 und `blobs_bereitstellen`: reservieren → Original-Blob hochladen → Objekt-Push mit `blob_refs`, `blob_missing` ⇒ neu hochladen und erneut pushen, `superseded` ⇒ zusammenführen und neu senden, Rückstellung bei Ablehnung, Kontingent/`too_many_blobs` sichtbar stoppen. Ordner vor Dateien.
- Lokal-Teilnahme prüfen (§4 Punkt 1) vor dem Upload und vor dem Push, auch nach langem Upload.
- Wenn der PC die Textfassung nachliefert: Pull, Text laden, Index abgleichen, Zustand wechselt auf synchronisiert; vorher nie im Abruf.
- Optional und nur nach Klärung (Fragen 4, 8): Umbenennen, Verschieben, Papierkorb, Konfliktwahl (`neue_wahl`).
- **Abnahmekriterien:**
  1. Ein vom iPhone importiertes PDF erscheint am Desktop unter dem gewählten Ordner, mit passendem Namen/Klasse; Desktop erstellt den Text, `texte` kommt zurück, das iPhone findet es per Suche; bis dahin „Text wird am PC erstellt“ und 0 Treffer.
  2. Abbruch nach jedem Schritt (nach `reserve`, nach Stück *n*, nach `commit`, vor Push, nach Push) verliert nichts und erzeugt keine Dopplung; Wiederanlauf setzt fort (Ledger `hochgeladen`, `blob_info`).
  3. Löschen ∥ Upload (Spec §9 D4a): PC löscht, iPhone hat reserviert ⇒ 409 `reserved`, Push gelingt; Reservierung abgelaufen ⇒ `blob_missing` ⇒ neu hochladen ⇒ angenommen.
  4. `nur_geraet`/`personal`/`confidential`-Ziel ⇒ nichts im Hub (Mitschnitt des Nachbaus), Import bleibt lokal.
  5. Zwei Köpfe durch gleichzeitigen Import auf PC und iPhone werden als Konflikt angezeigt, nie per Zeit aufgelöst.
  6. Kontingent voll und `too_many_blobs` zeigen sichtbaren Stopp ohne Datenverlust; Wiederaufnahme bei Freigabe von Platz.
  7. Alle Pushes tragen `blob_refs` = Ableitung aus der Nutzlast (Gegenprobe: Desktop entschlüsselt ohne `angehalten`).

## 6. Offene Fragen

1. **Hub-Vertrag lesen (wai-portal):** Spec nennt „fehlender Blob → 422“; der Rust-Client behandelt `blob_missing` als **Ergebnis** in der 200-Antwort (`rejected` + `reason`), nicht als HTTP-422. Welche Form gilt wirklich? Gibt es weitere `reason`-Werte als im Nachbau (`ablage_tests.rs:199-262`)?
   Welcher HTTP-Status gilt für `quota_exceeded`? Hub-Status für `too_many_blobs` ist laut Kommentar 507 (`ablage.rs:1628`).
2. **Globaler Pull:** Liefert `GET /sync/pull` ohne `collection` (genutzt von `LibrarySync.pull`, `LibrarySync.swift:186`) die Ablage-Sammlungen mit? `apply` ignoriert fremde Sammlungen, aber der gemeinsame Cursor würde über sie hinweglaufen. Bestätigen, dass Ablage nur mit `collection=` und eigenen Cursors gezogen wird, und dass die alte App-Version (ohne Ablage) dadurch nicht stört.
3. **Index-Technik:** FTS5 des System-SQLite (Parität mit `unicode61 remove_diacritics 2`) oder eigener BM25 in VoiceCore mit derselben Tokenisierung? Ist FTS5 in allen Zielversionen und in der Linux-Testumgebung von `swift test` verfügbar? Soll der Paritätstest (Abnahme 5) gegen eine feste Rust-Ausgabe (Fixture) oder gegen den laufenden Desktop laufen?
4. **Löschen und Papierkorb am iPhone:** Darf das iPhone in D5b in den Papierkorb legen (Register `papierkorb`) und endgültig löschen (versiegelter Grabstein)? Wer räumt die Blobs auf (bedingte Löschung braucht den vollständig gezogenen Bestand; Spec §6)? Empfehlung: D5 ohne Löschen, nur Anzeigen und Importieren.
5. **Assistent-Bereich:** Welche Modelle laufen auf dem iPhone (lokal, BookTalk-Relay zum PC?), und wie bildet sich D3 darauf ab? `freigabe.json` ist gerätelokal; der Kontextvertrag (ADR-0028 E6) und das Tor (Widerruf im Gesamtprompt, Spec §5.4) müssten auf dem iPhone nachgebaut werden. Reicht für D5 eine reine Suche ohne KI-Antworten und „Aus Ablage“ als Kontextquelle erst mit D3?
6. **Projektverweise und Deep Links:** Werden Projekte (P4) auf das iPhone synchronisiert? Wenn ja, braucht das iPhone die Weiterleitung alter IDs oder genügt „Datei nicht verfügbar“ (§2.10)?
7. **Größenlimits:** Welche Obergrenze gilt für die Nutzlast eines `ablage_datei` (viele Fassungen, viele Texte)? `maxPlaintext` 256 KiB am iPhone gegenüber „bis ~4,2 MB“ am Desktop (`client.rs:323`) und Hub-Grenze für `payload_size`; Pushstapel am iPhone heute höchstens 20 Objekte/400 000 Zeichen (`LibrarySync.swift:push`), Desktop 40.
8. **Klasse und Freigabe beim iPhone-Import:** Importe haben am Desktop `klasse_min = public` und erben vom Ordner (Vorgabe `internal`). Soll das iPhone dem Nutzer beim Import eine Klasse anbieten, oder nur in die geerbte Ordnerklasse legen? Welcher Standardordner („Allgemein“ mit `standard_id`) gilt, wenn das Konto noch keine Ablage hat?
9. **Pflege der Extraktorversion:** Der PC erstellt Texte mit der **aktuellen** Extraktorversion; ändert sich sie, entsteht eine neue `textrev` und alte Belege zeigen „Text wurde neu erstellt“ (Spec §4.2). Soll das iPhone ältere Textfassungen bis zum PC-Neuaufbau weiter nutzen (wie der Desktop) und Belege auf alte `textrev` anzeigen?
10. **Hintergrund und Strom:** Sync-Takt und Textvorabladen im Hintergrund (BackgroundTasks) oder nur im Vordergrund? Bestehender Takt in `LibrarySync.run()` läuft über `running`/`again`.

## Anhang A: Lücken im bestehenden iOS-Code (Stand `origin/main`)

- `apps/apple/VoiceCore/Sources/VoiceCore/LibrarySyncModel.swift`: `HubPushObject` ohne `blob_refs`; `HubRemoteObject` ohne `blob_refs`; `HubPushResult` ohne `missing`/`reason`-Auswertung für `blob_missing`/`tombstoned`; `canonicalHash` streicht `null`; `maxPlaintext` = 256 KiB.
- `apps/apple/VoiceCore/Sources/VoiceCore/SyncCrypto.swift`: `SyncCrypto.aad` vorhanden; fehlen `aadAblage`, deterministisches Siegel (`seal` hat optionale feste Nonce, aber keine Ableitung aus AAD und Klartext), `SecRandomCopyBytes` nur unter `#if canImport(CryptoKit)` (Linux-Tests benötigen feste Nonce).
- `apps/apple/iPhone/Library/LibrarySync.swift`: kein Pull je Sammlung (`pull`, `:186`), kein `reserve`, kein `delete`, kein `blob_refs` im Push (`push`, `:446`), keine Auswertung von `blob_missing`/`tombstoned`; Grabstein für Bücher mit leerer Nutzlast (`deleteEverywhere`); Konto, Schlüsselbund, `NoRedirects`, `uploadFile`/`downloadBlob` (Stücke, Hash, Fortsetzen), `wifiOnly`, `NWPathMonitor` sind wiederverwendbar.
- `apps/apple/VoiceCore/Sources/VoiceCore/PassthroughJSON.swift`: Rohdurchreichen unbekannter Werte als `Double`; für `extra` brauchbar, aber ganze Zahlen über 2^53 gehen verloren (Ablage-Zahlen liegen darunter).
- `apps/apple/scripts/mock_voice_hub.py`: kein `collection`-Filter, kein `reserve`, kein `blob_refs` (Treffer in der Suche: keine); muss für die Abnahme erweitert werden, oder die Abnahme läuft gegen den Desktop mit echtem Hub.
- `BookTalkSearch.swift`/`BookTalkIndex.swift`: eigener BM25 mit abweichender Tokenisierung, nur als Vorlage für die Struktur, nicht für Parität.
- `apps/apple/ShareExtension` und `ImportConsentView.swift`: vorhandene Einstiegspunkte für „eigene Importe“ (D5b).
