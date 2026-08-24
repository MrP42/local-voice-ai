import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { produce } from "immer";
import { listen } from "@tauri-apps/api/event";
import { commands, type TtsDownloadInfo } from "@/bindings";

interface DownloadProgress {
  model_id: string;
  downloaded: number;
  total: number;
  percentage: number;
}

// Piper runtime + voice downloads (Paket B-E3). A dedicated store, not part of
// `useModelStore`, because the domain is different (Piper, not ASR) — but it
// listens to the SAME Tauri events the ASR downloader emits (see
// `TtsModelManager::run_download`), so there is no new event plumbing.
interface TtsModelsStore {
  downloads: TtsDownloadInfo[];
  downloadingIds: Record<string, true>;
  verifyingIds: Record<string, true>;
  downloadProgress: Record<string, DownloadProgress>;
  loading: boolean;
  error: string | null;
  initialized: boolean;

  initialize: () => Promise<void>;
  loadDownloads: () => Promise<void>;
  downloadModel: (id: string) => Promise<boolean>;
  cancelDownload: (id: string) => Promise<boolean>;
  deleteModel: (id: string) => Promise<boolean>;
}

export const useTtsModelStore = create<TtsModelsStore>()(
  subscribeWithSelector((set, get) => ({
    downloads: [],
    downloadingIds: {},
    verifyingIds: {},
    downloadProgress: {},
    loading: true,
    error: null,
    initialized: false,

    loadDownloads: async () => {
      try {
        const result = await commands.ttsListDownloads();
        if (result.status === "ok") {
          set({ downloads: result.data, error: null });
        } else {
          set({ error: `Failed to load Piper downloads: ${result.error}` });
        }
      } catch (err) {
        set({ error: `Failed to load Piper downloads: ${err}` });
      } finally {
        set({ loading: false });
      }
    },

    downloadModel: async (id: string) => {
      set({ error: null });
      set(
        produce((state: TtsModelsStore) => {
          state.downloadingIds[id] = true;
          state.downloadProgress[id] = {
            model_id: id,
            downloaded: 0,
            total: 0,
            percentage: 0,
          };
        }),
      );
      try {
        const result = await commands.ttsDownloadModel(id);
        if (result.status !== "ok") {
          set(
            produce((state: TtsModelsStore) => {
              delete state.downloadingIds[id];
              delete state.downloadProgress[id];
            }),
          );
        }
        return result.status === "ok";
      } catch {
        set(
          produce((state: TtsModelsStore) => {
            delete state.downloadingIds[id];
            delete state.downloadProgress[id];
          }),
        );
        return false;
      }
    },

    cancelDownload: async (id: string) => {
      try {
        const result = await commands.ttsCancelDownload(id);
        set(
          produce((state: TtsModelsStore) => {
            delete state.downloadingIds[id];
            delete state.downloadProgress[id];
          }),
        );
        await get().loadDownloads();
        return result.status === "ok";
      } catch (err) {
        set({ error: `Failed to cancel Piper download: ${err}` });
        return false;
      }
    },

    deleteModel: async (id: string) => {
      try {
        const result = await commands.ttsDeleteModel(id);
        if (result.status === "ok") {
          await get().loadDownloads();
          return true;
        }
        set({ error: `Failed to delete Piper download: ${result.error}` });
        return false;
      } catch (err) {
        set({ error: `Failed to delete Piper download: ${err}` });
        return false;
      }
    },

    initialize: async () => {
      if (get().initialized) return;
      await get().loadDownloads();

      listen<DownloadProgress>("model-download-progress", (event) => {
        const progress = event.payload;
        if (!(progress.model_id in get().downloadingIds)) return;
        set(
          produce((state: TtsModelsStore) => {
            state.downloadProgress[progress.model_id] = progress;
          }),
        );
      });

      listen<string>("model-verification-started", (event) => {
        const id = event.payload;
        if (!(id in get().downloadingIds)) return;
        set(
          produce((state: TtsModelsStore) => {
            state.verifyingIds[id] = true;
          }),
        );
      });

      listen<string>("model-verification-completed", (event) => {
        const id = event.payload;
        set(
          produce((state: TtsModelsStore) => {
            delete state.verifyingIds[id];
          }),
        );
      });

      listen<string>("model-download-complete", (event) => {
        const id = event.payload;
        set(
          produce((state: TtsModelsStore) => {
            delete state.downloadingIds[id];
            delete state.verifyingIds[id];
            delete state.downloadProgress[id];
          }),
        );
        get().loadDownloads();
      });

      listen<{ model_id: string; error: string }>(
        "model-download-failed",
        (event) => {
          const { model_id: id, error } = event.payload;
          if (!(id in get().downloadingIds)) return;
          set(
            produce((state: TtsModelsStore) => {
              delete state.downloadingIds[id];
              delete state.verifyingIds[id];
              delete state.downloadProgress[id];
              state.error = error;
            }),
          );
        },
      );

      listen<string>("model-deleted", () => {
        get().loadDownloads();
      });

      set({ initialized: true });
    },
  })),
);
