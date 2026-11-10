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
 *   (`status: "freigabe"`) mit zwei Suchanfragen; mit `mitKontext` (lva#365 P4c-3) immer einen Halt
 *   (`withContext: true`) und nur, wenn die KI im Projekt frei ist (`__kiFrei`, sonst `projekt.ki_gesperrt`); `recherche_freigabe_entscheiden` sendet nur, was
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
        case "projekt_ki_status":
          return { freigegeben: !!w.__kiFrei };
        case "recherche_starten": {
          w.__calls?.push({ cmd, args });
          const p = s.projekte.find((x: any) => x.id === args.suiteProjekt);
          if (args.mitKontext && !w.__kiFrei) {
            return {
              status: "fehler",
              rechercheId: null,
              message: "Die KI im Projekt ist noch nicht freigegeben.",
              code: "projekt.ki_gesperrt",
              haltBleibt: false,
            };
          }
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
          if (
            !args.mitKontext &&
            (p.dataClass === "INTERNAL" || p.dataClass === "PUBLIC")
          ) {
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
            withContext: !!args.mitKontext,
            queries: [
              // Mit Projektkontext tragen Kopf und Anfragen dieselbe Klasse (wie das Backend: jede
              // Anfrage meldet ihre Klasse); ohne Kontext markiert nur die zweite Anfrage.
              {
                id: "q1",
                text: "Netzentgelte Lastgangmessung Berechnung",
                hintClass: args.mitKontext ? p.dataClass : null,
              },
              {
                id: "q2",
                text: "Netzentgelte Lastgangmessung Stadtwerke Kaiserslautern",
                hintClass: args.mitKontext ? p.dataClass : "PERSONAL",
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

/**
 * Attrappe der Projektseite mit „Als Kontext“ (lva#365 P4b) fuer `projekte-kontext.spec.ts`. Sie ersetzt
 * `installProjekteMock` (nicht zusammen verwenden) und spielt nach, was das Backend fuer
 * `projekte_liste`, `projekt_eintraege`, `projekt_kontext_*`, die Zuordnungen und die Projektdateien tut.
 * Die Regeln selbst (Vorgaben, Stand, Token-Schaetzung) pruefen die Rust-Tests in `src-tauri/src/projekte`.
 *
 * Szene (synthetisch, wie die Vorschau docs/vorschau/projekte-p4b): Projekt `p-fin` "Finanzwissen" mit
 * neun Eintraegen (2 Buecher, 1 Text, 2 Aufnahmen, 3 Dateien, 1 Recherche; fuenf davon an, zusammen
 * 15 000 Token), `p-ki` "KI-Strategie LRA" (23), `p-post` "Digitaler Posteingang" (17), `p-leer` (0).
 * Modellfenster 128 000 ("GPT-6.1-Terra"). Steuerung: `__kontext.reset` (Auswahl nicht lesbar),
 * `__kontext.gefunden` (Dateien ohne Journal), `__kontext.failSchalten` (Schalten scheitert), `__kontext.lage` (Dateiliste defekt/neuer/...).
 */
export const installProjekteKontextMock = async (page: Page) => {
  await page.addInitScript(() => {
    const w = window as any;
    const inner = w.__TAURI_INTERNALS__.invoke;
    const stamp = "2026-11-05T09:15:00Z";

    w.__settings.llm_connections = [
      { id: "c1", kind: "api", enabled: true, label: "Test" },
    ];
    w.__settings.llm_models = [
      {
        id: "m-terra",
        connection_id: "c1",
        remote_id: "gpt-6.1-terra",
        label: "GPT-6.1-Terra",
        enabled: true,
        context_limit: 128000,
      },
    ];
    w.__settings.llm_active_model_id = "m-terra";

    // Definition eines Eintrags: Formen mit Token, Hinweise je Form, Vorgabe.
    const def = (
      id: string,
      kind: string,
      title: string,
      link: string,
      forms: string[],
      tokens: Record<string, number | null>,
      opts: {
        subtitle?: string;
        an?: boolean;
        hint?: Record<string, string>;
        by?: string;
      } = {},
    ) => ({
      id,
      kind,
      title,
      link,
      forms,
      tokens,
      subtitle: opts.subtitle ?? null,
      an: opts.an ?? true,
      hint: opts.hint ?? {},
      by: opts.by ?? "person",
    });
    const HINTS: Record<string, string> = {
      "kontext.kein_kurzbuch": "Kein Kurzbuch – erstellen oder Volltext wählen",
      "kontext.kein_protokoll":
        "Kein Protokoll – erstellen oder Transkript wählen",
      "kontext.format": "Format nicht als Kontext lesbar",
    };
    const fin = [
      def(
        "rd",
        "book",
        "Rich Dad Poor Dad",
        "wai://lva/buch/rd",
        ["kurzbuch", "volltext"],
        { kurzbuch: 9000, volltext: 182000 },
        { subtitle: "Deutsch · Hauptfassung" },
      ),
      def(
        "ew",
        "book",
        "Der Einfache Weg zum Reichtum",
        "wai://lva/buch/ew",
        ["kurzbuch", "volltext"],
        { kurzbuch: null, volltext: 150000 },
        {
          subtitle: "Deutsch · Hauptfassung",
          hint: { kurzbuch: "kontext.kein_kurzbuch" },
        },
      ),
      def(
        "s1",
        "note",
        "KI-Verordnung Art. 4 Kompetenz",
        "wai://lva/seite/s1",
        ["text"],
        { text: 1200 },
        { subtitle: "Text ohne Buch" },
      ),
      def(
        "a1",
        "meeting",
        "Beratung Altersvorsorge 05.11.",
        "wai://lva/besprechung/a1",
        ["protokoll", "transkript"],
        { protokoll: 2400, transkript: 14000 },
      ),
      def(
        "a2",
        "meeting",
        "Telefonat Bank 06.11.",
        "wai://lva/besprechung/a2",
        ["protokoll", "transkript"],
        { protokoll: 1100, transkript: 9000 },
        { an: false, by: "automation" },
      ),
      def(
        "d1",
        "document",
        "Notizen-Bank.md",
        "wai://lva/projektdatei/p-fin/d1",
        ["text"],
        { text: 600 },
        { subtitle: "Kopie · 3 KB" },
      ),
      def(
        "d2",
        "document",
        "Depotauszug-2026.pdf",
        "wai://lva/projektdatei/p-fin/d2",
        ["text"],
        { text: null },
        {
          subtitle: "Kopie · 180 KB",
          an: false,
          hint: { text: "kontext.format" },
        },
      ),
      def(
        "d3",
        "document",
        "Haushaltsplan.xlsx",
        "wai://lva/projektdatei/p-fin/d3",
        ["text"],
        { text: null },
        {
          subtitle: "Kopie · 24 KB",
          an: false,
          hint: { text: "kontext.format" },
        },
      ),
      def(
        "r1",
        "document",
        "Recherche ETF-Kosten",
        "wai://lva/recherche/r1",
        ["text"],
        { text: 1800 },
        { subtitle: "Bericht · 12 Quellen" },
      ),
    ];
    const synth = (n: number, p: string) =>
      Array.from({ length: n }, (_, i) =>
        def(
          `${p}${i}`,
          "note",
          `Notiz ${p} ${i + 1}`,
          `wai://lva/seite/${p}${i}`,
          ["text"],
          { text: 300 },
          { an: i % 3 === 0 },
        ),
      );

    w.__kontext = {
      projekte: [
        { id: "p-fin", title: "Finanzwissen" },
        { id: "p-ki", title: "KI-Strategie LRA" },
        { id: "p-post", title: "Digitaler Posteingang" },
        { id: "p-leer", title: "Neue Ideen" },
      ],
      entries: {
        "p-fin": fin,
        "p-ki": [
          ...synth(22, "k"),
          // Buch mit nur einer Form, aber zwei Fassungen: das Menue braucht keine zweite Form.
          def(
            "ai",
            "book",
            "Alles Idioten!?",
            "wai://lva/buch/ai",
            ["volltext"],
            { volltext: 5000 },
            { subtitle: "Deutsch · Hauptfassung", an: false },
          ),
        ],
        "p-post": synth(17, "q"),
        "p-leer": [] as any[],
      } as Record<string, any[]>,
      // Schalter je Projekt und Eintrag: { an, form, edition }
      wahl: {} as Record<string, Record<string, any>>,
      reset: false,
      lage: null as null | string,
      gefunden: [
        { id: "g1", name: "Kontoauszug-Okt.txt", size: 2048 },
      ] as any[],
      failSchalten: false,
      // Verzoegerung in ms je Befehl: "befehl" oder "befehl:suiteId". Die Antwort wird sofort
      // berechnet (Stand zum Aufruf) und erst nach der Frist geliefert, wie bei einem langsamen Backend.
      delay: {} as Record<string, number>,
      // Befehl -> Fehlermeldung: der Befehl scheitert.
      fail: {} as Record<string, string>,
      // projekt_datei_entfernen meldet "nicht sicher entfernt" (false).
      entfernenUnsicher: false,
      // Zahl synthetischer Aufnahmen in meetings_list (0: Standard der Live-Attrappe).
      aufnahmen: 0,
      seq: 0,
    };
    const K = w.__kontext;

    const wahlVon = (pid: string, e: any) => {
      K.wahl[pid] ??= {};
      return (K.wahl[pid][e.id] ??= {
        an: e.an,
        form: e.forms[0],
        edition: null,
      });
    };
    const uebersicht = (pid: string) => {
      let summe = 0;
      let count = 0;
      const rows = (K.entries[pid] ?? []).map((e: any) => {
        const sel = wahlVon(pid, e);
        const an = sel.an && !K.reset;
        const hint = e.hint[sel.form] ?? null;
        const tokens = e.tokens[sel.form] ?? null;
        if (an && !hint && tokens != null) {
          summe += tokens;
          count += 1;
        }
        return {
          id: e.id,
          on: an,
          form: sel.form,
          forms: e.forms,
          edition: sel.edition,
          tokens,
          hint,
          hintText: hint ? HINTS[hint] : null,
        };
      });
      return {
        rows,
        tokens: summe,
        count,
        stand: `s${++K.seq}`,
        reset: K.reset,
      };
    };
    const sicht = (pid: string, e: any) => ({
      id: e.id,
      kind: e.kind,
      title: e.title,
      subtitle: e.subtitle,
      originApp: "lva",
      dataClass: "PERSONAL",
      available: true,
      assignedAt: stamp,
      assignedBy: e.by,
      link: e.link,
    });
    const projektSicht = (p: any) => ({
      id: p.id,
      title: p.title,
      goal: null,
      dataClass: "PERSONAL",
      specialCategory: false,
      archived: false,
      webResearchEnabled: false,
      // Absichtlich NICHT die Laenge der Eintraege: die Oberflaeche darf diesen Zaehler nicht nehmen.
      entryCount: 99,
      revision: "1",
    });
    const entfernen = (pid: string, link: string) => {
      const vorher = K.entries[pid].length;
      K.entries[pid] = K.entries[pid].filter((e: any) => e.link !== link);
      return K.entries[pid].length < vorher;
    };
    const zuordnen = (pid: string, e: any) => {
      if (!K.entries[pid].some((x: any) => x.link === e.link))
        K.entries[pid].push(e);
      return null;
    };

    const kern = async (cmd: string, args: Record<string, any> = {}) => {
      if (K.fail[cmd]) throw new Error(K.fail[cmd]);
      switch (cmd) {
        case "meetings_list":
          if (K.aufnahmen > 0)
            return Array.from(
              {
                length: Math.max(
                  0,
                  Math.min(args.limit, K.aufnahmen - args.offset),
                ),
              },
              (_, i) => ({
                id: `m${args.offset + i}`,
                title: `Aufnahme ${args.offset + i + 1}`,
                date: "2026-10-01T10:00:00Z",
                createdAt: "2026-10-01T10:00:00Z",
                updatedAt: "2026-10-01T10:00:00Z",
              }),
            );
          break;
        case "projekte_liste":
          return K.projekte.map(projektSicht);
        case "projekt_eintraege":
          return (K.entries[args.suiteId] ?? []).map((e: any) =>
            sicht(args.suiteId, e),
          );
        case "projekt_ki_status":
          return { freigegeben: !!w.__kiFrei };
        case "projekt_ki_dokument_erstellen": {
          w.__calls?.push({ cmd, args });
          if (K.dokumentFehler) throw new Error(K.dokumentFehler);
          if (K.dokumentMs)
            await new Promise((r) => setTimeout(r, K.dokumentMs));
          return {
            dat: "dat_01TESTDOKUMENT",
            name: `${args.name}.md`,
            ordner: "KI-Entwürfe",
            klasse: K.dokumentKlasse ?? "internal",
            nurGeraet: false,
            imProjekt: K.dokumentImProjekt !== false,
          };
        }
        case "projekt_kontext_uebersicht":
          w.__calls?.push({ cmd, args });
          return uebersicht(args.suiteId);
        case "projekt_kontext_schalten": {
          w.__calls?.push({ cmd, args });
          if (K.failSchalten)
            throw new Error("projekte.kontext: Schalten ist nicht möglich.");
          const e = K.entries[args.suiteId].find(
            (x: any) => x.id === args.eintragId,
          );
          const sel = wahlVon(args.suiteId, e);
          if (K.reset) {
            K.reset = false;
            for (const x of K.entries[args.suiteId])
              wahlVon(args.suiteId, x).an = false;
          }
          sel.an = args.on;
          if (args.form) sel.form = args.form;
          if (args.edition) sel.edition = args.edition;
          return uebersicht(args.suiteId);
        }
        case "projekt_dateien_lage":
          return args.suiteId === "p-fin" ? K.lage : null;
        case "projekt_dateien_neu_aufbauen":
          w.__calls?.push({ cmd, args });
          K.lage = null;
          return K.gefunden.length;
        case "projekt_dateien_gefunden":
          return args.suiteId === "p-fin" ? K.gefunden : [];
        case "projekt_datei_gefunden_aufnehmen": {
          w.__calls?.push({ cmd, args });
          const g = K.gefunden.find((x: any) => x.id === args.dateiId);
          K.gefunden = K.gefunden.filter((x: any) => x.id !== args.dateiId);
          K.entries[args.suiteId].push(
            def(
              `d-${g.id}`,
              "document",
              g.name,
              `wai://lva/projektdatei/${args.suiteId}/d-${g.id}`,
              ["text"],
              { text: 500 },
              { subtitle: "Kopie · 2 KB", an: false },
            ),
          );
          return null;
        }
        case "projekt_datei_entfernen": {
          w.__calls?.push({ cmd, args });
          return (
            entfernen(
              args.suiteId,
              `wai://lva/projektdatei/${args.suiteId}/${args.dateiId}`,
            ) && !K.entfernenUnsicher
          );
        }
        case "projekt_buch_entfernen":
          w.__calls?.push({ cmd, args });
          return entfernen(args.suiteId, `wai://lva/buch/${args.buchId}`);
        case "projekt_seite_entfernen":
          w.__calls?.push({ cmd, args });
          return entfernen(args.suiteId, `wai://lva/seite/${args.seiteId}`);
        case "projekt_besprechung_entfernen":
          w.__calls?.push({ cmd, args });
          return entfernen(
            args.suiteId,
            `wai://lva/besprechung/${args.meetingId}`,
          );
        case "projekt_buch_zuordnen":
          w.__calls?.push({ cmd, args });
          return zuordnen(
            args.suiteId,
            def(
              args.buchId,
              "book",
              args.buchId === "bb" ? "The Burning Room" : "Alles Idioten!?",
              `wai://lva/buch/${args.buchId}`,
              ["kurzbuch", "volltext"],
              { kurzbuch: 4000, volltext: 90000 },
            ),
          );
        case "projekt_seite_zuordnen":
          w.__calls?.push({ cmd, args });
          return zuordnen(
            args.suiteId,
            def(
              args.seiteId,
              "note",
              "Rundschreiben Posteingang",
              `wai://lva/seite/${args.seiteId}`,
              ["text"],
              { text: 700 },
            ),
          );
        case "projekt_besprechung_zuordnen":
          w.__calls?.push({ cmd, args });
          return zuordnen(
            args.suiteId,
            def(
              args.meetingId,
              "meeting",
              `Aufnahme ${args.meetingId}`,
              `wai://lva/besprechung/${args.meetingId}`,
              ["protokoll", "transkript"],
              { protokoll: 800, transkript: 6000 },
            ),
          );
        case "projekt_dateien_hinzufuegen": {
          w.__calls?.push({ cmd, args });
          for (const p of args.paths as string[]) {
            const name = p.split("/").pop();
            zuordnen(
              args.suiteId,
              def(
                `n-${++K.seq}`,
                "document",
                name,
                `wai://lva/projektdatei/${args.suiteId}/n-${K.seq}`,
                ["text"],
                { text: 100 },
                { an: false },
              ),
            );
          }
          return (args.paths as string[]).map((_, i) => `n-${i}`);
        }
        case "projekt_anlegen": {
          w.__calls?.push({ cmd, args });
          const p = { id: `p-${++K.seq}`, title: args.titel };
          K.projekte.push(p);
          K.entries[p.id] = [];
          return projektSicht(p);
        }
        case "library_list":
          return [
            {
              id: "rd",
              title: "Rich Dad Poor Dad",
              editions: [
                { id: "ed_rd", lang: "de", title: "Rich Dad Poor Dad" },
                { id: "ed_rd_en", lang: "en", title: "Rich Dad Poor Dad" },
              ],
            },
            {
              id: "ew",
              title: "Der Einfache Weg zum Reichtum",
              editions: [{ id: "ed_ew", lang: "de", title: "x" }],
            },
            { id: "bb", title: "The Burning Room", editions: [] },
            {
              id: "ai",
              title: "Alles Idioten!?",
              editions: [
                { id: "ed_ai", lang: "de", title: "Alles Idioten!?" },
                { id: "ed_ai_en", lang: "en", title: "Alles Idioten!?" },
              ],
            },
          ];
        case "pages_list":
          return [
            { id: "s1", title: "KI-Verordnung Art. 4 Kompetenz" },
            { id: "s9", title: "Rundschreiben Posteingang" },
          ];
        case "recherche_loeschen":
          w.__calls?.push({ cmd, args });
          for (const pid of Object.keys(K.entries))
            entfernen(pid, `wai://lva/recherche/${args.id}`);
          return null;
        case "recherche_status":
          return {
            available: false,
            reason: null,
            reasonCode: null,
            searxngUrl: null,
            canConfigure: false,
          };
      }
      return inner(cmd, args);
    };
    w.__TAURI_INTERNALS__.invoke = async (
      cmd: string,
      args: Record<string, any> = {},
    ) => {
      let ergebnis: any;
      let fehler: any;
      let gescheitert = false;
      try {
        ergebnis = await kern(cmd, args);
      } catch (e) {
        gescheitert = true;
        fehler = e;
      }
      const frist = K.delay[`${cmd}:${args.suiteId}`] ?? K.delay[cmd];
      if (frist) await new Promise((r) => setTimeout(r, frist));
      if (gescheitert) throw fehler;
      return ergebnis;
    };
  });
};
