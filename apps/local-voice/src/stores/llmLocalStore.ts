import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { produce } from "immer";
import { listen } from "@tauri-apps/api/event";
import { commands, type LlmDownloadInfo, type LocalLlmStatus } from "@/bindings";

interface DownloadProgress {
  model_id: string;
  downloaded: number;
  total: number;
  percentage: number;
}

// Lokales Sprachmodell: Laufzeitpakete und Modelle. Eigener Store wie bei
// Piper (`ttsModelStore`) — andere Domäne, aber dieselben Tauri-Ereignisse
// des Downloaders, also keine neue Ereignis-Verdrahtung.
interface LlmLocalStore {
  downloads: LlmDownloadInfo[];
  downloadingIds: Record<string, true>;
  verifyingIds: Record<string, true>;
  downloadProgress: Record<string, DownloadProgress>;
  status: LocalLlmStatus | null;
  loading: boolean;
  error: string | null;
  initialized: boolean;

  initialize: () => Promise<void>;
  loadDownloads: () => Promise<void>;
  refreshStatus: () => Promise<void>;
  downloadModel: (id: string) => Promise<boolean>;
  cancelDownload: (id: string) => Promise<boolean>;
  deleteModel: (id: string) => Promise<boolean>;
  activate: (id: string) => Promise<boolean>;
  stopServer: () => Promise<void>;
}

export const useLlmLocalStore = create<LlmLocalStore>()(
  subscribeWithSelector((set, get) => ({
    downloads: [],
    downloadingIds: {},
    verifyingIds: {},
    downloadProgress: {},
    status: null,
    loading: true,
    error: null,
    initialized: false,

    loadDownloads: async () => {
      try {
        // Fällt die Abfrage aus, bleibt die Liste leer statt undefiniert.
        set({ downloads: (await commands.llmLocalList()) ?? [], error: null });
      } catch (err) {
        set({ error: `Sprachmodelle nicht ladbar: ${err}` });
      } finally {
        set({ loading: false });
      }
    },

    refreshStatus: async () => {
      try {
        set({ status: await commands.llmLocalStatus() });
      } catch {
        // Ohne Backend (Browser-Test) bleibt der Status unbekannt.
      }
    },

    downloadModel: async (id) => {
      set({ error: null });
      set(
        produce((state: LlmLocalStore) => {
          state.downloadingIds[id] = true;
          state.downloadProgress[id] = {
            model_id: id,
            downloaded: 0,
            total: 0,
            percentage: 0,
          };
        }),
      );
      const clear = () =>
        set(
          produce((state: LlmLocalStore) => {
            delete state.downloadingIds[id];
            delete state.downloadProgress[id];
          }),
        );
      try {
        const result = await commands.llmLocalDownload(id);
        if (result.status !== "ok") {
          clear();
          set({ error: String(result.error) });
        }
        return result.status === "ok";
      } catch (err) {
        clear();
        set({ error: String(err) });
        return false;
      }
    },

    cancelDownload: async (id) => {
      try {
        const result = await commands.llmLocalCancel(id);
        set(
          produce((state: LlmLocalStore) => {
            delete state.downloadingIds[id];
            delete state.downloadProgress[id];
          }),
        );
        await get().loadDownloads();
        return result.status === "ok";
      } catch (err) {
        set({ error: String(err) });
        return false;
      }
    },

    deleteModel: async (id) => {
      try {
        const result = await commands.llmLocalDelete(id);
        if (result.status === "ok") {
          await get().loadDownloads();
          await get().refreshStatus();
          return true;
        }
        set({ error: String(result.error) });
        return false;
      } catch (err) {
        set({ error: String(err) });
        return false;
      }
    },

    // Aktivieren legt Verbindung, Freigabe und aktives Modell in einem Zug an
    // (Backend `llm_local_activate`); danach lesen alle Funktionen dieses
    // Modell. Der Server startet erst mit der ersten Anfrage.
    activate: async (id) => {
      set({ error: null });
      const result = await commands.llmLocalActivate(id);
      if (result.status !== "ok") {
        set({ error: String(result.error) });
        return false;
      }
      return true;
    },

    stopServer: async () => {
      await commands.llmLocalStop();
      await get().refreshStatus();
    },

    initialize: async () => {
      if (get().initialized) return;
      set({ initialized: true });
      await get().loadDownloads();
      await get().refreshStatus();
      listen<DownloadProgress>("model-download-progress", (event) => {
        const progress = event.payload;
        if (!(progress.model_id in get().downloadingIds)) return;
        set(
          produce((state: LlmLocalStore) => {
            state.downloadProgress[progress.model_id] = progress;
          }),
        );
      });
      listen<string>("model-verification-started", (event) => {
        if (!(event.payload in get().downloadingIds)) return;
        set(
          produce((state: LlmLocalStore) => {
            state.verifyingIds[event.payload] = true;
          }),
        );
      });
      listen<string>("model-verification-completed", (event) => {
        set(
          produce((state: LlmLocalStore) => {
            delete state.verifyingIds[event.payload];
          }),
        );
      });
      listen<string>("model-download-complete", async (event) => {
        if (!(event.payload in get().downloadingIds)) return;
        set(
          produce((state: LlmLocalStore) => {
            delete state.downloadingIds[event.payload];
            delete state.downloadProgress[event.payload];
            delete state.verifyingIds[event.payload];
          }),
        );
        await get().loadDownloads();
      });
    },
  })),
);
