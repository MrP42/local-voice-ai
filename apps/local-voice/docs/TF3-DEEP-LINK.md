# TF3 Deep-Link (lva#845)

Keine neue Abhaengigkeit. Windows registriert `localvoice` ausschliesslich im
HKCU des installierenden Kontos, portable Installationen registrieren nichts.
Ein vorhandener Handler wird nicht uebernommen. Die Deinstallation entfernt nur
zum eigenen EXE-Pfad passende Werte und anschliessend tatsaechlich leere Schluessel.
Parallele Installationen mit anderem Pfad uebernehmen den Handler daher nicht.

Vertrag: `localvoice://terminfindung/T-XXXX`, wobei X jeweils ein ASCII-Grossbuchstabe
oder eine Ziffer ist. Nur das Schema ist ohne Beachtung der Grossschreibung;
Autoritaet und ID sind exakt. Keine Queries, Fragmente, Escapes oder Zusatzpfade.
URL-Starts umgehen Clap und koennen keine CLI-Aktionsflags ausloesen. Windows nimmt
genau ein Argument an, auch nach der Weitergabe durch Single-Instance (dessen
Transport trennt Argumente mit `|`). Ungueltige Aufrufe erhoehen nur einen Zaehler.

Das Backend merkt ein letztes gueltiges Ziel und sendet nach `deep_link_ready`
`deep-link-open { ziel: "terminfindung", id }` ausschliesslich an `main`. Der
Frontend-Listener wird vor dem Handshake eingerichtet. Neuladen sperrt die
Zustellung wieder bis zum neuen Handshake. Kein Parameter und keine URL werden
geloggt; der Link fuehrt keine Fachaktion aus.

Anbindung fuer die parallel entstehende Terminfindungsansicht:
`src/lib/deepLink.ts` exportiert `deepLinkTarget.getSnapshot` und `subscribe`
fuer `useSyncExternalStore`. Es gibt genau ein letztes Ziel, ohne Persistenz.
Die Terminfindungsansicht existiert in diesem Zweig noch nicht; TF3 stellt ihren
Eingang bereit. Bestaetigungen fuer Fachaktionen bleiben in dieser Ansicht.

macOS: `bundle.macOS.infoPlist` bindet das bestehende Info.plist mit
`CFBundleURLTypes` ein. In der lokal vorhandenen Tauri-2.11.5-Quelle
(`src/app.rs`) ist `RunEvent::Opened { urls: Vec<url::Url> }` fuer macOS belegt.
Grenze: Tauri liefert hier bereits geparste URLs. Eine vor diesem Ereignis
erfolgte Normalisierung (z.B. von Punktsegmenten) ist nicht mehr erkennbar.
Die strenge Rohtextpruefung gilt fuer die dem Parser tatsaechlich uebergebene
Darstellung; sie ist keine Garantie ueber den urspruenglichen macOS-Aufruf.

## Pruefung und Uebergabe

- Offline: `node --test --test-isolation=none scripts/deep-link.test.mjs`
  (Verdrahtungsvertrag und echtes Frontend-Verhalten; vor Umsetzung rot).
- Durch Thread-Claude: `cargo test --lib deep_link::tests` in `src-tauri` sowie
  gezielter Rust-/Frontend-Bau. Die fuenf Rust-Tests pruefen Parser, Puffer,
  Fehlzustellung und CLI-/Transport-Gegenbeispiele mit synthetischen Daten.
  Ihr Lauf und ihre rote Gegenprobe sind hier noch nicht erfolgt.
- Windows-Tor nach Installation: `start localvoice://terminfindung/T-TEST`
  mit geschlossener und mit laufender App; jeweils Fenster vorn, nur eine
  Instanz, Ziel im Frontend, keine Fachaktion. Mehrere Links vor Bereitschaft:
  nur letztes gueltiges Ziel. Ungueltiger Link: keine Zustellung, nur Zaehler.
- Installer-Tor: portable Installation ohne Registrierung; fremder Handler
  bleibt unveraendert; Deinstallation entfernt eigenen Handler, erhaelt fremde
  Werte/Unterschluessel und einen inzwischen geaenderten Fremdhandler.
- macOS-Tor auf dem Mac: Bundle-Registrierung und Opened-Ereignis bei kalter
  und laufender App pruefen, einschliesslich der Normalisierungsgrenze oben.

Hier wurden weder Registrierung noch App-Starts, Builds oder Installationen
ausgefuehrt. Keine neue Lizenzangabe erforderlich.
