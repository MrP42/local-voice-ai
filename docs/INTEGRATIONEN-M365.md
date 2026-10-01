# Microsoft-365-Konto einrichten und prüfen

Stand 01.10.2026 (Paket A5, Goal „Integrationen“, Issue #66). Für Patrick: so legst du die
App-Registrierung an, meldest dich zum ersten Mal an und prüfst Mail und OneDrive.

## Was das Konto kann

| Fähigkeit | Was | Berechtigung (Microsoft Graph, delegiert) |
|---|---|---|
| Mail senden | Follow-up-Mail der Besprechung („Über … senden“), Testmail | `Mail.Send` |
| Dateien schreiben | Datei in einen OneDrive-Ordner legen; ab 4 MiB per Upload-Sitzung in Stücken | `Files.ReadWrite` (ganzes OneDrive) oder `Files.ReadWrite.AppFolder` (nur private Konten) |
| Termine anlegen | Notiz an den Outlook-Termin einer Besprechung hängen (Einladung bleibt unverändert) | `Calendars.ReadWrite` |

Dazu immer `offline_access` (damit du dich nicht ständig neu anmelden musst) und `User.Read` (dein
Name und deine Adresse zur Anzeige). **Die Anmeldung fragt nur die Berechtigungen der Fähigkeiten an, die
du eingeschaltet hast.** Wenn du später eine weitere einschaltest, zeigt das Konto „Zustimmung erweitern“;
bis du dich erneut angemeldet hast, läuft diese Fähigkeit nicht (es geht kein Aufruf ans Netz).

Alles Schreibende läuft über das Freigabe-Tor der Seite „Integrationen“: Du selbst brauchst in der App keine
Freigabe; Workflows und lokale Agenten fragen standardmäßig nach („fragen“), externe Agenten sind „aus“.
Jede Aktion steht im Protokoll (Reiter „Protokoll“).

## 1. App-Registrierung in Microsoft Entra anlegen

Du brauchst eine eigene Registrierung (öffentlicher Client). Die Bezeichnungen im Portal ändern sich gelegentlich;
Sinn und Reihenfolge bleiben. Hast du für den Microsoft-365-Kalender schon eine Registrierung („Local Voice AI“),
ergänze nur Schritt 4 und nimm dieselbe Anwendungs-ID.

1. Melde dich im Entra-Admin-Center an (`entra.microsoft.com`) und öffne **Identität → Anwendungen →
   App-Registrierungen → Neue Registrierung**.
2. **Name:** `Local Voice AI`. **Unterstützte Kontotypen:** „Konten in einem beliebigen Organisationsverzeichnis
   und persönliche Microsoft-Konten“ (so funktioniert `common`; nur dein Verzeichnis geht auch, dann trägst du
   später dessen Verzeichnis-ID oder Domain als „Verzeichnis (Mandant)“ ein).
3. **Umleitungs-URI:** Plattform **Mobil- und Desktopanwendungen**, Wert `http://localhost` (ohne Port und ohne
   Pfad; Microsoft vergleicht Loopback-Adressen unabhängig vom Port). Registrieren.
4. **API-Berechtigungen → Berechtigung hinzufügen → Microsoft Graph → Delegierte Berechtigungen:**
   `User.Read`, `offline_access`, `Mail.Send`, `Files.ReadWrite`, `Calendars.ReadWrite`.
   Nur für ein **privates** Konto mit App-Ordner zusätzlich `Files.ReadWrite.AppFolder` (bei Arbeits- und
   Schulkonten gibt es sie nicht). Verlangt dein Verzeichnis Administratorzustimmung, klicke
   „Administratorzustimmung erteilen“ oder bitte den Administrator darum (Fehlerbild unten).
5. **Authentifizierung → Erweiterte Einstellungen → Öffentliche Clientflows zulassen: Ja.**
6. Auf der Übersichtsseite die **Anwendungs-ID (Client)** kopieren (Format
   `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`). Sie ist kein Geheimnis. Ein Client-Geheimnis legst du **nicht** an;
   die App nutzt PKCE.

## 2. In der App einrichten

1. **Integrationen → Hinzufügen → „Microsoft 365“ → Konto einrichten.**
2. Name (z. B. „Büro-Konto“), **Anwendungs-ID** einfügen (leer lassen übernimmt die Client-ID des
   Microsoft-365-Kalenders, falls dort eine steht), **Verzeichnis** leer lassen (= `common`).
3. **Fähigkeiten** wählen. Voreinstellung: Mail senden und Dateien schreiben. Für OneDrive: „Ganzes OneDrive“
   (Standard, alle Kontotypen) oder „Nur der App-Ordner“ (nur private Konten) und den Ordnernamen
   (Standard `Local Voice AI`, wird in OneDrive angelegt).
4. **Anlegen.** Die Seite zeigt das Konto mit „Anmeldung nötig“ und der Liste der Berechtigungen, die angefragt
   werden.
5. **Mit Microsoft anmelden.** Der Standardbrowser öffnet sich; melde dich an und stimme zu. Die App wartet bis zu
   5 Minuten (Knopf „Abbrechen“). Danach steht dort „Angemeldet und bereit“ und dein Konto.

Das Token liegt verschlüsselt im Windows-Geheimnisspeicher der App (nur dein Windows-Benutzer kann es lesen),
nie in den Einstellungen, in der Datenbank, im Protokoll oder in einer Fehlermeldung.

## 3. Deine Prüfungen (Owner-Schritte)

Alle drei findest du im Konto unter **Ausprobieren** (erscheint, sobald das Konto bereit ist).

| Schritt | Was du tust | Erwartetes Ergebnis |
|---|---|---|
| Erster Login | „Mit Microsoft anmelden“, im Browser zustimmen | Zustand „Angemeldet und bereit“, Konto sichtbar; „Verbindung testen“ meldet „Verbunden als …“ |
| Testmail | „Testmail an mich senden“ | Meldung „Testmail gesendet“; die Mail liegt in deinem Posteingang und im Ordner „Gesendet“ |
| Datei in OneDrive | „Testdatei in OneDrive ablegen“ | Meldung mit dem Dateinamen (`local-voice-test-<Datum>.txt`); die Datei liegt in OneDrive im eingestellten Ordner |
| Follow-up-Mail | In einer Besprechung „Follow-up-Mail“, Entwurf erzeugen, **„Über <Konto> senden“**, bestätigen | Meldung „Gesendet über …“; Mail kommt an |
| Große Datei (optional) | Über einen Workflow oder Agenten (später) eine Datei über 4 MiB ablegen | wird in Stücken hochgeladen; die Rust-Tests decken das gegen einen Test-Server ab |

Die Notiz am Termin hat noch keinen eigenen Knopf in der Besprechung (der Befehl `m365_event_note` ist fertig und
getestet; die Oberfläche folgt mit den Workflows). Sie braucht die Fähigkeit „Termine anlegen“ und funktioniert
nur bei Terminen, die du selbst organisierst (Teilnehmende dürfen den Text eines Termins nicht ändern; die App sagt das).

## Wenn etwas nicht klappt

| Meldung oder Zustand | Ursache | Lösung |
|---|---|---|
| „Anmeldung nötig: … abgelaufen oder widerrufen“ | Microsoft hat das Token widerrufen (Kennwortwechsel, Richtlinie, lange nicht benutzt) | „Mit Microsoft anmelden“; das tote Token ist schon gelöscht |
| „Zustimmung erweitern“ | Du hast eine Fähigkeit eingeschaltet, deren Berechtigung noch nicht zugestimmt ist | erneut anmelden und zustimmen |
| Browser zeigt `AADSTS65001` oder „Administratorzustimmung erforderlich“ | Dein Verzeichnis lässt Nutzer nicht zustimmen | Administrator um „Administratorzustimmung erteilen“ für die App bitten (Schritt 1.4) |
| `AADSTS50011` „Umleitungs-URI stimmt nicht überein“ | Umleitung falsch registriert | Plattform „Mobil- und Desktopanwendungen“ mit `http://localhost` anlegen (nicht „Web“, kein Port) |
| `AADSTS7000218` / „client_assertion oder client_secret erforderlich“ | „Öffentliche Clientflows zulassen“ steht auf Nein | auf Ja stellen (Schritt 1.5) |
| `AADSTS70011` „invalid_scope“ bei OneDrive | „Nur App-Ordner“ gewählt, aber Arbeits- oder Schulkonto | „Ganzes OneDrive“ wählen |
| „Microsoft hat den Zugriff abgelehnt“ (403) | Mandant sperrt die Berechtigung, oder du bist nicht der Organisator des Termins | Administrator fragen; bei Termin-Notizen einen eigenen Termin wählen |
| „Der OneDrive-Speicher ist voll“ | Kontingent erschöpft | Platz schaffen; die Upload-Sitzung wird abgebrochen, es bleibt nichts Halbes liegen |
| „Unklar, ob die Aktion ausgeführt wurde“ | Die Verbindung riss, nachdem die Mail abgeschickt war | zuerst im Ordner „Gesendet“ nachsehen, nicht blind wiederholen |
| „Microsoft drosselt die Abfragen“ | Zu viele Anfragen (HTTP 429) | in der genannten Zeit erneut versuchen |

## Abmelden und Widerrufen

- **Abmelden** (im Konto): löscht das Token auf diesem Rechner. Das Konto bleibt, mit „Anmeldung nötig“.
- **Entfernen** (Detailseite, unten): löscht das Konto samt Rechten und Token; das Protokoll bleibt.
- **Zustimmung bei Microsoft zurückziehen:** Arbeits- und Schulkonto unter `myapps.microsoft.com` →
  Profil → „Apps verwalten“; privates Konto unter `account.live.com/consent/Manage`.

## Technik (für Entwickler)

- Code: `apps/local-voice/src-tauri/src/managers/integrations/m365/` (Konfiguration und Scopes, Dienst, Mail,
  OneDrive, Termin-Notiz, Tor), Befehle in `commands/integrations_m365.rs`, Oberfläche in
  `src/components/integrations/M365*.tsx` und im Follow-up-Dialog.
- Anmeldung, PKCE und Loopback-Listener sind die des Kalenders (`calendar/graph.rs`), mit frei wählbaren Scopes.
- Fehlerfälle und ihre Absicherung stehen im Kopf von `m365.rs` (Tabelle), Test je Zeile.
- Prüfen: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib integrations::m365`
  (Test-Server statt Microsoft, kein Browser) und `npx playwright test tests/integrations-m365.spec.ts`
  (Attrappe, kein echter Login).
