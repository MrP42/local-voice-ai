import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  ArrowDown,
  ArrowUp,
  Copy,
  Download,
  Pencil,
  Plus,
  Trash2,
  Upload,
} from "lucide-react";
import {
  commands,
  type SectionKind,
  type TemplateInfo,
  type TemplateSection,
  type TemplateSpec,
} from "@/bindings";
import {
  sectionIdFromTitle,
  TEMPLATE_LIMITS,
  templateErrorText,
  validateSpecLocally,
} from "@/lib/meetingNotes";
import { Dialog } from "../../../ui/Dialog";
import { Button } from "../../../ui/Button";
import { Input } from "../../../ui/Input";
import { Select } from "../../../ui/Select";
import { Textarea } from "../../../ui/Textarea";
import { Alert } from "../../../ui/Alert";
import Badge from "../../../ui/Badge";

interface TemplateManagerDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Die Vorlagenliste hat sich geaendert (Auswahllisten neu laden). */
  onChanged?: () => void;
}

/** Vorlage im Editor; neue Abschnitte tragen eine vorlaeufige ID, die beim Speichern aus dem Titel entsteht. */
interface Draft {
  id: string | null;
  title: string;
  spec: TemplateSpec;
}

const NEW_SECTION_PREFIX = "__neu_";

const emptyDraft = (): Draft => ({
  id: null,
  title: "",
  spec: {
    version: 1,
    context: "",
    sections: [
      {
        id: `${NEW_SECTION_PREFIX}1`,
        title: "",
        instruction: "",
        kind: "text",
      },
    ],
  },
});

/** Vorlaeufige Abschnitts-IDs durch title-abgeleitete ersetzen. */
const finalizeSpec = (spec: TemplateSpec): TemplateSpec => {
  const taken = spec.sections
    .filter((s) => !s.id.startsWith(NEW_SECTION_PREFIX))
    .map((s) => s.id);
  const sections = spec.sections.map((s) => {
    if (!s.id.startsWith(NEW_SECTION_PREFIX)) return s;
    const id = sectionIdFromTitle(s.title, taken);
    taken.push(id);
    return { ...s, id };
  });
  return { ...spec, sections };
};

export const TemplateManagerDialog: React.FC<TemplateManagerDialogProps> = ({
  open: isOpen,
  onOpenChange,
  onChanged,
}) => {
  const { t } = useTranslation();
  const [templates, setTemplates] = useState<TemplateInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const errText = useCallback(
    (code: string) => {
      const { key, params } = templateErrorText(code);
      return t(key, {
        ...params,
        defaultValue: t("meetings.templates.errors.generic", { error: code }),
      });
    },
    [t],
  );

  const reload = useCallback(async () => {
    setLoading(true);
    const result = await commands.meetingTemplatesList();
    setLoading(false);
    if (result.status === "ok") setTemplates(result.data);
    else setError(errText(result.error));
  }, [errText]);

  useEffect(() => {
    if (!isOpen) return;
    setError(null);
    setNotice(null);
    setDraft(null);
    setConfirmDelete(null);
    void reload();
  }, [isOpen, reload]);

  const changed = () => {
    void reload();
    onChanged?.();
  };

  const startEdit = (info: TemplateInfo) =>
    setDraft({
      id: info.id,
      title: info.title,
      spec: JSON.parse(JSON.stringify(info.spec)) as TemplateSpec,
    });

  const duplicate = async (info: TemplateInfo, thenEdit: boolean) => {
    setError(null);
    setNotice(null);
    setBusy(true);
    const result = await commands.meetingTemplatesDuplicate(info.id);
    setBusy(false);
    if (result.status === "error") {
      setError(errText(result.error));
      return;
    }
    changed();
    if (thenEdit) {
      setNotice(t("meetings.templates.editBuiltinNotice"));
      startEdit(result.data);
    } else {
      setNotice(
        t("meetings.templates.duplicated", { title: result.data.title }),
      );
    }
  };

  const remove = async (info: TemplateInfo) => {
    setError(null);
    setNotice(null);
    setBusy(true);
    const result = await commands.meetingTemplatesDelete(info.id);
    setBusy(false);
    setConfirmDelete(null);
    if (result.status === "error") {
      setError(errText(result.error));
      return;
    }
    changed();
  };

  const exportOne = async (info: TemplateInfo) => {
    setError(null);
    setNotice(null);
    const safeName =
      info.title.replace(/[\\/:*?"<>|]/g, "_").trim() || "vorlage";
    const target = await save({
      defaultPath: `${safeName}.lvtemplate.json`,
      filters: [
        { name: t("meetings.templates.fileType"), extensions: ["json"] },
      ],
    });
    if (typeof target !== "string") return;
    const result = await commands.meetingTemplatesExport(info.id, target);
    if (result.status === "error") {
      setError(errText(result.error));
      return;
    }
    setNotice(t("meetings.templates.exported", { path: target }));
  };

  const importOne = async () => {
    setError(null);
    setNotice(null);
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: t("meetings.templates.fileType"), extensions: ["json"] },
      ],
    });
    if (typeof picked !== "string") return;
    setBusy(true);
    const result = await commands.meetingTemplatesImport(picked);
    setBusy(false);
    if (result.status === "error") {
      setError(errText(result.error));
      return;
    }
    setNotice(t("meetings.templates.imported", { title: result.data.title }));
    changed();
  };

  const saveDraft = async () => {
    if (!draft) return;
    setError(null);
    const spec = finalizeSpec(draft.spec);
    const invalid = validateSpecLocally(draft.title, spec);
    if (invalid) {
      setError(errText(invalid));
      return;
    }
    setBusy(true);
    const result = await commands.meetingTemplatesSave(
      draft.id,
      draft.title.trim(),
      spec,
    );
    setBusy(false);
    if (result.status === "error") {
      setError(errText(result.error));
      return;
    }
    setDraft(null);
    setNotice(t("meetings.templates.saved", { title: result.data.title }));
    changed();
  };

  const patchSection = (index: number, patch: Partial<TemplateSection>) =>
    setDraft((d) =>
      d
        ? {
            ...d,
            spec: {
              ...d.spec,
              sections: d.spec.sections.map((s, i) =>
                i === index ? { ...s, ...patch } : s,
              ),
            },
          }
        : d,
    );

  const moveSection = (index: number, delta: number) =>
    setDraft((d) => {
      if (!d) return d;
      const target = index + delta;
      if (target < 0 || target >= d.spec.sections.length) return d;
      const sections = d.spec.sections.slice();
      [sections[index], sections[target]] = [sections[target], sections[index]];
      return { ...d, spec: { ...d.spec, sections } };
    });

  const removeSection = (index: number) =>
    setDraft((d) =>
      d
        ? {
            ...d,
            spec: {
              ...d.spec,
              sections: d.spec.sections.filter((_, i) => i !== index),
            },
          }
        : d,
    );

  const addSection = () =>
    setDraft((d) => {
      if (!d || d.spec.sections.length >= TEMPLATE_LIMITS.sections) return d;
      const next = d.spec.sections.length + 1;
      const used = new Set(d.spec.sections.map((s) => s.id));
      let n = next;
      while (used.has(`${NEW_SECTION_PREFIX}${n}`)) n++;
      return {
        ...d,
        spec: {
          ...d.spec,
          sections: [
            ...d.spec.sections,
            {
              id: `${NEW_SECTION_PREFIX}${n}`,
              title: "",
              instruction: "",
              kind: "text" as SectionKind,
            },
          ],
        },
      };
    });

  const sectionsLabel = (count: number) =>
    t("meetings.templates.sectionCount", { count });

  // -------------------------------------------------------------- Editor --
  if (draft) {
    const tasksTaken = (index: number) =>
      draft.spec.sections.some((s, i) => i !== index && s.kind === "tasks");
    return (
      <Dialog
        open={isOpen}
        onOpenChange={onOpenChange}
        title={
          draft.id
            ? t("meetings.templates.editTitle")
            : t("meetings.templates.newTitle")
        }
        closeLabel={t("meetings.templates.close")}
        className="max-w-2xl"
        contentFades={false}
      >
        <div className="space-y-3" data-testid="template-editor">
          {error && <Alert variant="error">{error}</Alert>}
          {notice && <Alert variant="info">{notice}</Alert>}
          <label className="block space-y-1">
            <span className="text-xs text-text/60">
              {t("meetings.templates.fieldName")}
            </span>
            <Input
              value={draft.title}
              maxLength={TEMPLATE_LIMITS.title}
              onChange={(e) => setDraft({ ...draft, title: e.target.value })}
              className="w-full"
              aria-label={t("meetings.templates.fieldName")}
            />
          </label>
          <label className="block space-y-1">
            <span className="text-xs text-text/60">
              {t("meetings.templates.fieldContext")}
            </span>
            <Textarea
              variant="compact"
              value={draft.spec.context}
              maxLength={TEMPLATE_LIMITS.context}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  spec: { ...draft.spec, context: e.target.value },
                })
              }
              placeholder={t("meetings.templates.fieldContextHint")}
              className="w-full"
              aria-label={t("meetings.templates.fieldContext")}
            />
          </label>

          <div className="space-y-2">
            <p className="text-xs font-medium uppercase tracking-wide text-mid-gray">
              {t("meetings.templates.sectionsTitle")}
            </p>
            {draft.spec.sections.map((section, index) => (
              <div
                key={section.id}
                data-testid="template-section"
                className="space-y-2 rounded-md border border-mid-gray/20 p-3"
              >
                <div className="flex items-center gap-2">
                  <Input
                    value={section.title}
                    maxLength={TEMPLATE_LIMITS.title}
                    onChange={(e) =>
                      patchSection(index, { title: e.target.value })
                    }
                    placeholder={t("meetings.templates.sectionTitle")}
                    aria-label={t("meetings.templates.sectionTitle")}
                    className="min-w-0 flex-1"
                  />
                  <div
                    className="w-40 shrink-0"
                    role="group"
                    aria-label={t("meetings.templates.sectionKind")}
                  >
                    <Select
                      value={section.kind}
                      isClearable={false}
                      options={[
                        {
                          value: "text",
                          label: t("meetings.templates.kindText"),
                        },
                        {
                          value: "tasks",
                          label: t("meetings.templates.kindTasks"),
                          isDisabled: tasksTaken(index),
                        },
                      ]}
                      onChange={(kind) => {
                        if (kind)
                          patchSection(index, { kind: kind as SectionKind });
                      }}
                    />
                  </div>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => moveSection(index, -1)}
                    disabled={index === 0}
                    aria-label={t("meetings.templates.moveUp")}
                    title={t("meetings.templates.moveUp")}
                  >
                    <ArrowUp width={14} height={14} />
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => moveSection(index, 1)}
                    disabled={index === draft.spec.sections.length - 1}
                    aria-label={t("meetings.templates.moveDown")}
                    title={t("meetings.templates.moveDown")}
                  >
                    <ArrowDown width={14} height={14} />
                  </Button>
                  <Button
                    size="sm"
                    variant="danger-ghost"
                    onClick={() => removeSection(index)}
                    disabled={draft.spec.sections.length <= 1}
                    aria-label={t("meetings.templates.removeSection")}
                    title={t("meetings.templates.removeSection")}
                  >
                    <Trash2 width={14} height={14} />
                  </Button>
                </div>
                <Textarea
                  variant="compact"
                  value={section.instruction}
                  maxLength={TEMPLATE_LIMITS.instruction}
                  onChange={(e) =>
                    patchSection(index, { instruction: e.target.value })
                  }
                  placeholder={t("meetings.templates.sectionInstruction")}
                  aria-label={t("meetings.templates.sectionInstruction")}
                  className="w-full"
                />
              </div>
            ))}
            <Button
              size="sm"
              variant="secondary"
              onClick={addSection}
              disabled={draft.spec.sections.length >= TEMPLATE_LIMITS.sections}
            >
              <Plus width={14} height={14} />
              {t("meetings.templates.addSection")}
            </Button>
          </div>

          <div className="flex justify-end gap-2 pt-1">
            <Button
              variant="secondary"
              onClick={() => {
                setDraft(null);
                setError(null);
                setNotice(null);
              }}
            >
              {t("meetings.templates.cancel")}
            </Button>
            <Button onClick={() => void saveDraft()} disabled={busy}>
              {t("meetings.templates.save")}
            </Button>
          </div>
        </div>
      </Dialog>
    );
  }

  // --------------------------------------------------------------- Liste --
  return (
    <Dialog
      open={isOpen}
      onOpenChange={onOpenChange}
      title={t("meetings.templates.title")}
      description={t("meetings.templates.description")}
      closeLabel={t("meetings.templates.close")}
      className="max-w-2xl"
      contentFades={false}
    >
      <div className="space-y-3" data-testid="template-manager">
        {error && <Alert variant="error">{error}</Alert>}
        {notice && <Alert variant="info">{notice}</Alert>}

        <div className="flex flex-wrap gap-2">
          <Button
            size="sm"
            onClick={() => {
              setError(null);
              setNotice(null);
              setDraft(emptyDraft());
            }}
          >
            <Plus width={14} height={14} />
            {t("meetings.templates.new")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => void importOne()}
            disabled={busy}
          >
            <Upload width={14} height={14} />
            {t("meetings.templates.import")}
          </Button>
        </div>

        {loading && templates.length === 0 ? (
          <p className="py-3 text-center text-sm text-text/60">
            {t("meetings.list.loading")}
          </p>
        ) : (
          <ul className="divide-y divide-mid-gray/20 rounded-md border border-mid-gray/20">
            {templates.map((info) => (
              <li
                key={info.id}
                data-testid="template-row"
                data-template-id={info.id}
                className="px-3 py-2"
              >
                <div className="flex flex-wrap items-center gap-2">
                  <div className="min-w-0 flex-1">
                    <p className="flex items-center gap-2 text-sm font-medium">
                      <span className="break-words">{info.title}</span>
                      {info.builtin && (
                        <Badge variant="secondary">
                          {t("meetings.templates.builtin")}
                        </Badge>
                      )}
                    </p>
                    <p className="text-xs text-text/60">
                      {sectionsLabel(info.spec.sections.length)}
                    </p>
                  </div>
                  <div className="flex items-center gap-1">
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => void duplicate(info, false)}
                      disabled={busy}
                      title={t("meetings.templates.duplicate")}
                      aria-label={`${t("meetings.templates.duplicate")}: ${info.title}`}
                    >
                      <Copy width={14} height={14} />
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() =>
                        info.builtin
                          ? void duplicate(info, true)
                          : startEdit(info)
                      }
                      disabled={busy}
                      title={
                        info.builtin
                          ? t("meetings.templates.editBuiltin")
                          : t("meetings.templates.edit")
                      }
                      aria-label={`${t("meetings.templates.edit")}: ${info.title}`}
                    >
                      <Pencil width={14} height={14} />
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => void exportOne(info)}
                      title={t("meetings.templates.export")}
                      aria-label={`${t("meetings.templates.export")}: ${info.title}`}
                    >
                      <Download width={14} height={14} />
                    </Button>
                    {!info.builtin && (
                      <Button
                        size="sm"
                        variant="danger-ghost"
                        onClick={() => setConfirmDelete(info.id)}
                        disabled={busy}
                        title={t("meetings.templates.delete")}
                        aria-label={`${t("meetings.templates.delete")}: ${info.title}`}
                      >
                        <Trash2 width={14} height={14} />
                      </Button>
                    )}
                  </div>
                </div>
                {confirmDelete === info.id && (
                  <div className="mt-2 flex flex-wrap items-center gap-2 rounded-md bg-red-500/10 px-3 py-2">
                    <span className="flex-1 text-sm text-red-400">
                      {t("meetings.templates.deleteConfirm", {
                        title: info.title,
                      })}
                    </span>
                    <Button
                      size="sm"
                      variant="danger"
                      onClick={() => void remove(info)}
                      disabled={busy}
                    >
                      {t("meetings.templates.deleteYes")}
                    </Button>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setConfirmDelete(null)}
                    >
                      {t("meetings.templates.cancel")}
                    </Button>
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </Dialog>
  );
};
