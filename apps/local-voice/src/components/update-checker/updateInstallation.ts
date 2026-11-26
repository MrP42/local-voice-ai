import { listen } from "@tauri-apps/api/event";
import { commands } from "../../bindings";
import { waitForUpdateGate } from "./updatePolicy";

/** Listener zuerst, dann atomar versuchen: auch ein fruehes Frei-Event geht nicht verloren. */
export async function beginUpdateInstallation(
  version: string,
  signal: AbortSignal,
): Promise<void> {
  return waitForUpdateGate(
    async () => {
      const result = await commands.updateInstallBegin(version);
      if (result.status === "ok") return "ok";
      if (result.error.code !== "belegt") throw new Error(result.error.code);
      return "belegt";
    },
    (notify) =>
      listen<{ belegt: boolean }>("lv-update-gate", ({ payload }) => {
        if (!payload.belegt) notify();
      }),
    signal,
  );
}
