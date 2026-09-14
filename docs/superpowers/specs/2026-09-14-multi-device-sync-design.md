# Design: Multi-Device mit Login und Ende-zu-Ende-verschlüsseltem Sync (E6)

Datum: 2026-09-14 · Stand: Entwurf v1, von Patrick am 14.09. mit „Vollgas, nicht anhalten"
freigegeben; die fünf offenen Fragen aus `docs/ROADMAP.md` (Nachtrag 14.09.) wurden mit dem
kleinsten reversiblen Weg entschieden (Abschnitt 0).

## 0 · Entscheidungen

| Frage | Entscheidung | Warum |
|---|---|---|
| Hub-Ansatz | **A: eigener Hub, Ende-zu-Ende verschlüsselt** | Muster liegt fertig im wai-portal (Bewerbungen-Hub, Spec 2026-09-03 v3); Server sieht nur Blobs |
| Wo läuft der Hub | **wai-portal** (PHP 8.3, MySQL, IONOS Webhosting Pro), neuer Scope `voice.sync` | kein zweiter Server, ApiTokenGuard, Migrationen, Testharness vorhanden |
| Konto | **Portal-Benutzer** (E-Mail + Passwort). Kein Self-Service-Register in Stufe 1 | Patrick ist der einzige Nutzer; Registrierung ist ein eigenes Paket (Abschnitt 8) |
| Sync-Umfang Stufe 1 | Vorlese-Seiten (Titel, Reihenfolge, Arbeitsstand), Einstellungen-Teilmenge | klein, textuell, konfliktarm; Audio/Modelle/Aufnahmen später |
| Stimmwechsler | nur Oberfläche entfernt (E5), Rust bleibt | reversibel |

## 1 · Ziel und Nicht-Ziel

**Ziel:** Ein Text, der auf dem PC angelegt wurde, steht nach dem Login auf dem Mac in derselben
Seitenliste und lässt sich dort vorlesen. Änderungen laufen in beide Richtungen, auch offline
(nachgeholt beim nächsten Kontakt). Der Server kann den Inhalt nicht lesen.

**Nicht-Ziel (Stufe 1):** Klon-Referenzaudios, Piper/Fish-Modelle, Meeting-Aufnahmen,
Diktat-Verlauf, geteilte Konten, Freigabe an Dritte, Echtzeit-Kollaboration, 2FA am
API-Login (Konto mit aktivem 2FA wird mit `2fa_required` abgewiesen, Abschnitt 8).

## 2 · Architektur

```
┌──────────── Gerät A (PC) ────────────┐        ┌──────── wai-portal (IONOS) ────────┐
│ UI: Einstellungen → Allgemein →      │        │ /api/v1/voice/auth/login           │
│     "Konto & Geräte"                 │  HTTPS │ /api/v1/voice/sync/push|pull|status│
│ Rust: sync::{account,crypto,ledger,  │ ─────► │ Tabellen: voice_objects,           │
│        client,engine}                │ ◄───── │  voice_changes_log, voice_counters │
│ Daten: projects/*, settings.json     │        │ sieht: user_id, collection,        │
│ Schlüssel: nur lokal (sync.json)     │        │  object_id, revision, ciphertext   │
└──────────────────────────────────────┘        └────────────────────────────────────┘
                     ▲ gleicher Ablauf auf Gerät B (Mac)
```

Drei Bausteine, je einzeln testbar:

1. **Portal-Hub** — speichert verschlüsselte Objekte je Benutzer mit Server-Revision und
   fortlaufender `server_seq`; CAS beim Push, Cursor beim Pull. Kopie des Bewerbungen-Musters,
   ohne Feldgruppen-Merge (der Server kann Blobs nicht mergen).
2. **Rust-Modul `sync`** in `src-tauri/src/sync/` — Konto, Schlüsselableitung, Ver-/Entschlüsselung,
   lokales Ledger, HTTP-Client, Zyklus.
3. **Oberfläche** — Login/Logout und Status im bestehenden Reiter „Allgemein", kein neuer
   Menüpunkt. Status zusätzlich als Symbol in der Fußleiste (grau aus, grün synchron, gelb
   ausstehend, orange Fehler) — dieselbe Farbsprache wie Server- und Sprachmodell-Symbol.

## 3 · Konto und Schlüssel

- **Login:** `POST /api/v1/voice/auth/login {email, password, device_name}` → prüft
  `password_verify` gegen `users`, Rate-Limit 5/Minute je IP und E-Mail, legt per
  `ApiTokenService::create` ein Token mit Scope `voice.sync` und Name `Voice · <device_name>`
  an, antwortet `{token, user:{id, email}, server_time}`. Token wird im Portal gehasht
  gespeichert, unter Admin → Agent-API sichtbar und widerrufbar (= Gerät abmelden).
- **Logout:** `POST /api/v1/voice/auth/logout` widerruft das eigene Token; lokal wird
  `sync.json` gelöscht.
- **Schlüsselableitung (nur Client):**
  `master = Argon2id(password, salt = SHA-256("local-voice-ai/v1/" + lowercase(email))[0..16], m=64 MiB, t=3, p=1)`
  `enc_key = HKDF-SHA256(master, info="lv-sync-enc-v1", 32 B)`.
  Passwort und `master` verlassen den Speicher nach der Ableitung (`zeroize`).
- **Offen benannt (nicht Stufe 1):** Das Passwort geht beim Login im Klartext (TLS) an den
  Server, der es verifiziert. Ein Betreiber könnte es dort abgreifen und den Schlüssel
  ableiten. Betreiber = Nutzer in Stufe 1. Für eine Veröffentlichung wird der Login auf ein
  abgeleitetes Auth-Geheimnis umgestellt (`auth = HKDF(master, "lv-auth-v1")`, Server
  speichert nur dessen Hash) — Abschnitt 8.
- **Lokale Ablage** `<appdata>/sync.json`: `{url, token, device_id, device_name, user_email,
  enc_key_b64, cursor}` mit Nutzer-ACL (Windows: nur aktueller Benutzer; Unix: 0600). Fehlt die
  Datei, läuft die App rein lokal. Kein Passwort auf Platte.
- **Passwortänderung im Portal** ändert den Schlüssel → alle Objekte müssen neu
  verschlüsselt werden. Stufe 1: der Client erkennt es an `key_id` (Abschnitt 4) und meldet
  „Passwort geändert — bitte auf diesem Gerät neu anmelden; Daten werden neu hochgeladen".
  Automatische Umschlüsselung ist Stufe 2.

## 4 · Objektmodell und Verschlüsselung

Ein **Objekt** = `(collection, object_id)` je Benutzer.

| collection | object_id | Klartext-Payload (JSON) | Quelle |
|---|---|---|---|
| `page` | Seiten-ID (`page_…`) | `{title, order, state:{text, summary, sourceUrl, tab}}` | `projects/index.json` + `state.json` |
| `settings` | `app` | Teilmenge von `AppSettings`: `tts_*` außer Pfaden/Ports, `app_language`, `theme`, `custom_words` | `settings.json` |

Ausgeschlossen bleiben Pfade, Ports, Hotkeys, Geräte-Auswahl (gerätespezifisch), Tokens.

**Verschlüsselung je Objekt:** `XChaCha20-Poly1305(enc_key, nonce = 24 B zufällig,
aad = user_id ‖ collection ‖ object_id)`; Chiffrat + Nonce base64 im Feld
`payload`. Die Revision ist bewusst NICHT Teil der AAD (Umsetzung 14.09.: einfacher, und der
Server ist ohnehin honest-but-curious, nicht boesartig). `aad` bindet ein Chiffrat an seinen Ort — ein serverseitig verschobenes Blob
entschlüsselt nicht. Zusätzlich ein `key_id = erste 8 Hex-Zeichen von SHA-256(enc_key)` im
Klartext, damit ein Gerät mit altem Schlüssel „falscher Schlüssel" von „kaputtes Blob"
unterscheiden kann. Max. Klartext 256 KB je Objekt (Seiten mit längerem Text werden mit
`too_large` im Status gemeldet und nicht synchronisiert — Stufe 1).

**Löschen:** Tombstone (`deleted = 1`, leeres Payload), bleibt 90 Tage im Log.

## 5 · Protokoll (Portal)

Alle Routen unter `/api/v1/voice/*`, Bearer-Token mit Scope `voice.sync`, JSON, UTF-8,
Body ≤ 512 KB (`ApiTokenGuard::for('voice.sync')` mit `$api(..., 512*1024)`).

| Endpunkt | Body / Query | Antwort |
|---|---|---|
| `POST voice/auth/login` (ohne Token, eigener Rate-Limiter) | `{email, password, device_name}` | `200 {token, user, server_time}` · `401 invalid_credentials` · `403 2fa_required` · `429` |
| `POST voice/auth/logout` | — | `{}` (Token widerrufen) |
| `POST voice/sync/push` | `{device_id, objects:[{collection, object_id, base_revision, payload, key_id, deleted}]}` (≤ 100 Objekte) | `{results:[{collection, object_id, outcome: accepted\|superseded\|rejected, revision, current?, reason?}], server_time}` |
| `GET voice/sync/pull?since=<seq>&limit=100` | — | `{objects:[{collection, object_id, revision, payload, key_id, deleted, device_id, server_seq}], cursor, more}` · `410 cursor_expired` |
| `GET voice/sync/status` | — | `{objects_per_collection, devices:[{device_id_hash, device_name, last_push_at}], server_time}` |

**CAS-Regel im Server:** `accepted`, wenn `base_revision == aktuelle revision` (oder Objekt
neu und `base_revision == 0`); dann `revision += 1`, `server_seq = nextSeq()`, Log-Eintrag.
Sonst `superseded` mit `current` (die Server-Fassung) — der Client entscheidet. `rejected`
nur bei Validierungsfehlern (Muster, Größe, unbekannte collection).

**Tabellen (Migration `0024_voice_sync.php`):**

- `voice_objects (user_id, collection, object_id, revision, server_seq, payload MEDIUMTEXT,
  key_id CHAR(8), deleted TINYINT, device_id VARCHAR(64), updated_at)` PK `(user_id,
  collection, object_id)`, Index `(user_id, server_seq)`.
- `voice_changes_log (id, user_id, server_seq, collection, object_id, revision, deleted,
  device_id, created_at)` — Pull liest hieraus per `server_seq`, joint das aktuelle Payload.
  Retention 90 Tage, Housekeeping am Ende eines Push (≤ 5 s, Batch 200).
- `voice_counters (user_id, name, value)` — `hub_seq` je Benutzer.

Validierung: `collection ∈ {page, settings}`, `object_id ~ ^[A-Za-z0-9_-]{1,64}$`,
`payload` base64 ≤ 350 KB, `key_id ~ ^[0-9a-f]{8}$`, `device_id ~ ^[A-Za-z0-9_-]{8,64}$`.

## 6 · Client (Rust `sync`)

```
src-tauri/src/sync/
  mod.rs        Tauri-Commands: sync_login, sync_logout, sync_status, sync_now
  account.rs    sync.json laden/speichern (ACL), Schlüsselableitung, Login-Aufruf
  crypto.rs     seal/open (XChaCha20-Poly1305 + AAD), key_id
  ledger.rs     <appdata>/sync_ledger.json: je Objekt {revision, dirty, local_hash}, cursor
  collect.rs    Objekte aus projects/* und settings.json bauen; Objekte anwenden
  client.rs     reqwest: push/pull/status/logout mit Backoff
  engine.rs     Zyklus (push dirty → pull seit cursor → anwenden), Trigger, Status
```

- **Dirty-Erkennung ohne Eingriff in jeden Schreibpfad:** der Zyklus berechnet je Objekt den
  Klartext-Hash (SHA-256) und vergleicht mit `local_hash` im Ledger. Abweichung = dirty.
  Das ist bei ≤ 200 Seiten billig und hält die Schreibpfade (`pages.rs`, Settings) unberührt
  — Regel 6 aus `AGENTS.md`. Zusätzlich stoßen `page_state_save`, `pages_*` und
  `set_settings` einen entprellten Lauf (2 s) an, damit Änderungen zügig hochgehen.
- **Zyklus** (tokio-Task, Mutex gegen Überlappung): alle 60 s; nach Trigger; beim Start
  nicht blockierend. Push in Seiten ≤ 100. `accepted` → Revision und Hash übernehmen.
  `superseded` → **Konfliktregel**: Server-Fassung anwenden; war das Objekt eine Seite mit
  abweichendem Text, wird die lokale Fassung als neue Seite „<Titel> (Konflikt von
  <device_name>)" angelegt und beim nächsten Lauf hochgeladen — nichts geht still verloren.
  Bei `settings` gewinnt der Server ohne Kopie. `rejected` → Dead-Letter im Status, nicht
  erneut senden.
- **Pull** bis `more=false`; Objekte, die lokal dirty sind, werden nicht überschrieben (sie
  kommen im nächsten Push als `superseded` zurück und durchlaufen die Konfliktregel).
  `410` → Cursor 0, Voll-Pull. Falscher `key_id` → Status `key_mismatch`, Lauf stoppt.
- **Anwenden von `page`:** Seite fehlt → Ordner + `state.json` anlegen, Index ergänzen;
  Seite existiert → `state.json` und Titel/Position schreiben; Tombstone → Ordner **nicht**
  löschen, sondern Seite aus dem Index nehmen und Ordner nach `projects/_geloescht/` schieben
  (Dateien bleiben; Nutzer räumt selbst auf).
- **Offline:** kein Fehler; Backoff 1/4/10 min; Status zeigt letzten Erfolg/Fehler.
- **Ereignis** `sync-changed` an die Oberfläche nach jedem Lauf; die Seitenliste lädt neu.

**Neue Crates:** `chacha20poly1305 = "0.10"`, `argon2 = "0.5"`, `hkdf = "0.12"`,
`zeroize = "1"` (RustCrypto, MIT/Apache-2.0; `cargo deny check licenses` bleibt grün).
`reqwest`, `sha2`, `serde_json`, `tokio` sind da. `rand` prüfen, sonst `rand = "0.8"`.

## 7 · Oberfläche

- **Einstellungen → Allgemein → Gruppe „Konto & Geräte"** (existierender Reiter):
  abgemeldet: E-Mail, Passwort, Gerätename (Vorschlag: Hostname), Knopf „Anmelden", Hinweis
  „Ende-zu-Ende verschlüsselt; der Server kann Ihre Texte nicht lesen." Angemeldet: E-Mail,
  Gerätename, letzter Abgleich, Anzahl ausstehend, Knöpfe „Jetzt abgleichen", „Abmelden";
  Liste der Geräte aus `status`.
- **Fußleiste:** Wolken-Symbol mit Farbzustand und Tooltip; Klick springt in die Gruppe.
- Fehler als Toast, nie als Dialog.

## 8 · Nicht in Stufe 1, benannt

1. Registrierung + E-Mail-Bestätigung im Portal (öffentliche Nutzer).
2. Auth-Geheimnis statt Klartext-Passwort am Login (Bitwarden-Muster), Umschlüsselung bei
   Passwortwechsel, Wiederherstellungscode.
3. 2FA am API-Login (TOTP-Code im Login-Body).
4. Sprecher-Registry (`meta.json` je Stimme) und Klon-Referenzaudios (Chunked Upload,
   ≤ 15 MB, eigene Freigabe je Stimme).
5. Meeting-Aufnahmen und Diktat-Verlauf.
6. Schlüssel im OS-Schlüsselbund statt in `sync.json`.

## 9 · Tests und Abnahme

- **Portal** `tests/cases/55_voice_sync_test.php`: Migration; Login gut/schlecht/Rate-Limit/
  2FA-Konto; Push neu → accepted; Push mit alter Revision → superseded mit current; Pull-Cursor
  und `410`; Validierung (collection, Größe, key_id); Benutzer-Trennung (User B sieht nichts
  von A); Logout widerruft.
- **Rust** (`cargo test --lib sync::`): seal/open Roundtrip, AAD-Bindung (falscher Ort →
  Fehler), key_id stabil, Ledger dirty-Erkennung, Konfliktregel erzeugt Kopie, Tombstone
  verschiebt statt löscht, Pull überschreibt dirty nicht. HTTP-Client gegen einen lokalen
  Fake-Server (`tokio` + `hyper`? → einfacher: Trait `Transport` mit In-Memory-Fake).
- **Oberfläche** (Playwright, gemockte Commands): Gruppe abgemeldet/angemeldet, Login-Fehler
  als Toast, Fußleisten-Symbol.
- **Abnahme (Artefakt):** Seite auf dem PC anlegen → auf dem Mac nach dem Login vorlesen; dazu
  Screenshot beider Seitenlisten und `voice/sync/status` mit zwei Geräten.

## 10 · Ablauf und Aufwand

| Schritt | Ort | kTok |
|---|---|---|
| 1 Portal: Migration, Service, Controller, Routen, Scope, Tests | wai-portal, Zweig `feat/voice-sync` | 80 |
| 2 Portal deployen + Migration (Server zuerst) | `deploy-portal.ps1 -Files …`, SSH-Migration | 10 |
| 3 Rust: crypto, account, ledger, collect, client, engine, Commands | App, Zweig `feat/multi-device-sync` | 150 |
| 4 Oberfläche: Konto-Gruppe, Fußleiste, bindings.ts | App | 50 |
| 5 Review (Codex) der Schlüssel-, Konto- und Konfliktlogik + Fixes | beide | 40 |
| 6 Abnahme PC ↔ Mac, Installer | beide | 20 |

Summe ~350 kTok. Harter Stopp bei 150 % (525) mit Lagebericht.
