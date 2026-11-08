# Datenordner auf ein lokales Laufwerk umziehen

Stand: 08.11.2026 (lva#371). Diese Anleitung gilt, wenn die App beim Zurückspielen einer Sicherung
meldet, dass das Laufwerk des Datenordners Änderungen nicht sicher festschreiben kann. Das betrifft
manche Netz- und Cloud-Laufwerke, etwa umgeleitete Benutzerordner oder eine portable App auf einer
Netzfreigabe.

Einen automatischen Umzug gibt es noch nicht; er kommt als eigenes Paket.

## Wo der Datenordner liegt

| Installation | Datenordner |
|---|---|
| Windows, installiert | `%LOCALAPPDATA%\de.wolffappliedai.localvoiceai` |
| Windows, portabel | Ordner `Data` neben `Local Voice AI.exe` |
| macOS | `~/Library/Application Support/de.wolffappliedai.localvoiceai` |

„Datenordner öffnen“ in den Einstellungen zeigt den Ordner, den die App gerade nutzt.

## Umziehen

1. **App vollständig beenden.** Auch das Symbol im Infobereich bzw. in der Menüleiste über „Beenden“
   schließen. Solange die App läuft, schreibt sie in den Ordner.
2. **Den gesamten Datenordner verschieben**, nie nur Teile davon. Bibliothek, Seiten, Sicherungen
   und Einstellungen gehören zusammen.
3. **Neuen Ort einrichten:**
   - *Portabel (Windows):* den ganzen Ordner der portablen App (Programm und `Data`) auf ein lokales
     Laufwerk verschieben, z. B. nach `C:\LocalVoiceAI`. Die App nutzt dann automatisch das `Data`
     daneben. Siehe [PORTABLE.md](PORTABLE.md).
   - *Installiert (Windows):* liegt `%LOCALAPPDATA%` auf einem Netzlaufwerk, die portable Version auf
     ein lokales Laufwerk entpacken und den Inhalt des alten Datenordners in deren `Data` kopieren.
     Ob `%LOCALAPPDATA%` umgeleitet ist, weiß die IT.
   - *macOS:* der Datenordner liegt immer lokal; betroffen ist nur ein Ordner, der selbst auf ein
     Netzlaufwerk verweist. Dann diesen Verweis durch einen echten Ordner ersetzen.
4. App starten und das Zurückspielen wiederholen.

## Ohne Umzug

„Sicherung exportieren“ kopiert eine Sicherung geprüft in einen neuen Ordner, etwa auf einem lokalen
Laufwerk. Am Datenordner ändert das nichts. Der Best-Effort-Modus („Trotzdem zurückspielen“) ist nur
wählbar, wenn das Laufwerk Dateien sicher schreibt und nur der Ordner-Sync fehlt. Die
Rückfallkopien bleiben dabei erhalten.
