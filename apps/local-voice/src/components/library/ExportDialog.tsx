import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  AlertCircle,
  BookOpen,
  Check,
  FileText,
  Headphones,
  Languages,
  Loader2,
  Sparkles,
  Volume2,
} from "lucide-react";
import { commands, type LibraryBook } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Select } from "../ui/Select";
import {
  audioChapters,
  isTextVersion,
  langLabel,
  listVersions,
  versionText,
  type BookVersion,
} from "@/lib/library/versions";
import { toSpokenScript } from "@/lib/reader/spokenScript";
import { startExport as startNarration } from "../settings/meetings/readAloudSession";
import { toast } from "sonner";
import { useBookJobs, type BookJob } from "@/lib/library/bookJobs";
import { jobStatus, jobTitle } from "./KiCreateDialog";
import "./fassungen.css";

/** Formate je Art. `narrate` = Text von der eingestellten Stimme vorgelesen (MP3/WAV). */
type TextFormat = "epub" | "pdf" | "docx" | "md" | "txt" | "narrate";
type AudioFormat = "original" | "m4b" | "mp3";
type Format = TextFormat | AudioFormat;

const TEXT_FORMATS: TextFormat[] = [
  "epub",
  "pdf",
  "docx",
  "md",
  "txt",
  "narrate",
];

type RowState =
  | { phase: "idle" }
  | { phase: "running" }
  | { phase: "done"; paths: string[]; background?: boolean }
  | { phase: "error"; message: string };

/** Voreinstellung: Bücher als EPUB, KI-Dokumente als PDF, Hörbuch unverändert. */
const defaultFormat = (v: BookVersion): Format =>
  v.kind === "original" || v.kind === "translation"
    ? "epub"
    : v.kind === "kurzbuch" || v.kind === "kidoc"
      ? "pdf"
      : "original";

/** Windows- und macOS-taugliche Namen; der Kern prüft nicht noch einmal. */
const safe = (s: string) =>
  s
    .replace(/[\\/:*?"<>|]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/^[.\s]+|[.\s]+$/g, "")
    .slice(0, 120)
    .replace(/[.\s]+$/g, "") || "Export";

const join = (dir: string, name: string) =>
  `${dir.replace(/[\\/]+$/, "")}${dir.includes("\\") ? "\\" : "/"}${name}`;

/**
 * Exportieren (lva#236): jede Fassung eines Buchs einzeln – Buchtext,
 * Übersetzungen, Kurzbücher, KI-Dokumente als EPUB, PDF, Word, Markdown oder
 * Text (oder vorgelesen als Audiodatei), das Hörbuch unverändert oder als
 * eine M4B/MP3-Datei, Vertonungen als Datei. Ziel ist ein Ordner; die Namen
 * nennen Buch und Fassung.
 */
export const ExportDialog: React.FC<{
  book: LibraryBook | null;
  /** Vorausgewählte Fassung (aus Vorlesen: der offene Reiter). */
  preselect?: string[];
  onClose(): void;
}> = ({ book, preselect, onClose }) => {
  const { t } = useTranslation();
  const open = book !== null;
  const [versions, setVersions] = useState<BookVersion[] | null>(null);
  const [chosen, setChosen] = useState<Record<string, boolean>>({});
  const [formats, setFormats] = useState<Record<string, Format>>({});
  const [ffmpeg, setFfmpeg] = useState(false);
  const [rows, setRows] = useState<Record<string, RowState>>({});
  const [running, setRunning] = useState(false);
  const [lastDir, setLastDir] = useState<string | null>(null);

  const key = book ? `${book.id}:${book.text?.page_id ?? ""}` : "";

  // Was gerade entsteht, ist noch nicht fertig und lässt sich nicht exportieren.
  const pageId = book?.text?.page_id ?? null;
  const jobs = useBookJobs().filter((j) => !!pageId && j.pageId === pageId);
  const bookLang = versions?.find((v) => v.kind === "original")?.lang ?? null;
  const jobFor = (v: BookVersion): BookJob | undefined =>
    jobs.find((j) => {
      if (v.kind === "translation")
        return j.kind === "translation" && j.target === v.target;
      const sameLang =
        (j.lang || bookLang || null) === (v.lang || bookLang || null);
      if (v.kind === "kurzbuch") return j.kind === "kurzbuch" && sameLang;
      if (v.kind === "kidoc")
        return (
          j.kind === "kidoc" && j.templateId === v.doc?.template_id && sameLang
        );
      return false;
    });
  const pendingJobs = jobs.filter(
    (j) => !(versions ?? []).some((v) => jobFor(v)?.id === j.id),
  );
  const busyKeys = new Set(
    (versions ?? []).filter((v) => !!jobFor(v)).map((v) => v.key),
  );
  useEffect(() => {
    if (!book) return;
    // Hinweise („läuft im Hintergrund“) würden die Dialogknöpfe verdecken;
    // den Stand zeigt der Dialog selbst (Codex #237).
    toast.dismiss();
    let alive = true;
    setVersions(null);
    setRows({});
    setLastDir(null);
    void listVersions(book).then((list) => {
      if (!alive) return;
      setVersions(list);
      const pre = preselect?.filter((k) => list.some((v) => v.key === k)) ?? [];
      const first = pre.length ? pre : list.length ? [list[0].key] : [];
      setChosen(Object.fromEntries(first.map((k) => [k, true])));
      setFormats(
        Object.fromEntries(list.map((v) => [v.key, defaultFormat(v)])),
      );
    });
    void commands
      .libraryFfmpegAvailable()
      .then((ok) => alive && setFfmpeg(ok))
      .catch(() => alive && setFfmpeg(false));
    return () => {
      alive = false;
    };
    // `preselect` gilt nur beim Öffnen; `key` steht für Buch und Seite.
  }, [key]);

  const label = (v: BookVersion): string => {
    const lang = langLabel(v.lang);
    switch (v.kind) {
      case "original":
        return lang
          ? t("library.export.originalLang", { lang })
          : t("library.export.original");
      case "translation":
        return t("library.export.translation", { lang: lang || v.target });
      case "kurzbuch":
        return lang
          ? t("library.export.kurzbuchLang", { lang })
          : t("library.export.kurzbuch");
      case "kidoc":
        return v.doc?.title ?? "";
      case "audiobook":
        return (v.paths?.length ?? 0) > 1
          ? t("library.export.audiobookParts", { count: v.paths!.length })
          : t("library.export.audiobook");
      case "narration":
        return t("library.export.narration", { file: v.file });
    }
  };

  const icon = (v: BookVersion) =>
    v.kind === "original"
      ? BookOpen
      : v.kind === "translation"
        ? Languages
        : v.kind === "kurzbuch" || v.kind === "kidoc"
          ? Sparkles
          : v.kind === "audiobook"
            ? Headphones
            : Volume2;

  const formatOptions = (v: BookVersion) =>
    isTextVersion(v)
      ? TEXT_FORMATS.map((f) => ({
          value: f,
          label: t(`library.export.format.${f}`),
        }))
      : v.kind === "audiobook"
        ? (
            ["original", ...(ffmpeg ? ["m4b", "mp3"] : [])] as AudioFormat[]
          ).map((f) => ({
            value: f,
            label: t(`library.export.format.${f}`),
          }))
        : [];

  const selected = (versions ?? []).filter(
    (v) => chosen[v.key] && !busyKeys.has(v.key),
  );
  const narrations = selected.filter(
    (v) => formats[v.key] === "narrate",
  ).length;

  // Je Lauf eindeutig: zwei KI-Dokumente gleichen Titels überschreiben sich nicht.
  const usedNames = useRef(new Map<string, string>());
  const fileName = (v: BookVersion) => {
    const known = usedNames.current.get(v.key);
    if (known) return known;
    const base = safe(`${book?.title ?? ""} – ${label(v)}`);
    const taken = new Set(
      [...usedNames.current.values()].map((x) => x.toLowerCase()),
    );
    let name = base;
    for (let n = 2; taken.has(name.toLowerCase()); n++) name = `${base} (${n})`;
    usedNames.current.set(v.key, name);
    return name;
  };

  const exportOne = async (v: BookVersion, dir: string): Promise<RowState> => {
    if (!book) return { phase: "error", message: "" };
    const format = formats[v.key] ?? defaultFormat(v);
    if (isTextVersion(v)) {
      const text = await versionText(book, v);
      if (!text?.trim())
        return { phase: "error", message: t("library.export.noText") };
      if (format === "narrate") {
        // Läuft im Kern weiter und meldet sich selbst (Fortschritt, Fertig-Hinweis).
        const target = join(dir, `${fileName(v)}.mp3`);
        const ok = await startNarration(
          `library-export:${v.key}`,
          toSpokenScript(text),
          target,
        );
        return ok
          ? { phase: "done", paths: [target], background: true }
          : { phase: "error", message: t("library.export.errors.narration") };
      }
      const r = await commands.libraryExportText(
        {
          title: v.kind === "kidoc" ? (v.doc?.title ?? book.title) : book.title,
          authors: book.authors,
          language: v.lang,
          cover_path: book.cover_path,
        },
        text,
        join(dir, `${fileName(v)}.${format}`),
      );
      return r.status === "ok"
        ? { phase: "done", paths: [r.data] }
        : { phase: "error", message: errorText(r.error) };
    }
    const paths = v.paths ?? [];
    const r = await commands.libraryExportAudio(
      paths,
      v.kind === "audiobook" ? audioChapters(book) : [],
      join(dir, fileName(v)),
      format === "m4b" || format === "mp3" ? format : "original",
      book.title,
    );
    return r.status === "ok"
      ? { phase: "done", paths: r.data }
      : { phase: "error", message: errorText(r.error) };
  };

  const errorText = (code: string) => {
    const base = code.split(":")[0];
    const known = [
      "export_empty",
      "export_unsupported_format",
      "export_no_audio",
      "export_audio_missing",
      "export_ffmpeg_missing",
      "export_ffmpeg_failed",
      "export_mixed_audio",
    ];
    return known.includes(base)
      ? t(`library.export.errors.${base}`)
      : t("library.export.errors.generic", { error: code });
  };

  const run = async () => {
    if (!selected.length) return;
    const picked = await openDialog({
      directory: true,
      title: t("library.export.pickFolder"),
    });
    const dir = Array.isArray(picked) ? picked[0] : picked;
    if (typeof dir !== "string" || !dir) return;
    setRunning(true);
    setLastDir(dir);
    usedNames.current = new Map();
    setRows(
      Object.fromEntries(
        selected.map((v) => [v.key, { phase: "idle" } as RowState]),
      ),
    );
    for (const v of selected) {
      setRows((r) => ({ ...r, [v.key]: { phase: "running" } }));
      let state: RowState;
      try {
        state = await exportOne(v, dir);
      } catch (e) {
        state = { phase: "error", message: String(e) };
      }
      setRows((r) => ({ ...r, [v.key]: state }));
    }
    setRunning(false);
  };

  if (!book) return null;
  const finished =
    !running &&
    Object.values(rows).length > 0 &&
    Object.values(rows).every((r) => r.phase === "done" || r.phase === "error");
  // Im Hintergrund vorgelesene Dateien gibt es noch nicht – dann den Ordner zeigen.
  const firstPath =
    Object.values(rows).flatMap((r) =>
      r.phase === "done" && !r.background ? r.paths : [],
    )[0] ?? lastDir;

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && !running && onClose()}
      dismissible={!running}
      title={t("library.export.title")}
      description={book.title}
      closeLabel={t("library.cancel")}
      className="lib-export library-dialog-touch"
      footer={
        finished ? (
          <>
            {firstPath && (
              <Button
                variant="primary"
                onClick={() =>
                  void revealItemInDir(firstPath).catch(() => {
                    if (lastDir) void revealItemInDir(lastDir).catch(() => {});
                  })
                }
                data-testid="export-reveal"
              >
                {t("library.export.reveal")}
              </Button>
            )}
            <Button onClick={onClose}>{t("library.export.close")}</Button>
          </>
        ) : (
          <>
            <Button
              variant="primary"
              onClick={() => void run()}
              disabled={running || !selected.length || narrations > 1}
              data-testid="export-run"
            >
              {running
                ? t("library.export.running")
                : t("library.export.run", { count: selected.length })}
            </Button>
            <Button onClick={onClose} disabled={running}>
              {t("library.cancel")}
            </Button>
          </>
        )
      }
    >
      <div className="lib-export__body" data-testid="export-dialog">
        {versions === null ? (
          <p className="library-meta" role="status">
            <Loader2 size={14} className="lib-spin" aria-hidden="true" />{" "}
            {t("library.export.loading")}
          </p>
        ) : versions.length === 0 ? (
          <p className="library-meta">{t("library.export.nothing")}</p>
        ) : (
          <ul
            className="lib-export__list"
            aria-label={t("library.export.listLabel")}
          >
            {versions.map((v) => {
              const Icon = icon(v);
              const opts = formatOptions(v);
              const state = rows[v.key];
              const name = label(v);
              const job = jobFor(v);
              const on = !!chosen[v.key] && !job;
              return (
                <li
                  key={v.key}
                  className={`lib-export__row${on ? " is-on" : ""}${job ? " is-busy" : ""}`}
                  data-testid={`export-row-${v.kind}`}
                  aria-disabled={job ? true : undefined}
                >
                  <label className="lib-export__pick">
                    <input
                      type="checkbox"
                      checked={on}
                      disabled={running || !!job}
                      onChange={(e) =>
                        setChosen((c) => ({ ...c, [v.key]: e.target.checked }))
                      }
                      aria-label={name}
                    />
                    <span className="lib-export__icon" aria-hidden="true">
                      <Icon size={16} />
                    </span>
                    <span className="lib-export__name">
                      <span>{name}</span>
                      {v.kind === "kidoc" && v.doc?.template_title && (
                        <small>{v.doc.template_title}</small>
                      )}
                    </span>
                  </label>
                  <span className="lib-export__format">
                    {opts.length > 1 ? (
                      <Select
                        menuPortal
                        isClearable={false}
                        value={formats[v.key] ?? defaultFormat(v)}
                        options={opts}
                        onChange={(f) =>
                          f !== null &&
                          setFormats((s) => ({ ...s, [v.key]: f as Format }))
                        }
                        ariaLabel={t("library.export.formatFor", { name })}
                        disabled={running || !!job}
                      />
                    ) : (
                      <span className="library-meta">
                        <FileText size={12} aria-hidden="true" />{" "}
                        {t("library.export.format.original")}
                      </span>
                    )}
                  </span>
                  {job && (
                    <span
                      className="lib-export__state is-busy"
                      role="status"
                      data-testid={`export-busy-${v.kind}`}
                    >
                      <Loader2
                        size={14}
                        className="lib-spin"
                        aria-hidden="true"
                      />
                      {t("library.export.rowBusy", {
                        status: jobStatus(t, job),
                      })}
                    </span>
                  )}
                  {state && state.phase !== "idle" && (
                    <span
                      className={`lib-export__state is-${state.phase}`}
                      role="status"
                      data-testid={`export-state-${v.kind}`}
                    >
                      {state.phase === "running" ? (
                        <>
                          <Loader2
                            size={14}
                            className="lib-spin"
                            aria-hidden="true"
                          />
                          {t("library.export.rowRunning")}
                        </>
                      ) : state.phase === "done" ? (
                        <>
                          <Check size={14} aria-hidden="true" />
                          {state.background
                            ? t("library.export.rowBackground")
                            : t("library.export.rowDone")}
                        </>
                      ) : (
                        <>
                          <AlertCircle size={14} aria-hidden="true" />
                          {state.message}
                        </>
                      )}
                    </span>
                  )}
                </li>
              );
            })}
            {pendingJobs.map((j) => (
              <li
                key={j.id}
                className="lib-export__row is-busy"
                aria-disabled
                data-testid="export-row-pending"
              >
                <span className="lib-export__pick">
                  <input
                    type="checkbox"
                    checked={false}
                    disabled
                    aria-label={jobTitle(t, j)}
                  />
                  <span className="lib-export__icon" aria-hidden="true">
                    {j.kind === "translation" ? (
                      <Languages size={16} />
                    ) : (
                      <Sparkles size={16} />
                    )}
                  </span>
                  <span className="lib-export__name">
                    <span>{jobTitle(t, j)}</span>
                  </span>
                </span>
                <span className="lib-export__state is-busy" role="status">
                  <Loader2 size={14} className="lib-spin" aria-hidden="true" />
                  {t("library.export.rowBusy", { status: jobStatus(t, j) })}
                </span>
              </li>
            ))}
          </ul>
        )}
        {narrations > 1 && (
          <p className="lib-export__warn" role="alert">
            {t("library.export.oneNarration")}
          </p>
        )}
        {versions !== null &&
          versions.some((v) => v.kind === "audiobook") &&
          !ffmpeg && (
            <p className="library-meta">{t("library.export.ffmpegHint")}</p>
          )}
        <p className="library-meta">{t("library.export.hint")}</p>
      </div>
    </Dialog>
  );
};
