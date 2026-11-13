# Seitenzustand: Versionen und Textkonflikte (lva#546, lva#545)

Stand 13.11.2026, Entwurf auf `main` ccb97418 (nach #549). Vorbedingung für Stufe I2 des Inhalte-Moduls
(`2026-11-12-inhalte-modul.md` §3.1, §13). Ziel: Das Modell für alle Schreiber von `state.json` wird **einmal**
festgelegt, damit es am Stück prüfbar ist und nicht Rennen für Rennen nachgebessert werden muss.

## 0 Ausgangslage (Code, nicht Annahme)

- **JS-Kette** (`src/lib/reader/pageState.ts`, `pageStore.ts`): ein Store, eine serielle Kette. Jeder Auftrag liest
  frisch und schreibt ganz (`commitLog`, `save`, `mutate`). Laufnummern `seq`/`lastWrite` entscheiden, ob ein
  Rettungssnapshot (`failedSaves`) veraltet ist. `einfuegungen` kommt immer aus der Datei. Ein Textkonflikt
  wird bisher nur für ausgehändigte Snapshots behandelt (`secureOverwrittenFile`: Oberfläche gewinnt, Dateitext
  wird Fassung über `bookAiDocumentImport`).
- **Lücken:** `read` liest außerhalb der Kette und ohne Basis. `save` schreibt für die offene Seite `live.build()`
  ungeprüft über den Text der Datei. `mutate`-Aufrufer arbeiten mit vorher gelesenen Werten (Rollback in
  `useLibrary.ts` Z. 417, `joined.text` in `merge.ts` Z. 355). Die Laufnummern sehen keine Rust-Schreiber.
- **Rust:** `page_state_save` schreibt unter `pages::lock()` blind (`pages.rs` Z. 564). `page_state_load` macht aus
  jedem Lesefehler `""` (Z. 559, #545). Der Sync schreibt in `collect.rs` Z. 187 (`apply_page`) unter derselben
  Sperre, aber an der JS-Kette vorbei. `collect`/`read_page` machen aus Lesefehlern `Value::Null`, der
  Seitenexport (`pages_package.rs` Z. 123/215) macht daraus einen leeren Zustand.

## 1 Invarianten (nummeriert, je mit Prüfstelle aus §6)

- **V1 – Nur gegen eine gelesene Version schreiben.** Jedes Schreiben über `page_state_save` nennt die Version,
  die der Schreiber gelesen hat. Rust vergleicht sie unter der Seitensperre mit der Datei und schreibt nur bei
  Gleichheit (Compare-and-Swap). Ausnahmen, ausdrücklich: Neuanlage einer frisch erzeugten Kennung (`null`)
  und `apply_page` des Sync (eigene Konfliktlogik unter derselben Sperre, §2.5).
- **V2 – Kein stilles Überschreiben eines neueren erfolgreichen Textstands.** Weicht die Datei von der Basis
  ab, wird gemischt (§2.3). Ein nicht mischbarer Text verschwindet nur aus der Datei, nachdem er vollständig als
  Fassung gesichert oder im Speicher gehalten **und** gemeldet ist (bestehende Wege `saveCopy`/`hold`/`onRescue`).
- **V3 – Ungesicherte Eingaben bleiben.** Eine Übernahme fremder Änderungen ersetzt nie Text der Oberfläche,
  der seit ihrer Basis geändert wurde. Bei Textkonflikt gewinnt die Oberfläche (I1-Regel), der Dateitext wird
  nach V2 gesichert.
- **V4 – Die Basis ist Vorfahr.** Die Basis der Oberfläche ist ein Dateistand, von dem der angezeigte Stand
  abstammt. Basis und Seitenzustand wechseln nur **im selben Render** (eine React-Zustandsänderung, gebündelt).
- **V5 – Rohfassungen wie in I1, unverändert.** Die Datei ist die einzige Quelle für `einfuegungen`; nur
  `commitLog` ändert das Feld; jede Mischung übernimmt den Dateiwert; eine Verweigerung („voll“) schreibt nichts.
- **V6 – Lesefehler ≠ leerer Zustand (#545).** Nur „Datei fehlt“ ist leer. Jeder andere Lesefehler (Rechte,
  E/A, kein UTF-8, Pfad ist kein File) ist ein Fehler. Auf einen gescheiterten Lesevorgang folgt kein Schreiben
  (JS-Kette, Autosave, Sync, Export); die Oberfläche zeigt den Fehler und sperrt den Autosave dieser Seite.
- **V7 – Größe.** `page_state_save` lehnt mehr als `MAX_STATE_BYTES` (64 MiB) ab, die Datei bleibt. Lesen hat
  keine Grenze. Größtes Rohfassungsprotokoll (16 MiB JSON) plus 1 MiB Text übersteht die Rundreise.
- **V8 – Rettungswege bleiben.** `failedSaves`, `held`, `saveCopy`, `onRescue` und ihre Meldungen bleiben in
  Bedeutung und Text; geändert wird nur, **woran** „Datei ist neuer“ erkannt wird (§2.6).
- **V9 – Format unverändert.** `state.json` bekommt kein neues Feld. Keine Migration, Sync-Objekte und ältere
  Apps/Geräte sehen dieselben Bytes wie heute.
- **V10 – Wiederholbar.** Jede Änderungsfunktion (`change` von `mutate`, `LogOp`) ist auf dem frischen Stand
  rein und darf nach einem CAS-Konflikt erneut laufen (höchstens 3 Versuche je Auftrag).

## 2 Versionsmodell

### 2.1 Wo die Version lebt: Hash der Bytes, kein Feld `rev`

**Version = SHA-256 (hex, klein) der Bytes von `state.json`;** fehlende Datei = Version von `""`
(`EMPTY_VERSION`, in Rust und TS gleich, per Test belegt). JS berechnet sie mit dem vorhandenen
`sha256Hex` (`pasteLog.ts`) über den gelesenen Text; Rust über die Datei. `read_to_string` ist verlustfrei für
gültiges UTF-8 (BOM inklusive), also stimmen beide überein.

Bewertung `rev`-Feld gegen Hash:

| | Feld `rev` in state.json | Hash der Bytes (gewählt) |
|---|---|---|
| Schreiber, die `rev` nicht kennen (Sync älterer Geräte, ältere App, Test-Attrappen, Hand) | unbemerkt, `rev` bleibt stehen | jede Byteänderung zählt |
| Sync | `rev` wird mitsynchronisiert, Zähler kollidieren zwischen Geräten | keine Änderung am Objekt |
| Migration | alte Dateien ohne `rev` brauchen Regel | keine |
| Kosten | Zähler | ein SHA-256 je Lesen/Schreiben (Größe in Paket A messen, siehe Risiken) |
| ABA (gleicher Inhalt kehrt zurück) | erkannt | gilt als unverändert – harmlos, Inhalt identisch |

### 2.2 Wo geprüft wird: CAS in Rust unter der Seitensperre

Eine Prüfung nur in JS (lesen, vergleichen, schreiben) hat ein Fenster, in dem `apply_page` (Sync) oder ein
zweites Fenster mit eigenem Store schreibt; die JS-Kette kennt beide nicht. Nur ein Vergleich **unter
`pages::lock()`** ist atomar gegenüber allen Schreibern im Prozess (Sync, Konfliktkopie, Paketimport,
`page_state_save` jedes Fensters). Deshalb:

```rust
// commands/pages.rs
pub(crate) const MAX_STATE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) fn state_version(bytes: &[u8]) -> String;                 // sha2, schon Abhängigkeit
/// Ok(None): Datei fehlt. Err("page_state:unreadable: …"): jeder andere Fehler (#545).
pub(crate) fn read_state(dir: &Path) -> Result<Option<String>, String>;
/// Hält die Seitensperre. expected=None schreibt ohne Vergleich (Neuanlage, Sync), aber nie über eine
/// unlesbare Datei. Fehler: "page_state:conflict", "page_state:unreadable", "page_state:too_large".
pub(crate) fn write_state(dir: &Path, bytes: &[u8], expected: Option<&str>) -> Result<String, String>;

#[tauri::command] pub fn page_state_load(app, id: String) -> Result<String, String>;   // Signatur gleich
#[tauri::command] pub fn page_state_save(app, id: String, state: String,
                                          expected: Option<String>) -> Result<(), String>;
```

`page_state_load` bleibt ein `String` (14 Spec-Dateien fangen den Befehl mit Rohtext ab; Rust-Aufrufer
`app_host`, `hoersync_live`, `voice_tools`, `app_context` bleiben unverändert). `page_state_save` bekommt nur
den zusätzlichen Parameter; Attrappen, die ihn ignorieren, verhalten sich wie bisher. Fehler als Codes im Stil
`book_ai:…`. `write_state` nutzen auch `apply_page`, `conflict_copy` und `pages_import` (Größengrenze, V6).
Rust bleibt schemafrei (Moduldoku `pages.rs`): gemischt wird nur in JS.

### 2.3 Mischung bei Versionskonflikt (`src/lib/reader/pageMerge.ts`, rein, neu)

```ts
export type Prefer = "ours" | "theirs";
export interface MergeResult { state: StateObject; textConflict: boolean; lostText: string | null }
export function mergeState(base: StateObject, ours: StateObject, theirs: StateObject,
  opts: { prefer: Prefer; fields?: readonly string[] }): MergeResult;
export function mergeText(base: string, ours: string, theirs: string):
  { text: string } | { conflict: true };
export const OWN_FIELDS = ["text","summary","kiDoc","sourceUrl","tab","voices","voicesChosen",
  "lang","translatedLangs","autoTag"] as const;   // = pageStateJson() in TtsSettings
```

Feldregeln (B Basis, O ours, T theirs/Datei; Vergleich per kanonischem JSON):

| Feld | Regel |
|---|---|
| `einfuegungen` | immer T (V5) |
| Felder außerhalb `fields` (Standard `OWN_FIELDS`) | immer T – behebt nebenbei, dass der Autosave unbekannte Felder neuerer Geräte löscht |
| Skalare/Objekte (`summary`, `kiDoc`, `sourceUrl`, `tab`, `lang`, `autoTag`) | O=B → T; T=B → O; O=T → O; sonst `prefer` |
| `translatedLangs` | Mengen-3-Wege: (B∩O∩T) ∪ (O∖B) ∪ (T∖B), Reihenfolge von T, dann neue aus O |
| `voices`, `voicesChosen` | je Schlüssel wie Skalar; Schlüsselkonflikt → `prefer` |
| `text` | `mergeText`; Konflikt → Text von `prefer`, der andere geht als `lostText` an V2 |

`mergeText` ohne neue Abhängigkeit, O(n): je Seite die geänderte Spanne gegen B über gemeinsamen Präfix/Suffix
(Grenzen nicht in einem Surrogatpaar). Gleiche Änderung beidseitig → sauber. Spannen getrennt (strikt, ohne
Berührung) → beide anwenden. Überlappen oder berühren → Konflikt. Deckt den Hauptfall (Anhängen am Ende,
Bearbeiten in der Mitte) ab; verstreute Änderungen (Sprecher ersetzen) werden eher Konflikt – sicher, nur mit
mehr Fassungen (offene Entscheidung E1).

### 2.4 Wer welche Politik bekommt

| Schreiber | Basis | `prefer` bei Konflikt | gesichert wird |
|---|---|---|---|
| Autosave/Export/KI-Dokument der offenen Seite | Basis der Oberfläche (V4) | ours | Dateitext → Fassung |
| ausgehändigter Rettungssnapshot (Lade-Autosave) | Basis des Snapshots | ours | Dateitext → Fassung (wie I1) |
| nicht ausgehändigter Snapshot (`resolveFailed`) | Basis des Snapshots | – (I1: kein Mischen) | Snapshot-Text → Fassung, Datei bleibt |
| `mutate` (Bibliothek, Fassungen, Sprecher, Migration) | keine: Änderung auf frischem Stand | – | – |
| Rollback Anhängen | eigener geschriebener Stand | theirs | nichts (Systemwert, rekonstruierbar) |
| Sync `apply_page` | `synced_hash` (Bestand) | Konfliktkopie als Seite (Bestand) | – |

### 2.5 Sync (Rust) und Kompatibilität

`apply_page` bleibt fachlich gleich (Vergleich mit `synced_hash` und Konfliktkopie unter der Sperre), schreibt
aber über `write_state(…, None)` und **verweigert** das Überschreiben einer unlesbaren lokalen Datei (Fehler für
diese Seite, nächster Lauf versucht erneut). `collect` hält unlesbare Seiten wie `held` zurück statt `state:
null` hochzuladen. Eine Altformat-Datei (kein JSON) wird von `conflict_copy` als Bytes kopiert statt verworfen.
Die JS-Seite bemerkt den Sync-Stand über die Version (nächstes Schreiben mischt, §3) und über das bestehende
Ereignis `sync-changed` (Übernahme, §3). Kein neues Ereignis.
Ältere App-Versionen und Geräte: gleiche Dateien, gleiche Sync-Objekte (V9); sie prüfen nicht, ihre
Schreibvorgänge erkennt diese Version am Hash. Ein Doppel aus Konfliktkopie (Sync) und Fassung (Mischung) ist
möglich und gewollt (nichts geht verloren).

### 2.6 Store-Änderungen (`pageState.ts`)

- `io.load(page)` → `string | null` wie bisher; der Store berechnet `version`. `io.save(page, json, expected)` →
  `"ok" | "conflict" | "failed" | "too_large"`. Prüfung `utf8Bytes(json) > PAGE_STATE_MAX_BYTES` vor dem Aufruf.
- `PageBase = { version: string; raw: string }`. `PageRead` bekommt `base: PageBase`.
- `PageStateLive.build(): { json: string; base: PageBase | null }` (Stand **und** Basis desselben Renders, V4).
  `save(page, snap: { json; base })` statt `save(page, snapshot)`.
- Jeder Auftrag schreibt mit `expected = version(frisch gelesen)`; bei `conflict` frisch lesen und neu rechnen,
  höchstens 3-mal, dann `PageSaveError("conflict", kept)` wie ein Schreibfehler (V10).
- `FailedSave` bekommt `base: PageBase`; `seq`, `lastWrite`, `newerWrite`, `known` entfallen. „Datei ist neuer“
  heißt: `T` unterscheidet sich von `base` in einem Feld außer `einfuegungen` (ein reines Protokollschreiben
  zählt wie in I1 nicht). Nach dem Sichern eines Dateitexts als Fassung wird die Basis des Snapshots auf diesen
  Dateistand gesetzt (ersetzt `known`, verhindert Doppelfassungen).
- Neu: `refresh(page)` (Kettenauftrag, schreibt nie) und Rückruf `onAdopt(page, a: Adoption)` mit
  `Adoption = { from: string /*O beim Bau*/; to: string /*neuer Stand*/; base: PageBase | null }`.
  `bindPageStateLive(view, { onCommit, onRescue, onAdopt })` statt drei Positionsparametern.
- `mark`/`changedSince` (Anzeige des Protokolls) bleiben unverändert.

## 3 Editor übernimmt fremde Änderungen (Fall 4)

**Auslöser:** (a) nach jedem gelungenen `mutate` auf die offene Seite (`afterWrite` stellt `refresh` ein),
(b) `sync-changed` in `TtsSettings` (zusätzlich zu `reloadPages`), (c) KeepAlive wieder sichtbar – ersetzt den
heutigen Block mit `savePending`/`setText` (`TtsSettings.tsx` Z. 3046–3080).

**`refresh(page)` in der Kette:** nicht offen → nichts. `T` frisch lesen (Fehler → nichts, V6). `{json: O, base: B}
= live.build()`. `version(T) = B.version` → nichts. Sonst `m = mergeState(B, O, T, {prefer:"ours"})`:
- kein Textkonflikt → `onAdopt({from: O, to: m.state, base: T})`;
- Textkonflikt → `onAdopt({from: O, to: m.state /*Text = O*/, base: null})`: Nichttext-Felder werden übernommen,
  die Basis bleibt B; der nächste Autosave stellt denselben Konflikt fest und sichert den Dateitext (V2), bevor er
  schreibt. `refresh` selbst sichert und schreibt nie.

**Autosave (`save` der offenen Seite):** `{json: O, base: B} = live.build()` beim Ausführen, `T` frisch.
`version(T) = B.version` → `O` (mit `einfuegungen` aus T) gegen `version(T)` schreiben; danach
`onSaved(page, {from: B, base: geschrieben})`: die Oberfläche setzt die Basis nur, wenn ihre Basis noch B ist.
Sonst `m = mergeState(B, O, T, {prefer:"ours"})`, `lostText` → `secureText` **vor** dem Schreiben, `m.state`
gegen `version(T)` schreiben, dann `onAdopt({from: O, to: m.state, base: geschrieben})`.

**In der Oberfläche (`TtsSettings`):** Basis ist React-Zustand `pageBase`, gesetzt nur zusammen mit dem
Seitenzustand: beim Laden (aus `PageRead.base`), in `onSaved`, in `onAdopt`. `onAdopt` mischt synchron
`local = mergeState(from, aktuell, to, {prefer:"ours"})` – so bleiben Eingaben zwischen Bau und Übernahme
erhalten – und setzt alle Felder in **einem** Handler über eine gemeinsame `applyPageState` (aus dem
Lade-Handler Z. 1173–1278 herausgezogen; Lade- und Übernahmeweg teilen sie, Tab/Modus werden bei der Übernahme
nicht angefasst). Hat `local` selbst einen Textkonflikt, bleibt der Text der Oberfläche und die Basis alt.
Textänderung im fokussierten Editor: die fremde Spanne per `setSelectionRange` + `insertText` (Muster
`TtsChipEditor.tsx` Z. 530–547) – Cursor und Rückgängig bleiben; verdeckt oder ohne Fokus `setText`.

**Rettungssnapshots/KeepAlive:** Ein ausgehändigter Snapshot (`read.unsaved`) kommt mit seiner Basis; der
Lade-Autosave mischt gegen die Datei (statt pauschal zu überschreiben), der Snapshot fällt nach dem gelungenen
Schreiben weg (wie I1). Solange ein ausgehändigter Snapshot aussteht, liefert `refresh` nichts, was ihn ersetzt
(Mischung mit `prefer:"ours"`, V3).

## 4 Die Fälle einzeln: alt → neu

1. **Ladende Seite.** Alt: `read` liest ohne Basis, ein `mutate` schreibt „Fremd“, die verspätete Ladeantwort
   „Vorher“ landet in der Oberfläche, der Lade-Autosave schreibt sie blind zurück. Neu: Die Ladeantwort trägt
   `base = Vorher`; der Autosave liest „Fremd“, Version ≠ Basis, `mergeState(Vorher, Vorher, Fremd)` = Fremd,
   geschrieben wird „Fremd“, die Oberfläche übernimmt per `onAdopt`.
2. **Anhängen + Rollback** (`useLibrary.ts`). Alt: Rollback löscht alle Felder und setzt den alten Stand – eine
   Zwischenänderung geht verloren. Neu: `restore` merkt `{vorher, geschrieben}`; die Rollback-Änderung auf dem
   frischen Stand ist `mergeState(geschrieben, vorher, frisch, {prefer:"theirs", fields: alle außer
   einfuegungen})`. Unverändert seit dem Anhängen → alter Stand; fremd geändert → der fremde Stand bleibt.
   Gemischt wurde nicht sauber → kein automatischer Neuversuch des Anhängens (sonst doppelter Text), sondern
   `giveUp` mit Meldung (E3).
3. **Zusammenführen** (`merge.ts`). Alt: `joined.text` aus vorher gelesenen Texten ersetzt den Zieltext.
   Neu: `joinedText` merkt je Teilseite den gelesenen Text. In `storeJoined` baut die `mutate`-Änderung den
   gemeinsamen Text neu, wenn der frische Zieltext abweicht (`joinTranscripts` mit dem frischen Text als Teil
   des Ziels); `sync` kommt aus derselben Rechnung zurück (Muster `outcome` wie in `useLibrary`). Quellseiten
   werden nur gelöscht, wenn ihre Version noch der gelesenen entspricht (`pages_delete(id, expected)`, gleiche
   CAS-Hilfe); sonst bleiben sie mit Meldung. Ein Lesefehler einer Teilseite bricht das Zusammenführen ab
   (heute: Teil fehlt still und die Seite wird gelöscht).
4. **Offene Seite.** Alt: Fremde `mutate`-Änderung steht in der Datei, der Editor zeigt sie nicht, der nächste
   Autosave überschreibt sie. Neu: §3 – `refresh` nach `mutate`, Übernahme in den Editor, ungesicherte Eingaben
   bleiben; selbst ohne `refresh` mischt der nächste Autosave.
5. **Geräte-Sync** (`collect.rs` Z. 187). Alt: Sync ersetzt die Datei, die Oberfläche überschreibt sie beim
   nächsten Autosave mit ihrem alten Stand (der Sync-Stand ist danach nur noch auf dem Server bzw. als
   Konfliktkopie anderswo). Neu: Der Autosave erkennt die fremde Version (Rust-CAS schließt das Fenster zwischen
   JS-Lesen und -Schreiben) und mischt; `sync-changed` stößt die Übernahme an.
6. **#545 Lesefehler.** Alt: `unwrap_or_default` → `""` → Oberfläche lädt „leer“ (inkl. Altbestands-Übernahme
   aus `localStorage`), der Lade-Autosave überschreibt Text und `einfuegungen`. Neu: `page_state_load` → Fehler,
   `store.read` → `null`, die Oberfläche zeigt „Seite konnte nicht gelesen werden“ mit „Erneut versuchen“,
   Editor schreibgeschützt, `pageLoaded` bleibt `false` (kein Autosave). Ebenso: `mutate`/`commitLog`
   schreiben nicht (Bestand), Sync lädt nicht hoch und überschreibt nicht (§2.5), Export bricht ab
   (`pages_package.rs`, `versions.loadPageState` wirft statt leer).

## 5 Zustände und Übergänge (eine offene Seite)

| Zustand | Ereignis | Bedingung | Aktion | Folge |
|---|---|---|---|---|
| geschlossen | Öffnen (`read`) | Snapshot steht aus, nicht ausgehändigt | `resolveFailed` (§2.4) | weiter mit Lesen |
| geschlossen | Öffnen | Lesen ok | Stand + `base` in einem Render setzen | synchron |
| geschlossen | Öffnen | Lesefehler | Meldung, Editor gesperrt | Lesefehler |
| Lesefehler | „Erneut versuchen“ | – | `read` | wie Öffnen |
| synchron (UI = Basis) | Eingabe | – | Autosave in 500 ms | geändert |
| synchron | fremdes Schreiben / `sync-changed` / sichtbar | Version ≠ Basis | `refresh` → `onAdopt` (kein Konflikt möglich, O = B) | synchron (neue Basis) |
| geändert | Speichern | Version = Basis | schreiben (CAS) → `onSaved` | synchron bzw. geändert |
| geändert | Speichern | Version ≠ Basis, Text mischbar | mischen, schreiben → `onAdopt` | geändert/synchron |
| geändert | Speichern | Textkonflikt | Dateitext → Fassung (`onRescue`), UI-Text schreiben → `onAdopt` | synchron |
| geändert | Speichern | Rust meldet `conflict` | frisch lesen, neu mischen (≤ 3) | wie oben / Fehler |
| geändert | Speichern | Schreib-/Lesefehler, Seite offen | `PageSaveError`, Meldung, UI hält Stand | geändert |
| geändert | Seite verlassen + Speichern scheitert | – | Snapshot mit Basis (`kept`) | Rettung |
| Rettung | Öffnen | erneutes Schreiben scheitert | `read` liefert `unsaved` + Basis des Snapshots | geändert (ausgehändigt) |
| beliebig | Speichern | > 64 MiB | `too_large`, Datei bleibt, Meldung | unverändert |
| geschlossen | Sync `apply_page` | lokal unlesbar | Seite überspringen | geschlossen |

## 6 Testplan

**Node** (`scripts/page-versions.test.mjs`, neu; in `package.json` an `test:paste` angehängt; die bestehenden
`pageState:`-Tests in `scripts/paste-cleanup.test.mjs` bleiben unverändert grün):
- `mergeState`: Tabelle der Feldregeln je Feld (O=B, T=B, O=T, beide verschieden × `prefer`), Mengenregel,
  Schlüsselregel, unbekannte Felder aus T, `einfuegungen` immer aus T (V5).
- `mergeText`: Anhängen vs. Mitte (sauber), Überlappen und Berühren (Konflikt), gleiche Änderung (sauber),
  Surrogatpaar an der Grenze, 3-MiB-Text linear (Zeitgrenze wie `joinFragments`-Test).
- Store mit Attrappe `io` (Datei als Variable, `expected` echt geprüft): Fall 1 (verspätete Ladeantwort +
  `mutate`, danach `save` → Datei „Fremd“, `onAdopt`); Fall 4 (offene Seite, `mutate` → `refresh` → `onAdopt`,
  Eingabe zwischen Bau und Übernahme bleibt); Fall 5 (Datei an der Kette vorbei ändern → `save` mischt);
  CAS-Konflikt aus `io.save` (zwischen Lesen und Schreiben ändert die Attrappe die Datei) → Wiederholung, nach 3
  `PageSaveError`; Textkonflikt → `saveCopy` mit Dateitext vor dem Schreiben, danach Datei = Oberfläche;
  nicht ausgehändigter Snapshot + Inhaltsänderung → Fassung, Datei bleibt; + nur Protokolländerung →
  Rückschreiben; Lesefehler → kein `io.save` in `save`/`mutate`/`commitLog`/`refresh` (V6); zu groß → kein
  `io.save` (V7); `EMPTY_VERSION` = `sha256Hex("")`.

**Rust** (`commands/pages.rs` `mod tests`, Tempordner; `collect.rs` über Ordner-Hilfen):
`zustand_fehlt_ist_leer`, `zustand_unlesbar_ist_fehler` (Verzeichnis namens `state.json`; ungültiges UTF-8),
`schreiben_mit_passender_version`, `fremde_version_ist_konflikt_datei_bleibt`, `cas_zwei_threads_genau_einer_gewinnt`,
`leere_version_ist_sha256_leer`, `rundreise_groesstes_protokoll` (16 MiB JSON-Protokoll + 1 MiB Text, Bytes
gleich), `zu_gross_wird_abgewiesen`, `unlesbare_datei_wird_nie_ueberschrieben` (auch `expected=None`),
`sync_haelt_unlesbare_seite_zurueck`, `konfliktkopie_altformat_bytegleich`.
Befehl: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib pages` und `--lib sync::collect`.

**Playwright:** Die Attrappe in `editor-paste-cleanup.spec.ts` (Z. 78) prüft `expected` (SHA-256 des
gespeicherten Texts) und wirft `page_state:conflict`. Es fallen alle drei `test.fail`-Marker „lva#546“ ohne
Änderung der Erwartungen: `editor-paste-cleanup.spec.ts` „a foreign text change during a page load…“,
`library.spec.ts` „Anhängen scheitert und rollt zurück: …behält ihren Text“ (der Rollback schreibt weiter, jetzt
den gemischten Stand), „Zusammenführen: …behält ihren Text“. Neu: Lesefehler (Attrappe wirft) → Meldung, kein
`page_state_save` für die Seite; offene Seite + `mutate` (Muster Z. 1925) → Editor zeigt Anhang ohne Neuladen,
vorherige Eingabe bleibt; Sync (Speicher direkt ändern + `sync-changed` auslösen) → Übernahme; Textkonflikt →
`book_ai_document_import` mit Dateitext, Datei = Editor, Meldung. Abschluss: `grep -rn "lva#546" tests` leer.

## 7 Paketschnitt

| PR | Scope | Akzeptanz (Befehl → Ergebnis) | hängt ab |
|---|---|---|---|
| **A – #545 + CAS-Grundlage** (S–M) | Rust: `read_state`, `write_state`, `state_version`, `MAX_STATE_BYTES`; `page_state_load` Fehler statt leer; `page_state_save(…, expected)`; `apply_page`/`conflict_copy`/`pages_import` über `write_state`; `collect` hält Unlesbares zurück; Export bricht bei Lesefehler ab. `bindings.ts` neu; TS-Aufrufer übergeben `null`. Oberfläche: Lesefehler-Zustand. `versions.loadPageState`, `merge.loadText` werfen. Store reicht `expected` noch nicht durch. | Rust-Tests §6 → ok; Playwright Lesefehler → grün; `node --test scripts/paste-cleanup.test.mjs` → grün | – |
| **B – Versionsmodell im Store, Fälle 1, 4, 5** (L) | `pageMerge.ts`; `pageState.ts` (§2.6) inkl. Ablösung von `seq`/`known`; `pageStore.ts` (`expected`, Ergebnis-Codes, `onAdopt`); `TtsSettings`: `pageBase`, `applyPageState`, Übernahme, `refresh` bei `sync-changed`/KeepAlive; Attrappe mit CAS | `node --test scripts/page-versions.test.mjs scripts/paste-cleanup.test.mjs` → grün; Playwright `editor-paste-cleanup.spec.ts` → grün, Marker Fall 1 entfernt | A |
| **C – Fremdschreiber mit veralteter Eingabe, Fälle 2, 3** (M) | Rollback per `mergeState(prefer theirs)` + kein Neuversuch bei unsauberem Rollback; `merge.ts` Neubau in der Kette; `pages_delete(id, expected)` für Quellseiten | Playwright `library.spec.ts` → grün, beide Marker entfernt; `grep -rn "lva#546" tests` → leer | B (`mergeState`) |

## 8 Risiken

- **Basis-Disziplin (V4):** Jeder Weg, der Dateiinhalt in die Oberfläche bringt, muss über `applyPageState` mit
  Basis gehen. `TtsSettings.tsx` hat 20 `setText(`-Stellen; B prüft jede (Eingabe = „ours“, kein Basiswechsel;
  Dateiinhalt = mit Basis). Review-Punkt.
- **Kosten:** Zusätzlich je Schreiben ein SHA-256 in JS und ein Lesen + SHA-256 in Rust über bis zu 64 MiB, beim
  500-ms-Autosave. Keine Zahl ohne Messung: A misst Speichern einer 17-MiB-Seite (Debug und Release, Windows)
  und legt das Ergebnis in den PR. Bei Bedarf: Rust vergleicht erst Länge + mtime, hasht nur bei Gleichheit nicht.
- **Span-Mischung** erzeugt bei verstreuten fremden Änderungen mehr Konflikte (mehr Fassungen, nie Verlust).
- **Tauri-IPC** mit sehr großen Strings ist nicht gemessen; die Rust-Rundreise prüft nur die Datei.
- **Fremde Prozesse** (Cloud-Ordner, Handbearbeitung) schreiben ohne Sperre: Hash erkennt es, ein Restfenster
  zwischen Rust-Vergleich und Umbenennen bleibt.
- **I1-Tests:** Tests mit Rettungserwartung ersetzen den Text ganz → Konflikt → gleiche Ausgabe. Ändert ein Test
  nicht überlappend, ist die neue Ausgabe (gemischt, keine Fassung) gewollt; Anpassung nur mit Begründung im PR.

## 9 Offene Entscheidungen (mit Empfehlung)

- **E1 Textmischung:** Span-Verfahren ohne Abhängigkeit (empfohlen) oder diff3-Bibliothek (`node-diff3`, MIT;
  `diff`/jsdiff, BSD-3-Clause). Empfehlung: Span jetzt, diff3 erst, wenn Konfliktfassungen im Alltag auffallen.
- **E2 Konfliktpolitik der offenen Seite:** Oberfläche gewinnt, Dateitext wird Fassung (empfohlen, I1-konform)
  oder Datei gewinnt, Eingabe wird Fassung. Empfehlung: Oberfläche gewinnt.
- **E3 Unsauberer Rollback:** kein automatischer Neuversuch, Meldung (empfohlen) oder Neuversuch mit Risiko
  doppelten Texts.
- **E4 Größengrenze:** 64 MiB (empfohlen; 16 MiB Protokoll + reichlich Text, schützt nur gegen Ausreißer) oder
  32 MiB.
- **E5 Quellseiten beim Zusammenführen:** `pages_delete(id, expected)` in C (empfohlen) oder eigenes Issue.
- **E6 Eigenes Ereignis `page-state-changed` aus Rust:** nicht nötig (`sync-changed` + Version reichen);
  Empfehlung: nein.
- **E7 Bezug I2:** `write_state` (CAS mit erwarteter Version) wird in I2 auch für `inhalt/aktuell.json` genutzt;
  die Revision dort ist ebenfalls ein Hash (§3.2 Inhalte-Modul). Empfehlung: in I2 übernehmen, hier nichts
  vorwegnehmen.
