import { commands } from "@/bindings";
import { toast } from "sonner";
import i18n from "@/i18n";
import { useSettingsStore } from "@/stores/settingsStore";

/** Startup uses the same error text as editing a shortcut in settings. */
export async function initializeShortcutsWithNotice() {
  const result = await commands.initializeShortcuts();
  if (result.status === "error") {
    toast.error(
      i18n.t("settings.general.shortcut.errors.set", { error: result.error }),
    );
    // A migrated custom binding may have returned to its previous backend.
    await useSettingsStore.getState().refreshSettings();
  }
}
