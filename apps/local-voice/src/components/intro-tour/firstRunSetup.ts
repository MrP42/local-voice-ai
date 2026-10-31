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
let running: Promise<boolean> | null = null;

/** Einmal je App-Start, auch wenn der Abschluss der Berechtigungen doppelt meldet. */
export const startFirstRunDictation = (): Promise<boolean> =>
  (running ??= setupDictation().finally(() => {
    running = null;
  }));

/** Lädt das Diktatmodell; bei Fehler zeigt die Tour „Wiederholen“. */
const download = (id: string) => {
  const tour = useIntroTourStore.getState();
  tour.setFailedModel(null);
  tour.setPendingModel(id);
  void useModelStore
    .getState()
    .downloadModel(id)
    .then((ok) => {
      if (ok) return;
      // Die Ursache meldet der Modell-Store per Toast.
      const state = useIntroTourStore.getState();
      state.setPendingModel(null);
      state.setFailedModel(id);
    });
};

/**
 * „Wiederholen“ (nur aus der Tour): nach gescheitertem Download erneut laden,
 * nach gescheiterter Modellwahl nur erneut auswählen (kein zweiter Download).
 */
export const retryFirstRunDownload = () => {
  const tour = useIntroTourStore.getState();
  const id = tour.failedModelId;
  if (!id) return;
  const model = useModelStore.getState().models.find((m) => m.id === id);
  if (!model?.is_downloaded) {
    download(id);
    return;
  }
  tour.setFailedModel(null);
  tour.setPendingModel(id);
};

const setupDictation = async (): Promise<boolean> => {
  if (useIntroTourStore.getState().pendingModelId) return true;
  const id = await readFirstRunDictationModel();
  if (!id) return false;
  if (!useModelStore.getState().models.length) {
    await useModelStore.getState().loadModels();
  }
  const models = useModelStore.getState().models;
  const model = models.find((m) => m.id === id);
  if (!model) return false;
  // Liegt schon ein anderes Diktatmodell vor (z. B. Einstellungen nicht lesbar, Onboarding
  // abgebrochen), nichts erzwingen: der Nutzer wählt selbst.
  if (models.some((m) => m.is_downloaded && m.id !== id)) return false;

  useIntroTourStore.getState().setPendingModel(id);
  if (!model.is_downloaded) download(id);
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
  const currentModel = useModelStore((s) => s.currentModel);
  const selectModel = useModelStore((s) => s.selectModel);
  const done = useRef<string | null>(null);

  useEffect(() => {
    if (!pendingModelId) {
      // Nach Fehler oder Abschluss darf „Wiederholen“ erneut auswählen.
      done.current = null;
      return;
    }
    if (!ready || done.current === pendingModelId) return;
    done.current = pendingModelId;
    const { setPendingModel, setFailedModel } = useIntroTourStore.getState();
    // Hat der Nutzer inzwischen selbst ein Modell gewählt, bleibt es dabei.
    if (currentModel && currentModel !== pendingModelId) {
      setPendingModel(null);
      return;
    }
    void selectModel(pendingModelId).then((ok) => {
      if (ok) {
        // Läuft die Tour noch, meldet sie „bereit“ selbst (ohne Toast über
        // ihren Knöpfen) und räumt beim Schließen auf.
        if (useIntroTourStore.getState().open) return;
        toast.success(i18n.t("introTour.download.selected"));
        setPendingModel(null);
      } else {
        setPendingModel(null);
        setFailedModel(pendingModelId);
        toast.error(i18n.t("onboarding.errors.selectModel"));
      }
    });
  }, [pendingModelId, ready, currentModel, selectModel]);
};
