import type { Page } from "@playwright/test";

/**
 * Attrappe der Suite-Projekte und der Recherche (ADR-0026 PR-7) fuer `projekte-recherche.spec.ts`.
 * Sie legt sich ueber `installLiveMock` und spielt nur nach, was das Backend fuer die Befehle
 * `projekt*` und `recherche*` tut (die Regeln selbst -- Default-Deny, Halt, Freigabe, Audit ohne
 * Inhalte -- pruefen die Rust-Tests in `src-tauri/src/projekte/tests.rs`):
 *
 * - Ordner `f2` ("Geschaeftlich") ist mit dem gleichnamigen Suite-Projekt `sp1` (INTERNAL, Web frei)
 *   verknuepft, Ordner `f3` ("Kunde Stadtwerke") mit dem gleichnamigen `sp2` (PERSONAL, Web frei),
 *   Ordner `f1` ("Privat") mit `sp3` (INTERNAL, Web gesperrt), Ordner `f4` mit nichts. Die Szene der
 *   Vorschaubilder ist durchgaengig `f3`/`sp2`: Frage, Anfragen und Ergebnis gehoeren zu diesem Projekt.
 * - Fehler lassen sich einspielen: `__suite.failFreigaben` (Lesen der Halte scheitert),
 *   `__suite.entscheidenMs` (Entscheidung dauert), `__suite.entscheidenFehler` (Entscheidung scheitert).
 * - `recherche_starten` liefert bei INTERNAL sofort ein Ergebnis, ab PERSONAL einen Halt
 *   (`status: "freigabe"`) mit zwei Suchanfragen; `recherche_freigabe_entscheiden` sendet nur, was
 *   gewaehlt wurde. Ein Projekt ohne Web-Freigabe lehnt mit `fehler` ab.
 */
export const installProjekteMock = async (page: Page) => {
  await page.addInitScript(() => {
    const w = window as any;
    const inner = w.__TAURI_INTERNALS__.invoke;
    const stamp = "2026-10-30T09:15:00Z";

    w.__suite = {
      projekte: [
        {
          id: "sp1",
          title: "Geschäftlich",
          goal: null,
          dataClass: "INTERNAL",
          specialCategory: false,
          archived: false,
          webResearchEnabled: true,
          entryCount: 3,
          revision: "1",
        },
        {
          id: "sp2",
          title: "Kunde Stadtwerke",
          goal: null,
          dataClass: "PERSONAL",
          specialCategory: false,
          archived: false,
          webResearchEnabled: true,
          entryCount: 1,
          revision: "1",
        },
        {
          id: "sp3",
          title: "Privat",
          goal: null,
          dataClass: "INTERNAL",
          specialCategory: false,
          archived: false,
          webResearchEnabled: false,
          entryCount: 0,
          revision: "1",
        },
      ],
      links: { f2: "sp1", f3: "sp2", f1: "sp3" } as Record<string, string>,
      holds: [] as any[],
      failFreigaben: false,
      entscheidenMs: 0,
      entscheidenFehler: null as null | string,
      rechercheDocs: {} as Record<string, any>,
      seq: 0,
    };

    const eintrag = (
      id: string,
      kind: string,
      title: string,
      link: string,
      cls: string,
    ) => ({
      id,
      kind,
      title,
      subtitle: null,
      originApp: "lva",
      dataClass: cls,
      available: true,
      assignedAt: stamp,
      assignedBy: "person",
      link,
    });
    const eintraegeVon = (suiteId: string) => {
      const base =
        suiteId === "sp1"
          ? [
              eintrag(
                "e1",
                "meeting",
                "Jour fixe Vertrieb KW 40",
                "wai://lva/besprechung/m1",
                "INTERNAL",
              ),
              eintrag(
                "e2",
                "meeting",
                "Projektreview Netzpuls Sprint 12",
                "wai://lva/besprechung/m3",
                "INTERNAL",
              ),
              eintrag(
                "e3",
                "book",
                "Leitfaden Netzentgelte",
                "wai://lva/buch/b1",
                "PUBLIC",
              ),
            ]
          : suiteId === "sp2"
            ? [
                eintrag(
                  "e4",
                  "meeting",
                  "Kundentermin Stadtwerke Kaiserslautern – Lastgang",
                  "wai://lva/besprechung/m2",
                  "PERSONAL",
                ),
                eintrag(
                  "e5",
                  "book",
                  "Leitfaden Netzentgelte",
                  "wai://lva/buch/b1",
                  "PUBLIC",
                ),
              ]
            : [];
      const recherchen = Object.values(w.__suite.rechercheDocs)
        .filter((d: any) => d.projectId === suiteId)
        .map((d: any) =>
          eintrag(
            `r-${d.id}`,
            "document",
            d.question,
            `wai://lva/recherche/${d.id}`,
            d.dataClass,
          ),
        );
      return [...base, ...recherchen];
    };
    const projektMit = (id: string) => {
      const p = w.__suite.projekte.find((x: any) => x.id === id);
      return { ...p, entryCount: eintraegeVon(id).length };
    };

    const ergebnis = (id: string, frage: string, projectId: string) => {
      const p = w.__suite.projekte.find((x: any) => x.id === projectId);
      return {
        id,
        question: frage,
        createdAt: stamp,
        dataClass: p.dataClass,
        projectId,
        webApproved: true,
        stoppedReason: null,
        model: "lokal-schnell",
        answer: [
          {
            text: "Die Netzentgelte für Lastgangmessung setzen sich aus Arbeits-, Leistungs- und Messpreis zusammen.",
            citations: [1],
            unsupported: false,
          },
        ],
        sources: [
          {
            index: 1,
            title: "Netzentgelte Strom - Bundesnetzagentur",
            url: "https://www.example.org/netzentgelte",
            retrievedAt: "2026-11-02T08:30:00Z",
            excerpt:
              "Entgelte für die Netznutzung bestehen aus Arbeits-, Leistungs- und Messpreis.",
            sourceType: "official",
          },
        ],
        contradictions: [],
        followUps: ["Wie hoch war der Leistungspreis 2025?"],
        queries: [frage],
        readAloud: "Die Netzentgelte werden festgelegt.",
      };
    };
    const speichern = (frage: string, projectId: string, id?: string) => {
      id = id ?? `rc${++w.__suite.seq}`;
      w.__suite.rechercheDocs[id] = ergebnis(id, frage, projectId);
      return id;
    };

    w.__invoke = inner;
    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args: Record<string, any> = {},
    ) => {
      const s = w.__suite;
      switch (cmd) {
        case "meeting_folders_list": {
          const list = await inner(cmd, args);
          return list.map((f: any) => ({
            ...f,
            projekt_id: s.links[f.id] ?? null,
          }));
        }
        case "projekte_liste":
          return s.projekte.map((p: any) => projektMit(p.id));
        case "projekt_eintraege":
          return eintraegeVon(args.suiteId);
        case "projekte_der_besprechung": {
          w.__calls?.push({ cmd, args });
          const ids = (w.__folderMap[args.meetingId] ?? [])
            .map((f: string) => s.links[f])
            .filter(Boolean);
          return s.projekte
            .filter((p: any) => ids.includes(p.id))
            .map((p: any) => projektMit(p.id));
        }
        case "projekt_web_freigabe": {
          const p = s.projekte.find((x: any) => x.id === args.id);
          p.webResearchEnabled = args.an;
          return projektMit(p.id);
        }
        case "projekt_ordner_verknuepfen":
          s.links[args.ordnerId] = args.suiteId;
          return { zugeordnet: 2, bereits: 0, uebersprungen: 0, entfernt: 0 };
        case "projekt_ordner_loesen":
          delete s.links[args.ordnerId];
          return 0;
        case "projekt_abgleichen":
          return { zugeordnet: 0, bereits: 3, uebersprungen: 0, entfernt: 0 };
        case "library_list":
          return [];
        case "recherche_status":
          return {
            available: true,
            reason: null,
            reasonCode: null,
            searxngUrl: "https://suche.example.org",
            canConfigure: true,
          };
        case "recherche_starten": {
          w.__calls?.push({ cmd, args });
          const p = s.projekte.find((x: any) => x.id === args.suiteProjekt);
          if (!p?.webResearchEnabled) {
            return {
              status: "fehler",
              rechercheId: null,
              message:
                "Das Web ist für dieses Suite-Projekt nicht freigegeben.",
              code: "web_gesperrt",
              haltBleibt: false,
            };
          }
          if (p.dataClass === "INTERNAL" || p.dataClass === "PUBLIC") {
            return {
              status: "fertig",
              rechercheId: speichern(args.frage, p.id),
              message: null,
              code: null,
              haltBleibt: false,
            };
          }
          s.holds.push({
            id: `h${++s.seq}`,
            question: args.frage,
            dataClass: p.dataClass,
            projectId: p.id,
            expiresInSecs: 900,
            queries: [
              {
                id: "q1",
                text: "Netzentgelte Lastgangmessung Berechnung",
                hintClass: null,
              },
              {
                id: "q2",
                text: "Netzentgelte Lastgangmessung Stadtwerke Kaiserslautern",
                hintClass: "PERSONAL",
              },
            ],
          });
          return {
            status: "freigabe",
            rechercheId: s.holds[s.holds.length - 1].id,
            message: "Die Suchanfragen warten auf Ihre Freigabe.",
            code: "freigabe_wartet",
            haltBleibt: false,
          };
        }
        case "recherche_freigaben":
          if (s.failFreigaben) throw new Error("recherche.storage: kaputt");
          w.__calls?.push({ cmd, args });
          // Wie das echte Backend: jede Antwort ist eine neue Kopie (sonst erkennt React keine Aenderung).
          return JSON.parse(JSON.stringify(s.holds));
        case "recherche_freigabe_entscheiden": {
          w.__calls?.push({ cmd, args });
          if (s.entscheidenMs) {
            await new Promise((r) => setTimeout(r, s.entscheidenMs));
          }
          if (s.entscheidenFehler) throw new Error(s.entscheidenFehler);
          const i = s.holds.findIndex((h: any) => h.id === args.id);
          const hold = s.holds[i];
          s.holds.splice(i, 1);
          if (args.aktion === "abbrechen") {
            return {
              status: "abgelehnt",
              rechercheId: null,
              message: "Es wurde nichts gesendet.",
              code: "nichts_gesendet",
              haltBleibt: false,
            };
          }
          return {
            status: "fertig",
            rechercheId: speichern(hold.question, hold.projectId, hold.id),
            message: null,
            code: null,
            haltBleibt: false,
          };
        }
        case "recherche_lesen":
          return s.rechercheDocs[args.id];
        case "recherche_loeschen": {
          w.__calls?.push({ cmd, args });
          delete s.rechercheDocs[args.id];
          return null;
        }
        case "recherche_liste":
          return [];
      }
      return inner(cmd, args);
    };
  });
};
