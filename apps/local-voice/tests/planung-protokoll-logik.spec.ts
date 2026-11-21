import { test, expect } from "@playwright/test";
import type { ProtokollStand } from "../src/bindings";
import {
  ERREICHT_WERTE,
  PROTOKOLL_FEHLER,
  angebot,
  fehlerCode,
  fehlerMeldung,
  protokollFehlerSchluessel,
} from "../src/components/planung/protokoll";
import {
  minutesErrorCode,
  minutesErrorDetail,
} from "../src/lib/meetingMinutes";

// P5 (lva#736): reine Logik der Protokoll-Karte, ohne Browser.

const stand = (extra: Partial<ProtokollStand> = {}): ProtokollStand => ({
  plan_id: "p1",
  meeting_id: "m1",
  vorlage_id: null,
  dokument_id: null,
  dokument_version: null,
  erreicht: null,
  kopf_fehlt: false,
  freigegeben: false,
  freigegeben_am: null,
  geaendert_seit_freigabe: false,
  ...extra,
});

test("Angebot folgt allein dem Stand des Backends", () => {
  expect(angebot(null)).toEqual({
    erzeugen: false,
    freigeben: false,
    entwurf: false,
  });
  expect(angebot(stand({ meeting_id: null }))).toEqual({
    erzeugen: false,
    freigeben: false,
    entwurf: false,
  });
  expect(angebot(stand())).toEqual({
    erzeugen: true,
    freigeben: false,
    entwurf: false,
  });
  expect(angebot(stand({ dokument_id: "d1", dokument_version: 1 }))).toEqual({
    erzeugen: true,
    freigeben: true,
    entwurf: false,
  });
  // Entwurf nur aus der freigegebenen, unveränderten Fassung.
  expect(
    angebot(
      stand({ dokument_id: "d1", dokument_version: 1, freigegeben: true }),
    ),
  ).toEqual({ erzeugen: true, freigeben: false, entwurf: true });
  expect(
    angebot(
      stand({
        dokument_id: "d1",
        dokument_version: 2,
        geaendert_seit_freigabe: true,
      }),
    ),
  ).toEqual({ erzeugen: true, freigeben: true, entwurf: false });
});

test("Ziel erreicht kennt genau ja, teilweise, nein", () => {
  expect([...ERREICHT_WERTE]).toEqual(["ja", "teilweise", "nein"]);
});

test("Fehlercodes werden aus der Meldung gelesen", () => {
  expect(fehlerCode("besprechung_fehlt")).toBe("besprechung_fehlt");
  expect(fehlerCode("Error: protokoll_fehlt")).toBe("protokoll_fehlt");
  expect(fehlerCode("nur_notizen: offen")).toBe("nur_notizen");
  for (const code of PROTOKOLL_FEHLER) {
    expect(protokollFehlerSchluessel(code)).toBe(
      `planung.protokoll.fehler.${code}`,
    );
  }
  expect(protokollFehlerSchluessel("irgendwas")).toBe(
    "planung.protokoll.fehler.allgemein",
  );
});

test("das Einwilligungs-Tor der Protokoll-Erzeugung hat einen eigenen Code", () => {
  expect(minutesErrorCode("nur_notizen: abgelehnt")).toBe("nur_notizen");
  expect(minutesErrorDetail("nur_notizen: abgelehnt")).toBe("abgelehnt");
  expect(minutesErrorCode("etwas Unbekanntes")).toBe("llm_failed");
});

test("die Meldung eines Fehlers ist sein Text, nicht Error: plus Text", () => {
  expect(fehlerMeldung(new Error("nur_notizen: offen"))).toBe(
    "nur_notizen: offen",
  );
  expect(fehlerMeldung("protokoll_fehlt")).toBe("protokoll_fehlt");
  // Genau der Weg der Protokoll-Karte: Error aus `api`, dann der Code des Tors.
  expect(minutesErrorCode(fehlerMeldung(new Error("nur_notizen: offen")))).toBe(
    "nur_notizen",
  );
  expect(minutesErrorCode(String(new Error("nur_notizen: offen")))).toBe(
    "llm_failed",
  );
});
