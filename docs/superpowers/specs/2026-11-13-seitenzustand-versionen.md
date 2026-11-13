# Seitenzustand: Versionen und Textkonflikte (lva#546, lva#545)

Stand 13.11.2026, **Fassung 2** (Codex R1 auf 1a73dbf0: „tragfähig NEIN in dieser Fassung“, Grundmodell geeignet;
alle 8 Blocker und die Hinweise sind eingearbeitet, Zuordnung in §10). Basis `main` ccb97418 (nach #549).
Vorbedingung für Stufe I2 des Inhalte-Moduls (`2026-11-12-inhalte-modul.md` §3.1, §13). Ziel: Das Modell für alle
Schreiber von `state.json` wird **einmal** festgelegt, damit es am Stück prüfbar ist.

## 0 Ausgangslage (Code, nicht Annahme)

- **JS-Kette** (`src/lib/reader/pageState.ts`, `pageStore.ts`): ein Store, eine serielle Kette; jeder Auftrag liest
  frisch und schreibt ganz (`commitLog`, `save`, `mutate`). Laufnummern `seq`/`lastWrite` entscheiden, ob ein
  Rettungssnapshot (`failedSaves`) veraltet ist. `einfuegungen` kommt immer aus der Datei. Ein Textkonflikt wird
  nur für ausgehändigte Snapshots behandelt (`secureOverwrittenFile`: Oberfläche gewinnt, Dateitext wird Fassung
  über `bookAiDocumentImport`).
- **Lücken:** `read` liest außerhalb der Kette und ohne Basis. `save` schreibt `live.build()` ungeprüft und nur
  mit den Feldern der Oberfläche (unbekannte Felder der Datei fallen weg). `mutate`-Aufrufer arbeiten mit vorher
  gelesenen Werten (Rollback `useLibrary.ts` Z. 417, `joined.text` `merge.ts` Z. 355). Laufnummern sehen keine
  Rust-Schreiber.
- **Rust:** `page_state_save` schreibt unter `pages::lock()` blind (`pages.rs` Z. 564); `page_state_load` macht
  aus jedem Lesefehler `""` (Z. 559, #545). Der Sync schreibt in `collect.rs` Z. 187 (`apply_page`) unter derselben
  Sperre an der JS-Kette vorbei; sein Konfliktvergleich `pages_differ` (Z. 276) sieht nur Titel, `text`,
  `summary`, `translation`. `collect`/`read_page` machen aus Lese- **und** Parsefehlern `Value::Null`; der
  Seitenexport (`pages_package.rs` Z. 123/215) macht aus Lesefehlern leer. `pages_delete` löscht per
  `remove_dir_all` (Z. 528).
- **Prozesse:** `tauri-plugin-single-instance` (`lib.rs` Z. 2997) außer im Headless-Modus.

## 1 Invarianten

Regeln in Anführungszeichen sind wörtlich aus Codex R1 übernommen („akzeptierte Regel“).

- **V1 – CAS.** „Jeder reguläre Bestandsüberschreiber verwendet CAS; vergleichslose interne Schreibwege haben klar
  begrenzte Vorbedingungen.“ Liste und Vorbedingungen: §2.6. Neuanlagen schreiben gegen `EMPTY_VERSION`.
- **V2 – Inhalt entscheidet.** „Die CAS-Entscheidung beruht auf dem tatsächlichen Inhalt unter der Sperre.“ Keine
  Abkürzung über Länge oder mtime.
- **V3 – Kein stilles Überschreiben eines neueren erfolgreichen Textstands.** Weicht die Datei von der Basis ab,
  wird gemischt (§2.4). Ein nicht mischbarer Textwert (`text`, `summary`) verschwindet nur aus der Datei, nachdem
  er als Fassung gesichert oder im Speicher gehalten **und** gemeldet ist (`saveCopy`/`hold`/`onRescue`;
  `held` ist ausdrücklich flüchtig und wird als solches gemeldet).
- **V4 – Ungesicherte Eingaben bleiben.** Eine Übernahme ersetzt nie einen Wert der Oberfläche, der seit ihrer
  Basis geändert wurde. Bei Textkonflikt gewinnt die Oberfläche, der Dateitext wird nach V3 gesichert.
- **V5 – Dateifelder.** „Nicht vom Editor verwaltete Felder kommen bei *jedem* Schreiben aus T.“ Das gilt auch
  ohne Versionsabweichung (Schnellpfad, §2.4).
- **V6 – Basis.** „Übernommene Feldwerte werden mit ihrer passenden Basis geführt; lokale Ansichtsentscheidungen
  brauchen eine ausdrückliche Politik.“ Umsetzung: Es gibt nur ganze Übernahmen (alle Felder des neuen Stands
  samt dessen Basis, im selben Render) oder keine; Ansichtsfelder haben die Politik aus §2.4.
- **V7 – Rohfassungen in JS-Schreibern wie I1.** Für alle Aufträge der JS-Kette ist die Datei die einzige Quelle
  für `einfuegungen`; nur `commitLog` ändert das Feld; jede Mischung übernimmt den Dateiwert; „voll“ schreibt
  nichts.
- **V8 – Sync.** „Auch ausschließlich in Rohfassungen vorhandene Nutzdaten dürfen beim Sync nicht still
  verschwinden.“ Nutzdaten sind alle Felder außer den Ansichtsfeldern `tab`, `kiDoc`; nur diese dürfen beim
  Ersetzen durch einen Server-Stand ohne Sicherung verloren gehen (§2.7).
- **V9 – Dekodierung.** „Fehlend, gültiges Zustandsobjekt, unterstützter Rohtext und nicht interpretierbarer
  Inhalt werden ausdrücklich unterschieden; vorhandener Inhalt wird nicht zu leer.“ Eine Regel für JS, Sync,
  Export (§2.3).
- **V10 – Lesefehler.** „Lesefehler stoppen den jeweiligen Schreibvorgang; Eingaben bleiben erhalten, der Fehler
  wird sichtbar und die Wiederaufnahme ist definiert.“ Ladefehler und Fehler der offenen Seite: §3.2.
- **V11 – Quelllöschen.** „Automatisches Quelllöschen darf keine seit dem Zusammenführungsstand hinzugekommenen,
  nicht übernommenen Inhalte entfernen.“ (§4 Fall 3).
- **V12 – Größe.** `page_state_save` lehnt mehr als `MAX_STATE_BYTES` = 64 MiB UTF-8-Bytes ab, die Datei bleibt.
  Lesen und Exportieren haben keine Grenze; bestehende größere Dateien bleiben lesbar und exportierbar. Größtes
  Rohfassungsprotokoll (16 MiB JSON) plus 1 MiB Text übersteht die Rundreise. Keine Leistungszusage.
- **V13 – Rettung.** `failedSaves`, `held`, `saveCopy`, `onRescue` bleiben in Bedeutung und Text. Gesicherte
  Konflikttexte und Mischbasis werden getrennt geführt: Ein Text wird je Konflikt höchstens einmal gesichert
  (auch über CAS-Wiederholungen), und kein ungesicherter Stand gilt als berücksichtigt (§2.8).
- **V14 – Wiederholung.** Jede Änderungsfunktion (`change` von `mutate`, `LogOp`) läuft auf dem frischen Stand
  und darf nach einem CAS-Konflikt erneut laufen (höchstens 3 Versuche); ein abgewiesener Versuch hat nichts
  geschrieben, also wird nichts doppelt angewendet. Stabile Wiederholung von `mergeState`: Nach jedem gelungenen
  Schreiben rückt die Basis auf das Geschriebene vor (§2.8), eine Mischung sieht ihr eigenes Ergebnis also nicht
  mit alter Basis wieder; `mergeState(M, O, M)` ändert an `M` nichts außer den Editorfeldern aus `O`. Wo das
  Ergebnis doch gegen eine alte Basis läuft (lokaler Konflikt in `onAdopt`, §3.1), endet es konservativ als
  Konflikt mit Sicherung, nie als doppelte Anwendung: das Spannenverfahren setzt überlappende Änderungen nie
  zusammen.
- **V15 – Format.** `state.json` bekommt kein neues Feld; keine Dateimigration.

## 2 Versionsmodell

### 2.1 Version: Bytehash, kein Feld `rev`

**Version = SHA-256 (hex, klein) der Bytes von `state.json`.** Fehlende und vorhandene leere Datei sind bewusst
versionsgleich (`EMPTY_VERSION` = SHA-256 von `""`, in Rust und TS gleich, per Test belegt). JS hasht den
gelesenen Text mit `sha256Hex` (`pasteLog.ts`), Rust die Datei; `read_to_string` ist für gültiges UTF-8 verlustfrei
(BOM inklusive). Es ist ein **Bytehash, kein kanonischer Inhaltshash**: Feldreihenfolge, Whitespace, BOM und
Unicode-Normalisierung ändern ihn absichtlich. Das kostet nur zusätzliche Mischprüfungen, nie Daten; vor dem
Hashen wird nichts normalisiert. SHA-256-Kollisionen sind ein vertretbares Restrisiko. Fehlt `crypto.subtle`
(`sha256Hex` liefert `""`), schreibt der Store nicht und meldet einen Fehler (kein stilles Abschalten von CAS).

| | Feld `rev` in state.json | Bytehash (gewählt) |
|---|---|---|
| Schreiber ohne `rev`-Kenntnis (Sync älterer Geräte, ältere App, Attrappen, Hand) | unbemerkt; `rev` ist nur mit kontrollierten Schreibern monoton | jede Byteänderung zählt |
| Sync | wird mitsynchronisiert, Zähler kollidieren | Objekt unverändert |
| Migration | Regel für Dateien ohne `rev` | keine |
| ABA (gleicher Inhalt kehrt zurück) | erkannt | gilt als unverändert – harmlos |

### 2.2 CAS in Rust unter der Seitensperre

Eine Prüfung nur in JS hat ein Fenster, in dem `apply_page` oder ein zweites Fenster (eigener Store) schreibt. Nur
ein Vergleich **unter `pages::lock()`** ist atomar gegenüber allen Schreibern im Prozess.

```rust
// commands/pages.rs
pub(crate) const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const EMPTY_VERSION: &str = "e3b0c442…b855";              // SHA-256("")
pub(crate) fn state_version(bytes: &[u8]) -> String;                 // sha2, schon Abhängigkeit
/// Bytes lesen: Ok(None) = fehlt; Err("page_state:unreadable: …") = jeder andere E/A-Fehler.
pub(crate) fn read_state_bytes(dir: &Path) -> Result<Option<Vec<u8>>, String>;
/// Hält die Seitensperre (wiedereintrittsfähig, daher auch unter apply_page). Liest die Datei, hasht sie,
/// vergleicht mit `expected`, schreibt per write_atomic. Fehler: "page_state:conflict",
/// "page_state:unreadable" (nie über eine unlesbare Datei, auch nicht mit expected=None), "page_state:too_large".
pub(crate) fn write_state(dir: &Path, bytes: &[u8], expected: Option<&str>) -> Result<String, String>;

#[tauri::command] pub fn page_state_load(app, id: String) -> Result<String, String>;   // Signatur gleich
#[tauri::command] pub fn page_state_save(app, id: String, state: String,
                                          expected: Option<String>) -> Result<(), String>;
```

- `page_state_load` behält die Signatur (14 Spec-Dateien fangen den Befehl mit Rohtext ab; Rust-Aufrufer
  `app_host`, `hoersync_live`, `voice_tools`, `app_context` bleiben). Kein UTF-8 → `page_state:unreadable`.
- **Sperren:** Reihenfolge wie in `pages.rs` Z. 66–77 dokumentiert. Unter der Sperre nur Lesen, Hashen, Ersetzen;
  keine JS-Mischung, keine Rettungs-IPC (`bookAiDocumentImport`), keine Netzarbeit.
- **Teilwirkung:** `apply_page` ersetzt erst den Zustand, dann den Index. Scheitert `store_index` danach, steht der
  neue Zustand mit altem Titel da; das ist zulässig und wird vom nächsten Sync-Lauf ausgeglichen. Gleiches für
  `conflict_copy` (Ordner ohne Indexeintrag wird von `page_lookup` nicht gefunden, bleibt aber auf der Platte).
- **Prozesse:** Zwei Fenster desselben Prozesses sind geschützt. Zwei Prozesse teilen den Mutex nicht; die
  Einzelinstanz-Garantie (`lib.rs` Z. 2997) gilt außer im Headless-Modus, der Seitenzustände nicht schreiben darf
  (Paket A prüft und belegt das per Suche). Fremde Prozesse (Cloud-Ordner, Handbearbeitung) behalten ein
  Restfenster zwischen Vergleich und Umbenennen; der Hash erkennt ihre Änderungen, solange sie bestehen.

### 2.3 Dekodierung (eine Regel für JS, Sync, Export)

| Klasse | Erkennung | JS-Store / Oberfläche | Sync (`collect`, `read_page`, `apply_page`) | Export / Konfliktkopie |
|---|---|---|---|---|
| fehlend | Datei fehlt | `{}`, Version `EMPTY_VERSION` | Seite ohne Zustand (wie heute) | leerer Zustand ist korrekt |
| Objekt | JSON-Objekt | normal | normal | Bytes unverändert |
| Rohtext (unterstützt) | UTF-8, aber kein JSON-Objekt | wie Bestand: Text = Rohtext; Mischbasis `{text: roh}`; erstes Schreiben macht daraus ein Objekt, der Text bleibt | Upload als `{"text": roh}` (einziges Schemawissen neben `pages_differ`/Vorschau); Konfliktvergleich gegen diese Form | **Rohbytes**: `apply_page` liest die Bytes unter der Sperre und übergibt sie `conflict_copy`, das sie bytegleich schreibt |
| nicht interpretierbar | E/A-Fehler, kein UTF-8 | Fehler (V10), kein Schreiben | Seite in diesem Lauf zurückhalten (wie `held`), nicht hochladen, nicht überschreiben | Export bricht mit Meldung ab |

Rust bleibt schemafrei bis auf die genannten Feldnamen (`text`, `tab`, `kiDoc`, dazu `pages_differ`).

### 2.4 Mischung (`src/lib/reader/pageMerge.ts`, rein, neu)

```ts
export const EDITOR_FIELDS = ["text","summary","kiDoc","sourceUrl","tab","voices","voicesChosen",
  "lang","translatedLangs","autoTag"] as const;   // = pageStateJson() in TtsSettings
export const VIEW_FIELDS = ["tab", "kiDoc"] as const;           // Ansichtsentscheidungen der Oberfläche
export const TEXT_FIELDS = ["text", "summary"] as const;        // Konfliktverlierer werden gesichert
export interface MergeResult { state: StateObject; conflicts: string[]; lost: { field: string; value: string }[] }
export function mergeState(base: StateObject, ours: StateObject, theirs: StateObject,
  opts: { prefer: "ours" | "theirs"; fields?: readonly string[] /* Standard EDITOR_FIELDS */;
          view?: "ours" | "merge" /* Standard "ours" */ }): MergeResult;
export function mergeText(base: string, ours: string, theirs: string): { text: string } | { conflict: true };
```

**Vergleich:** kanonisches JSON (Objektschlüssel rekursiv sortiert, Arrays in Reihenfolge). Fehlender Schlüssel
und `undefined` sind gleich, `null` ist ein eigener Wert. Ein in O fehlendes Editorfeld bedeutet „entfernt“.

| Feldklasse | Regel |
|---|---|
| `einfuegungen` | immer T (V7) |
| alle Felder außerhalb `fields` | immer T (V5), auch im Schnellpfad |
| Ansichtsfelder `tab`, `kiDoc` (`view: "ours"`) | immer O: die offene Seite entscheidet ihre Ansicht; die Basis ist für diese Felder bedeutungslos und wird nie zum Mischen benutzt |
| `text`, `summary` | `mergeText`; Konflikt → Wert von `prefer`, der andere als `lost` |
| Skalare/Objekte (`sourceUrl`, `lang`, `autoTag`) | O=B → T; T=B → O; O=T → O; sonst `prefer` |
| `translatedLangs` | (B∩O∩T) ∪ (O∖B) ∪ (T∖B), Reihenfolge von T, dann neue aus O |
| `voices`, `voicesChosen` | je Schlüssel wie Skalar; Entfernen eines Schlüssels ist eine Änderung |

`mergeText`: **zuerst** die Identitäten O=B → T, T=B → O, O=T → O. Erst dann je Seite die geänderte Spanne gegen B
über gemeinsamen Präfix/Suffix (Grenzen nie in einem Surrogatpaar); getrennt (strikt, ohne Berührung) → beide
anwenden; überlappend oder berührend → Konflikt. O(n), ohne Abhängigkeit.
**Schnellpfad = derselbe Weg:** Ist die Dateiversion gleich der Basis, wird `mergeState(T, O, T)` geschrieben –
Ergebnis: Editorfelder aus O, alles andere aus T (V5). Es gibt keinen Schreibweg, der `O` roh schreibt.
**Algebra:** wegen `prefer`, Dateivorrang und Mengenreihenfolge absichtlich nicht kommutativ. Gefordert und
getestet: stabile Wiederholung (V14: `mergeState(M, O, M)`, erneute Mischung gegen alte Basis endet als
Konflikt) und keine doppelte Anwendung derselben Änderung.

### 2.5 Politik je Schreiber

| Schreiber | Basis | `prefer` | gesichert wird |
|---|---|---|---|
| Autosave/Export/KI-Dokument der offenen Seite | Basis der Oberfläche (V6) | ours | `lost` (Dateitext) vor dem Schreiben |
| ausgehändigter Rettungssnapshot (Lade-Autosave) | Basis des Snapshots | ours | `lost` vor dem Schreiben (wie I1) |
| nicht ausgehändigter Snapshot (`resolveFailed`) | Basis des Snapshots | – (I1: nicht mischen) | Snapshot-Text → Fassung, Datei bleibt |
| `mutate` (Bibliothek, Fassungen, Sprecher, Migration) | – (Änderung auf frischem Stand) | – | – |
| Rollback Anhängen | eigener geschriebener Stand | theirs, `view: "merge"`, `fields`: alle außer `einfuegungen` | nichts (Systemwert) |
| Sync `apply_page` | `synced_hash` | – | Konfliktkopie als Seite (§2.7) |

### 2.6 Schreiber von state.json und CAS-Modus (endgültig, mit Übergang)

| Schreibweg | Fundstelle | `expected` ab A | ab B |
|---|---|---|---|
| Store-Aufträge (`save`, `mutate`, `commitLog`, Rettung) | `pageStore.ts` Z. 43 | Version des im selben Auftrag gelesenen Stands | zusätzlich Mischung gegen Oberflächen-/Snapshotbasis |
| Neue Seite (Transkript, Zusammenführen, Datei-Import) | `useLibrary.ts` Z. 337, `merge.ts` Z. 370, `importFile.ts` Z. 176 | `EMPTY_VERSION` | gleich |
| Agent legt Seite an | `app_host.rs` Z. 118 | `Some(EMPTY_VERSION)` | gleich |
| Sync `apply_page` | `collect.rs` Z. 187 | `None`; Vorbedingung: hält die Sperre über frisches Lesen, Konfliktentscheidung und Schreiben | gleich |
| `conflict_copy` | `collect.rs` Z. 246 | `None`; Vorbedingung: frische Kennung, unter der Sperre von `apply_page` | gleich |
| Seitenpaket-Import | `pages_package.rs` Z. 332 | `None`; Vorbedingung: frische Kennung unter der Sperre | gleich |

Kein TS-Aufrufer übergibt `null`. **Übergang:** Nach A gibt es Schutz gegen Rust-Schreiber zwischen Lesen und
Schreiben eines Auftrags, aber **noch keinen Schutz gegen veraltete Oberflächenstände** (Fälle 1, 4, 5 bleiben
offen, ihr Marker bleibt). Nach B gilt V1–V7 für die offene Seite, nach C für alle Fälle.

### 2.7 Sync (Rust) und Kompatibilität

- `pages_differ` vergleicht den ganzen Zustand außer `tab`, `kiDoc` (statt nur `text`/`summary`/`translation`).
  Eine nur lokale Änderung an `einfuegungen`, `voices` usw. führt damit vor dem Ersetzen zur Konfliktkopie (V8).
  Zulässiger Verlust beim Ersetzen: nur `tab`, `kiDoc` des lokalen, ungesyncten Stands. Mehr Konfliktkopien als
  heute sind die bewusste Folge.
- `apply_page` schreibt über `write_state(…, None)`, verweigert das Überschreiben einer nicht interpretierbaren
  Datei (Fehler für diese Seite, nächster Lauf versucht es erneut) und kopiert Rohtext bytegleich (§2.3).
- Die offene Seite bemerkt Sync-Stände über die Version (nächstes Schreiben mischt) und über `sync-changed`
  (Übernahme, §3). Ein Doppel aus Konfliktkopie (Sync) und Fassung (Mischung) ist möglich und gewollt.
- **Kompatibilität:** Keine Dateimigration; gleiche Sync-Objekte (V15). Ältere Apps, ein Downgrade und die
  iPhone-App (`apps/apple`) behalten ihre alten Verlustpfade; der Hash erkennt deren Änderungen nur, solange sie
  noch in der Datei stehen. 64 MiB lokale Speichergrenze und 3 MiB Sync-Klartext je Objekt
  (`collect.rs` `MAX_PLAINTEXT`) bleiben zwei verschiedene Zusagen.

### 2.8 Store-Änderungen (`pageState.ts`)

- `io.load(page)` → `string | null` wie bisher; der Store berechnet die Version. `io.save(page, json, expected)` →
  `true | false | "conflict" | "too_large" | "unreadable"` (`true`/`false` bleiben für die bestehenden
  Attrappen gültig). Größenprüfung in UTF-8-Bytes vor dem Aufruf.
- `PageBase = { version: string; raw: string }`; `PageRead` bekommt `base`. `PageStateLive.build()` →
  `{ json, base }` (Stand und Basis desselben Renders). `save(page, snap: { json; base })`.
- Jeder Auftrag schreibt gegen die Version seines frischen Lesens; bei `"conflict"` frisch lesen und neu rechnen,
  höchstens 3-mal, dann `PageSaveError("conflict", kept)` wie ein Schreibfehler.
- **Rettung getrennt von Basis (V13):** Jeder Auftrag führt `securedInJob: Set<sha256(text)>`; ein `lost`-Text,
  der dort steht, wird bei einer CAS-Wiederholung nicht erneut gesichert. `FailedSave` bekommt `base` und
  `secured: Set<sha256(text)>` (ersetzt `known`); `seq`, `lastWrite`, `newerWrite` entfallen. „Datei ist neuer“
  heißt: T unterscheidet sich von `base` in einem Nutzdatenfeld außer `einfuegungen` (reines Protokollschreiben
  zählt wie in I1 nicht). Eine Basis rückt nur nach einem gelungenen Schreiben oder einer ganzen Übernahme vor;
  ein nur gehaltener (`held`) Text zählt als gesichert-flüchtig und bleibt gemeldet.
- Neu: `refresh(page)` (Kettenauftrag, schreibt nie), Rückrufe `onSaved(page, {used: PageBase, base: PageBase})`
  und `onAdopt(page, {from: string, to: string, base: PageBase})`, `onUnreadable(page)`.
  `bindPageStateLive(view, { onCommit, onRescue, onSaved, onAdopt, onUnreadable })`.
- `mark`/`changedSince` (Anzeige des Protokolls) bleiben.

## 3 Offene Seite

### 3.1 Übernahme fremder Änderungen (Fall 4)

**Auslöser und Erkennungszeitpunkt:** (a) nach jedem gelungenen `mutate` derselben Seite in diesem Fenster,
(b) `sync-changed`, (c) KeepAlive wieder sichtbar (ersetzt `TtsSettings.tsx` Z. 3046–3080), (d) spätestens beim
nächsten Speichern (Mischung). Änderungen aus einem anderen Fenster werden nicht sofort, sondern bei (b)–(d)
erkannt; Schreibsicherheit gibt CAS unabhängig davon.

**`refresh(page)`:** nicht offen → nichts. T lesen; Lesefehler → `onUnreadable` (§3.2). `{json: O, base: B} =
live.build()`; `version(T) = B.version` → nichts. Sonst `m = mergeState(B, O, T, {prefer: "ours"})`:
- keine Konflikte in `TEXT_FIELDS` → `onAdopt({from: O, to: m.state, base: T})`;
- Textkonflikt → **keine Übernahme**, Basis bleibt; der Autosave wird angestoßen (Muster `rescueTick`) und löst
  den Konflikt über den Speicherweg (Sicherung vor Überschreiben). Eine Teilübernahme gibt es nicht (V6).

**Speichern der offenen Seite:** `{O, B} = live.build()` beim Ausführen, T frisch. `m = mergeState(B, O, T,
{prefer: "ours"})` (bei gleicher Version ist das der Schnellpfad `mergeState(T, O, T)`). `m.lost` → `secureText`
(dedupliziert, V13) **vor** dem Schreiben; `m.state` gegen `version(T)` schreiben. Danach: hat T Felder gegenüber
B geändert → `onAdopt({from: O, to: geschrieben, base: geschrieben})`, sonst `onSaved({used: B, base:
geschrieben})`.

**In der Oberfläche (`TtsSettings`):** Basis ist React-Zustand `pageBase`, gesetzt nur zusammen mit dem
Seitenzustand. Rückrufe für eine andere als die aktive Seite werden verworfen (der Stand steht in der Datei).
- `onSaved`: Basis = `base`, nur wenn `pageBase` noch `used` ist (sonst hat eine spätere Übernahme sie schon
  fortgeschrieben). Eingaben seit dem Bau stammen von O ab, also auch vom Geschriebenen.
- `onAdopt`: synchron `local = mergeState(from, aktuell, to, {prefer: "ours"})`. Ohne Textkonflikt: alle Felder
  von `local` und `pageBase = base` in **einem** Handler setzen (`applyPageState`, aus dem Lade-Handler
  Z. 1173–1278 herausgezogen; Ansichtsfelder bleiben die der Oberfläche). Mit Textkonflikt (Eingabe genau in der
  fremd geänderten Stelle während des Auftrags): **nichts** übernehmen, Basis bleibt; der nächste Speichervorgang
  sichert den Dateitext und schreibt den der Oberfläche. Das kann eine überflüssige Fassung erzeugen, nie einen
  Verlust.
- Textänderung im fokussierten Editor: fremde Spanne per `setSelectionRange` + `insertText` (Muster
  `TtsChipEditor.tsx` Z. 530–547), Cursor und Rückgängig bleiben; verdeckt oder ohne Fokus `setText`.
- Rettungssnapshots: ein ausgehändigter Snapshot (`read.unsaved`) kommt mit seiner Basis; der Lade-Autosave
  mischt gegen die Datei. Solange er aussteht, ersetzt keine Übernahme ihn (Mischung `prefer: "ours"`, V4).

### 3.2 Lesefehler (#545): Laden und offene Seite getrennt

- **Ladefehler (Öffnen):** `read` → `null`. Oberfläche zeigt „Seite konnte nicht gelesen werden“ mit „Erneut
  versuchen“; Editor schreibgeschützt; `pageLoaded` bleibt `false` (kein Autosave); kein Altbestands-Import aus
  `localStorage`. Wiederaufnahme: „Erneut versuchen“, Seitenwechsel hin und zurück.
- **Lesefehler der offenen Seite** (in `save`, `refresh`, Export): Der Auftrag schreibt nicht
  (`PageSaveError("unreadable")`), `onUnreadable` setzt den Zustand „Datei unlesbar“: Meldung (eine, feste
  `id`), Eingaben bleiben **bearbeitbar** in der Oberfläche, Basis bleibt, `refresh` übernimmt nichts. Autosave
  versucht es bei jeder weiteren Änderung und über „Erneut versuchen“; der erste gelungene Lesevorgang hebt den
  Zustand auf und mischt normal. Verlassen der Seite in diesem Zustand → Rettungssnapshot (`kept`), Wiederaufnahme
  beim nächsten Öffnen (§5).
- Ebenso: `mutate`/`commitLog` schreiben nicht (Bestand), Sync hält zurück (§2.3), Export bricht ab
  (`pages_package.rs`, `versions.loadPageState` wirft statt leer), `merge.loadText` bricht das Zusammenführen ab.

## 4 Die Fälle einzeln: alt → neu

1. **Ladende Seite.** Alt: `read` ohne Basis; `mutate` schreibt „Fremd“, die verspätete Ladeantwort „Vorher“
   landet in der Oberfläche, der Lade-Autosave schreibt sie zurück. Neu: Ladeantwort mit `base = Vorher`; der
   Autosave liest „Fremd“, `mergeState(Vorher, Vorher, Fremd)` = Fremd wird geschrieben und übernommen.
2. **Anhängen + Rollback** (`useLibrary.ts`). Alt: Rollback löscht alle Felder und setzt den alten Stand. Neu:
   `restore` merkt `{vorher, geschrieben}`; die Rollback-Änderung auf dem frischen Stand ist `mergeState(
   geschrieben, vorher, frisch, {prefer: "theirs", view: "merge", fields: alle außer einfuegungen})`.
   **„Sauber“** heißt: `conflicts` leer. Ergebnis immer: nicht konfliktbehaftete Felder auf `vorher`, konflikt-
   behaftete auf dem frischen Stand. Der Rollback schreibt dieses Ergebnis (auch wenn es dem frischen Stand
   gleicht). **Unsauber** → kein automatischer Neuversuch des Anhängens (sonst doppelter Text), `giveUp` mit
   Meldung.
3. **Zusammenführen** (`merge.ts`). Alt: `joined.text` aus vorher gelesenen Texten ersetzt den Zieltext; Quellen
   werden endgültig gelöscht, ein unlesbarer Teil fehlt still. Neu: `joinedText` merkt je Teilseite Text und
   Version; ein Lesefehler bricht ab. Die `mutate`-Änderung in `storeJoined` baut den gemeinsamen Text neu, wenn
   der frische Zieltext abweicht (`joinTranscripts` mit frischem Zielteil), `sync` kommt aus derselben Rechnung.
   **Quellen (V11):** Scheitert das Zielschreiben, wird keine Quelle angerührt. Sonst wird jede Quellseite
   **wiederherstellbar entfernt**: neuer Rust-Weg `pages_retire(id, expected)` unter der Seitensperre – nur wenn
   `state.json` noch die gelesene Version hat, wird der Ordner wie in `tombstone_page` (`collect.rs` Z. 204) nach
   `projects/_geloescht/` verschoben, der Index-Titel als `titel.txt` hineingeschrieben und der Indexeintrag
   entfernt. Titeländerungen und neue Projektdateien gehen so nicht verloren (der Ordner bleibt vollständig),
   und eine inhaltlich geänderte Quelle bleibt als Seite stehen (Meldung). `pages_delete` bleibt für das
   Löschen durch den Nutzer unverändert.
4. **Offene Seite.** Alt: fremde Änderung steht in der Datei, der Editor zeigt sie nicht, der Autosave überschreibt
   sie. Neu: §3.1.
5. **Geräte-Sync.** Alt: Sync ersetzt die Datei, der nächste Autosave überschreibt sie mit dem alten Stand; nur
   lokal geänderte Rohfassungen gehen beim Ersetzen ohne Kopie verloren. Neu: CAS schließt das Fenster zwischen
   JS-Lesen und -Schreiben, der Autosave mischt, `sync-changed` stößt die Übernahme an, `pages_differ` schützt
   alle Nutzdaten (V8).
6. **#545 Lesefehler.** Alt: `""` → Oberfläche lädt „leer“ (inkl. Altbestands-Import), der Lade-Autosave
   überschreibt Text und `einfuegungen`. Neu: §3.2, §2.3.

## 5 Zustände und Übergänge (eine Seite)

| Zustand | Ereignis | Bedingung | Aktion | Folge |
|---|---|---|---|---|
| geschlossen | Öffnen | Snapshot aussteht, nicht ausgehändigt | `resolveFailed` (§2.5) | weiter mit Lesen |
| geschlossen | Öffnen | Snapshot aussteht, Schreiben gelingt | Snapshot (gemischt bzw. I1-Regel) geschrieben, gelöscht | synchron |
| geschlossen | Öffnen | Snapshot aussteht, Datei inhaltlich neuer | Snapshot-Text → Fassung, Datei bleibt (I1) | synchron |
| geschlossen | Öffnen | Lesen ok | Stand + Basis in einem Render | synchron |
| geschlossen | Öffnen | Lesefehler | Meldung, Editor gesperrt | Ladefehler |
| Ladefehler | „Erneut versuchen“ | – | `read` | wie Öffnen |
| synchron | Eingabe | – | Autosave in 500 ms | geändert |
| synchron | `refresh`-Auslöser | Version ≠ Basis | ganze Übernahme (kein Konflikt möglich, O = B) | synchron |
| geändert | `refresh`-Auslöser | Version ≠ Basis, kein Textkonflikt | ganze Übernahme, Eingaben bleiben | geändert |
| geändert | `refresh`-Auslöser | Textkonflikt | nichts übernehmen, Autosave anstoßen | geändert |
| geändert | Speichern | gelungen, keine Eingabe während des Auftrags | `onSaved`/`onAdopt` | synchron |
| geändert | Speichern | gelungen, Eingabe während des Auftrags | `onSaved`/`onAdopt`, Eingabe bleibt | geändert |
| geändert | Speichern | Textkonflikt | `lost` sichern, Oberflächentext schreiben, `onAdopt` | synchron, nur ohne weitere Eingabe |
| geändert | Speichern | Rust `conflict` | frisch lesen, neu mischen (≤ 3, Sicherung dedupliziert) | wie oben / Fehler |
| geändert | Speichern | Schreibfehler | `PageSaveError`, Meldung, Oberfläche hält Stand | geändert |
| geändert / synchron | Speichern oder `refresh` | Lesefehler | `onUnreadable`, nichts schreiben | Datei unlesbar |
| Datei unlesbar | Eingabe / „Erneut versuchen“ | Lesen gelingt | normal mischen und schreiben | synchron |
| Datei unlesbar / geändert | Seite verlassen, Sichern scheitert | – | Snapshot mit Basis (`kept`) | Rettung |
| Rettung | Öffnen | erneutes Schreiben scheitert | `read` liefert `unsaved` + Snapshotbasis | geändert (ausgehändigt) |
| beliebig | Speichern | > 64 MiB | `too_large`, Datei bleibt, Meldung | unverändert |
| beliebig | Sync `apply_page` | lokal nicht interpretierbar | Seite überspringen | unverändert |

## 6 Abnahme je Paket (deterministisch)

Zeitmessungen sind Leistungsbelege, keine Korrektheitstests; entfernte `test.fail`-Marker allein belegen die
Invarianten nicht. Jede Gruppe enthält eine **Mutationsprobe**: die genannte Stelle im Code entfernen → die Tests
müssen rot werden (im PR protokolliert, nicht eingecheckt).

**A (Rust, `commands/pages.rs` und `sync/collect.rs` `mod tests`, Tempordner):** synchronisierte Threads mit
`std::sync::Barrier`: genau ein CAS-Gewinner; Sync (`apply_page`-Hilfe) zwischen Lesen und Schreiben → Konflikt;
Fehler beim Lesen (Verzeichnis namens `state.json`), beim Temp-Schreiben und beim Ersetzen (Ziel ist Verzeichnis)
→ Datei unverändert; kein UTF-8 → `unreadable`; Rohtext und Parsefehler → Rohbytes-Konfliktkopie bytegleich;
nur `einfuegungen` lokal geändert → Konfliktkopie; Größe Grenze−1/Grenze/Grenze+1 in Bytes mit Mehrbytezeichen
an der Grenze; `EMPTY_VERSION` = SHA-256(""); unlesbare Datei auch mit `expected=None` nicht überschrieben;
`collect` hält unlesbare Seite zurück. Mutation: Versionsvergleich oder Fehlerweitergabe in `write_state`
entfernen. Befehl: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib pages` und
`--lib sync::collect` → ok. Node: Store-Aufträge übergeben `expected`, Attrappe meldet `"conflict"` →
Wiederholung. Playwright: Ladefehler → Meldung, kein `page_state_save` für die Seite.

**B (Node `scripts/page-versions.test.mjs`, an `test:paste` angehängt; verzögerbare `io` und
Übernahme-Rückrufe):** Tabelle der Feldregeln inkl. `null`/fehlend/Schlüsselentfernung; `mergeText`-Identitäten
vor Spannen, Berühren = Konflikt, Surrogatpaar, stabile Wiederholung (V14), Anhängen nach CAS-Abweisung genau
einmal; unbekanntes Feld + unveränderte Version +
Texteingabe + Autosave → Feld bleibt (V5); Eingabe während Speicherung bleibt; zwei aufeinanderfolgende
Fremdänderungen nach Textkonflikt (summary A→B→C) → C bleibt; Seitenwechsel vor Rückruf → Basis der neuen Seite
unberührt; drei CAS-Konflikte → genau eine Fassung je Konflikttext, danach `PageSaveError`; offene Seite wird
unlesbar → kein Schreiben, Eingaben bleiben, Wiederaufnahme nach Lesbarkeit; Fall 1 und Fall 4 als Rennen; alle
bestehenden `pageState:`-Tests in `scripts/paste-cleanup.test.mjs` grün. Mutation: Basis vorzeitig fortschreiben;
Dateifelder durch Oberflächenwerte ersetzen. Playwright: Attrappe in `editor-paste-cleanup.spec.ts` Z. 78 prüft
`expected`; Marker Fall 1 fällt ohne Änderung der Erwartung; neu: offene Seite + `mutate` → Editor zeigt Anhang,
vorherige Eingabe bleibt; `sync-changed` mit geändertem Speicher → Übernahme; Textkonflikt →
`book_ai_document_import` mit Dateitext, Datei = Editor.

**C (Node + Playwright `library.spec.ts`):** Fremdänderung vor Rollback → bleibt; unsauberer Rollback → kein
zweiter Anhang (auch nach CAS-Wiederholung); Fremdänderung des Ziels vor `storeJoined` → im Ergebnis; Teilseite
unlesbar → Abbruch, keine Quelle entfernt; Zielschreiben scheitert → keine Quelle entfernt; Quelle nach dem Lesen
geändert (Zustand) → bleibt Seite; Titel oder Projektdatei nach dem Lesen geändert → im `_geloescht`-Ordner
vollständig. Mutation: Versionsvergleich in `pages_retire` oder Abbruch bei Teilfehler entfernen. Beide Marker
fallen ohne Änderung der Erwartungen; `grep -rn "lva#546" tests` → leer.

## 7 Paketschnitt

| PR | Scope | hängt ab |
|---|---|---|
| **A – #545, Dekodierung, Rust-CAS** (M) | `read_state_bytes`, `write_state`, `state_version`, `EMPTY_VERSION`, `MAX_STATE_BYTES`; `page_state_load` Fehler statt leer; `page_state_save(…, expected)`; alle Rust-Schreiber über `write_state` (§2.6); `pages_differ` erweitert; Rohbytes-Konfliktkopie; `collect` hält Unlesbares zurück; Export bricht ab; Store-Aufträge mit `expected` + Wiederholung (ohne Oberflächenbasis); Neuanlagen mit `EMPTY_VERSION`; Ladefehler-Zustand; `bindings.ts` neu (`cargo run --example bindings_export -- ../src/bindings.ts`). **Garantie nach A:** wie §2.6 „Übergang“. | – |
| **B – Basis, Mischung, Übernahme** (L) | `pageMerge.ts`; `pageState.ts` §2.8 inkl. Ablösung von `seq`/`known`; `pageStore.ts`; `TtsSettings` (`pageBase`, `applyPageState`, Übernahme, Zustand „Datei unlesbar“, `refresh`-Auslöser); Attrappe mit CAS. **Garantie nach B:** Fälle 1, 4, 5 geschlossen. | A |
| **C – veraltete Eingaben der Bibliothek** (M) | Rollback (§4.2), `merge.ts` Neubau und Abbruch, `pages_retire`. **Garantie nach C:** alle Fälle, V11. | B (`mergeState`) |

## 8 Risiken

- **Basis-Disziplin (V6):** `TtsSettings.tsx` hat 20 `setText(`-Stellen; B ordnet jede zu (Eingabe = „ours“, kein
  Basiswechsel; Dateiinhalt = über `applyPageState` mit Basis). Review-Punkt.
- **Kosten:** je Schreiben ein SHA-256 in JS und ein Lesen + SHA-256 in Rust über bis zu 64 MiB beim
  500-ms-Autosave. Keine Zahl ohne Messung: A misst eine 17-MiB-Seite (Release, Windows) und legt das Ergebnis in
  den PR. Eine Abkürzung über Länge/mtime ist ausgeschlossen (V2); denkbar wäre nur ein Cache, der nachweislich bei
  jedem Schreiben invalidiert wird – nicht Teil dieses Entwurfs.
- **Mehr Konflikte:** Spannenverfahren und erweitertes `pages_differ` erzeugen mehr Fassungen bzw. Konfliktkopien,
  nie Verlust.
- **Tauri-IPC** mit sehr großen Strings ist nicht gemessen; die Rust-Rundreise prüft nur die Datei.
- **I1-Tests:** Tests mit Rettungserwartung ersetzen den Text ganz → Konflikt → gleiche Ausgabe. Ändert ein Test
  nicht überlappend, ist die neue Ausgabe (gemischt, keine Fassung) gewollt; Anpassung nur mit Begründung im PR.

## 9 Entscheidungen (nach Codex R1)

| | Entscheidung | Stand |
|---|---|---|
| E1 | Spannenverfahren ohne Abhängigkeit; diff3 (`node-diff3` MIT, jsdiff BSD-3-Clause) erst bei Bedarf | tragfähig |
| E2 | Oberfläche gewinnt; Sicherung **vor** dem erfolgreichen Überschreiben; `held` bleibt flüchtig und gemeldet | tragfähig |
| E3 | Unsauberer Rollback (Definition §4.2): kein automatischer Neuversuch, Meldung | tragfähig |
| E4 | 64 MiB UTF-8-Bytes beim Schreiben; Lesen/Export ohne Grenze; keine Leistungszusage | Startwert |
| E5 | Bedingtes, wiederherstellbares Entfernen der Quellen (`pages_retire`) in C | tragfähig mit V11 |
| E6 | Kein neues Ereignis; Erkennungszeitpunkte §3.1 | tragfähig |
| E7 | `write_state` wird in I2 auch für `inhalt/aktuell.json` genutzt (Revision dort ebenfalls Hash) | Empfehlung |
| E8 | Ansichtsfelder `tab`, `kiDoc` führt die Oberfläche; beim Sync dürfen nur sie ungesichert verloren gehen | neu, Empfehlung |

## 10 Codex R1 → Änderung

| Codex R1 | Änderung (Abschnitt) |
|---|---|
| B1 unbekannte Felder im Autosave | V5; §2.4 Feldklasse „außerhalb `fields`“ und „Schnellpfad = derselbe Weg“; §3.1 Speichern; Test §6 B |
| B2 Teilübernahme mit alter Basis, `tab` | V6; §2.4 Ansichtsfelder; §3.1 keine Teilübernahme in `refresh` und `onAdopt`; Test „summary A→B→C“ §6 B |
| B3 Sync überschreibt Rohfassungen | V7 auf JS-Schreiber begrenzt; V8; §2.7 `pages_differ`; Test §6 A |
| B4 Altformat/Parsefehler | V9; §2.3 Dekodierungstabelle mit Rohbytes-Pfad; Tests §6 A |
| B5 V6 bei offener Seite | V10; §3.2 Ladefehler vs. offene Seite; §5 Zustand „Datei unlesbar“; Test §6 B |
| B6 CAS-Geltungsbereich, `null` | V1; §2.6 Schreiberliste mit Übergang je Paket; §7 Garantien; kein TS-Aufrufer mit `null` |
| B7 mtime-Abkürzung | V2; §8 Kosten (gestrichen) |
| B8 Löschen schützt nur state.json | V11; §4 Fall 3 `pages_retire` (Version + wiederherstellbar); Tests §6 C |
| H Bytehash | §2.1 (Bytehash, keine Normalisierung, Kollision, leer = fehlend) |
| H kanonischer Vergleich, Identitäten | §2.4 „Vergleich“, `mergeText` |
| H Algebra | V14; §2.4 „Algebra“; Test §6 B |
| H Sperren, Teilwirkung | §2.2 „Sperren“, „Teilwirkung“ |
| H Fenster/Prozesse | §2.2 „Prozesse“; §3.1 Erkennungszeitpunkte |
| H Rettung | V13; §2.8 `securedInJob`, `secured` |
| H Zustandstabelle | §5 (geändert + refresh, aufgelöste Rettung, Rettung nach Fremdänderung, Eingabe während Speichern; „synchron nur ohne weitere Eingabe“) |
| H Kompatibilität, 64 MiB vs. 3 MiB | §2.7 „Kompatibilität“; V12 |
| E1–E6 | §9 |
| Deterministische Abnahme | §6 (Barrieren, Fehlerstellen, Grenzwerte, Mutationsproben) |
