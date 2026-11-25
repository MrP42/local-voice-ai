import type { Page } from "@playwright/test";

// Attrappe der Terminfindung (lva#845 TF5) über der Planungs-Attrappe: Befehle poll_*, das
// Microsoft-365-Konto und die Verfügbarkeit. Nur synthetische Daten (example.org/.net/.com).
// Muss NACH installMock() installiert werden; sie umhüllt dessen invoke.

export type TfZustand = "leer" | "entwurf" | "offen" | "entschieden";

export async function installTerminfindung(page: Page, zustand: TfZustand) {
  await page.addInitScript((zustand: TfZustand) => {
    if (window.top !== window) return;
    const w = window as any;
    const MIN = 60_000;
    const TAG = 86_400_000;
    const heute = new Date();
    const wt = (heute.getDay() + 6) % 7;
    // Montag der nächsten Woche, 00:00 Ortszeit.
    const mo = new Date(
      heute.getFullYear(),
      heute.getMonth(),
      heute.getDate() - wt + 7,
    ).getTime();
    const um = (tag: number, h: number, m = 0) =>
      mo + tag * TAG + (h * 60 + m) * MIN;
    const v = (nr: number, start: number) => ({
      nr,
      start,
      ende: start + 60 * MIN,
      ortszeit: null,
    });
    const personen = [
      {
        adresse: "anna.koenig@example.org",
        name: "Anna König",
        intern: true,
        pflicht: true,
      },
      {
        adresse: "malte.brandt@example.org",
        name: "Malte Brandt",
        intern: true,
        pflicht: false,
      },
      {
        adresse: "sven.lorenz@example.org",
        name: "Sven Lorenz",
        intern: true,
        pflicht: false,
      },
      {
        adresse: "julia.hahn@example.net",
        name: "Julia Hahn",
        intern: false,
        pflicht: true,
      },
      {
        adresse: "tobias.roth@example.com",
        name: "Tobias Roth",
        intern: false,
        pflicht: false,
      },
      {
        adresse: "eva.maurer@example.com",
        name: "Eva Maurer",
        intern: false,
        pflicht: false,
      },
    ];
    const vorschlaege = [
      v(1, um(1, 9)),
      v(2, um(2, 13, 30)),
      v(3, um(3, 10)),
      v(4, um(4, 15)),
    ];
    const poll = {
      id: "T-7K3Q",
      plan_id: "p_entwurf",
      titel: "Abstimmung Pilotstart",
      titel_an_externe: false,
      dauer_min: 60,
      frist: um(-3, 12),
      status:
        zustand === "entschieden"
          ? "entschieden"
          : zustand === "offen"
            ? "offen"
            : "entwurf",
      vorschlaege:
        zustand === "entwurf" ? vorschlaege.slice(0, 3) : vorschlaege,
      teilnehmende: personen,
    };
    const a = (
      person: string,
      nr: number,
      wert: string,
      quelle: string,
      sicher = true,
    ) => ({
      person,
      vorschlag_nr: nr,
      wert,
      quelle,
      sicher,
      eingegangen: Date.now() - 3600_000,
    });
    const reihe = (person: string, quelle: string, werte: string[]) =>
      werte.map((w, i) => a(person, i + 1, w, quelle));
    const antworten =
      zustand === "offen" || zustand === "entschieden"
        ? [
            ...reihe("anna.koenig@example.org", "kalender", [
              "passt",
              "passt",
              "passt",
              "passt_nicht",
            ]),
            ...reihe("malte.brandt@example.org", "kalender", [
              "passt",
              "passt",
              "notfalls",
              "passt",
            ]),
            ...reihe("sven.lorenz@example.org", "knopf", [
              "passt",
              "passt_nicht",
              "passt",
              "passt_nicht",
            ]),
            ...reihe("julia.hahn@example.net", "auswahl", [
              "passt",
              "passt_nicht",
              "notfalls",
              "passt",
            ]),
          ]
        : [];
    const history =
      zustand === "offen"
        ? [
            {
              id: 7,
              sender: "tobias.roth@example.com",
              person: "tobias.roth@example.com",
              received_at: Date.now() - 20 * MIN,
              result: {
                art: "pruefen",
                werte: [
                  { vorschlag_nr: 1, wert: "passt" },
                  { vorschlag_nr: 3, wert: "notfalls" },
                ],
                quelle: "freitext",
                sicher: false,
                grund: "freitext_unsicher",
              },
              confirmed: false,
            },
          ]
        : [];
    const raenge = [
      {
        nr: 1,
        punkte: 22,
        ausgeschlossen: false,
        begruendung:
          "Alle 2 Pflicht-Teilnehmenden können; 4 von 4 Antwortenden passt es.",
      },
      {
        nr: 3,
        punkte: 13,
        ausgeschlossen: false,
        begruendung: "Pflicht vollständig, 2 Antworten nur „notfalls“.",
      },
      {
        nr: 2,
        punkte: 0,
        ausgeschlossen: true,
        begruendung: "Pflicht-Person Julia Hahn kann nicht.",
      },
      {
        nr: 4,
        punkte: 0,
        ausgeschlossen: true,
        begruendung: "Pflicht-Person Anna König kann nicht.",
      },
    ];
    // Verfügbarkeit Mo 00:00 – Sa 00:00, 30-min-Raster, Muster je Person.
    const muster = (seed: number) =>
      Array.from({ length: 240 }, (_, i) => {
        const tag = Math.floor(i / 48);
        const halb = i % 48;
        if (halb < 16 || halb >= 36) return "free";
        if (halb === 24 || halb === 25) return "busy";
        const r = (tag * 7 + halb * 3 + seed * 5) % 17;
        return r === 0
          ? "busy"
          : r === 1
            ? "tentative"
            : r === 2 && seed === 2
              ? "oof"
              : "free";
      });
    const verfuegbarkeit = [
      { address: "patrick@example.org", intervals: muster(0) },
      { address: "anna.koenig@example.org", intervals: muster(1) },
      { address: "malte.brandt@example.org", intervals: muster(2) },
      {
        address: "sven.lorenz@example.org",
        intervals: Array(240).fill("unknown"),
      },
    ];
    const ansicht = () => ({
      poll,
      answers: antworten,
      history,
      deliveries: [],
    });
    const vorher = w.__TAURI_INTERNALS__.invoke;
    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args?: Record<string, any>,
    ) => {
      if (cmd === "integrations_list")
        return [
          {
            integration: {
              id: "m365-1",
              kind: "m365",
              label: "Microsoft 365",
              enabled: true,
            },
          },
        ];
      if (cmd === "m365_status") return { account: "patrick@example.org" };
      if (cmd === "poll_list") return zustand === "leer" ? [] : [poll];
      if (cmd === "poll_get") return ansicht();
      if (cmd === "poll_create" || cmd === "poll_update") {
        Object.assign(poll, args?.poll ?? {}, {
          id: "T-7K3Q",
          status: "entwurf",
        });
        return ansicht();
      }
      if (cmd === "poll_availability") return verfuegbarkeit;
      if (cmd === "poll_rank") return raenge;
      if (cmd === "poll_send") {
        poll.status = "offen";
        return ansicht();
      }
      if (cmd === "poll_answer_confirm") {
        history.forEach(
          (h) => (h.confirmed = h.id === args?.answerId ? true : h.confirmed),
        );
        return ansicht();
      }
      if (cmd === "poll_close") {
        poll.status = args?.decided ? "entschieden" : "abgebrochen";
        return ansicht();
      }
      return vorher(cmd, args);
    };
  }, zustand);
}
