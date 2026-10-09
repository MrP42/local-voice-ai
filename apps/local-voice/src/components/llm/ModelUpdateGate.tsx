import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { listen } from "@tauri-apps/api/event";
import { commands, type ModelUpdate } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { modelLabel } from "@/lib/modelNames";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";

/** Erste Pruefung kurz nach dem Start, damit sie den Start nicht bremst. */
const FIRST_CHECK_MS = 20_000;
/** Danach einmal am Tag, solange die App offen bleibt. */
const DAILY_MS = 24 * 60 * 60 * 1000;

const name = (remoteId: string) => modelLabel(remoteId, remoteId);

/**
 * Prueft nach dem Start (und taeglich), ob die Anbieter neue Modelle haben,
 * und fragt beim ersten Fund, ob kuenftig automatisch aktualisiert werden
 * soll. Die Einstellung steht auch unter Einstellungen > KI-Modelle & Anbieter.
 *
 * Das Backend entscheidet, was zu tun ist (`ModelCheck`): bei "automatisch"
 * hat es schon uebernommen (`applied`, hier nur ein Hinweis), bei "fragen"
 * wartet der Dialog (`pending`), bei "nein" bleibt es bei der Kennzeichnung.
 */
export const ModelUpdateGate: React.FC = () => {
  const { t } = useTranslation();
  const { refreshSettings } = useSettings();
  const [pending, setPending] = useState<ModelUpdate[]>([]);
  const [busy, setBusy] = useState(false);

  const announce = useCallback(
    (applied: ModelUpdate[]) => {
      if (applied.length === 0) return;
      toast.success(
        t("llmUpdate.applied", { count: applied.length }),
        {
          description: applied
            .map((u) =>
              u.replaces_remote_id
                ? t("llmUpdate.replaces", {
                    model: name(u.remote_id),
                    old: name(u.replaces_remote_id),
                  })
                : t("llmUpdate.added", { model: name(u.remote_id) }),
            )
            .join("\n"),
        },
      );
    },
    [t],
  );

  const check = useCallback(async () => {
    try {
      const result = await commands.llmCheckNewModels();
      if (result.status !== "ok" || !result.data) return;
      announce(result.data.applied);
      if (result.data.applied.length > 0 || result.data.pending.length > 0)
        await refreshSettings();
      setPending(result.data.pending);
    } catch {
      /* Die Pruefung ist Beiwerk: kein Netz, keine CLI -- kein Hinweis. */
    }
  }, [announce, refreshSettings]);

  useEffect(() => {
    const first = window.setTimeout(() => void check(), FIRST_CHECK_MS);
    const daily = window.setInterval(() => void check(), DAILY_MS);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(daily);
    };
  }, [check]);

  // Die Codex-CLI aktualisiert sich selbst, wenn sie ein Modell nicht annimmt
  // (Backend: managers::llm::cli_update). Hier nur die Rueckmeldung.
  useEffect(() => {
    const un = listen<{
      cli: string;
      state: "started" | "done" | "failed" | "manual";
      version: string | null;
      message: string | null;
    }>("cli-update", (event) => {
      const { state, version, message } = event.payload;
      const id = "cli-update";
      if (state === "started")
        toast.loading(t("llmUpdate.cli.started"), { id });
      else if (state === "done")
        toast.success(t("llmUpdate.cli.done", { version }), {
          id,
          description: t("llmUpdate.cli.retry"),
          duration: 15000,
        });
      else
        toast.error(t(`llmUpdate.cli.${state}`), {
          id,
          description: message ?? undefined,
          duration: 20000,
        });
    });
    return () => void un.then((off) => off());
  }, [t]);

  const answer = async (choice: "always" | "once" | "never") => {
    setBusy(true);
    try {
      const result = await commands.llmAnswerModelUpdates(choice);
      if (result.status === "ok") announce(result.data);
      else toast.error(String(result.error));
      await refreshSettings();
    } finally {
      setBusy(false);
      setPending([]);
    }
  };

  return (
    <Dialog
      open={pending.length > 0}
      onOpenChange={(open) => {
        // Schliessen ohne Antwort: die Frage kommt bei der naechsten Pruefung wieder.
        if (!open) setPending([]);
      }}
      title={t("llmUpdate.title", { count: pending.length })}
      closeLabel={t("llmUpdate.close")}
      description={t("llmUpdate.question")}
      className="max-w-lg"
      footer={
        <div className="flex flex-wrap justify-end gap-2">
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => void answer("never")}
            data-testid="model-update-never"
          >
            {t("llmUpdate.never")}
          </Button>
          <Button
            variant="secondary"
            disabled={busy}
            onClick={() => void answer("once")}
            data-testid="model-update-once"
          >
            {t("llmUpdate.once")}
          </Button>
          <Button
            disabled={busy}
            onClick={() => void answer("always")}
            data-testid="model-update-always"
          >
            {t("llmUpdate.always")}
          </Button>
        </div>
      }
    >
      <ul className="space-y-2 text-sm" data-testid="model-update-list">
        {pending.map((u) => (
          <li key={`${u.connection_id}:${u.remote_id}`}>
            <span className="font-medium">{name(u.remote_id)}</span>
            <span className="text-text/60"> · {u.connection_label}</span>
            <div className="text-text/70">
              {u.replaces_remote_id
                ? t(
                    u.replaces_active
                      ? "llmUpdate.replacesActive"
                      : "llmUpdate.replacesInList",
                    { old: name(u.replaces_remote_id) },
                  )
                : t("llmUpdate.isNew")}
            </div>
          </li>
        ))}
      </ul>
    </Dialog>
  );
};
