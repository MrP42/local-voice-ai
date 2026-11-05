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

## 4 · Ablauf

```
Desktop (angemeldet: T_d, K)          Hub                         iPhone
1  erzeugt X25519 (d_pk, d_sk)
   POST /pair/start {d_pk}  ───────▶  legt Kopplung an:
                                       id, secret (nur Hash),
                                       expires = jetzt + 120 s
                            ◀───────  {id, secret, expires_at}
2  zeigt QR:
   localvoice://pair?v=1&h=<hub>&p=<id>&s=<secret>&k=<d_pk>
                                                          3  scannt, erzeugt (p_pk, p_sk)
                                       ◀─── POST /pair/claim {id, secret, p_pk, device_name}
                                       prüft Hash, Frist, Status
                                       pending → claimed (atomar),
                                       gibt poll_token zurück ───▶
                                                             zeigt Vergleichscode SAS
4  GET /pair/{id} (Polling 1 s) ──▶   claimed, device_name, p_pk
   zeigt: „iPhone von … möchte sich
   anmelden. Code 482 913“ [Bestätigen] [Ablehnen]
5  Bestätigen:
   s = X25519(d_sk, p_pk) → HKDF
   sealed = XChaCha20-Poly1305(s, {K, email, user_id, key_id})
   POST /pair/{id}/approve {sealed} ─▶ stellt Gerätetoken T_p aus
                                       (Scope voice.sync, Name „Voice · <Gerät>“),
                                       Status approved
                                       ◀─── GET /pair/{id}/result (poll_token)
                                       liefert {T_p, sealed} genau einmal,
                                       löscht danach T_p und sealed ───▶
                                                          6  s = X25519(p_sk, d_pk) → HKDF
                                                             öffnet sealed, prüft key_id,
                                                             speichert Konto im Schlüsselbund,
                                                             startet Abgleich
```

**Vergleichscode (SAS):** 6 Ziffern aus
`SHA-256("LV-PAIR-SAS-v1" ‖ id ‖ d_pk ‖ p_pk)`. Beide Geräte rechnen ihn aus dem, was sie
selbst sehen. Er deckt zwei Angriffe ab:
- Jemand fotografiert den QR und kommt dem eigenen iPhone zuvor → anderer Gerätename,
  anderer Code, und das eigene iPhone meldet „Code schon verwendet“.
- Der Hub (oder ein Mittelsmann) tauscht `p_pk` aus → Codes stimmen nicht überein.
  `d_pk` kommt über den QR, also am Hub vorbei.

**HKDF-Info:** `"LV-PAIR-KEY-v1" ‖ id`; Salt = `d_pk ‖ p_pk`. Nonce 24 Byte zufällig.

## 5 · Sicherheitsregeln

1. Im QR steht **kein** Passwort, **kein** Token und **nicht** `K`.
2. `secret` = 32 Byte Zufall, im Hub nur als SHA-256 gespeichert, Vergleich in konstanter Zeit.
3. Frist **120 s** ab Start für den gesamten Ablauf (Start → Ergebnis abgeholt). Danach
   `expired`, alles Sensible gelöscht.
4. **Einmalig:** genau ein `claim` (atomarer Statuswechsel), genau ein Abholen des Ergebnisses.
5. Bestätigung nur am PC und nur durch dasselbe Konto (`T_d` des Starters).
6. Je Konto höchstens **eine** offene Kopplung; ein neuer Start bricht die alte ab.
7. Drosselung: `claim` je IP 10/15 min; falsches `secret` zählt als Fehlversuch, nach 3 auf
   eine Kopplung wird sie verworfen.
8. Gerätetoken `T_p` ist ein eigenes Token und einzeln widerrufbar; `T_d` wird nie weitergegeben.
9. Flüchtige Schlüssel (`d_sk`, `p_sk`) nur im Speicher, nach Abschluss/Ablauf verworfen.
10. Keine Inhalte und keine Geheimnisse im Log; nur Kopplungs-ID-Präfix, Konto-ID, Gerätename.

## 6 · Audit (Hub, `AuditService::log`)

| Ereignis | Wann |
|---|---|
| `voice_pairing.started` | Desktop startet |
| `voice_pairing.claimed` | iPhone hat gescannt (Gerätename, IP-Präfix) |
| `voice_pairing.approved` | Bestätigt, Token angelegt (Token-ID) |
| `voice_pairing.rejected` | Am PC abgelehnt |
| `voice_pairing.expired` | Frist abgelaufen |
| `voice_pairing.secret_failed` | Falsches Geheimnis |
| `api_token.revoked` | Gerät in der Desktop-App abgemeldet |

## 7 · Hub-Schnittstelle (wai-portal)

| Methode | Pfad | Auth | Zweck |
|---|---|---|---|
| POST | `/api/v1/voice/pair/start` | `voice.sync` | `{desktop_pub}` → `{pairing_id, secret, expires_at}` |
| POST | `/api/v1/voice/pair/claim` | keins (Geheimnis) | `{pairing_id, secret, phone_pub, device_name}` → `{poll_token, expires_at}` |
| GET | `/api/v1/voice/pair/{id}` | `voice.sync` (Starter) | Status, `device_name`, `phone_pub` |
| POST | `/api/v1/voice/pair/{id}/approve` | `voice.sync` (Starter) | `{sealed}` |
| POST | `/api/v1/voice/pair/{id}/reject` | `voice.sync` (Starter) | Abbruch |
| GET | `/api/v1/voice/pair/{id}/result` | `poll_token` (Header) | `pending` / `rejected` / `{token, sealed, hub_user}` einmalig |
| GET | `/api/v1/voice/devices` | `voice.sync` | Tokens des Kontos mit Scope `voice.sync`: id, Name, angelegt, zuletzt genutzt, „dieses Gerät“ |
| POST | `/api/v1/voice/devices/{id}/revoke` | `voice.sync` | Token widerrufen (nicht das eigene) |

Tabelle `voice_pairings`: `id` (Text, 22 Zeichen base64url), `user_id`, `secret_hash`,
`poll_token_hash`, `desktop_pub`, `phone_pub`, `device_name`, `status`
(`pending|claimed|approved|rejected|expired|done`), `sealed`, `issued_token_id`,
`issued_token_plain` (nur bis zur Abholung, danach NULL), `failed_attempts`,
`created_at`, `expires_at`. Abgelaufene Zeilen räumt jeder Aufruf mit auf.

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
  Schlüsselrotation braucht Passwortwechsel (eigenes Paket, nicht Teil von #289).
- Der Desktop muss online und am Hub angemeldet sein.
- Keycloak: Wechsel betrifft nur Token-Ausstellung (Schritt 5) und Geräteliste.

## 11 · Umsetzung

| Paket | Repo | Inhalt |
|---|---|---|
| P1 | local-voice-ai | dieses Konzept, D15, Issue #289 |
| P2 | wai-portal | Migration `voice_pairings`, Controller, Tests, Audit |
| P3 | local-voice-ai | Rust `sync/pairing.rs` (X25519 `x25519-dalek`, BSD-3), QR (`qrcode`, MIT/Apache), Tauri-Befehle, Dialog + Geräteliste; Vorschau → Codex |
| P4 | local-voice-ai `apps/apple` | Scanner, Claim, SAS, Öffnen von `sealed` (CryptoKit Curve25519 + HKDF; XChaCha wie `SyncCrypto`), Keychain |
