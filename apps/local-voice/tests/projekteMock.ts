import type { Page } from "@playwright/test";

/**
 * Attrappe der Suite-Projekte und der Recherche (ADR-0026 PR-7) fuer `projekte-recherche.spec.ts`.
 * Sie legt sich ueber `installLiveMock` und spielt nur nach, was das Backend fuer die Befehle
 * `projekt*` und `recherche*` tut (die Regeln selbst -- Default-Deny, Halt, Freigabe, Audit ohne
 * Inhalte -- pruefen die Rust-Tests in `src-tauri/src/projekte/tests.rs`):
 *
 * - Ordner `f2` ("Geschaeftlich") ist mit dem Suite-Projekt `sp1` (INTERNAL, Web frei) verknuepft,
 *   Ordner `f3` ("Kunde Stadtwerke") mit `sp2` (PERSONAL, Web frei), Ordner `f1` mit `sp3`
 *   (INTERNAL, Web gesperrt), Ordner `f4` mit nichts.
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
          title: "Netzausbau Stadtwerke",
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
          title: "Personalgespräche",
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
          title: "Interne Planung",
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
                  "Kundentermin Stadtwerke Kaiserslautern",
                  "wai://lva/besprechung/m2",
                  "PERSONAL",
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

    const klassenName: Record<string, string> = {
      PUBLIC: "öffentlich",
      INTERNAL: "intern",
      PERSONAL: "personenbezogen",
    };
    const ergebnis = (id: string, frage: string, projectId: string) => {
      const p = w.__suite.projekte.find((x: any) => x.id === projectId);
      return {
        id,
        question: frage,
        createdAt: stamp,
        dataClass: p.dataClass,
        dataClassLabel: klassenName[p.dataClass],
        projectId,
        webApproved: true,
        stoppedReason: null,
        model: "lokal-schnell",
        answer: [
          {
            text: "Die Netzentgelte werden von der Bundesnetzagentur festgelegt.",
            citations: [1],
            unsupported: false,
          },
        ],
        sources: [
          {
            index: 1,
            title: "Netzentgelte - Bundesnetzagentur",
            url: "https://www.example.org/netzentgelte",
            retrievedAt: "2026-11-02T08:30:00Z",
            excerpt: "Die Entgelte für die Netznutzung werden reguliert.",
            sourceType: "amtlich",
          },
        ],
        contradictions: [],
        followUps: ["Wie hoch waren die Entgelte 2025?"],
        queries: [frage],
        readAloud: "Die Netzentgelte werden festgelegt.",
      };
    };
    const speichern = (frage: string, projectId: string) => {
      const id = `rc${++w.__suite.seq}`;
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
              message: "Das Web ist für dieses Projekt nicht freigegeben.",
            };
          }
          if (p.dataClass === "INTERNAL" || p.dataClass === "PUBLIC") {
            return {
              status: "fertig",
              rechercheId: speichern(args.frage, p.id),
              message: null,
            };
          }
          s.holds.push({
            id: `h${++s.seq}`,
            question: args.frage,
            dataClass: p.dataClass,
            dataClassLabel: klassenName[p.dataClass],
            projectId: p.id,
            queries: [
              { id: "q1", text: args.frage, hint: null },
              {
                id: "q2",
                text: `${args.frage} Stadtwerke Kaiserslautern`,
                hint: "enthält möglicherweise Personenbezug",
              },
            ],
          });
          return { status: "freigabe", rechercheId: null, message: null };
        }
        case "recherche_freigaben":
          w.__calls?.push({ cmd, args });
          // Wie das echte Backend: jede Antwort ist eine neue Kopie (sonst erkennt React keine Aenderung).
          return JSON.parse(JSON.stringify(s.holds));
        case "recherche_freigabe_entscheiden": {
          w.__calls?.push({ cmd, args });
          const i = s.holds.findIndex((h: any) => h.id === args.id);
          const hold = s.holds[i];
          s.holds.splice(i, 1);
          if (args.aktion === "abbrechen") {
            return { status: "abgelehnt", rechercheId: null, message: null };
          }
          return {
            status: "fertig",
            rechercheId: speichern(hold.question, hold.projectId),
            message: null,
          };
        }
        case "recherche_lesen":
          return s.rechercheDocs[args.id];
        case "recherche_liste":
          return [];
      }
      return inner(cmd, args);
    };
  });
};
