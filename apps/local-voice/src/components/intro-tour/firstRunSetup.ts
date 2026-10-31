import { useEffect, useRef } from "react";
import { toast } from "sonner";
import i18n from "@/i18n";
import { readFirstRunDictationModel } from "@/lib/firstRun";
import { useIntroTourStore } from "@/stores/introTourStore";
import { useModelStore } from "@/stores/modelStore";

/**
 * Erster Start: Diktatmodell ohne Auswahlbildschirm einrichten. Gibt `false`
 * zurück, wenn die Auswahl beim Nutzer bleibt (Installer-Haken aus oder
 * Modell unbekannt) — dann folgt die bisherige Modellauswahl.
 */
export const startFirstRunDictation = async (): Promise<boolean> => {
  const id = await readFirstRunDictationModel();
  if (!id) return false;
  if (!useModelStore.getState().models.length) {
    await useModelStore.getState().loadModels();
  }
  const model = useModelStore.getState().models.find((m) => m.id === id);
  if (!model) return false;

  const { setPendingModel } = useIntroTourStore.getState();
  setPendingModel(id);
  if (!model.is_downloaded) {
    void useModelStore
      .getState()
      .downloadModel(id)
      .then((ok) => {
        if (ok) return;
        setPendingModel(null);
        // Die Ursache meldet der Modell-Store; hier nur der Weg weiter.
        toast.info(i18n.t("introTour.download.failed"));
      });
  }
  return true;
};

/**
 * Sobald das beim ersten Start geladene Modell fertig ist, wird es zum
 * Diktatmodell (setzt im Backend auch `onboarding_completed`).
 */
export const useFirstRunModelSelection = () => {
  const pendingModelId = useIntroTourStore((s) => s.pendingModelId);
  const ready = useModelStore((s) => {
    if (!pendingModelId) return false;
    const model = s.models.find((m) => m.id === pendingModelId);
    return (
      !!model?.is_downloaded &&
      !(pendingModelId in s.downloadingModels) &&
      !(pendingModelId in s.verifyingModels) &&
      !(pendingModelId in s.extractingModels)
    );
  });
  const selectModel = useModelStore((s) => s.selectModel);
  const done = useRef<string | null>(null);

  useEffect(() => {
    if (!pendingModelId || !ready || done.current === pendingModelId) return;
    done.current = pendingModelId;
    void selectModel(pendingModelId).then((ok) => {
      if (ok) {
        toast.success(i18n.t("introTour.download.selected"));
      } else {
        done.current = null;
        toast.error(i18n.t("onboarding.errors.selectModel"));
      }
    });
  }, [pendingModelId, ready, selectModel]);
};
