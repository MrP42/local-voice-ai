-- Fixture fuer die Migration Index 5 (A1, Goal Integrationen): so sehen
-- `calendar_sources`-Zeilen aus, wie sie der Kalenderdienst schreibt (alle
-- Zeiten Millisekunden UTC). Nachgebildet, KEINE echten Daten: Adressen sind
-- `*.invalid`, Geheimnisse (ICS-Adresse, Graph-Token) stehen nie in der Datenbank.
--
-- Drei Quellen: eine ICS-Quelle (mit Teilnehmerdaten, erfolgreich abgerufen), eine
-- Graph-Quelle (Microsoft 365) und eine entfernte ICS-Quelle (`deleted_at` gesetzt,
-- die Zeile bleibt fuer verknuepfte Besprechungen). Dazu ein Termin der ersten
-- Quelle, damit die Migration auch Zeilen in Nachbartabellen unberuehrt lassen muss.
INSERT INTO calendar_sources (id, kind, label, account_hint, enabled, has_attendee_data, etag,
                              last_modified, last_sync_at, last_ok_at, last_error,
                              created_at, updated_at, deleted_at)
VALUES
  ('01K8Z3Q6M2V7N4T9X5B1C8D0EF', 'ics', 'Outlook privat', 'outlook.office365.com', 1, 1,
   '"etag-1"', 'Tue, 29 Sep 2026 07:00:00 GMT', 1790000000000, 1790000000000, NULL,
   1789000000000, 1790000000000, NULL),
  ('graph-3fa9c1d27be04a58', 'graph', 'Microsoft 365 (Arbeit)', 'konto@example.invalid', 1, 1,
   NULL, NULL, 1790000100000, 1790000100000, NULL,
   1789100000000, 1790000100000, NULL),
  ('01K8Z3R0AA0000000000REMOVD', 'ics', 'Alter Kalender', 'calendar.example.invalid', 0, 0,
   NULL, NULL, 1780000000000, NULL, 'Adresse nicht gefunden (HTTP 404)',
   1779000000000, 1781000000000, 1781000000000);

INSERT INTO calendar_events (key, source_id, uid, title, starts_at, ends_at, all_day, cancelled,
                             location, join_url, description, reminded_at, dismissed_at, fetched_at)
VALUES
  ('01K8Z3Q6M2V7N4T9X5B1C8D0EF:uid-1:1790100000000', '01K8Z3Q6M2V7N4T9X5B1C8D0EF', 'uid-1',
   'Jour fixe', 1790100000000, 1790103600000, 0, 0, NULL, NULL, NULL, NULL, NULL, 1790000000000);
