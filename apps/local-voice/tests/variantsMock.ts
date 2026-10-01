import type { Page } from "@playwright/test";

/**
 * Attrappe der Transkript-Fassungen, der Herkunft und der yt-dlp-Wege (A3, #66)
 * fuer `youtube-versions.spec.ts`. Sie legt sich ueber `recLayoutMock` UND
 * `youtubeMock` und uebernimmt nur die neuen Kommandos.
 *
 * - Die Besprechung `y1` (YouTube) hat zwei Fassungen: v1 Untertitel (manuell,
 *   aktiv) und v2 eigene Transkription. Das aktive Transkript folgt der Wahl.
 * - Die Zusammenfuehrung spiegelt die Fehlercodes des Backends; die echte Logik
 *   (Schema, Laengen- und Ueberdeckungspruefung) pruefen die Rust-Tests.
 * - Jede Herkunft hat die Felder des Backends (`ProvenanceEntry`).
 */

export const V1_TEXT = [
  "Wir sprechen heute über den Lastgang.",
  "Danach folgt der Speicher.",
];
export const V2_TEXT = [
  "Wir sprechen heute ueber den Lastgang von Stadtwerken.",
  "Danach folgt der Speicher.",
];

export interface VariantsMockOptions {
  /** Schalter „privat“ (Standard: an). */
  privateOn?: boolean;
  /** yt-dlp gefunden (Standard: ja). */
  toolFound?: boolean;
  /** Untertitelspuren, die `youtube_subtitle_tracks` liefert. */
  tracks?: { language: string; name: string; auto: boolean }[];
  /** Nur Fassung v1 (Standard: zwei). */
  single?: boolean;
  /** Die Zusammenfuehrung wird verworfen (`merge_rejected`). */
  rejectMerge?: boolean;
}

export const installVariantsMock = async (
  page: Page,
  options: VariantsMockOptions = {},
) => {
  await page.addInitScript(
    ({ v1, v2, opts }) => {
      const w = window as any;
      const seg = (texts: string[]) =>
        texts.map((text, i) => ({
          segment_index: i,
          text,
          start_ms: i * 6000 + 1000,
          end_ms: i * 6000 + 5000,
          channel: 2,
          speaker_index: null,
        }));
      const variant = (
        id: string,
        number: number,
        kind: string,
        texts: string[],
        active: boolean,
        language: string | null = "de",
      ) => ({
        id,
        meeting_id: "y1",
        kind,
        language,
        model: null,
        number,
        created_at: 1_790_100_000 + number,
        active,
        segment_count: texts.length,
      });
      w.__variantTexts = { va: v1, vb: v2 };
      w.__variants = {
        y1: [
          variant("va", 1, "subtitles_manual", v1, true),
          ...(opts.single ? [] : [variant("vb", 2, "stt", v2, false)]),
        ],
      };
      w.__settings.meeting_youtube_private = opts.privateOn ?? true;
      w.__toolStatus = {
        ...(w.__toolStatus ?? {}),
        found: opts.toolFound ?? true,
        error: opts.toolFound === false ? "not_found" : null,
      };
      w.__tracks = opts.tracks ?? [
        { language: "de", name: "German", auto: false },
      ];
      w.__durations = [];
      const provenance = (overrides: Record<string, unknown>) => ({
        id: `p-${Math.random().toString(36).slice(2, 8)}`,
        subject_kind: "document",
        subject_id: "x",
        subject_revision: null,
        created_at: 1_790_200_000_000,
        operation: "minutes",
        actor_kind: "user",
        actor_ref: null,
        provider: "local",
        locality: "local",
        model_id: "llm-gemma4-e4b-q4",
        model_label: "Gemma 4 E4B",
        usage_event_id: 7,
        prompt_tokens: 1200,
        completion_tokens: 340,
        duration_ms: 18_400,
        sources: [
          { kind: "transcript", ref: "y1", title: "Transkript v1", url: null },
        ],
        confidence: null,
        params_json: null,
        origin: "recorded",
        ...overrides,
      });
      w.__provenance = {
        "transcript_variant:va": [
          provenance({
            subject_kind: "transcript_variant",
            subject_id: "va",
            operation: "subtitles_import",
            provider: "youtube",
            locality: null,
            model_id: null,
            model_label: null,
            usage_event_id: null,
            prompt_tokens: null,
            completion_tokens: null,
            duration_ms: null,
            sources: [
              {
                kind: "subtitle",
                ref: "dQw4w9WgXcQ",
                title: "German",
                url: null,
              },
            ],
          }),
        ],
        "document:d-ai": [provenance({ operation: "notes", confidence: 0.82 })],
        "document:d-min": [provenance({ operation: "minutes" })],
      };
      w.__documents = [
        ...(w.__documents ?? []),
        {
          id: "d-ai",
          meeting_id: "y1",
          kind: "enhanced_notes",
          body_format: "enhanced@1",
          body: "{}",
          version: 1,
          created_at: 1_790_100_900,
          template_id: null,
          updated_at: 300,
        },
        {
          id: "d-min",
          meeting_id: "y1",
          kind: "minutes",
          body_format: "markdown@1",
          body: "## Zusammenfassung\n\nEin Video über den Lastgang.",
          version: 1,
          created_at: 1_790_100_901,
          template_id: null,
          updated_at: 301,
        },
      ];

      const active = (id: string) =>
        (w.__variants[id] ?? []).find((v: any) => v.active);
      const original = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, any> = {},
      ) => {
        switch (cmd) {
          case "transcript_variants":
            w.__calls.push({ cmd, args });
            return w.__variants[args.meetingId] ?? [];
          case "transcript_variant_segments":
            w.__calls.push({ cmd, args });
            return seg(w.__variantTexts[args.variantId] ?? []);
          case "transcript_variant_activate": {
            w.__calls.push({ cmd, args });
            const list = w.__variants.y1;
            list.forEach((v: any) => (v.active = v.id === args.variantId));
            return list.find((v: any) => v.id === args.variantId);
          }
          case "transcript_variants_merge": {
            w.__calls.push({ cmd, args });
            if (opts.rejectMerge) throw "merge_rejected: 1/1";
            const id = "vc";
            const texts = w.__variantTexts[args.baseId];
            w.__variantTexts[id] = texts;
            const created = variant(id, 3, "merged", texts, false);
            w.__variants.y1.push(created);
            return created;
          }
          case "meetings_get_segments":
            if (args.meetingId === "y1") {
              const current = active("y1");
              return current ? seg(w.__variantTexts[current.id]) : [];
            }
            break;
          case "youtube_subtitle_tracks":
            w.__calls.push({ cmd, args });
            return w.__tracks.map((t: any, i: number) => ({
              ...t,
              recommended: i === 0,
            }));
          case "youtube_subtitles_fetch": {
            w.__calls.push({ cmd, args });
            const id = `vs${w.__variants.y1.length}`;
            w.__variantTexts[id] = v1;
            const created = variant(
              id,
              w.__variants.y1.length + 1,
              args.track.auto ? "subtitles_auto" : "subtitles_manual",
              v1,
              false,
              args.track.language,
            );
            w.__variants.y1.push(created);
            return created;
          }
          case "youtube_own_transcription":
            w.__calls.push({ cmd, args });
            return null;
          case "youtube_set_duration":
            w.__calls.push({ cmd, args });
            w.__durations.push(args.seconds);
            return true;
          case "provenance_get":
            w.__calls.push({ cmd, args });
            return w.__provenance[`${args.contentType}:${args.id}`] ?? [];
        }
        return original(cmd, args);
      };
    },
    { v1: V1_TEXT, v2: V2_TEXT, opts: options },
  );
};
