import { commands, type Meeting } from "@/bindings";

/** Sucht eine Besprechung seitenweise (es gibt keinen Einzelabruf). */
export const findMeeting = async (id: string): Promise<Meeting | null> => {
  const PAGE = 200;
  for (let offset = 0; offset < 50 * PAGE; offset += PAGE) {
    const result = await commands.meetingsList(offset, PAGE);
    if (result.status !== "ok") return null;
    const page = result.data ?? [];
    const hit = page.find((m) => m.id === id);
    if (hit) return hit;
    if (page.length < PAGE) return null;
  }
  return null;
};
