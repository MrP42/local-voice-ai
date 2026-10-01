import type { Page } from "@playwright/test";

/**
 * Attrappe der Mehrsprachigkeit (G5, #70) fuer `meeting-translation.spec.ts`. Sie legt sich ueber
 * `installLiveMock` (Besprechung `m2`: Import mit Audio) und uebernimmt die neuen Befehle:
 *
 * - `m2` ist ein ENGLISCHES Interview: Fassung v1 (aktiv), Sprache `en`, erkannt durch das
 *   Modell (`probe`).
 * - `transcript_variant_translate` legt Fassung v2 (Uebersetzung, `de`) an; der Satz 2 hat eine
 *   abweichende Zahl und steht im Pruefbericht. Das Original bleibt aktiv.
 * - Die Wahl der aktiven Fassung bestimmt, was `meetings_get_segments` liefert.
 * - Die echte Logik (Erkennung, Modellwahl, Treuepruefung) pruefen die Rust-Tests.
 */

export const EN_TEXT = [
  "Good morning, this is Anna from Siemens.",
  "We pay 1,200.50 euros every month for the storage.",
  "Thomas will send the offer on Monday.",
  "The peaks are highest between half past seven and nine.",
  "A battery of 400 kilowatt hours cuts about a third of them.",
  "I need this in writing for the management board.",
];

export const DE_TEXT = [
  "Guten Morgen, hier ist Anna von Siemens.",
  "Wir zahlen jeden Monat 120,50 Euro für den Speicher.",
  "Thomas schickt das Angebot am Montag.",
  "Die Spitzen sind zwischen halb acht und neun am höchsten.",
  "Eine Batterie mit 400 Kilowattstunden kappt etwa ein Drittel davon.",
  "Das brauche ich schriftlich für die Geschäftsführung.",
];

export interface TranslationMockOptions {
  /** Das Modell der Transkription deckt die Sprache NICHT ab (Hinweis im Chip-Dialog). */
  modelNotCovering?: boolean;
  /** Die Uebersetzung bleibt haengen, bis `__tr.release()` gerufen wird (Fortschrittsanzeige). */
  holdTranslate?: boolean;
  /** Der Server faellt aus: die Uebersetzung endet mit `translate_llm_failed`. */
  failTranslate?: boolean;
  /** Schon eine zweite Fassung (zusammengefuehrt, ohne Beziehung zu einer Uebersetzung). */
  withMerged?: boolean;
}

export const installTranslationMock = async (
  page: Page,
  options: TranslationMockOptions = {},
) => {
  await page.addInitScript(
    ({ en, de, opts }) => {
      const w = window as any;
      const seg = (texts: string[]) =>
        texts.map((text, i) => ({
          segment_index: i,
          text,
          start_ms: i * 6000 + 1000,
          end_ms: i * 6000 + 5500,
          channel: 2,
          speaker_index: null,
        }));
      const variant = (over: Record<string, unknown>) => ({
        id: "v1",
        meeting_id: "m2",
        kind: "stt",
        language: "en",
        model: "parakeet-tdt-0.6b-v3",
        number: 1,
        created_at: 1_790_100_001,
        active: true,
        segment_count: en.length,
        source_variant_id: null,
        source_language: null,
        flagged: 0,
        ...over,
      });
      const tr: any = (w.__tr = {
        variants: [
          variant({}),
          ...(opts.withMerged
            ? [
                variant({
                  id: "vm",
                  kind: "merged",
                  model: null,
                  number: 2,
                  active: false,
                }),
              ]
            : []),
        ],
        texts: {
          v1: en,
          ...(opts.withMerged
            ? { vm: en.map((t: string) => t.replace("Anna", "Hanna")) }
            : {}),
        } as Record<string, string[]>,
        reports: {} as Record<string, unknown>,
        info: {
          code: "en",
          source: "probe",
          confidence: 1,
          forced: null,
          mismatch: null,
          model_id: opts.modelNotCovering
            ? "parakeet-v2-de"
            : "parakeet-tdt-0.6b-v3",
          model_name: opts.modelNotCovering
            ? "Parakeet v2 (nur Deutsch)"
            : "Parakeet TDT 0.6B v3",
          model_covers: !opts.modelNotCovering,
          suggestion: opts.modelNotCovering
            ? {
                model_id: "qwen3-asr-1.7b",
                name: "Qwen3-ASR 1.7B",
                downloaded: true,
              }
            : null,
        },
        release: () => {},
      });
      // Besprechung m2 ist englisch.
      w.__meetings = w.__meetings.map((m: any) =>
        m.id === "m2" ? { ...m, language: "en" } : m,
      );
      w.__provenance = {
        "transcript_variant:v2": [
          {
            id: "p-tr",
            subject_kind: "transcript_variant",
            subject_id: "v2",
            subject_revision: null,
            created_at: 1_790_200_000_000,
            operation: "translation",
            actor_kind: "user",
            actor_ref: null,
            provider: "local",
            locality: "local",
            model_id: "llm-gemma4-e4b-q4",
            model_label: "Gemma 4 E4B",
            usage_event_id: 9,
            prompt_tokens: 1800,
            completion_tokens: 900,
            duration_ms: 42_000,
            sources: [
              {
                kind: "transcript",
                ref: "v1",
                title: "v1 stt (en)",
                url: null,
              },
            ],
            confidence: null,
            params_json: JSON.stringify({
              source_variant: "v1",
              source_language: "en",
              target_language: "de",
            }),
            origin: "recorded",
          },
        ],
      };
      // Dokumente: Protokoll und KI-Notizen mit Grundlage (Kopfzeile im Protokolltext).
      w.__docBasis = {} as Record<string, unknown>;
      w.__trCalls = [];

      const active = () => tr.variants.find((v: any) => v.active);
      const original = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, any> = {},
      ) => {
        switch (cmd) {
          case "transcript_variants":
            w.__calls.push({ cmd, args });
            return args.meetingId === "m2"
              ? tr.variants.map((v: any) => ({ ...v }))
              : [];
          case "transcript_variant_segments":
            w.__calls.push({ cmd, args });
            return seg(tr.texts[args.variantId] ?? []);
          case "transcript_variant_report":
            w.__calls.push({ cmd, args });
            return tr.reports[args.variantId] ?? null;
          case "transcript_variant_activate": {
            w.__calls.push({ cmd, args });
            tr.variants.forEach(
              (v: any) => (v.active = v.id === args.variantId),
            );
            return tr.variants.find((v: any) => v.id === args.variantId);
          }
          case "meetings_get_segments":
            if (args.meetingId === "m2") {
              const current = active();
              return seg(tr.texts[current.id]);
            }
            break;
          case "meetings_language_info":
            w.__calls.push({ cmd, args });
            return args.meetingId === "m2" ? { ...tr.info } : null;
          case "meetings_set_language": {
            w.__calls.push({ cmd, args });
            const covers = args.language === "en" || args.language === "de";
            tr.info = {
              ...tr.info,
              code: args.language,
              source: "user",
              confidence: 1,
              forced: null,
              mismatch: null,
              model_covers: opts.modelNotCovering
                ? args.language === "de"
                : covers,
              suggestion:
                args.language === "en"
                  ? null
                  : {
                      model_id: "qwen3-asr-1.7b",
                      name: "Qwen3-ASR 1.7B",
                      downloaded: true,
                    },
            };
            return { ...tr.info };
          }
          case "meetings_model_for_language":
            w.__calls.push({ cmd, args });
            return args.language === "en"
              ? {
                  model_id: "parakeet-tdt-0.6b-v3",
                  name: "Parakeet TDT 0.6B v3",
                  downloaded: true,
                }
              : {
                  model_id: "qwen3-asr-1.7b",
                  name: "Qwen3-ASR 1.7B",
                  downloaded: true,
                };
          case "transcript_variant_translate": {
            w.__calls.push({ cmd, args });
            if (opts.holdTranslate) {
              await new Promise<void>((resolve) => (tr.release = resolve));
            }
            if (opts.failTranslate)
              throw "translate_llm_failed: connection reset";
            const number = tr.variants.length + 1;
            const id = `v${number}`;
            tr.texts[id] = de;
            tr.reports[id] = {
              source_variant_id: args.sourceVariantId,
              source_language: "en",
              target_language: args.targetLanguage,
              model: "llm-gemma4-e4b-q4",
              checked: de.length,
              flagged: [{ segment_index: 1, reasons: ["numbers"] }],
              failed_blocks: 0,
              blocks: 1,
            };
            const created = variant({
              id,
              kind: "translation",
              language: args.targetLanguage,
              model: "llm-gemma4-e4b-q4",
              number,
              created_at: 1_790_100_000 + number,
              active: false,
              segment_count: de.length,
              source_variant_id: args.sourceVariantId,
              source_language: "en",
              flagged: 1,
            });
            tr.variants.push(created);
            return created;
          }
          case "meetings_document_basis": {
            w.__calls.push({ cmd, args });
            return w.__docBasis[args.kind] ?? null;
          }
          case "meetings_generate_minutes":
          case "meeting_notes_enhance":
            w.__calls.push({ cmd, args });
            // Ein Lauf der Attrappe endet sofort mit einem Fehlerhinweis des Backends nicht;
            // geprueft werden die Argumente. Die Grundlage gilt fuer die Anzeige danach.
            w.__docBasis[
              cmd === "meetings_generate_minutes" ? "minutes" : "enhanced_notes"
            ] = {
              variant_id: args.basis?.variant_id ?? active().id,
              variant_number:
                (tr.variants.find(
                  (v: any) => v.id === (args.basis?.variant_id ?? active().id),
                )?.number as number) ?? 1,
              variant_kind:
                tr.variants.find(
                  (v: any) => v.id === (args.basis?.variant_id ?? active().id),
                )?.kind ?? "stt",
              language:
                tr.variants.find(
                  (v: any) => v.id === (args.basis?.variant_id ?? active().id),
                )?.language ?? "en",
              output_language: args.basis?.output_language ?? null,
            };
            throw cmd === "meetings_generate_minutes"
              ? "minutes_cancelled"
              : "stopped";
          case "provenance_get":
            w.__calls.push({ cmd, args });
            return w.__provenance[`${args.contentType}:${args.id}`] ?? [];
        }
        return original(cmd, args);
      };
    },
    { en: EN_TEXT, de: DE_TEXT, opts: options },
  );
};

/** Haengende Uebersetzung freigeben. */
export const releaseTranslation = (page: Page) =>
  page.evaluate(() => (window as any).__tr.release());
