# Konzept: Geräte-Sync ohne eigenes Portal

Stand 15.09.2026. Antwort auf Patricks Frage: Wie können veröffentlichte Nutzer ihre
Geräte synchronisieren, ohne dass Wolff Applied AI die Infrastruktur betreibt, und mit
einem Login, den sie schon haben (Google, GitHub)?

## 1 · Die Kernfrage zuerst: Login ist nicht Speicher

Ein „Sign in with Google" oder „Sign in with GitHub" liefert eine Identität, aber keinen
Ort, an dem Daten liegen. Irgendwo muss der verschlüsselte Stand gespeichert werden, den
das zweite Gerät abholt. Die Frage „wo werden die Daten zentral verwaltet?" hat deshalb nur
drei ehrliche Antworten:

| Ort | Wer betreibt | Login nötig |
|---|---|---|
| Eigener Hub (heute: wai-portal) | Wolff Applied AI | Portal-Konto |
| Speicher im Konto des Nutzers (GitHub-Repo, Google Drive, Dropbox, WebDAV) | der Anbieter des Nutzers | Konto des Nutzers, per OAuth |
| Ein Ordner, den der Nutzer bereits synchronisiert (OneDrive, iCloud Drive, Dropbox, Nextcloud-Client) | der Nutzer, ohne unser Zutun | keiner |

Die Verschlüsselung bleibt in allen drei Fällen Ende-zu-Ende: der Speicher sieht nur
Blobs. Das ist heute schon so (Argon2id → HKDF → XChaCha20-Poly1305, Spec E6). Nur der
Schlüssel darf nicht mehr aus dem Portal-Passwort kommen, sondern aus einer eigenen
**Sync-Passphrase**, die der Nutzer auf jedem Gerät eingibt. Login und Schlüssel sind dann
entkoppelt: der Login öffnet den Speicher, die Passphrase öffnet die Daten.

## 2 · Was heute schon steht und bleiben kann

Der Rust-Client in `src-tauri/src/sync/` ist fast vollständig speicherunabhängig:
Objektmodell, Verschlüsselung, Ledger mit Hash-basierter Dirty-Erkennung, Konfliktkopien,
Tombstones. Nur `client.rs` (HTTP zum Portal) und `account.rs` (Portal-Login,
Schlüsselableitung aus dem Passwort) kennen das Portal. Genau dort setzt der Umbau an.

## 3 · Vorschlag: ein Speicher-Trait, mehrere Backends

```
trait SyncBackend {
    fn push(&self, objects: &[SealedObject], expected_revisions: &[Rev]) -> Result<PushResult>;
    fn pull(&self, since: Cursor) -> Result<(Vec<SealedObject>, Cursor)>;
    fn status(&self) -> Result<BackendStatus>;
}
```

`engine.rs` spricht nur noch mit dem Trait. Die Backends, in der Reihenfolge, in der sie
Nutzen bringen:

### 3.1 Ordner-Backend (kein Login, kein Server) — zuerst

Der Nutzer wählt einen Ordner, der ohnehin zwischen seinen Geräten synchron gehalten wird
(OneDrive, iCloud Drive, Dropbox, Nextcloud, Syncthing). Die App legt dort je Objekt eine
verschlüsselte Datei ab (`<collection>/<id>.lvs`), plus ein kleines Manifest je Gerät.
Revision = Dateiinhalt-Hash; Konflikt = beide Geräte haben seit dem letzten Abgleich
geschrieben, dann wie heute Konfliktkopie. Cursor = Änderungszeit plus Hash.

- Für: funktioniert für jeden sofort, null Infrastruktur, null Konto, null Datenschutzfrage
  über Blobs hinaus, und macOS↔Windows über iCloud/OneDrive ist bei Patrick real vorhanden.
- Gegen: kein Push-Signal, Abgleich per Intervall (wie heute); Anbieter-Clients
  synchronisieren mit Verzögerung (Sekunden bis Minuten).
- Aufwand: Trait-Umbau 1 Tag, Ordner-Backend 1 Tag, Oberfläche 0,5 Tag.

### 3.2 GitHub-Repo-Backend (Login mit GitHub) — als „Server ohne Server"

Ein privates Repository im Konto des Nutzers (`local-voice-sync`), angelegt von der App.
Anmeldung per **GitHub Device Flow**: die App zeigt einen Code, der Nutzer bestätigt ihn im
Browser, kein Client-Secret in der App nötig. Objekte liegen als Dateien im Repo; die
Contents-API verlangt beim Schreiben die aktuelle Datei-SHA, das ist genau das
Compare-and-Swap, das der Push heute mit der Server-Revision macht. Pull = Vergleich des
Baum-SHA oder Commit-Liste seit dem letzten Cursor.

- Für: GitHub trägt Speicher, Versionierung, Verfügbarkeit; der Nutzer besitzt sein Repo
  und kann es jederzeit einsehen oder löschen; 5.000 API-Aufrufe je Stunde reichen für
  minütlichen Abgleich; Voraussetzung ist nur eine registrierte OAuth-App (kostenlos).
- Gegen: nur GitHub-Nutzer; Dateien bis 1 MB je Objekt (Texte ja, Audio nein);
  private Repos sind bei GitHub Free unbegrenzt, aber „Repo als Datenbank" ist für
  Nicht-Entwickler erklärungsbedürftig.
- Aufwand: 2 bis 3 Tage (Device Flow, Contents-API, CAS, Fehlerbilder, Rate-Limit).

### 3.3 Google-Drive-Backend (Login mit Google)

Google Drive hat einen für Nutzer unsichtbaren `appDataFolder`, in dem Apps eigene Dateien
ablegen. Anmeldung per OAuth mit PKCE über den lokalen Loopback (Standard für
Desktop-Apps). Der Scope `drive.appdata` gilt bei Google als nicht sensibel, die
Verifizierung der OAuth-App bleibt damit überschaubar. Objekte als Dateien, Revision =
Drive-Datei-Version, Pull über die Changes-API mit Seiten-Token als Cursor.

- Für: die größte Nutzerbasis; der Ordner ist für den Nutzer unsichtbar und wird mit der
  App gelöscht; 15 GB kostenloser Speicher.
- Gegen: OAuth-Consent-Screen, Google-Prüfung bei Veröffentlichung, ein Google-Cloud-
  Projekt, das jemand pflegt; die Token-Verwaltung (Refresh) ist mehr Code als bei GitHub.
- Aufwand: 2 bis 3 Tage plus die Google-Verifizierung als Wartezeit.

### 3.4 Portal-Backend (heutiger Stand)

Bleibt als Backend erhalten, wird nur hinter den Trait geschoben. Für Patrick selbst und
für alle, die einen betreuten Hub wollen. Kein Pflichtweg mehr.

## 4 · Was mit dem Login passiert

- **Sync-Passphrase statt Portal-Passwort.** Beim Einrichten des Sync auf dem ersten Gerät
  vergibt der Nutzer eine Passphrase; sie erzeugt den Schlüssel wie heute (Argon2id mit
  festem Salt aus einer Zufalls-ID, die neben den Daten liegt). Auf dem zweiten Gerät:
  Speicher verbinden, Passphrase eingeben, fertig. Verliert er die Passphrase, sind die
  Blobs wertlos, das ist der Preis von Ende-zu-Ende und muss in der Oberfläche stehen.
- **Der Login des Backends dient nur dem Speicherzugang.** GitHub-Token bzw. Google-Token
  liegen wie heute das Portal-Token unter `sync.json` mit eingeschränkten Dateirechten.
- **Ein „Google-Login ohne Google-Speicher" gibt es bewusst nicht.** Er würde nur die
  Identität liefern und wieder einen Hub von uns brauchen.

## 5 · Empfehlung

1. Trait-Umbau plus **Ordner-Backend** (3.1). Damit hat jeder Nutzer sofort einen Sync,
   und Patrick kann PC↔Mac über iCloud Drive oder OneDrive prüfen, ohne den Portal-Deploy
   abzuwarten.
2. **GitHub-Backend** (3.2) als zweites: Patricks Wunsch „GitHub als Server" ist damit
   erfüllt, und die technischen Nutzer der App haben ohnehin ein GitHub-Konto.
3. **Google Drive** (3.3) erst, wenn Nutzer danach fragen. Die Verifizierung ist der
   Aufwandstreiber, nicht der Code.

Gesamt für 1 und 2: rund fünf bis sechs Arbeitstage, plus einen Tag Oberfläche
(Backend-Auswahl in „Konto & Geräte", Passphrase-Dialog, Statuszeile je Backend).

## 6 · Was ich nicht empfehle

- Ein von uns gehosteter Vermittler „nur für den Login", der die Blobs dann doch bei uns
  ablegt. Das ist das Portal mit anderem Namen.
- Ein öffentliches GitHub-Repo oder Gists als Speicher: die Blobs wären zwar verschlüsselt,
  aber Existenz, Größe und Änderungsrhythmus der Texte wären für jeden sichtbar.
- Firebase, Supabase und ähnliche BaaS: bequem, aber wieder ein Konto und ein Projekt,
  das Wolff Applied AI pflegen und bezahlen muss.
