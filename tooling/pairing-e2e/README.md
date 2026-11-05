# Ende-zu-Ende-Test der Geräte-Kopplung (local-voice-ai#289)

Prüft den Desktop-Teil (`apps/local-voice/src-tauri/src/sync/pairing.rs` und
`pairing_proto.rs`) gegen einen **echten** Hub (wai-portal). Das iPhone wird im Test
mit denselben Protokollfunktionen nachgebildet. Die Tauri-Makros sind durch
Platzhalter ersetzt (`fakemacros`, `faketauri`), damit der Test ohne WebView läuft
(Linux-Container, CI).

Geprüfte Pfade:

1. Erfolgsfall: QR → claim → challenge → reveal → gleicher Vergleichscode →
   Freigabe → iPhone öffnet `sealed` (K, E-Mail, user_id) → Bestätigung → „Verbunden“;
   das neue Token synchronisiert; die Geräteliste zeigt `paired_by`.
2. Falsches QR-Geheimnis `q` (Hub oder Dritter): Desktop bricht ab.
3. Ungültige Bestätigung: „nicht bestätigt“ mit dem neuen Gerät zum Abmelden;
   nach dem Abmelden ist dessen Token ungültig.
4. „Nein“ beim Vergleichscode: iPhone erhält 403.
5. Nach dem Abbruch ist keine Kopplung mehr offen.
6. Schließen nach der Freigabe, vor dem Abholen: Freigabe zurückgezogen, iPhone erhält 403.
7. Schließen nach Abholen und gültiger Bestätigung: „Verbunden“, kein Abmeldeangebot.

Ohne `HUB_E2E` laufen nur die Einheitstests (Testvektoren, Kodierung, QR).

## Lauf

```bash
# Hub lokal (wai-portal-Checkout, PHP 8.3, SQLite)
cd ../wai-portal
export DB_DATABASE=/tmp/lv-pair-e2e.sqlite
php bin/console.php fresh && php bin/console.php seed
php bin/console.php seed:admin patrick@example.org 'korrekt-123'
AGENT_API_ENABLED=true php -S 127.0.0.1:8099 -t public public/_router.php &

# Test
cd local-voice-ai/tooling/pairing-e2e
HUB_E2E=http://127.0.0.1:8099 cargo test
```

Der Hub drosselt `claim` auf 10 je 15 Minuten und Netz; ein Lauf braucht 6.
Vor jedem weiteren Lauf innerhalb von 15 Minuten die Datenbank mit `fresh` neu aufbauen.
