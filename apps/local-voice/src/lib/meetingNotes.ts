// Reine Logik des Notizblocks (M1, P1c): keine React-, keine Tauri-Importe,
// damit sie sich ohne Browser pruefen laesst (`node`, Type-Stripping).
//
// Die Typen kommen aus `bindings.ts` (nur als Typ importiert, wird beim
// Ausfuehren entfernt).

import type {
  EnhancedEntry,
  EnhancedNotes,
  NoteBlock,
  NoteBlockKind,
  TemplateSpec,
} from "@/bindings";

/** Groesste Notizgroesse (JSON), die das Frontend noch speichert. */
export const NOTES_MAX_BYTES = 200 * 1024;

/** Entprellzeit des Autosave: hoechster Verlust bei einem Absturz. */
export const AUTOSAVE_DEBOUNCE_MS = 700;

/** Standardvorlage, solange keine andere gewaehlt ist (`templates.rs`). */
export const DEFAULT_TEMPLATE_ID = "builtin:allgemein";

/**
 * P1k: "Automatisch (nach Inhalt)". Keine Vorlage, sondern die Wahl selbst
 * (`templates.rs`): das Backend waehlt beim Erzeugen anhand des Inhalts.
 */
export const AUTO_TEMPLATE_ID = "auto";

// ---------------------------------------------------------------------------
// Zeit
// ---------------------------------------------------------------------------

/** Audioposition als `mm:ss` (ab 100 Minuten dreistellig, nie `h:mm:ss`). */
export const formatAt = (ms: number): string => {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes.toString().padStart(2, "0")}:${seconds
    .toString()
    .padStart(2, "0")}`;
};

// ---------------------------------------------------------------------------
// Bloecke
// ---------------------------------------------------------------------------

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** ULID (26 Zeichen, nach Zeit sortierbar); die ID erzeugt das Frontend. */
export const newBlockId = (
  now: number = Date.now(),
  random: (n: number) => Uint8Array = (n) =>
    crypto.getRandomValues(new Uint8Array(n)),
): string => {
  let time = "";
  let t = Math.max(0, Math.floor(now));
  for (let i = 0; i < 10; i++) {
    time = CROCKFORD[t % 32] + time;
    t = Math.floor(t / 32);
  }
  const bytes = random(16);
  let rand = "";
  for (let i = 0; i < 16; i++) rand += CROCKFORD[bytes[i] % 32];
  return time + rand;
};

export const makeBlock = (
  partial: Partial<NoteBlock> & { kind?: NoteBlockKind } = {},
): NoteBlock => ({
  id: partial.id ?? newBlockId(),
  kind: partial.kind ?? "paragraph",
  text: partial.text ?? "",
  at_ms: partial.at_ms ?? null,
  checked: partial.checked ?? false,
});

/**
 * Markdown-Kuerzel am Anfang eines Absatzes: `# ` Ueberschrift, `- ` / `* `
 * Stichpunkt, `[ ] ` / `[x] ` Aufgabe. `null`, wenn der Text keins traegt.
 */
export const blockFromMarkdownPrefix = (
  text: string,
): { kind: NoteBlockKind; text: string; checked: boolean } | null => {
  if (text.startsWith("# "))
    return { kind: "heading", text: text.slice(2), checked: false };
  if (text.startsWith("- ") || text.startsWith("* "))
    return { kind: "bullet", text: text.slice(2), checked: false };
  if (text.startsWith("[ ] "))
    return { kind: "todo", text: text.slice(4), checked: false };
  if (text.startsWith("[x] ") || text.startsWith("[X] "))
    return { kind: "todo", text: text.slice(4), checked: true };
  return null;
};

/** Art des Blocks, der nach Enter entsteht: Listen laufen weiter, sonst Absatz. */
export const kindAfterEnter = (kind: NoteBlockKind): NoteBlockKind =>
  kind === "bullet" || kind === "todo" ? kind : "paragraph";

/**
 * Enter im Block `index` an der Cursorposition `caret` (bei Markierung bis
 * `caretEnd`, die Markierung entfaellt): der Text dahinter wandert in einen neuen
 * Block direkt darunter. Ein leerer Listenpunkt beendet die Liste
 * (wird zum Absatz), statt einen weiteren leeren Punkt anzuhaengen.
 * Rueckgabe: neue Liste und die ID des Blocks, der den Cursor bekommt.
 */
export const splitBlock = (
  blocks: NoteBlock[],
  index: number,
  caret: number,
  fresh: NoteBlock,
  caretEnd: number = caret,
): { blocks: NoteBlock[]; focusId: string } => {
  const current = blocks[index];
  if (
    (current.kind === "bullet" || current.kind === "todo") &&
    current.text === ""
  ) {
    const next = blocks.slice();
    next[index] = { ...current, kind: "paragraph", checked: false };
    return { blocks: next, focusId: current.id };
  }
  const before = current.text.slice(0, caret);
  const after = current.text.slice(caretEnd);
  const created: NoteBlock = {
    ...fresh,
    kind: kindAfterEnter(current.kind),
    text: after,
    checked: false,
  };
  const next = blocks.slice();
  next[index] = { ...current, text: before };
  next.splice(index + 1, 0, created);
  return { blocks: next, focusId: created.id };
};

/**
 * Backspace am Anfang des Blocks `index`: der Text wird an den vorigen Block
 * gehaengt (dessen Art und Zeitstempel bleiben). Der erste Block hat keinen
 * Vorgaenger: `null`. `caret` ist die Stelle der Verschmelzung.
 */
export const mergeWithPrevious = (
  blocks: NoteBlock[],
  index: number,
): { blocks: NoteBlock[]; focusId: string; caret: number } | null => {
  if (index <= 0 || index >= blocks.length) return null;
  const previous = blocks[index - 1];
  const current = blocks[index];
  const merged: NoteBlock = { ...previous, text: previous.text + current.text };
  const next = blocks.slice();
  next.splice(index - 1, 2, merged);
  return { blocks: next, focusId: previous.id, caret: previous.text.length };
};

/** Was gespeichert wird: leere Absaetze (Enter ohne Text) gehen nicht in die DB. */
export const blocksForSave = (blocks: NoteBlock[]): NoteBlock[] =>
  blocks.filter((b) => b.text.trim() !== "");

/** Groesse der gespeicherten Fassung in Bytes (UTF-8). */
export const notesBytes = (blocks: NoteBlock[]): number =>
  new TextEncoder().encode(JSON.stringify(blocksForSave(blocks))).length;

export const notesTooLarge = (blocks: NoteBlock[]): boolean =>
  notesBytes(blocks) > NOTES_MAX_BYTES;

const sameBlock = (a: NoteBlock, b: NoteBlock) =>
  a.kind === b.kind &&
  a.text === b.text &&
  a.checked === b.checked &&
  a.at_ms === b.at_ms;

/**
 * Autosave-Konflikt (`revision_conflict`, z. B. zwei Fenster): der gespeicherte
 * Stand gewinnt die Reihenfolge, lokale Bloecke, die er nicht oder anders
 * kennt, werden als Kopie angehaengt. Nichts wird still verworfen.
 */
export const mergeConflict = (
  remote: NoteBlock[],
  local: NoteBlock[],
  freshId: () => string = newBlockId,
): NoteBlock[] => {
  const merged = remote.slice();
  const remoteById = new Map(remote.map((b) => [b.id, b]));
  for (const block of blocksForSave(local)) {
    const same = remoteById.get(block.id);
    if (!same) {
      merged.push(block);
    } else if (!sameBlock(same, block)) {
      merged.push({ ...block, id: freshId() });
    }
  }
  return merged;
};

// ---------------------------------------------------------------------------
// Fehlercodes -> i18n-Schluessel
// ---------------------------------------------------------------------------

export interface ErrorText {
  key: string;
  params?: Record<string, string>;
}

/**
 * Store-Fehlercodes (`revision_conflict`, `template_readonly`,
 * `template_invalid:<grund>[:<abschnitt>]`, `template_import:<grund>`, ...) als
 * i18n-Schluessel. Unbekanntes wird zum allgemeinen Fehler samt Rohtext.
 */
export const templateErrorText = (code: string): ErrorText => {
  const [head, reason, detail] = code.split(":");
  if (head === "template_invalid" && reason) {
    return {
      key: `meetings.templates.errors.invalid.${reason}`,
      params: { section: detail ?? "" },
    };
  }
  if (head === "template_import" && reason) {
    return { key: `meetings.templates.errors.import.${reason}` };
  }
  if (head === "template_export") {
    return { key: "meetings.templates.errors.exportFailed" };
  }
  if (
    head === "template_readonly" ||
    head === "template_not_found" ||
    head === "revision_conflict"
  ) {
    return { key: `meetings.templates.errors.${head}` };
  }
  return { key: "meetings.templates.errors.generic", params: { error: code } };
};

// ---------------------------------------------------------------------------
// Vorlagen-Editor
// ---------------------------------------------------------------------------

const UMLAUTS: Record<string, string> = {
  ä: "ae",
  ö: "oe",
  ü: "ue",
  ß: "ss",
};

/** Abschnitts-ID aus dem Titel: `[a-z0-9_]{1,32}`, eindeutig gegenueber `taken`. */
export const sectionIdFromTitle = (title: string, taken: string[]): string => {
  const base =
    title
      .toLowerCase()
      .replace(/[äöüß]/g, (c) => UMLAUTS[c])
      .replace(/[^a-z0-9]+/g, "_")
      .replace(/^_+|_+$/g, "")
      .slice(0, 28) || "abschnitt";
  let id = base;
  let n = 2;
  while (taken.includes(id)) id = `${base}_${n++}`;
  return id;
};

export const TEMPLATE_LIMITS = {
  title: 60,
  instruction: 500,
  context: 1000,
  sections: 10,
} as const;

/** Vorabpruefung wie `templates::validate_spec` (die endgueltige Pruefung macht das Backend). */
export const validateSpecLocally = (
  title: string,
  spec: TemplateSpec,
): string | null => {
  const t = title.trim();
  if (t.length === 0 || [...t].length > TEMPLATE_LIMITS.title)
    return "template_invalid:title";
  if ([...spec.context].length > TEMPLATE_LIMITS.context)
    return "template_invalid:context_too_long";
  if (
    spec.sections.length === 0 ||
    spec.sections.length > TEMPLATE_LIMITS.sections
  )
    return "template_invalid:sections_count";
  for (const s of spec.sections) {
    const st = s.title.trim();
    if (st.length === 0 || [...st].length > TEMPLATE_LIMITS.title)
      return `template_invalid:section_title:${s.id}`;
    if ([...s.instruction].length > TEMPLATE_LIMITS.instruction)
      return `template_invalid:section_instruction_too_long:${s.id}`;
  }
  if (spec.sections.filter((s) => s.kind === "tasks").length > 1)
    return "template_invalid:tasks_sections";
  return null;
};

// ---------------------------------------------------------------------------
// KI-Notizen (P1d)
// ---------------------------------------------------------------------------

/** Dokumentart und Format der KI-Notizen (`enhance.rs`: DOC_KIND, DOC_FORMAT). */
export const ENHANCED_KIND = "enhanced_notes";
export const ENHANCED_FORMAT = "enhanced@1";

/** So viele Quellen zeigt ein Eintrag, der Rest steht hinter "+n". */
export const MAX_VISIBLE_SOURCES = 3;

/** Dauer der Markierung des Segments nach einem Quellsprung. */
export const SOURCE_HIGHLIGHT_MS = 2000;

/** Body eines KI-Notizen-Dokuments; `null` bei kaputtem oder fremdem Format. */
export const parseEnhanced = (body: string): EnhancedNotes | null => {
  try {
    const value = JSON.parse(body) as Partial<EnhancedNotes> | null;
    if (
      value &&
      value.format === ENHANCED_FORMAT &&
      Array.isArray(value.sections)
    ) {
      // D5: Dokumente von vor den Folien kennen `source_slide_ids` nicht.
      return {
        ...value,
        sections: value.sections.map((section) => ({
          ...section,
          entries: (section.entries ?? []).map((entry) => ({
            ...entry,
            source_slide_ids: entry.source_slide_ids ?? [],
          })),
        })),
      } as EnhancedNotes;
    }
  } catch {
    // kaputter Body: wie ein fremdes Format behandeln
  }
  return null;
};

/**
 * Quellverweise veraltet? Eine Neu-Transkription erhoeht die Epoche der
 * Segmente; die Nummern der Notizen zeigen dann auf andere Woerter. Ist die
 * aktuelle Epoche noch unbekannt (`null`), gilt nichts als veraltet.
 */
export const isStale = (
  notes: EnhancedNotes,
  currentEpoch: number | null,
): boolean => currentEpoch !== null && notes.segment_epoch !== currentEpoch;

/** KI-Text (grau) ist nur, was die KI schrieb und der Nutzer nicht angefasst hat. */
export const isAiText = (entry: EnhancedEntry): boolean =>
  entry.origin === "ai" && !entry.flags.edited;

/** KI-Eintrag ohne gueltigen Beleg: bekommt das gelbe "ohne Beleg"-Zeichen. */
export const lacksEvidence = (entry: EnhancedEntry): boolean =>
  isAiText(entry) &&
  (entry.flags.unsupported ||
    (entry.source_segment_ids.length === 0 &&
      (entry.source_slide_ids ?? []).length === 0));

/** Sichtbare Quellen und die Zahl der weiteren (`+n`). */
export const splitSources = (
  ids: number[],
  max: number = MAX_VISIBLE_SOURCES,
): { shown: number[]; rest: number[] } => ({
  shown: ids.slice(0, max),
  rest: ids.slice(max),
});

const ENHANCE_ERROR_CODES = [
  "no_provider",
  "no_model",
  "memory_low",
  "recording_active",
  "enhance_busy",
  "no_transcript",
  "llm_failed",
  "meeting_not_finished",
  "meeting_not_found",
  "template_not_found",
  "document_not_found",
  "not_enhanced_notes",
  "stale_document",
  "stale_sources",
  "edit_invalid",
  "instruction_invalid",
  "store_failed",
] as const;

/** Code eines Fehlertexts der Form `<code>` oder `<code>: <grund>`; sonst `null`. */
export const enhanceErrorCode = (error: string): string | null => {
  const head = error.split(":")[0].trim();
  return (ENHANCE_ERROR_CODES as readonly string[]).includes(head)
    ? head
    : null;
};

/**
 * Fehler der KI-Notizen-Commands und `MeetingNotesEvent::failed` als
 * i18n-Schluessel. Unbekanntes wird zum allgemeinen Fehler samt Rohtext.
 */
export const enhanceErrorText = (error: string): ErrorText => {
  const code = enhanceErrorCode(error);
  if (code) {
    const detail = error.includes(":")
      ? error.slice(error.indexOf(":") + 1).trim()
      : "";
    return { key: `meetings.enhanced.errors.${code}`, params: { detail } };
  }
  return { key: "meetings.enhanced.errors.generic", params: { error } };
};

/** Ab welchem Fehler der Nutzer in die Einstellungen der Sprachmodelle muss. */
export const needsProviderSetup = (error: string): boolean => {
  const code = enhanceErrorCode(error);
  return code === "no_provider" || code === "no_model";
};

/** Kopie mit geaendertem Text eines Eintrags; er zaehlt dann als Nutzertext. */
export const withEntryText = (
  notes: EnhancedNotes,
  entryId: string,
  text: string,
): EnhancedNotes => ({
  ...notes,
  sections: notes.sections.map((section) => ({
    ...section,
    entries: section.entries.map((entry) =>
      entry.id === entryId
        ? {
            ...entry,
            text,
            flags: { ...entry.flags, edited: true },
          }
        : entry,
    ),
  })),
});

/** Neuer Nutzereintrag am Ende eines Abschnitts (die endgueltige ID vergibt das Backend). */
export const withNewEntry = (
  notes: EnhancedNotes,
  sectionId: string,
  id: string,
  text: string,
): EnhancedNotes => ({
  ...notes,
  sections: notes.sections.map((section) =>
    section.id === sectionId
      ? {
          ...section,
          entries: [
            ...section.entries,
            {
              id,
              origin: "user",
              text,
              note_id: null,
              source_segment_ids: [],
              source_slide_ids: [],
              assignee: null,
              due: null,
              flags: {
                unsupported: false,
                dropped_sources: 0,
                placed_by_fallback: false,
                edited: true,
              },
            },
          ],
        }
      : section,
  ),
});

/** Ohne Eintraege mit leerem Text (das Backend entfernt sie ohnehin). */
export const withoutEmptyEntries = (notes: EnhancedNotes): EnhancedNotes => ({
  ...notes,
  sections: notes.sections.map((section) => ({
    ...section,
    entries: section.entries.filter((entry) => entry.text.trim() !== ""),
  })),
});

export const hasEmptyEntry = (notes: EnhancedNotes): boolean =>
  notes.sections.some((section) =>
    section.entries.some((entry) => entry.text.trim() === ""),
  );

/** Alle Eintrags-IDs, in Reihenfolge (Vergleich vor/nach dem Speichern). */
export const entryIds = (notes: EnhancedNotes): string[] =>
  notes.sections.flatMap((section) => section.entries.map((e) => e.id));

/** Dateiname-tauglicher Titel fuer den Export. */
export const exportFileName = (title: string): string =>
  `${title.replace(/[\\/:*?"<>|]/g, "_").trim() || "besprechung"}`;
