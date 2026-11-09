# Konzept: Stimmprofile – bekannte Stimmen automatisch erkennen

Stand: 04.11.2026 · Bezug: Issue #266 (Teil 1: Zusammenführen und Trennen, gemergt mit #267) · Status: 2a umgesetzt, 2b/2c Entwurf zur Freigabe

## 1. Ziel

Wer in einer Besprechung einen Sprecher benennt („Person 3“ → „Patrick Wolff“), soll dieses Wissen für spätere Aufnahmen nutzen können:

- Bekannte Stimmen (zuerst die eigene) werden in neuen Aufnahmen automatisch benannt oder als Vorschlag angeboten.
- Jede Korrektur (Umbenennen, Zusammenführen, Trennen) verbessert das Profil.
- Zerlegt die Sprechertrennung eine Person in mehrere Sprecher, schlägt die App das Zusammenführen vor („Person 3 klingt wie Patrick Wolff“).

Alles läuft lokal. Es gibt keinen Cloud-Dienst, keine Synchronisierung und kein Rohaudio im Profil.

## 2. Ausgangslage im Code

| Baustein | Stand |
|---|---|
| Sprechertrennung | Sortformer 4spk v2.1 (GGUF, transcribe-cpp). Das Modell ist ein End-to-End-Modell mit höchstens 4 Sprechern und liefert **keine** Stimm-Embeddings. |
| Sprecherzuordnung | `speakers`-Tabelle (Kanal, Nummer → Name, `human_id`, ungenutztes Feld `consent_state`). Personen in `humans` mit `is_self`. |
| Korrekturen | Umbenennen, Zusammenführen und Trennen (#261, #266), Segment einzeln umhängen. |
| ONNX-Laufzeit | Vorhanden (`transcribe-rs`, Feature `onnx`, CPU auf allen Plattformen). |

Folgerung: Für Stimmprofile braucht es ein **zusätzliches, kleines Sprecher-Embedding-Modell**. Die Sprechertrennung bleibt unverändert.

## 3. Modellwahl

| Kandidat | Größe | Lizenz (Gewichte) | Bemerkung |
|---|---|---|---|
| **3D-Speaker CAM++ (VoxCeleb, 16 kHz)** | ca. 28 MB | Apache-2.0 (vor Aufnahme prüfen) | 192-dim, schnell auf CPU, ONNX-Export über sherpa-onnx verbreitet. **Empfehlung.** |
| WeSpeaker ResNet34 (VoxCeleb) | ca. 26 MB | CC-BY-4.0 | Gute Qualität, Namensnennung nötig |
| NVIDIA TitaNet-small | ca. 25 MB | CC-BY-4.0 | NeMo-Export, sonst gleichwertig |

Nur permissive Lizenzen. Das Modell kommt in den Katalog (`catalog.json`, neuer Zweck `speaker_embedding`, Prüfsumme) und wird wie das Diarisierungsmodell bei Bedarf geladen. Für den Speicher gilt dasselbe RAM-Tor wie bei der Sprechertrennung.

## 4. Ablauf

```
Aufnahme / Import
  └─ Sprechertrennung (Sortformer)            → Turns je Kanal
       └─ Embedding je Sprecher (CAM++)       → Schwerpunkt aus den längsten Sätzen (≥ 2 s, bis 60 s)
            ├─ (A) Vergleich innerhalb der Besprechung
            │     cos ≥ 0,65 bei je ≥ 8 s Sprache → Vorschlag „zusammenführen“ (Teil 1)
            └─ (B) Vergleich mit Stimmprofilen (nur bei Opt-in)
                  cos ≥ 0,72  → Name automatisch, Hinweis „automatisch erkannt“ + Rückgängig
                  0,60–0,72   → Vorschlag wie die bestehenden Namensvorschläge
                  < 0,60      → nichts
```

- Die Schwellen für (B) sind Startwerte. Sie werden am eigenen Material kalibriert und sind in den Einstellungen nicht frei wählbar (nur „vorsichtig / ausgewogen“).
- **Kalibrierung (A), 04.11.2026:** Mini-LibriSpeech dev-clean-2 (CC-BY-4.0, 26 Sprecher, nur lokal gemessen, nicht im Repo). Je Sprecher zwei getrennte Hälften als „ein Mensch, zwei Sprecher“, alle anderen Paare als verschiedene Personen (650 Paare).

  | Sprache je Seite | gleiche Person: min / Median | verschiedene: max | Treffer bei 0,65 | Fehlvorschläge bei 0,65 |
  |---|---|---|---|---|
  | 4 s | 0,33 / 0,60 | 0,44 | 23 % | 0 |
  | 6 s | 0,63 / 0,72 | 0,52 | 96 % | 0 |
  | 8 s | 0,68 / 0,79 | 0,52 | 100 % | 0 |
  | 30 s | 0,79 / 0,93 | 0,53 | 100 % | 0 |
  | 60 s | 0,83 / 0,94 | 0,54 | 100 % | 0 |

  Deshalb: Schwelle 0,65, Vorschlag nur bei mindestens 8 s Sprache je Sprecher. Vorgelesene Sprache ist sauberer als eine Besprechung; die Werte werden mit echten Aufnahmen nachgeprüft.
- **ONNX Runtime:** CAM++ mittelt in Abschnitten zu 100 Zeitschritten (`AveragePool` mit `ceil_mode`). Einen angebrochenen Abschnitt teilt ONNX Runtime 1.24 (in der App) durch 100, 1.29 wie PyTorch durch die echte Länge. Die App rechnet deshalb nur mit ganzen Abschnitten (Vielfache von 2 s); dann stimmen beide Versionen und die Python-Referenz überein.
- Jede Stimme wird höchstens einem Sprecher je Besprechung zugeordnet (die beste Zuordnung zuerst). Zwei Sprecher, die beide zu einem Profil passen, ergeben einen Zusammenführungs-Vorschlag statt zweier gleicher Namen.
- Automatisch vergebene Namen sind markiert (`speakers.consent_state` bzw. neues Feld `name_source = auto`) und lassen sich mit einem Klick verwerfen.

## 5. Lernen aus Korrekturen

Ein Profil besteht nicht aus einem einzigen Vektor, sondern aus **Beiträgen je Besprechung**:

`voice_profile_contributions(profile_id, meeting_id, channel, speaker_index, embedding, speech_ms, created_at)`

Der Profilvektor ist der nach Redezeit gewichtete Mittelwert der Beiträge (gedeckelt, damit lange Besprechungen nicht dominieren).

| Aktion | Wirkung aufs Profil |
|---|---|
| Sprecher benennen (Person mit Profil) | Beitrag dieser Besprechung anlegen |
| Automatischen Namen bestätigen | Beitrag anlegen |
| Automatischen Namen verwerfen / umbenennen | Beitrag dieser Besprechung entfernen, Paar als „nicht diese Person“ merken |
| Zusammenführen / Trennen | Beitrag neu berechnen |
| Besprechung löschen (auch durch Aufbewahrungsfrist) | Beiträge dieser Besprechung löschen |

Weil die Beiträge einzeln vorliegen, lässt sich jede Korrektur sauber zurücknehmen. Auch die Löschung einer Besprechung erreicht das Profil.

## 6. Datenschutz (Art. 9 DSGVO)

Stimmprofile sind biometrische Daten zur eindeutigen Identifizierung, also eine besondere Kategorie nach Art. 9 DSGVO.

| Anforderung | Umsetzung |
|---|---|
| Rechtsgrundlage | Ausdrückliche Einwilligung (Art. 9 Abs. 2 lit. a). Für das eigene Profil erteilt der Nutzer sie selbst. Für andere Personen bestätigt der Nutzer ausdrücklich, dass die Person eingewilligt hat (Kontrollkästchen, kein Standardhaken). Gespeichert werden Datum und Version des Hinweistexts. |
| Standard | **Aus.** Ein Hauptschalter in den Einstellungen; ohne ihn werden weder Embeddings gespeichert noch Profile verglichen. |
| Opt-in je Person | Ein Profil entsteht nur über „Stimmprofil anlegen“ an der Person, nie stillschweigend beim Benennen. |
| Transparenz | Hinweis beim Einschalten: was gespeichert wird (Zahlenvektor, kein Audio), wo (lokal), wie lange (bis zur Löschung), wie man es löscht. |
| Löschung | Einzelnes Profil, alle Profile, ein Beitrag (eine Besprechung). Wirksam sofort, ohne Papierkorb. |
| Auskunft | Export je Profil als JSON (Art. 15/20): Person, Einwilligung, Beiträge mit Datum, ohne Vektoren auf Wunsch. |
| Datenminimierung | Kein Rohaudio. Nur Schwerpunkt-Vektoren. Keine Übertragung, keine Geräte-Sync in v1, kein Agenten-/MCP-Zugriff. |
| Sicherheit | Vektoren verschlüsselt (Schlüssel im Betriebssystem-Tresor: DPAPI unter Windows, Schlüsselbund unter macOS/iOS). Eigene Tabelle, nie im Log. |
| Vergleich innerhalb einer Besprechung (4A) | Kein Abgleich mit gespeicherten Identitäten, keine Speicherung über die Besprechung hinaus. Läuft ohne Profile, ist aber abschaltbar. Die Einordnung klärt der Datenschutzbeauftragte. |

**Dienstlicher Einsatz (Landratsamt):** Vor dem Einsatz mit fremden Stimmen sind eine DSFA (Art. 35, Biometrie), der Datenschutzbeauftragte und der Personalrat (LPVG BW, technische Einrichtung) einzubinden. Bis dahin bleibt die Funktion dienstlich auf das eigene Profil beschränkt. Das Gleiche gilt für die Poststelle (#267).

## 7. Oberfläche

- **Einstellungen › Besprechungen › Stimmprofile:** Hauptschalter, Hinweistext, Liste der Profile (Name, Zahl der Besprechungen, zuletzt gelernt, Einwilligung vom), je Profil „Beiträge ansehen“, „Löschen“, „Exportieren“; unten „Alle Stimmprofile löschen“.
- **Sprechertabelle:** In der Zeile eines benannten Sprechers gibt es „Stimmprofil anlegen“ bzw. „lernt mit“. Ein automatisch erkannter Name hat eine Markierung „erkannt“ mit „Stimmt nicht“. Vorschläge erscheinen wie die heutigen Namensvorschläge, Zusammenführungs-Vorschläge wie der Hinweis bei gleichem Namen aus Teil 1.
- **Personen:** An der Person steht der Profilstatus mit einem Link zur Verwaltung.

## 8. Umsetzung in Schritten

| PR | Inhalt | Abhängig von |
|---|---|---|
| 2a (umgesetzt) | Embedding-Modell in Katalog und Laufzeit, Embeddings je Sprecher berechnen (nur im Speicher), Zusammenführungs-Vorschläge innerhalb der Besprechung (4A), Kalibrierung | #266 |
| 2b | Profile mit Einwilligung, Beiträge, Verschlüsselung, automatisches Benennen und Vorschläge, Einstellungsseite, Export, Löschen | 2a, Freigabe Hinweistext |
| 2c | Mac und iOS mit Funktionsgleichheit (CoreML/ONNX), mit dem nächsten großen Release | 2b |

## 9. Offene Entscheidungen

1. ~~Modell~~: CAM++ (Apache-2.0, geprüft am 04.11.2026 im ModelScope-Repo iic/speech_campplus_sv_en_voxceleb_16k) ist mit 2a im Katalog.
2. ~~Vergleich innerhalb einer Besprechung~~: mit 2a standardmäßig an, aber erst nach dem Laden des Modells per Knopf (Einstellung „Gleiche Stimmen erkennen“). Die Einordnung durch den Datenschutzbeauftragten steht aus.
3. Automatisch benennen ab „ausgewogen“, oder nur Vorschläge, bis Patrick die Trefferquote am eigenen Material gesehen hat? Empfehlung: zuerst nur Vorschläge, das eigene Profil darf automatisch benennen.
