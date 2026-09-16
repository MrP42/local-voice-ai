import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, type TtsDownloadInfo } from "@/bindings";
import { useSettings } from "./useSettings";
import { selectablePiperVoices } from "@/lib/tts/availability";

const empty = {
  systemVoices: [] as { name: string; language: string }[],
  systemDefaultVoice: null as string | null,
  fishInstalled: false,
  fishSupported: false,
  piperInstalled: false,
  piperVoices: [] as TtsDownloadInfo[],
  loading: true,
};

/** Rendering a voice selector must never start a server or download a module. */
export function useTtsAvailability() {
  const [state, setState] = useState(empty);
  const { getSetting } = useSettings();
  const fishPath = getSetting("tts_fish_dir");
  useEffect(() => {
    let disposed = false;
    let generation = 0;
    const refresh = async () => {
      const request = ++generation;
      try {
        const [modules, downloads] = await Promise.all([
          commands.ttsModuleAvailability(),
          commands.ttsListDownloads(),
        ]);
        if (disposed || generation !== request) return;
        const rows = downloads.status === "ok" ? (downloads.data ?? []) : [];
        setState({
          systemVoices: modules?.system_voices ?? [],
          systemDefaultVoice: modules?.system_default_voice ?? null,
          fishInstalled: modules?.fish_installed === true,
          fishSupported: modules?.fish_supported === true,
          piperInstalled: rows.some(
            (row) => row.id === "piper-runtime" && row.is_downloaded,
          ),
          piperVoices: selectablePiperVoices(rows),
          loading: false,
        });
      } catch {
        if (!disposed && generation === request)
          setState({ ...empty, loading: false });
      }
    };
    const events = [
      "model-download-complete",
      "model-deleted",
      "model-download-failed",
    ];
    const subscriptions = events.map((event) =>
      listen(event, () => void refresh()),
    );
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    void refresh();
    return () => {
      disposed = true;
      window.removeEventListener("focus", onFocus);
      subscriptions.forEach((subscription) => {
        void subscription.then((unlisten) => unlisten()).catch(() => {});
      });
    };
  }, [fishPath]);
  return state;
}
