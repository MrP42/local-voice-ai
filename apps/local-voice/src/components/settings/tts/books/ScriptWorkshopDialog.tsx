import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import {
  BookOpen,
  Plus,
  RotateCcw,
  Save,
  Sparkles,
  Trash2,
  Users,
} from "lucide-react";
import {
  commands,
  type Book,
  type BookCharacter,
  type BookPreview,
  type GenerateOptions,
  type GeneratedScript,
  type MemoryFile,
  type PageInfo,
  type ScriptTemplate,
  type VoiceInfo,
} from "@/bindings";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { Textarea } from "@/components/ui/Textarea";
import { TAG_REGISTRY } from "@/lib/tags/registry";
import { exportFileName } from "@/lib/utils/exportName";

type Step = "book" | "characters" | "memory" | "template" | "generate";
const STEPS: Step[] = ["book", "characters", "memory", "template", "generate"];
const BOOK_EXT = "lvbook";

interface ScriptWorkshopDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  pages: PageInfo[];
  activePageId: string;
  /** Legt eine neue Seite mit diesem Text an und macht sie aktiv. */
  onCreatePage: (title: string, text: string) => Promise<string | null>;
  onPagesChanged: () => void;
}

const uiLangOf = (lng: string | undefined) => (lng ?? "en").split("-")[0];

/**
 * Die Skript-Werkstatt: Buch waehlen oder anlegen, Figuren mit festen
 * Stimmen, Gedaechtnis (vier Markdown-Dateien) lesen und pflegen, Vorlage
 * waehlen und anpassen, Skript erzeugen -- mit Vorschau und dem Vorschlag
 * des Modells, wie das Gedaechtnis fortzuschreiben ist. Uebernommen wird
 * erst auf Klick: neue Seite im Buch, Gedaechtnis aktualisiert.
 */
export const ScriptWorkshopDialog: React.FC<ScriptWorkshopDialogProps> = ({
  open: isOpen,
  onOpenChange,
  pages,
  activePageId,
  onCreatePage,
  onPagesChanged,
}) => {
  const { t, i18n } = useTranslation();
  const uiLang = uiLangOf(i18n.language);
  const [step, setStep] = useState<Step>("book");
  const [books, setBooks] = useState<Book[]>([]);
  const [bookId, setBookId] = useState<string | null>(null);
  const [newTitle, setNewTitle] = useState("");
  const [voices, setVoices] = useState<VoiceInfo[]>([]);
  const [memory, setMemory] = useState<MemoryFile[]>([]);
  const [memoryDirty, setMemoryDirty] = useState<Record<string, string>>({});
  const [templates, setTemplates] = useState<ScriptTemplate[]>([]);
  const [templateId, setTemplateId] = useState("geschichte");
  const [templateDraft, setTemplateDraft] = useState<string | null>(null);
  const [options, setOptions] = useState<GenerateOptions>({
    book_id: null,
    template_id: "geschichte",
    prompt: "",
    part_title: "",
    length_words: 500,
    audience: "",
    tone: "",
    language: uiLang === "de" ? "de" : "en",
    with_tags: true,
    allowed_tags: TAG_REGISTRY.filter((x) => x.verified).map((x) => x.insert),
    character_names: [],
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<GeneratedScript | null>(null);
  const [exportPreview, setExportPreview] = useState<BookPreview | null>(null);
  const [exportRights, setExportRights] = useState(false);

  const book = useMemo(
    () => books.find((b) => b.id === bookId) ?? null,
    [books, bookId],
  );

  const reloadBooks = useCallback(async () => {
    const r = await commands.booksList();
    if (r.status === "ok") {
      setBooks(r.data);
      return r.data;
    }
    setError(r.error);
    return [];
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    setError(null);
    setResult(null);
    setExportPreview(null);
    void (async () => {
      const list = await reloadBooks();
      if (!bookId && list.length > 0) setBookId(list[0].id);
      const v = await commands.ttsListVoiceInfos();
      setVoices(v ?? []);
      const tpl = await commands.booksTemplates();
      if (tpl.status === "ok") setTemplates(tpl.data);
    })();
    // reloadBooks ist stabil; bookId absichtlich nicht als Ausloeser.
  }, [isOpen, reloadBooks]);

  // Gedaechtnis des gewaehlten Buchs laden.
  useEffect(() => {
    if (!bookId) {
      setMemory([]);
      return;
    }
    void commands.booksMemoryRead(bookId).then((r) => {
      if (r.status === "ok") {
        setMemory(r.data);
        setMemoryDirty({});
      }
    });
    setOptions((o) => ({ ...o, book_id: bookId }));
  }, [bookId]);

  useEffect(() => {
    setOptions((o) => ({ ...o, template_id: templateId }));
    setTemplateDraft(null);
  }, [templateId]);

  const currentTemplate = templates.find((x) => x.id === templateId) ?? null;

  // ---- Buch ---------------------------------------------------------------
  const createBook = async () => {
    const r = await commands.booksCreate(newTitle.trim());
    if (r.status === "error") {
      setError(r.error);
      return;
    }
    setNewTitle("");
    await reloadBooks();
    setBookId(r.data.id);
    setStep("characters");
  };
  const deleteBook = async () => {
    if (!book) return;
    await commands.booksDelete(book.id);
    setBookId(null);
    await reloadBooks();
  };
  const updateBook = async (next: Book) => {
    setBooks((all) => all.map((b) => (b.id === next.id ? next : b)));
    const r = await commands.booksUpdate(next);
    if (r.status === "error") setError(r.error);
  };
  const addActivePage = async () => {
    if (!book || !activePageId) return;
    const r = await commands.booksAddPage(book.id, activePageId);
    if (r.status === "ok")
      setBooks((all) => all.map((b) => (b.id === r.data.id ? r.data : b)));
  };

  // ---- Figuren ------------------------------------------------------------
  const setCharacters = (characters: BookCharacter[]) => {
    if (book) void updateBook({ ...book, characters });
  };

  // ---- Gedaechtnis --------------------------------------------------------
  const saveMemory = async (kind: string) => {
    if (!book) return;
    const text = memoryDirty[kind];
    if (text === undefined) return;
    const r = await commands.booksMemoryWrite(book.id, kind, text);
    if (r.status === "error") {
      setError(r.error);
      return;
    }
    setMemory((m) => m.map((f) => (f.kind === kind ? { ...f, text } : f)));
    setMemoryDirty((d) => {
      const next = { ...d };
      delete next[kind];
      return next;
    });
  };

  // ---- Vorlage ------------------------------------------------------------
  const saveTemplate = async () => {
    if (!currentTemplate || templateDraft === null) return;
    const r = await commands.booksTemplateSave({
      ...currentTemplate,
      body: templateDraft,
    });
    if (r.status === "error") {
      setError(r.error);
      return;
    }
    const tpl = await commands.booksTemplates();
    if (tpl.status === "ok") setTemplates(tpl.data);
    setTemplateDraft(null);
  };
  const resetTemplate = async () => {
    if (!currentTemplate) return;
    await commands.booksTemplateReset(currentTemplate.id);
    const tpl = await commands.booksTemplates();
    if (tpl.status === "ok") {
      setTemplates(tpl.data);
      if (!tpl.data.some((x) => x.id === templateId))
        setTemplateId("geschichte");
    }
    setTemplateDraft(null);
  };
  const [newTemplateName, setNewTemplateName] = useState("");
  const createTemplate = async () => {
    const name = newTemplateName.trim();
    if (!name) return;
    const id = `eigene-${Date.now().toString(36)}`;
    const r = await commands.booksTemplateSave({
      id,
      name,
      body: currentTemplate?.body ?? "",
      builtin: false,
      modified: false,
    });
    if (r.status === "error") {
      setError(r.error);
      return;
    }
    setNewTemplateName("");
    const tpl = await commands.booksTemplates();
    if (tpl.status === "ok") setTemplates(tpl.data);
    setTemplateId(id);
  };

  // ---- Erzeugen -----------------------------------------------------------
  const generate = async () => {
    setError(null);
    setResult(null);
    setBusy(true);
    const r = await commands.booksGenerate({ ...options, book_id: bookId });
    setBusy(false);
    if (r.status === "error") {
      setError(r.error);
      return;
    }
    setResult(r.data);
  };
  const apply = async () => {
    if (!result) return;
    setBusy(true);
    const title =
      options.part_title.trim() ||
      (book
        ? `${book.title} – Teil ${book.page_ids.length + 1}: ${result.title}`
        : result.title);
    const pageId = await onCreatePage(title, result.script);
    if (pageId && book) {
      await commands.booksAddPage(book.id, pageId);
      // Gedaechtnis fortschreiben: Verlauf anhaengen, Figuren/Welt ergaenzen.
      const part = book.page_ids.length + 1;
      const append = async (kind: string, addition: string) => {
        const add = addition.trim();
        if (!add) return;
        const current = memory.find((m) => m.kind === kind)?.text ?? "";
        const heading =
          kind === "verlauf"
            ? `\n\n## Teil ${part}: ${result.title}\n`
            : "\n\n";
        await commands.booksMemoryWrite(
          book.id,
          kind,
          `${current.trimEnd()}${heading}${add}\n`,
        );
      };
      await append("verlauf", result.memory.verlauf);
      await append("figuren", result.memory.figuren);
      await append("welt", result.memory.welt);
      await reloadBooks();
    }
    setBusy(false);
    onPagesChanged();
    toast.success(t("tts.workshop.applied", { title }));
    setResult(null);
    onOpenChange(false);
  };

  // ---- Export / Import ----------------------------------------------------
  const loadExportPreview = async () => {
    if (!book) return;
    const r = await commands.booksExportPreview(book.id);
    if (r.status === "ok") {
      setExportPreview(r.data);
      setExportRights(false);
    } else setError(r.error);
  };
  const [exportVoiceIds, setExportVoiceIds] = useState<Set<string>>(new Set());
  useEffect(() => {
    if (exportPreview) {
      setExportVoiceIds(
        new Set(exportPreview.voices.filter((v) => v.present).map((v) => v.id)),
      );
    }
  }, [exportPreview]);
  const runExport = async () => {
    if (!book) return;
    let target: string | null = null;
    try {
      target = await save({
        defaultPath: exportFileName(book.title, "Buch", BOOK_EXT),
        filters: [
          { name: t("tts.workshop.bookFilter"), extensions: [BOOK_EXT] },
        ],
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return;
    }
    if (typeof target !== "string") return;
    setBusy(true);
    const r = await commands.booksExport(
      book.id,
      target,
      [...exportVoiceIds],
      exportRights,
    );
    setBusy(false);
    if (r.status === "error") {
      setError(r.error);
      return;
    }
    toast.success(t("tts.workshop.exported", { path: r.data }));
    setExportPreview(null);
  };
  const runImport = async () => {
    let picked: string | string[] | null = null;
    try {
      picked = await open({
        multiple: false,
        filters: [
          { name: t("tts.workshop.bookFilter"), extensions: [BOOK_EXT] },
        ],
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return;
    }
    if (typeof picked !== "string") return;
    const info = await commands.booksInspect(picked);
    if (info.status === "error") {
      setError(info.error);
      return;
    }
    const importVoices = info.data.voices.some((v) => !v.present)
      ? window.confirm(
          t("tts.workshop.importVoicesAsk", {
            count: info.data.voices.filter((v) => !v.present).length,
            rights: info.data.rights_confirmed
              ? t("tts.workshop.rightsYes")
              : t("tts.workshop.rightsNo"),
          }),
        )
      : false;
    setBusy(true);
    const r = await commands.booksImport(picked, importVoices);
    setBusy(false);
    if (r.status === "error") {
      setError(r.error);
      return;
    }
    await reloadBooks();
    setBookId(r.data.id);
    onPagesChanged();
    toast.success(t("tts.workshop.imported", { title: r.data.title }));
  };

  const voiceOptions = [
    { value: "", label: t("tts.workshop.voiceDefault") },
    ...voices.map((v) => ({ value: v.id, label: v.meta.display_name || v.id })),
  ];

  const stepLabel = (s: Step) => t(`tts.workshop.steps.${s}`);

  return (
    <Dialog
      open={isOpen}
      onOpenChange={onOpenChange}
      title={t("tts.workshop.title")}
      closeLabel={t("common.close")}
      className="max-w-4xl"
      contentClassName="min-h-[28rem]"
    >
      <div className="flex gap-4 text-sm">
        {/* Schritte links */}
        <nav
          className="w-40 shrink-0 space-y-1"
          aria-label={t("tts.workshop.title")}
        >
          {STEPS.map((s) => (
            <button
              key={s}
              type="button"
              onClick={() => setStep(s)}
              disabled={
                s !== "book" && s !== "generate" && s !== "template" && !book
              }
              className={`w-full rounded-md px-2 py-1.5 text-start ${
                step === s
                  ? "bg-logo-primary/15 text-text font-medium"
                  : "text-text/70 hover:bg-mid-gray/15 hover:text-text disabled:opacity-40"
              }`}
              data-testid={`workshop-step-${s}`}
            >
              {stepLabel(s)}
            </button>
          ))}
          {book && (
            <p
              className="pt-2 text-xs text-text/50 truncate"
              title={book.title}
            >
              <BookOpen width={12} height={12} className="inline mr-1" />
              {book.title}
            </p>
          )}
        </nav>

        <div className="min-w-0 flex-1 space-y-3">
          {error && <p className="text-red-400 break-words">{error}</p>}

          {step === "book" && (
            <div className="space-y-3">
              <p className="text-text/70">{t("tts.workshop.bookIntro")}</p>
              <div className="flex flex-wrap items-center gap-2">
                <div className="w-64">
                  <Select
                    value={bookId}
                    options={books.map((b) => ({
                      value: b.id,
                      label: b.title,
                    }))}
                    placeholder={t("tts.workshop.bookPick")}
                    onChange={(v) => setBookId(v)}
                    isClearable={true}
                  />
                </div>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => void deleteBook()}
                  disabled={!book}
                  title={t("tts.workshop.bookDelete")}
                >
                  <Trash2 width={14} height={14} />
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => void runImport()}
                  disabled={busy}
                >
                  {t("tts.workshop.bookImport")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => void loadExportPreview()}
                  disabled={!book || busy}
                >
                  {t("tts.workshop.bookExport")}
                </Button>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Input
                  type="text"
                  value={newTitle}
                  onChange={(e) => setNewTitle(e.target.value)}
                  placeholder={t("tts.workshop.bookNewPlaceholder")}
                  className="w-64"
                  data-testid="workshop-book-title"
                />
                <Button
                  size="sm"
                  onClick={() => void createBook()}
                  disabled={!newTitle.trim()}
                  data-testid="workshop-book-create"
                >
                  <Plus width={14} height={14} />
                  {t("tts.workshop.bookCreate")}
                </Button>
              </div>
              {book && (
                <div className="space-y-2 rounded-md border border-mid-gray/20 p-2">
                  <div className="flex items-center justify-between">
                    <span className="font-medium">
                      {t("tts.workshop.parts", { count: book.page_ids.length })}
                    </span>
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => void addActivePage()}
                      disabled={
                        !activePageId || book.page_ids.includes(activePageId)
                      }
                    >
                      {t("tts.workshop.addActivePage")}
                    </Button>
                  </div>
                  <ol className="list-decimal pl-5 text-text/80">
                    {book.page_ids.map((id) => (
                      <li key={id}>
                        {pages.find((p) => p.id === id)?.title ?? id}
                      </li>
                    ))}
                  </ol>
                  <label className="flex items-center gap-2">
                    <span className="text-text/70">
                      {t("tts.workshop.language")}
                    </span>
                    <Input
                      type="text"
                      value={book.language}
                      onChange={(e) =>
                        void updateBook({ ...book, language: e.target.value })
                      }
                      className="w-20"
                    />
                  </label>
                </div>
              )}
              {exportPreview && (
                <div className="space-y-2 rounded-md border border-amber-500/30 p-2">
                  <p className="font-medium">
                    {t("tts.workshop.exportSummary", {
                      title: exportPreview.title,
                      pages: exportPreview.pages,
                    })}
                  </p>
                  {exportPreview.voices.map((v) => (
                    <label
                      key={v.id}
                      className={`flex items-center gap-2 ${v.present ? "" : "opacity-50"}`}
                    >
                      <input
                        type="checkbox"
                        disabled={!v.present}
                        checked={exportVoiceIds.has(v.id)}
                        onChange={() =>
                          setExportVoiceIds((s) => {
                            const n = new Set(s);
                            if (n.has(v.id)) n.delete(v.id);
                            else n.add(v.id);
                            return n;
                          })
                        }
                      />
                      {v.display_name}
                    </label>
                  ))}
                  <label className="flex items-start gap-2 text-xs">
                    <input
                      type="checkbox"
                      checked={exportRights}
                      disabled={exportVoiceIds.size === 0}
                      onChange={(e) => setExportRights(e.target.checked)}
                      className="mt-0.5"
                    />
                    {t("tts.pages.package.rightsBody")}
                  </label>
                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      onClick={() => void runExport()}
                      disabled={
                        busy || (exportVoiceIds.size > 0 && !exportRights)
                      }
                    >
                      {t("tts.pages.package.exportRun")}
                    </Button>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setExportPreview(null)}
                    >
                      {t("tts.stopConfirmCancel")}
                    </Button>
                  </div>
                </div>
              )}
            </div>
          )}

          {step === "characters" && book && (
            <div className="space-y-2">
              <p className="text-text/70">
                {t("tts.workshop.charactersIntro")}
              </p>
              {book.characters.map((c, i) => (
                <div
                  key={i}
                  className="flex flex-wrap items-center gap-2 rounded-md border border-mid-gray/20 p-2"
                >
                  <Input
                    type="text"
                    value={c.name}
                    onChange={(e) =>
                      setCharacters(
                        book.characters.map((x, j) =>
                          j === i ? { ...x, name: e.target.value } : x,
                        ),
                      )
                    }
                    placeholder={t("tts.workshop.characterName")}
                    className="w-40"
                  />
                  <div className="w-48">
                    <Select
                      value={c.voice_id ?? ""}
                      options={voiceOptions}
                      isClearable={false}
                      onChange={(v) =>
                        setCharacters(
                          book.characters.map((x, j) =>
                            j === i ? { ...x, voice_id: v || null } : x,
                          ),
                        )
                      }
                    />
                  </div>
                  <Input
                    type="text"
                    value={c.description}
                    onChange={(e) =>
                      setCharacters(
                        book.characters.map((x, j) =>
                          j === i ? { ...x, description: e.target.value } : x,
                        ),
                      )
                    }
                    placeholder={t("tts.workshop.characterDescription")}
                    className="min-w-0 flex-1"
                  />
                  <Button
                    variant="danger-ghost"
                    size="sm"
                    onClick={() =>
                      setCharacters(book.characters.filter((_, j) => j !== i))
                    }
                    title={t("tts.workshop.characterRemove")}
                  >
                    <Trash2 width={14} height={14} />
                  </Button>
                </div>
              ))}
              <Button
                variant="secondary"
                size="sm"
                onClick={() =>
                  setCharacters([
                    ...book.characters,
                    { name: "", voice_id: null, description: "" },
                  ])
                }
                data-testid="workshop-character-add"
              >
                <Users width={14} height={14} />
                {t("tts.workshop.characterAdd")}
              </Button>
            </div>
          )}

          {step === "memory" && book && (
            <div className="space-y-3">
              <p className="text-text/70">{t("tts.workshop.memoryIntro")}</p>
              {memory.map((m) => (
                <div key={m.kind} className="space-y-1">
                  <div className="flex items-center justify-between">
                    <span className="font-medium">
                      {t(`tts.workshop.memory.${m.kind}`)}
                    </span>
                    {memoryDirty[m.kind] !== undefined && (
                      <Button size="sm" onClick={() => void saveMemory(m.kind)}>
                        <Save width={14} height={14} />
                        {t("tts.workshop.memorySave")}
                      </Button>
                    )}
                  </div>
                  <Textarea
                    value={memoryDirty[m.kind] ?? m.text}
                    onChange={(e) =>
                      setMemoryDirty((d) => ({
                        ...d,
                        [m.kind]: e.target.value,
                      }))
                    }
                    rows={4}
                    className="w-full font-mono text-xs"
                  />
                </div>
              ))}
            </div>
          )}

          {step === "template" && (
            <div className="space-y-2">
              <p className="text-text/70">{t("tts.workshop.templateIntro")}</p>
              <div className="flex flex-wrap items-center gap-2">
                <div className="w-64">
                  <Select
                    value={templateId}
                    options={templates.map((x) => ({
                      value: x.id,
                      label: x.modified ? `${x.name} *` : x.name,
                    }))}
                    isClearable={false}
                    onChange={(v) => v && setTemplateId(v)}
                  />
                </div>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => void saveTemplate()}
                  disabled={templateDraft === null}
                >
                  <Save width={14} height={14} />
                  {t("tts.workshop.templateSave")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => void resetTemplate()}
                  disabled={
                    !currentTemplate ||
                    (currentTemplate.builtin && !currentTemplate.modified)
                  }
                  title={
                    currentTemplate?.builtin
                      ? t("tts.workshop.templateReset")
                      : t("tts.workshop.templateDelete")
                  }
                >
                  {currentTemplate?.builtin ? (
                    <RotateCcw width={14} height={14} />
                  ) : (
                    <Trash2 width={14} height={14} />
                  )}
                </Button>
                <Input
                  type="text"
                  value={newTemplateName}
                  onChange={(e) => setNewTemplateName(e.target.value)}
                  placeholder={t("tts.workshop.templateNewPlaceholder")}
                  className="w-44"
                />
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => void createTemplate()}
                  disabled={!newTemplateName.trim()}
                >
                  <Plus width={14} height={14} />
                </Button>
              </div>
              <Textarea
                value={templateDraft ?? currentTemplate?.body ?? ""}
                onChange={(e) => setTemplateDraft(e.target.value)}
                rows={12}
                className="w-full font-mono text-xs"
              />
              <p className="text-xs text-text/50">
                {t("tts.workshop.placeholders")}
              </p>
            </div>
          )}

          {step === "generate" && (
            <div className="space-y-3">
              {!result ? (
                <>
                  <Textarea
                    value={options.prompt}
                    onChange={(e) =>
                      setOptions({ ...options, prompt: e.target.value })
                    }
                    rows={4}
                    placeholder={t("tts.workshop.promptPlaceholder")}
                    className="w-full"
                    data-testid="workshop-prompt"
                  />
                  <div className="grid grid-cols-2 gap-2">
                    <label className="space-y-1">
                      <span className="text-xs text-text/60">
                        {t("tts.workshop.partTitle")}
                      </span>
                      <Input
                        type="text"
                        value={options.part_title}
                        onChange={(e) =>
                          setOptions({ ...options, part_title: e.target.value })
                        }
                        className="w-full"
                      />
                    </label>
                    <label className="space-y-1">
                      <span className="text-xs text-text/60">
                        {t("tts.workshop.length")}
                      </span>
                      <Input
                        type="number"
                        value={options.length_words}
                        onChange={(e) =>
                          setOptions({
                            ...options,
                            length_words: Math.max(
                              50,
                              Number(e.target.value) || 0,
                            ),
                          })
                        }
                        className="w-full"
                      />
                    </label>
                    <label className="space-y-1">
                      <span className="text-xs text-text/60">
                        {t("tts.workshop.audience")}
                      </span>
                      <Input
                        type="text"
                        value={options.audience}
                        onChange={(e) =>
                          setOptions({ ...options, audience: e.target.value })
                        }
                        className="w-full"
                        placeholder={t("tts.workshop.audiencePlaceholder")}
                      />
                    </label>
                    <label className="space-y-1">
                      <span className="text-xs text-text/60">
                        {t("tts.workshop.tone")}
                      </span>
                      <Input
                        type="text"
                        value={options.tone}
                        onChange={(e) =>
                          setOptions({ ...options, tone: e.target.value })
                        }
                        className="w-full"
                        placeholder={t("tts.workshop.tonePlaceholder")}
                      />
                    </label>
                    <label className="space-y-1">
                      <span className="text-xs text-text/60">
                        {t("tts.workshop.templateLabel")}
                      </span>
                      <div>
                        <Select
                          value={templateId}
                          options={templates.map((x) => ({
                            value: x.id,
                            label: x.name,
                          }))}
                          isClearable={false}
                          onChange={(v) => v && setTemplateId(v)}
                        />
                      </div>
                    </label>
                    <label className="flex items-center gap-2 self-end">
                      <input
                        type="checkbox"
                        checked={options.with_tags}
                        onChange={(e) =>
                          setOptions({
                            ...options,
                            with_tags: e.target.checked,
                          })
                        }
                      />
                      {t("tts.workshop.withTags")}
                    </label>
                  </div>
                  {book && book.characters.length > 0 && (
                    <div className="space-y-1">
                      <span className="text-xs text-text/60">
                        {t("tts.workshop.charactersInPart")}
                      </span>
                      <div className="flex flex-wrap gap-2">
                        {book.characters
                          .filter((c) => c.name.trim())
                          .map((c) => {
                            const on = options.character_names.includes(c.name);
                            return (
                              <label
                                key={c.name}
                                className={`flex items-center gap-1 rounded-md border px-2 py-0.5 ${on ? "border-logo-primary bg-logo-primary/10" : "border-mid-gray/30"}`}
                              >
                                <input
                                  type="checkbox"
                                  checked={on}
                                  onChange={() =>
                                    setOptions({
                                      ...options,
                                      character_names: on
                                        ? options.character_names.filter(
                                            (n) => n !== c.name,
                                          )
                                        : [...options.character_names, c.name],
                                    })
                                  }
                                />
                                {c.name}
                              </label>
                            );
                          })}
                      </div>
                    </div>
                  )}
                  <Button
                    onClick={() => void generate()}
                    disabled={busy || !options.prompt.trim()}
                    data-testid="workshop-generate"
                  >
                    <Sparkles
                      width={14}
                      height={14}
                      className={busy ? "animate-spin" : undefined}
                    />
                    {busy
                      ? t("tts.workshop.generating")
                      : t("tts.workshop.generate")}
                  </Button>
                </>
              ) : (
                <>
                  <p className="font-medium">{result.title}</p>
                  <Textarea
                    value={result.script}
                    onChange={(e) =>
                      setResult({ ...result, script: e.target.value })
                    }
                    rows={12}
                    className="w-full"
                    data-testid="workshop-result"
                  />
                  {book && (
                    <div className="space-y-1 rounded-md border border-mid-gray/20 p-2">
                      <span className="text-xs font-medium text-text/70">
                        {t("tts.workshop.memoryProposal")}
                      </span>
                      {(["verlauf", "figuren", "welt"] as const).map((k) => (
                        <label key={k} className="block space-y-0.5">
                          <span className="text-xs text-text/60">
                            {t(`tts.workshop.memory.${k}`)}
                          </span>
                          <Textarea
                            value={result.memory[k]}
                            onChange={(e) =>
                              setResult({
                                ...result,
                                memory: {
                                  ...result.memory,
                                  [k]: e.target.value,
                                },
                              })
                            }
                            rows={2}
                            className="w-full text-xs"
                          />
                        </label>
                      ))}
                    </div>
                  )}
                  <div className="flex gap-2">
                    <Button
                      onClick={() => void apply()}
                      disabled={busy}
                      data-testid="workshop-apply"
                    >
                      {t("tts.workshop.apply")}
                    </Button>
                    <Button
                      variant="secondary"
                      onClick={() => void generate()}
                      disabled={busy}
                    >
                      {t("tts.workshop.again")}
                    </Button>
                    <Button
                      variant="secondary"
                      onClick={() => setResult(null)}
                      disabled={busy}
                    >
                      {t("tts.workshop.adjust")}
                    </Button>
                  </div>
                </>
              )}
            </div>
          )}
        </div>
      </div>
    </Dialog>
  );
};
