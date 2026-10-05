import { create } from "zustand";
import {
  commands,
  type Assessment,
  type ComplianceProfile,
  type ShieldStatus,
} from "@/bindings";

/** Ereignis, nach dem Schild und Modellauswahl neu fragen (Regelwerk,
 * Verbindung oder aktives Modell geändert). */
export const COMPLIANCE_CHANGED = "lv-compliance-changed";

export const notifyComplianceChanged = () =>
  window.dispatchEvent(new CustomEvent(COMPLIANCE_CHANGED));

interface ComplianceStore {
  status: ShieldStatus | null;
  /** Bewertung je freigegebenem Modell (`LlmModelConfig.id`). */
  byModel: Record<string, Assessment>;
  refresh: () => Promise<void>;
  setProfile: (profile: ComplianceProfile) => Promise<boolean>;
  setTrainingOptOut: (connectionId: string, optOut: boolean) => Promise<boolean>;
}

export const useComplianceStore = create<ComplianceStore>()((set, get) => ({
  status: null,
  byModel: {},

  refresh: async () => {
    try {
      const [status, models] = await Promise.all([
        commands.complianceStatus(),
        commands.complianceAssessModels(),
      ]);
      const byModel: Record<string, Assessment> = {};
      for (const m of models ?? []) byModel[m.model_id] = m.assessment;
      set({ status: status ?? null, byModel });
    } catch {
      // Ohne Backend (Browser-Test) bleibt das Schild unbekannt.
    }
  },

  setProfile: async (profile) => {
    const result = await commands.complianceSetProfile(profile);
    await get().refresh();
    notifyComplianceChanged();
    return result.status === "ok";
  },

  setTrainingOptOut: async (connectionId, optOut) => {
    const result = await commands.complianceSetTrainingOptOut(connectionId, optOut);
    await get().refresh();
    notifyComplianceChanged();
    return result.status === "ok";
  },
}));
