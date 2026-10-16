import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { open, save } from "@tauri-apps/plugin-dialog";
import { commands, type Letterhead } from "@/bindings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Dropdown } from "../../ui/Dropdown";
import { Button } from "../../ui/Button";
import { Input } from "../../ui/Input";
import { Alert } from "../../ui/Alert";
import { useSettings } from "../../../hooks/useSettings";

/** Vorgabe (Owner-Annahme 3, `letterhead::DEFAULT_ID`). */
const DEFAULT_ID = "neutral";

/** Id eines eigenen Briefkopfs aus dem Namen: `[a-z0-9-]`, eindeutig. */
export const idFromName = (name: string, taken: string[]): string => {
  const base =
    name
      .toLowerCase()
      .replace(/ä/g, "ae")
      .replace(/ö/g, "oe")
      .replace(/ü/g, "ue")
      .replace(/ß/g, "ss")
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 32) || "briefkopf";
  let id = base;
  for (let n = 2; taken.includes(id); n += 1) id = `${base}-${n}`;
  return id;
};

/** Beim Tippen: Zeilen roh (auch leere, sonst verschwindet jeder Zeilenumbruch). */
const rawLines = (text: string, max: number) => text.split("\n").slice(0, max);

/** Beim Speichern: leere Zeilen und Rand-Leerzeichen weg. */
const clean = (lines: string[] | undefined) =>
  (lines ?? []).map((l) => l.trim()).filter((l) => l.length > 0);

/** Das Profil so, wie es gespeichert wird. */
export const cleanLetterhead = (lh: Letterhead): Letterhead => ({
  ...lh,
  name: lh.name.trim(),
  firma: {
    ...lh.firma,
    name: lh.firma.name.trim(),
    zeilen: clean(lh.firma.zeilen),
  },
  fusszeile: { ...lh.fusszeile, zeilen: clean(lh.fusszeile.zeilen) },
});

/**
 * Gruppe "Dokumente" im Reiter Ausgabe (Entwurf 2026-10-13, Etappe 2): welcher
 * Briefkopf in Word und PDF steht. Mitgelieferte Briefköpfe sind schreibgeschützt;
 * "Eigenen anlegen" kopiert den gewählten, danach lassen sich Name, Firma, Zeilen,
 * Farben, Fußzeile und Logo ändern.
 */
export const DocumentsTab: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [list, setList] = useState<Letterhead[]>([]);
  const [draft, setDraft] = useState<Letterhead | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const selectedId =
    (getSetting("document_letterhead_id") as string | undefined) ?? DEFAULT_ID;
  const selected = list.find((l) => l.id === selectedId) ?? null;

  const load = useCallback(async () => {
    const result = await commands.letterheadsList();
    if (result.status === "ok") setList(result.data ?? []);
    else setError(result.error);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    setDraft(selected && !selected.builtin ? { ...selected } : null);
  }, [selected]);

  const errorText = (raw: string) =>
    t(`settings.documents.errors.${raw.split(":")[0]}`, {
      defaultValue: raw,
    });

  const choose = (id: string) => {
    setError(null);
    setNotice(null);
    void updateSetting("document_letterhead_id", id);
  };

  const createOwn = async () => {
    if (!selected) return;
    const name = t("settings.documents.copyName", { name: selected.name });
    const copy: Letterhead = {
      ...selected,
      id: idFromName(
        name,
        list.map((l) => l.id),
      ),
      name,
      builtin: false,
      // Eingebaute Logos gehören zum mitgelieferten Profil; ein eigenes wählt seins.
      logo: null,
    };
    const result = await commands.letterheadSave(copy);
    if (result.status !== "ok") {
      setError(errorText(result.error));
      return;
    }
    await load();
    choose(copy.id);
  };

  const saveDraft = async () => {
    if (!draft) return;
    const result = await commands.letterheadSave(cleanLetterhead(draft));
    if (result.status !== "ok") {
      setError(errorText(result.error));
      return;
    }
    setError(null);
    setNotice(t("settings.documents.saved"));
    await load();
  };

  const remove = async () => {
    if (!draft) return;
    const result = await commands.letterheadDelete(draft.id);
    if (result.status !== "ok") {
      setError(errorText(result.error));
      return;
    }
    await load();
    choose(DEFAULT_ID);
  };

  const pickLogo = async (clear: boolean) => {
    if (!draft) return;
    let path: string | null = null;
    if (!clear) {
      const picked = await open({
        multiple: false,
        filters: [{ name: "Logo", extensions: ["png", "jpg", "jpeg"] }],
      });
      if (typeof picked !== "string") return;
      path = picked;
    }
    // Erst die Änderungen sichern, sonst überschreibt das Logo sie mit dem alten Stand.
    const saved = await commands.letterheadSave(cleanLetterhead(draft));
    if (saved.status !== "ok") {
      setError(errorText(saved.error));
      return;
    }
    const result = await commands.letterheadSetLogo(draft.id, path);
    if (result.status !== "ok") {
      setError(errorText(result.error));
      return;
    }
    setError(null);
    await load();
  };

  const sample = async () => {
    const target = await save({
      filters: [
        { name: "PDF", extensions: ["pdf"] },
        { name: "Word", extensions: ["docx"] },
      ],
      defaultPath: `Briefkopf ${selected?.name ?? ""}.pdf`,
    });
    if (typeof target !== "string") return;
    const result = await commands.letterheadExportSample(selectedId, target);
    if (result.status !== "ok") setError(errorText(result.error));
    else setNotice(t("settings.documents.sampleSaved", { path: target }));
  };

  const set = (patch: Partial<Letterhead>) =>
    setDraft((d) => (d ? { ...d, ...patch } : d));

  return (
    <SettingsGroup
      title={t("settings.documents.title")}
      description={t("settings.documents.description")}
    >
      <div data-testid="documents-settings">
        <SettingContainer
          title={t("settings.documents.letterhead")}
          description={t("settings.documents.letterheadHint")}
          grouped={true}
        >
          <div className="flex flex-wrap items-center gap-2">
            <Dropdown
              options={list.map((l) => ({
                value: l.id,
                label: l.builtin
                  ? l.name
                  : `${l.name} (${t("settings.documents.own")})`,
              }))}
              selectedValue={selectedId}
              onSelect={choose}
              disabled={isUpdating("document_letterhead_id")}
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void sample()}
              data-testid="letterhead-sample"
            >
              {t("settings.documents.sample")}
            </Button>
            {selected?.builtin && (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void createOwn()}
                data-testid="letterhead-copy"
              >
                {t("settings.documents.createOwn")}
              </Button>
            )}
          </div>
        </SettingContainer>
        {error && (
          <div className="px-4 py-2">
            <Alert variant="error">{error}</Alert>
          </div>
        )}
        {notice && !error && (
          <p className="px-4 py-2 text-xs text-text/60 break-all">{notice}</p>
        )}
        {draft && (
          <div
            className="grid gap-3 px-4 py-3 sm:grid-cols-2"
            data-testid="letterhead-form"
          >
            <label className="flex flex-col gap-1 text-sm">
              <span>{t("settings.documents.name")}</span>
              <Input
                value={draft.name}
                onChange={(e) => set({ name: e.target.value })}
                data-testid="letterhead-name"
              />
            </label>
            <label className="flex flex-col gap-1 text-sm">
              <span>{t("settings.documents.firm")}</span>
              <Input
                value={draft.firma.name}
                onChange={(e) =>
                  set({ firma: { ...draft.firma, name: e.target.value } })
                }
                data-testid="letterhead-firm"
              />
            </label>
            <label className="flex flex-col gap-1 text-sm">
              <span>{t("settings.documents.lines")}</span>
              <textarea
                className="w-full rounded-md border border-mid-gray/30 bg-transparent px-2 py-1 text-sm"
                rows={3}
                data-testid="letterhead-lines"
                value={(draft.firma.zeilen ?? []).join("\n")}
                onChange={(e) =>
                  set({
                    firma: {
                      ...draft.firma,
                      zeilen: rawLines(e.target.value, 4),
                    },
                  })
                }
              />
            </label>
            <label className="flex flex-col gap-1 text-sm">
              <span>{t("settings.documents.footer")}</span>
              <textarea
                className="w-full rounded-md border border-mid-gray/30 bg-transparent px-2 py-1 text-sm"
                rows={3}
                value={(draft.fusszeile.zeilen ?? []).join("\n")}
                onChange={(e) =>
                  set({
                    fusszeile: {
                      ...draft.fusszeile,
                      zeilen: rawLines(e.target.value, 3),
                    },
                  })
                }
              />
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="color"
                value={draft.farben.akzent}
                onChange={(e) =>
                  set({
                    farben: {
                      ...draft.farben,
                      akzent: e.target.value.toUpperCase(),
                    },
                  })
                }
                data-testid="letterhead-accent"
              />
              <span>{t("settings.documents.accent")}</span>
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="color"
                value={draft.farben.titel}
                onChange={(e) =>
                  set({
                    farben: {
                      ...draft.farben,
                      titel: e.target.value.toUpperCase(),
                    },
                  })
                }
              />
              <span>{t("settings.documents.titleColor")}</span>
            </label>
            <div className="flex flex-wrap items-center gap-2 text-sm sm:col-span-2">
              <span>
                {draft.logo
                  ? t("settings.documents.logoSet")
                  : t("settings.documents.logoNone")}
              </span>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void pickLogo(false)}
                data-testid="letterhead-logo"
              >
                {t("settings.documents.logoPick")}
              </Button>
              {draft.logo && (
                <>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => void pickLogo(true)}
                  >
                    {t("settings.documents.logoRemove")}
                  </Button>
                  <Dropdown
                    options={[
                      {
                        value: "links",
                        label: t("settings.documents.logoLeft"),
                      },
                      {
                        value: "rechts",
                        label: t("settings.documents.logoRight"),
                      },
                    ]}
                    selectedValue={draft.logo.position}
                    onSelect={(v) =>
                      draft.logo &&
                      set({
                        logo: {
                          ...draft.logo,
                          position: v === "rechts" ? "rechts" : "links",
                        },
                      })
                    }
                  />
                </>
              )}
            </div>
            <div className="flex gap-2 sm:col-span-2">
              <Button
                size="sm"
                onClick={() => void saveDraft()}
                data-testid="letterhead-save"
              >
                {t("settings.documents.save")}
              </Button>
              <Button
                variant="danger-ghost"
                size="sm"
                onClick={() => void remove()}
                data-testid="letterhead-delete"
              >
                {t("settings.documents.delete")}
              </Button>
            </div>
          </div>
        )}
      </div>
    </SettingsGroup>
  );
};
