# Konzept: iPhone per QR-Code am PC anmelden (Geräte-Kopplung)

Stand 05.11.2026 · Issue #289 · Entscheidung D15 in `docs/DECISIONS.md`

## 1 · Ziel

Die Desktop-App (Windows/macOS) ist am Hub angemeldet. Das iPhone soll sich mit
demselben Konto verbinden, **ohne** dass E-Mail und Passwort erneut eingegeben werden:
Desktop zeigt einen QR-Code, iPhone scannt, Nutzer bestätigt am PC, fertig.

## 2 · Ausgangslage

- Konten und Sync liegen heute im **Hub** (wai-portal, `portal.wolffappliedai.de`,
  IONOS). Anmeldung: `POST /api/v1/voice/auth/login` mit E-Mail + Passwort, Antwort ist
  ein API-Token mit Scope `voice.sync` je Gerät (`ApiTokenService::create`).
- Die Daten sind Ende-zu-Ende verschlüsselt. Der Objekt-Schlüssel `K` entsteht **auf dem
  Gerät** aus Passwort und E-Mail (Argon2id → HKDF, `sync/crypto.rs`,
  `SyncCrypto.deriveKey`). Der Hub sieht `K` nie.
- Folge: Ein reines Anmelde-Token reicht dem iPhone nicht. Es braucht zusätzlich `K` –
  sonst kann es nichts entschlüsseln. Genau das hätte auch ein Keycloak-Login nicht
  gelöst. Die Kopplung muss also **zwei Dinge** liefern: ein eigenes Gerätetoken (vom
  Hub) und `K` (vom Desktop, am Hub vorbei).

## 3 · Bewertete Varianten

| Variante | Token | Schlüssel K | Bewertung |
|---|---|---|---|
| A · Token + K direkt im QR | Langzeittoken im Bild | Klartext im Bild | **verworfen** – ein Foto des Bildschirms ist ein Vollzugriff |
| B · OAuth Device Authorization (RFC 8628), iPhone zeigt Code, Desktop bestätigt | sauber | fehlt | nur Hälfte der Lösung; QR-Richtung umgekehrt zum Wunsch |
| **C · Kopplungssitzung mit Einmalgeheimnis + ECDH + Vergleichscode** | eigenes Gerätetoken vom Hub | per X25519 Gerät-zu-Gerät | **gewählt** |

C ist die „Cross-Device“-Form von RFC 8628 (Desktop als bestätigendes Gerät) plus
Schlüsselübergabe wie bei Signal/WhatsApp-Gerätekopplung. Wenn später Keycloak die Konten
führt, wird nur Schritt 5 (Token ausstellen) durch einen Device-Grant gegen Keycloak
ersetzt; QR, ECDH und Oberfläche bleiben.

## 4 · Protokoll v2 (nach Codex- und Opus-Review 05.11.)

### 4.1 Kodierung (verbindlich, Testvektoren in `2026-11-05-geraete-kopplung-qr-vektoren.json`)

- `H` = SHA-256, `HMAC` = HMAC-SHA-256, `‖` = Verkettung von Bytes.
- `id` (pair_id) geht als **ASCII-Text** (22 Byte, base64url) in alle Hashes ein, nicht dekodiert.
- Alle übrigen Werte sind 32 Rohbytes; über die Leitung als **kanonisches base64url ohne
  Padding** (43 Zeichen). Nicht kanonische Werte lehnen Hub und Clients ab.
- `name` = Gerätename, UTF-8, NFC, 1–60 Zeichen (`len16be` = Länge in **UTF-8-Bytes**, Vektor `zusatz_utf8`), ohne `\p{C}` (Steuer-, Format-/Bidi-,
  Nullbreiten-, Privatzeichen), ohne Rand-Leerzeichen. Der Client bereinigt vorher; der Hub
  lehnt ab statt zu korrigieren, weil der Name in das Commitment eingeht.
- Alle Labels enden auf `-v2`.

### 4.2 Werte

| Wert | Wer erzeugt | Wer sieht ihn |
|---|---|---|
| `secret` (32 B) | Hub | Hub (nur Hash), QR |
| `q` (32 B) | Desktop | **nur QR**, nie der Hub |
| `d_sk/d_pk` (X25519) | Desktop | `d_pk` nur im QR |
| `p_sk/p_pk`, `n_p` (32 B) | iPhone | Hub erst nach Reveal |
| `n_d` (32 B) | Desktop, erst nach Sicht auf C | Hub, iPhone |

QR-Inhalt: `localvoice://pair?v=2&h=<hub>&p=<id>&s=<secret>&q=<q>&k=<d_pk>`

### 4.3 Ablauf

```
Desktop (angemeldet: T_d, K)                Hub                           iPhone
1  POST /pair/start {} ─────────────▶   id, secret, Frist 120 s
   (d_sk,d_pk), q ← Zufall; QR anzeigen
                                                            2  scannt; (p_sk,p_pk), n_p ← Zufall
                                                               C   = H("LV-PAIR-COMMIT-v2" ‖ id ‖ p_pk ‖ n_p ‖ len16be(name) ‖ name)
                                                               mac = HMAC(q, "LV-PAIR-CLAIM-v2" ‖ id ‖ C)
                                    ◀── POST /pair/claim {id, secret, C, mac, name}
3  GET /pair/{id} → claimed, C, mac, name
   prüft mac mit q (sonst Abbruch: „nicht dieses Handy“)
   hält C UNVERÄNDERLICH fest, n_d ← Zufall
   POST /pair/{id}/challenge {n_d} ──▶  claimed → challenged
                                    ◀── GET result → {challenge, n_d}   iPhone hält n_d fest
                                    ◀── POST /pair/{id}/reveal {p_pk, n_p}  (genau einmal)
                                        Hub prüft C → revealed
4  GET /pair/{id} → p_pk, n_p
   prüft H(...) == festgehaltenes C, p_pk ≠ d_pk, mac erneut
   z = X25519(d_sk, p_pk); z == 0³² → Abbruch
   SAS = u32be(H("LV-PAIR-SAS-v2" ‖ id ‖ d_pk ‖ p_pk ‖ n_p ‖ n_d)[0..4]) mod 10⁶
   zeigt SAS + „vom Gerät angegeben: name“                  zeigt denselben SAS
5  Ja: s = HKDF-SHA256(ikm=z, salt=q ‖ d_pk ‖ p_pk, info="LV-PAIR-KEY-v2" ‖ id, 32 B)
   P = JSON {"v":1,"k":b64u(K),"email":…,"user_id":…,"key_id":hex(H(K)[0..4])}
   sealed = base64(nonce24 ‖ XChaCha20-Poly1305(s, P, AAD = "LV-PAIR-SEAL-v2" ‖ id ‖ d_pk ‖ p_pk))
   POST /pair/{id}/approve {sealed} ─▶  revealed → approved (kein Token)
                                    ◀── GET result → approved → done + Token T_p (eine Transaktion)
                                        {token, sealed, user}
                                                            6  z = X25519(p_sk, d_pk); z == 0³² → Abbruch
                                                               s wie oben (q aus dem QR), öffnet sealed,
                                                               prüft key_id == hex(H(k)[0..4]) und
                                                               user_id == result.user.id,
                                                               T_p und K in den Schlüsselbund
                                    ◀── POST /pair/{id}/confirm {HMAC(s, "LV-PAIR-CONFIRM-v2" ‖ id)}
7  GET /pair/{id} → phone_confirm; prüft HMAC mit s
   erst jetzt „Verbunden“. Falsch oder fehlt bis expires_at + 30 s → Warnung, Gerät abmelden anbieten.
   (Hub nimmt confirm bis expires_at + 60 s an, danach wird das Abhol-Token verworfen.)
```

### 4.4 Warum das hält

- **Commit/Reveal (RFC 6189 §4.4.1.1):** Das iPhone legt sich mit `C` fest, bevor `n_d`
  existiert; der Desktop erzeugt `n_d` erst nach Sicht auf `C` und prüft gegen **sein
  festgehaltenes** `C`. Ein bösartiger Hub hat damit genau einen Versuch (10⁻⁶).
- **QR-Geheimnis q:** Der Hub (oder wer nur `secret` kennt) kann sich nicht als iPhone
  ausgeben: ohne `q` kein gültiges `mac`, kein `s`, kein Öffnen von `sealed`, keine
  gültige Bestätigung. Ein Angreifer mit **Foto des QR** bleibt durch den Vergleichscode
  und den sichtbaren Gerätenamen begrenzt.
- **Null-Prüfung (RFC 7748 §6.1):** beide Seiten vor HKDF; Rust
  `SharedSecret::was_contributory()`, Swift Vergleich mit 32 Nullbytes in konstanter Zeit.
- **Reflexion:** Desktop bricht ab, wenn `p_pk == d_pk`.
- **Schlüsselbestätigung:** Der Desktop meldet „Verbunden“ erst nach gültigem Bestätigungs-HMAC.

## 5 · Zustandsautomaten

### 5.1 Hub (wai-portal, `VoicePairingService`)

| Zustand | Erreicht durch | Erlaubte nächste | Token T_p | Sensible Felder |
|---|---|---|---|---|
| `pending` | start | claimed, rejected, expired | – | secret-Hash |
| `claimed` | claim (secret ok) | challenged, rejected, expired | – | + poll-Hash, C, mac, name |
| `challenged` | challenge (Starter-Token, einmal) | revealed, rejected, expired | – | + n_d |
| `revealed` | reveal (C geprüft, einmal) | approved, rejected, expired | – | + p_pk, n_p |
| `approved` | approve (Starter-Token, einmal) | done, rejected, expired | – | + sealed |
| `done` | result (einmal, Transaktion) | (confirm) | **angelegt** | sealed gelöscht; poll-Hash bis confirm |
| `rejected` | reject | – | – | sealed gelöscht |
| `expired` | Frist, 3 Fehlversuche, C falsch, neuer Start | – | – | sealed, poll-Hash gelöscht |

Regeln: jeder Übergang ist `UPDATE … WHERE status = <von> AND expires_ts >= jetzt`;
der Verlierer erhält 409/410. `reject` nach `done` antwortet **409 mit `status: done`**,
der Desktop bietet dann „Gerät abmelden“ an. Der Token-Klartext steht nur in der einen
Antwort, im Hub nur sein Hash. Widerruf wirkt sofort (kein Auth-Cache, `verify` je Aufruf).
`done`-Zeilen bleiben ohne Geheimnisse erhalten (Herkunft in der Geräteliste);
`rejected/expired` werden nach einem Tag gelegentlich aufgeräumt.

### 5.2 Desktop (P3, Rust `sync/pairing.rs`)

| Zustand | Ereignis | Aktion | Nächster |
|---|---|---|---|
| Start | Knopf | `start`, Schlüssel + q erzeugen, QR | WarteAufScan |
| WarteAufScan | Status `claimed` | mac mit q prüfen → falsch: Abbruch. C **und Name einmalig speichern** (angezeigt und in H(...) wird nur dieser Name verwendet), n_d erzeugen, `challenge` | WarteAufReveal |
| WarteAufReveal | Status `revealed` | C gegen gespeichertes C, `p_pk ≠ d_pk`, Nullprüfung, SAS **einmal** berechnen | ZeigeSAS |
| WarteAufReveal | `phone_commit` ≠ gespeichertes C | Abbruch, Warnung | Abbruch |
| ZeigeSAS | Ja | `sealed` bauen, `approve` | WarteAufBestätigung |
| ZeigeSAS | Nein | `reject` | Ende |
| WarteAufBestätigung | `phone_confirm` gültig | „Verbunden“ | Ende |
| WarteAufBestätigung | ungültig, oder nichts bis `expires_at` + 30 s | Warnung, „Gerät abmelden“ anbieten (inkl. Liste der mit abgemeldeten Geräte über `paired_by`) | Ende |
| jeder Zustand ab `challenge` | jeder Fehler, 409, Zeitablauf | Kopplung beenden, **kein** neues n_d | Abbruch |

### 5.3 iPhone (P4, Swift)

| Zustand | Ereignis | Aktion | Nächster |
|---|---|---|---|
| Scan | QR gelesen, v=2 | Schlüssel, n_p, C, mac; `claim` | WarteAufChallenge |
| WarteAufChallenge | `challenge` mit n_d | n_d **einmalig speichern**, `reveal` (genau einmal), SAS **einmal** berechnen | ZeigeSAS |
| ZeigeSAS | `challenge` mit **gleichem** n_d (Reveal-Antwort verloren) | `reveal` mit identischen Werten wiederholen; 410 darauf gilt als erledigt | ZeigeSAS |
| ZeigeSAS | `challenge` mit **abweichendem** n_d | Abbruch, Warnung | Abbruch |
| ZeigeSAS | Ergebnis `ok` | Nullprüfung, s, öffnen, key_id + user_id prüfen, Schlüsselbund, `confirm` | Verbunden |
| ZeigeSAS | 403 rejected | „Am PC abgelehnt“ | Ende |
| jeder | 410 | „Code abgelaufen oder schon verwendet“ | Ende |
| jeder | Netzfehler | Wiederholen nur für Abfragen (Intervall ≥ 1 s, wegen Drosselung 120/min); `reveal` nie mit anderen Werten | – |

Tests für 5.2/5.3 entstehen mit P3/P4 gegen die Testvektoren.

## 5a · Sicherheitsregeln

1. Im QR steht **kein** Passwort, **kein** Token und **nicht** `K`.
2. `secret` und `q` je 32 Byte Zufall; `secret` im Hub nur als SHA-256; `q` nie beim Hub.
3. Frist **120 s** ab Start für den gesamten Ablauf; Restzeit aus `server_time`/`expires_at`.
4. **Einmalig:** claim, challenge, reveal, approve, Abholen, confirm.
5. Sehen, challenge, approve, reject nur mit dem Starter-Token; Kopplung und Geräteliste
   verlangen `voice.sync` **explizit** (ein Agenten-Token mit `*` genügt nicht).
6. Je Konto höchstens **eine** offene Kopplung; ein neuer Start verwirft die alte.
7. Offene Routen (claim, reveal, result, confirm): Body ≤ 8 KB; IP-Drosselung claim
   10/15 min, Abfragen 120/min. Hinter CGNAT kann ein Dritter im selben Netz die Kopplung
   stören (akzeptiert). Unbekannt, abgelaufen, verwendet und falsches Geheimnis antworten
   gleich (410).
8. Flüchtige Schlüssel (`d_sk`, `p_sk`, `z`, `s`, `q`) nur im Speicher, danach genullt.
9. Keine Inhalte und keine Geheimnisse im Log; Audit nur mit IP-Präfix (/24, /48).
10. Widerruf eines Geräts widerruft auch alle Geräte, die **über** dieses Gerät gekoppelt
    wurden (Kette, `VoicePairingService::revokeChain`), damit ein abgemeldetes Gerät keinen
    Zugang über selbst ausgestellte Tokens behält. Gilt für „Abmelden“ in der Geräteliste
    und für das eigene Abmelden (`/auth/logout`). Die Oberfläche nennt vorher, welche Geräte
    mit abgemeldet werden.
12. Betrieb: `post_max_size` des Hubs klein halten (offene Routen prüfen nur
    `Content-Length`); `login_attempts` wird mit der bestehenden Bereinigung des Portals geleert.
11. Gerätetokens laufen wie Login-Tokens nicht ab (Entscheidung D15); Kontrolle über die
    Geräteliste.

## 6 · Audit (Hub, `AuditService::log`, Hash-Kette)

`voice_pairing.started`, `.claimed` (Name, IP-Präfix), `.secret_failed` (Zähler, IP-Präfix),
`.challenged`, `.revealed`, `.approved`, `.delivered` (Token-ID), `.confirmed`,
`.rejected`, `.expired` (Grund), dazu `api_token.created` / `api_token.revoked`.

## 7 · Hub-Schnittstelle (wai-portal)

| Methode | Pfad | Auth | Zweck |
|---|---|---|---|
| POST | `/api/v1/voice/pair/start` | `voice.sync` explizit | `{}` → `{pair_id, secret, expires_at, ttl, server_time}` |
| POST | `/api/v1/voice/pair/claim` | Geheimnis | `{pairing_id, secret, phone_commit, phone_mac, device_name}` → `{poll_token, expires_at}` |
| GET | `/api/v1/voice/pair/{id}` | Starter-Token | Status, Name, C, mac, p_pk, n_p, phone_confirm, Zeiten |
| POST | `/api/v1/voice/pair/{id}/challenge` | Starter-Token | `{desktop_nonce}` |
| POST | `/api/v1/voice/pair/{id}/reveal` | `X-Pairing-Token` | `{phone_pub, phone_nonce}` |
| POST | `/api/v1/voice/pair/{id}/approve` | Starter-Token | `{sealed}` (base64, ≤ 4 KB) |
| POST | `/api/v1/voice/pair/{id}/reject` | Starter-Token | 200 oder 409 `{status}` |
| GET | `/api/v1/voice/pair/{id}/result` | `X-Pairing-Token` | 202 waiting · 200 challenge+n_d · 403 rejected · 200 `{token, sealed, user}` einmalig · 410 |
| POST | `/api/v1/voice/pair/{id}/confirm` | `X-Pairing-Token` | `{confirm}` einmalig |
| GET | `/api/v1/voice/devices` | `voice.sync` explizit | id, Name, angelegt, zuletzt aktiv, `current`, `via_pairing`, `paired_by` |
| POST | `/api/v1/voice/devices/{id}/revoke` | `voice.sync` explizit | Widerruf inkl. Kette (nicht das eigene → 409) |

## 8 · Oberfläche Desktop

Ort: **Einstellungen › Konto & Geräte**, unter der Sync-Karte.

1. Knopf **„Handy verbinden“** (nur aktiv, wenn angemeldet; sonst Hinweis „Zuerst hier anmelden“).
2. Dialog Schritt 1: QR (≥ 240 px, hoher Kontrast, auch im Dunkelmodus schwarz auf weiß),
   Anleitung in drei Zeilen, Restzeit-Anzeige `1:47`, „Neuen Code“.
3. Schritt 2: „**iPhone von Patrick** (vom Gerät angegeben) möchte sich mit Ihrem Konto verbinden.“ Vergleichscode
   groß `482 913`, „Steht derselbe Code auf dem iPhone?“ [**Ja, verbinden**] [Nein, abbrechen].
4. Schritt 3: „Verbunden. Das iPhone gleicht jetzt ab.“ [Fertig]
5. Abgelaufen: „Der Code ist abgelaufen.“ [Neuen Code].
6. Liste **„Verbundene Geräte“**: Name, verbunden seit, zuletzt aktiv, „dieses Gerät“;
   je Zeile „Abmelden“ mit Rückfrage.

## 9 · Oberfläche iPhone (Mac-Thread)

- In „Konto & Sync“: „**Mit Desktop-App verbinden**“ (Scanner, VisionKit
  `DataScannerViewController`) zusätzlich zur Passwort-Anmeldung.
- Universal/Custom-URL `localvoice://pair?...` öffnet denselben Ablauf aus der System-Kamera.
- Anzeige des Vergleichscodes, „Bitte am PC bestätigen“, danach Abgleich.
- Ohne Kamerafreigabe bleibt die Passwort-Anmeldung.

## 10 · Grenzen

- Widerruf sperrt den Hub-Zugang des Geräts. Das iPhone kennt `K` aber weiter; echte
  Schlüsselrotation bei Verlust oder Kompromittierung ist ein eigenes Folge-Issue
  (Codex 05.11.: Hinweis genügt für #289).
- Der Desktop muss online und am Hub angemeldet sein.
- Geht die Antwort mit T_p verloren, existiert T_p ungenutzt; ohne Bestätigung zeigt der
  Desktop eine Warnung und bietet „Gerät abmelden“ an.
- Keycloak: Wechsel betrifft nur Token-Ausstellung (Schritt 5) und Geräteliste.

## 11 · Umsetzung

| Paket | Repo | Inhalt |
|---|---|---|
| P1 | local-voice-ai | dieses Konzept, D15, Issue #289 |
| P2 | wai-portal | Migration `voice_pairings`, Controller, Tests, Audit |
| P3 | local-voice-ai | Rust `sync/pairing.rs` (X25519 `x25519-dalek`, BSD-3), QR (`qrcode`, MIT/Apache), Tauri-Befehle, Dialog + Geräteliste; Vorschau → Codex |
| P4 | local-voice-ai `apps/apple` | Scanner, Claim, SAS, Öffnen von `sealed` (CryptoKit Curve25519 + HKDF; XChaCha wie `SyncCrypto`), Keychain |
