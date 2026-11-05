# Bibliothek-Sync zwischen Desktop und iPhone (Epic #210, P4–P6)

Stand 02.11.2026. Baut auf dem E2E-Sync aus `2026-09-14-multi-device-sync-design.md` auf
(gleiches Konto, gleicher Schlüssel, gleicher Hub). Neu sind drei Sammlungen und eine
Dateiablage (wai-portal PR #8).

## Schlüssel und Verschlüsselung
Unverändert: Argon2id (64 MiB, t=3, p=1) → HKDF-SHA256 `lv-sync-enc-v1`, XChaCha20-Poly1305,
Objekt-AAD `user_id ␟ collection ␟ object_id`. Swift-Gegenstück
`apps/apple/VoiceCore/Sources/VoiceCore/SyncCrypto.swift`, gegen Rust geprüft
(`deriveKey("geheim", "mail@example.com")` = `f5037b3d…a678ee`).

## Sammlungen (Klartext-JSON vor dem Versiegeln, je ≤ 256 KiB)

| Sammlung | object_id | Wert |
| --- | --- | --- |
| `book` | Buch-Kennung (`book_…`, ≤ 64 Zeichen) | `buch.json` ohne lokale Felder (`cover_path`, `audio_path`, `audio_paths`), dazu `files` (unten) und `updated` (ms) |
| `progress` | `p_` + erste 40 Hex von SHA-256(Schlüssel) | `{"key": "audio:<buch>", "record": <ProgressRecord>}`; Schlüssel ohne `progress.`-Präfix |
| `bookmark` | `b_` + erste 40 Hex von SHA-256(Schlüssel) | `{"key": "tts.bookmarks.<seite>", "value": "<Text wie im localStorage>"}`, ebenso `tts.resume.*` |

Gelöscht wird über den Grabstein des Hubs (`deleted: true`).

**Zusammenführen beim Empfang:** `book` feldweise, neueres `updated` gewinnt; `files` des
neueren. `progress` mit den Regeln aus `progress.ts` (`mergeRecordText`): Stelle und Anker vom
neueren `updated`, gehörte und gelesene Bereiche vereinigt, `finished` bleibt, Länge = Maximum.
Ein Gerät schickt seinen eigenen Stand unverändert; zusammengeführt wird nur beim Empfang.

**Kanonische Form und Ledger:** Hash über JSON mit sortierten Schlüsseln, `null`-Felder entfernt,
Ganzzahlen ohne `.0`. Nach dem Anwenden eines empfangenen Objekts speichert das Ledger den Hash
der **lokalen** kanonischen Form (nicht des empfangenen Klartexts), sonst schickt jedes Gerät den
gerade erhaltenen Stand in eigener Schreibweise zurück. Eigene Echos (gleiche `device_id`,
Revision bekannt) werden beim Pull übersprungen. Objekte über 256 KiB Klartext und vom Hub
abgewiesene Objekte werden als „tot“ vermerkt und erst mit geändertem Inhalt erneut gesendet.
Neue Felder (z. B. `favorite`, lva#222) müssen beide Seiten kennen, sonst gehen sie beim
Zurückschreiben verloren.

## Dateien (`files` im Buch)

```json
"files": {
  "text":  {"blob": "<64 hex>", "size": 812345, "chunks": 1},
  "cover": {"blob": "<64 hex>", "size": 51234, "chunks": 1, "type": "jpg"},
  "audio": [{"file": "teil_001.mp3", "blob": "<64 hex>", "size": 41234567, "chunks": 10}]
}
```

- `text` ist der Buchtext als UTF-8 (`\n\n` zwischen Absätzen, Überschriften `# `), damit auch
  Bücher über 256 KiB ankommen. Die Vorlesen-Seite bleibt am Desktop die Quelle.
- **Blob-Kennung:** `hex(SHA-256(key ‖ "lv-blob-v1" ‖ SHA-256(klartext)))`. Gleiche Datei ergibt
  dieselbe Kennung (kein Doppel-Upload), der Server kann sie keinem Inhalt zuordnen.
- **Stücke:** Klartext in 3-MiB-Stücke (Dateien bis 3 827 302 Byte bleiben ein Stück; das Hosting lehnt Anfragen ab etwa 5 MiB ab), jedes einzeln mit XChaCha20-Poly1305 versiegelt,
  AAD `user_id ␟ blob ␟ <blob_id> ␟ <index>`, Stück = `nonce ‖ chiffrat ‖ tag` (roh).
  Transport als JSON `{data: base64, sha256: hex(SHA-256(stück))}` an
  `POST /api/v1/voice/blobs/{id}/chunks/{n}`, Abschluss `POST …/{id}/commit {chunks}`.
- **Reihenfolge beim Hochladen:** erst alle Dateien des Buchs hochladen und abschließen, dann
  das `book`-Objekt pushen. Ein Gerät sieht so nie ein Buch, dessen Dateien fehlen.
- `audio[].file` ist genau der Dateiname aus `audio.tracks[].file` (bzw. `audio.file` bei einem
  Teil), ohne Pfad. Eine leere Datei hat 1 Stück: `chunks = size <= 3827302 ? 1 : ceil(size / 3 MiB)`.
- **Herunterladen:** iPhone lädt Text sofort (Cover folgt; bis dahin erzeugtes Cover), Hörbuchteile erst beim ersten Öffnen
  (oder auf Wunsch), Einstellung „nur im WLAN“. Stücke einzeln prüfen (sha256 und Tag), dann
  Stück für Stück in `<ziel>.part` schreiben (Wiederaufnahme am ersten fehlenden Stück), erst
  vollständig umbenennen.
- **Testvektor** (Desktop und iPhone, mit libsodium bestätigt): Schlüssel 0x00..0x1f, user_id 7,
  Klartext „Hallo Buch“, Nonce 0x40..0x57, Stück 0 → Blob `08d8fe6e…0a7322`, Stück
  `404142…5657` (Nonce) + `9c58691c…3c4bbf` (vollständig in `LibrarySyncModelTests`).
- **Löschen:** „Auf allen Geräten löschen“ sendet einen Grabstein; „Hörbuch vom iPhone
  entfernen“ löscht nur die geladenen Dateien.

## Zuständig
- P4 Hub: wai-portal PR #8.
- P5 Desktop (Rust `sync/collect.rs`, `library.rs`): Sammlungen erzeugen und anwenden, Dateien hochladen.
- P6 iPhone (`apps/apple`): Login, Zyklus, Download bei Bedarf.
