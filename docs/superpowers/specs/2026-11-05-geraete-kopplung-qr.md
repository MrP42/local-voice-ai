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

## 4 · Ablauf (Revision 2 nach Codex 05.11.: Commit/Reveal)

Schreibweise: `H` = SHA-256, `‖` = Verkettung der Rohbytes, `b64u` = base64url ohne Padding.
Alle Zufallswerte 32 Byte aus dem System-CSPRNG.

```
Desktop (angemeldet: T_d, K)               Hub                              iPhone
1  (d_sk, d_pk) ← X25519
   POST /pair/start {d_pk} ──────────▶  id, secret, Frist 120 s
                            ◀──────────  {id, secret, expires_at}
2  QR: localvoice://pair?v=1&h=<hub>&p=<id>&s=<secret>&k=<d_pk>
                                                               3  (p_sk, p_pk) ← X25519, n_p ← Zufall
                                                                  C = H("LV-PAIR-COMMIT-v1" ‖ id ‖ p_pk ‖ n_p)
                                        ◀── POST /pair/claim {id, secret, phone_commit=C, device_name}
                                        pending → claimed, poll_token ──▶
4  GET /pair/{id}: claimed, C, Gerätename
   n_d ← Zufall (erst JETZT, nach Sicht auf C)
   POST /pair/{id}/challenge {n_d} ──▶  claimed → challenged
                                        ◀── GET /pair/{id}/result (poll_token): {challenge, n_d}
                                        ◀── POST /pair/{id}/reveal {p_pk, n_p}
                                        prüft C (sonst Abbruch), → revealed
5  GET /pair/{id}: p_pk, n_p
   prüft C selbst (sonst Abbruch)
   z = X25519(d_sk, p_pk); z == 0³² → Abbruch                    zeigt SAS, „Bitte am PC bestätigen“
   SAS = (erste 4 Byte von H("LV-PAIR-SAS-v1" ‖ id ‖ d_pk ‖ p_pk ‖ n_p ‖ n_d), big-endian) mod 10⁶, 6 Ziffern mit führenden Nullen
   zeigt SAS + Gerätename [Ja, verbinden] [Nein]
6  Ja: s = HKDF-SHA256(z, salt=d_pk‖p_pk, info="LV-PAIR-KEY-v1"‖id), 32 Byte
   sealed = nonce24 ‖ XChaCha20-Poly1305(s, {K, email, user_id, key_id}, AAD=id)
   POST /pair/{id}/approve {sealed} ─▶  revealed → approved (nur Paket, KEIN Token)
                                        ◀── GET /pair/{id}/result
                                        approved → done UND Token T_p anlegen
                                        in einer Transaktion; Klartext nur in
                                        dieser Antwort ──▶ {T_p, sealed, user}
                                                               7  z = X25519(p_sk, d_pk); z == 0³² → Abbruch
                                                                  s wie oben, öffnet sealed, prüft key_id,
                                                                  T_p und K in den Schlüsselbund
```

**Warum Commit/Reveal (Codex-Befund 1):** Ohne Commitment könnte ein bösartiger Hub nach
dem Claim ein Ersatz-`p_pk'` erzeugen und offline ~10⁶ Varianten durchprobieren, bis der
6-stellige Code zum echten iPhone-Code passt. Jetzt gilt (Muster RFC 6189 §4.4.1.1, ZRTP):
- Das iPhone legt sich mit `C` auf `(p_pk, n_p)` fest, **bevor** es `n_d` kennt.
- Der Desktop erzeugt `n_d` erst, **nachdem** er `C` gesehen hat. Ein Hub, der dem Desktop
  ein eigenes `C'` gibt, ist damit festgelegt, bevor `n_d` existiert.
- Ein Hub, der dem iPhone ein falsches `n_d'` gibt, muss es wählen, bevor das iPhone
  `(p_pk, n_p)` aufdeckt. In beiden Fällen bleibt ihm genau **ein** Versuch, Trefferchance 10⁻⁶.
- `d_pk` kommt über den QR am Hub vorbei; der Desktop prüft `C` selbst, nicht nur der Hub.

**Null-Prüfung (Codex-Befund 2):** Liefert X25519 ein Ergebnis aus 32 Nullbytes (Punkt
kleiner Ordnung, RFC 7748 §6.1), brechen beide Seiten **vor** HKDF ab. Rust:
`x25519_dalek::SharedSecret::was_contributory()`; Swift: Vergleich der Rohbytes von
`sharedSecretFromKeyAgreement` mit 32 Nullbytes in konstanter Zeit.

## 5 · Zustandsautomat und Token-Lebenszyklus (Codex-Befund 3)

| Zustand | Erreicht durch | Erlaubte nächste | Token T_p | Sensible Felder im Hub |
|---|---|---|---|---|
| `pending` | start | claimed, rejected, expired | – | secret-Hash |
| `claimed` | claim (secret ok, atomar) | challenged, rejected, expired | – | + poll_token-Hash, C |
| `challenged` | challenge (nur Starter-Token) | revealed, rejected, expired | – | + n_d |
| `revealed` | reveal (C geprüft) | approved, rejected, expired | – | + p_pk, n_p |
| `approved` | approve (nur Starter-Token, einmal) | done, rejected, expired | – | + sealed (Chiffrat) |
| `done` | result (einmal, Transaktion) | – | **angelegt** | sealed und poll_token-Hash gelöscht |
| `rejected` | reject | – | – | sealed gelöscht |
| `expired` | Frist, 3 Fehlversuche, Commitment falsch, neuer Start | – | widerrufen, falls je vermerkt | sealed und poll_token-Hash gelöscht |

Regeln:
1. Jeder Übergang ist ein bedingtes `UPDATE … WHERE status = <erwartet>`; der Verlierer
   eines Wettlaufs bekommt 409/410.
2. **T_p entsteht erst beim Abholen** im selben Datenbankvorgang wie `approved → done`.
   Zwischen Bestätigung und Abholung existiert kein Token, das ablaufen oder liegen
   bleiben könnte. Der Klartext steht nur in der einen HTTP-Antwort, im Hub nur der
   SHA-256-Hash (`api_tokens`).
3. Verfällt eine Kopplung, die je ein Token vermerkt hat, wird es **widerrufen**
   (`revoked_at`), nicht gelöscht.
4. Widerruf wirkt sofort: `ApiTokenService::verify` liest bei jedem Aufruf aus der
   Datenbank, es gibt keinen Auth-Cache.
5. Auf den Geräten liegen T_p und K im Schlüsselbund (iOS Keychain,
   `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`).
6. Grenze: Geht die Antwort von Schritt 6 verloren, existiert T_p ungenutzt. Es
   erscheint in „Verbundene Geräte“ und kann dort abgemeldet werden.

Tests (wai-portal `tests/cases/55_voice_sync_pairing_test.php`): voller Ablauf mit
Audit-Spur, einmaliger Claim, 3 Fehlversuche, approve vor reveal (409), approve doppelt
(409), Ablauf nach Bestätigung ohne Token, getauschtes p_pk (Commitment-Abbruch),
reveal vor challenge, Ablehnung, fremdes Gerät/Konto, Drosselung, Geräteliste/Widerruf.

## 5a · Sicherheitsregeln

1. Im QR steht **kein** Passwort, **kein** Token und **nicht** `K`.
2. `secret` = 32 Byte Zufall, im Hub nur als SHA-256 gespeichert, Vergleich in konstanter Zeit.
3. Frist **120 s** ab Start für den gesamten Ablauf.
4. **Einmalig:** genau ein `claim`, ein `challenge`, ein `reveal`, ein `approve`, ein Abholen.
5. Sehen, challenge, approve und reject nur mit dem Token, das die Kopplung gestartet hat.
6. Je Konto höchstens **eine** offene Kopplung; ein neuer Start verwirft die alte.
7. Drosselung: `claim` je IP 10/15 min; nach 3 falschen Geheimnissen wird die Kopplung verworfen.
   Unbekannt, abgelaufen, verwendet und falsches Geheimnis antworten gleich (410).
8. Flüchtige Schlüssel (`d_sk`, `p_sk`, `z`, `s`) nur im Speicher, nach Abschluss genullt.
9. Keine Inhalte und keine Geheimnisse im Log.

## 6 · Audit (Hub, `AuditService::log`)

| Ereignis | Wann |
|---|---|
| `voice_pairing.started` | Desktop startet |
| `voice_pairing.claimed` | iPhone hat gescannt (Gerätename, IP-Präfix) |
| `voice_pairing.approved` | Am PC bestätigt (noch kein Token) |
| `voice_pairing.delivered` | Abgeholt, Token angelegt (Token-ID) |
| `voice_pairing.rejected` | Am PC abgelehnt |
| `voice_pairing.expired` | Frist, Fehlversuche, Commitment falsch, neuer Start |
| `voice_pairing.secret_failed` | Falsches Geheimnis |
| `api_token.revoked` | Gerät in der Desktop-App abgemeldet |

## 7 · Hub-Schnittstelle (wai-portal)

| Methode | Pfad | Auth | Zweck |
|---|---|---|---|
| POST | `/api/v1/voice/pair/start` | `voice.sync` | `{desktop_pub}` → `{pair_id, secret, expires_at, ttl}` |
| POST | `/api/v1/voice/pair/claim` | Geheimnis | `{pairing_id, secret, phone_commit, device_name}` → `{poll_token, expires_at}` |
| GET | `/api/v1/voice/pair/{id}` | Starter-Token | Status, `device_name`, `phone_commit`, `phone_pub`, `phone_nonce` |
| POST | `/api/v1/voice/pair/{id}/challenge` | Starter-Token | `{desktop_nonce}` |
| POST | `/api/v1/voice/pair/{id}/reveal` | `X-Pairing-Token` | `{phone_pub, phone_nonce}` |
| POST | `/api/v1/voice/pair/{id}/approve` | Starter-Token | `{sealed}` (base64, ≤ 4 KB) |
| POST | `/api/v1/voice/pair/{id}/reject` | Starter-Token | Abbruch |
| GET | `/api/v1/voice/pair/{id}/result` | `X-Pairing-Token` | 202 waiting · 200 challenge+n_d · 403 rejected · 200 `{token, sealed, user}` einmalig · 410 |
| GET | `/api/v1/voice/devices` | `voice.sync` | Geräte = Tokens mit `voice.sync`: id, Name, angelegt, zuletzt aktiv, `current`, `via_pairing` |
| POST | `/api/v1/voice/devices/{id}/revoke` | `voice.sync` | Token widerrufen (nicht das eigene → 409) |

## 8 · Oberfläche Desktop

Ort: **Einstellungen › Konto & Geräte**, unter der Sync-Karte.

1. Knopf **„Handy verbinden“** (nur aktiv, wenn angemeldet; sonst Hinweis „Zuerst hier anmelden“).
2. Dialog Schritt 1: QR (≥ 240 px, hoher Kontrast, auch im Dunkelmodus schwarz auf weiß),
   Anleitung in drei Zeilen, Restzeit-Anzeige `1:47`, „Neuen Code“.
3. Schritt 2: „**iPhone von Patrick** möchte sich mit Ihrem Konto verbinden.“ Vergleichscode
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
- Keycloak: Wechsel betrifft nur Token-Ausstellung (Schritt 5) und Geräteliste.

## 11 · Umsetzung

| Paket | Repo | Inhalt |
|---|---|---|
| P1 | local-voice-ai | dieses Konzept, D15, Issue #289 |
| P2 | wai-portal | Migration `voice_pairings`, Controller, Tests, Audit |
| P3 | local-voice-ai | Rust `sync/pairing.rs` (X25519 `x25519-dalek`, BSD-3), QR (`qrcode`, MIT/Apache), Tauri-Befehle, Dialog + Geräteliste; Vorschau → Codex |
| P4 | local-voice-ai `apps/apple` | Scanner, Claim, SAS, Öffnen von `sealed` (CryptoKit Curve25519 + HKDF; XChaCha wie `SyncCrypto`), Keychain |
