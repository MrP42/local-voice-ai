import { commands, events } from "@/bindings";
import type { BookTalkPort } from "./port";

/** Die Tauri-Umsetzung des Kern-Zugangs; Fehler des Kerns werden zu `Error(code)`. */
export const tauriBookTalkPort: BookTalkPort = {
  async ask(req) {
    const result = await commands.bookTalkAsk(req);
    if (result.status === "error") throw new Error(result.error);
    return result.data;
  },
  cancel: (requestId) => commands.bookTalkCancel(requestId),
  async history(bookId) {
    const result = await commands.bookTalkHistory(bookId);
    return result.status === "ok" ? result.data : [];
  },
  async clear(bookId) {
    const result = await commands.bookTalkClear(bookId);
    if (result.status === "error") throw new Error(result.error);
  },
  async command(bookId, text) {
    const result = await commands.bookTalkCommand(bookId, text);
    return result.status === "ok" ? result.data : null;
  },
  async subscribe(handler) {
    return events.bookTalkEvent.listen((e) => handler(e.payload));
  },
};
