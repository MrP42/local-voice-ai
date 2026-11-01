import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  commands,
  type KontoAenderung,
  type KontoProfilDto,
  type KontoStatus,
} from "@/bindings";
import type {
  Anmeldestatus,
  KontoFeld,
  KontoProfil,
  ProfilAenderung,
} from "./profil";

/** Ereignis des Kerns, sobald sich der Kontospeicher geändert hat (auch durch andere Apps). */
export const KONTO_EVENT = "konto-geaendert";

/** Gleiches Fenster: nach eigenen Änderungen laden alle Nutzer des Hooks sofort neu (ohne auf den Kern zu warten). */
const LOKAL_EVENT = "lv-konto-geaendert";

/** Ohne Kern (Vorschau, Tests) bleibt die Oberfläche mit diesem leeren lokalen Profil benutzbar. */
const ERSATZ_PROFIL: KontoProfil = { id: "", anzeigename: "", farbe: 15 };

function pngBlob(base64: string): Blob | undefined {
  try {
    const roh = atob(base64);
    const bytes = new Uint8Array(roh.length);
    for (let i = 0; i < roh.length; i++) bytes[i] = roh.charCodeAt(i);
    return new Blob([bytes], { type: "image/png" });
  } catch {
    return undefined;
  }
}

function zuProfil(dto: KontoProfilDto, bild?: Blob): KontoProfil {
  const opt = (v: string | null) => v ?? undefined;
  return {
    id: dto.id,
    anzeigename: dto.anzeigename,
    vorname: opt(dto.vorname),
    nachname: opt(dto.nachname),
    email: opt(dto.email),
    telefon: opt(dto.telefon),
    mobil: opt(dto.mobil),
    position: opt(dto.position),
    abteilung: opt(dto.abteilung),
    organisation: opt(dto.organisation),
    standort: opt(dto.standort),
    farbe: dto.farbe,
    gesperrt: dto.gesperrt,
    bild,
    anmeldung: dto.anmeldung
      ? {
          anbieter: dto.anmeldung.issuer,
          seitMs: dto.anmeldung.seitMs,
          idpFelder: dto.anmeldung.idpFelder as KontoFeld[],
        }
      : undefined,
  };
}

function alsAenderung(a: ProfilAenderung): KontoAenderung {
  return {
    anzeigename: a.anzeigename ?? null,
    vorname: a.vorname ?? null,
    nachname: a.nachname ?? null,
    email: a.email ?? null,
    telefon: a.telefon ?? null,
    mobil: a.mobil ?? null,
    position: a.position ?? null,
    abteilung: a.abteilung ?? null,
    organisation: a.organisation ?? null,
    standort: a.standort ?? null,
    farbe: null,
  };
}

async function dateiZuBase64(datei: Blob): Promise<string> {
  const bytes = new Uint8Array(await datei.arrayBuffer());
  let roh = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    roh += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return btoa(roh);
}

export interface KontoAnsicht {
  /** Aktives Profil; ohne Kern ein leeres lokales Profil. */
  aktiv: KontoProfil;
  /** Weitere Profile auf diesem PC (ohne das aktive). */
  weitere: KontoProfil[];
  /** Paket 1: immer `lokal`, solange das Profil keine Anmeldung trägt. */
  status: Anmeldestatus;
  /** Der Kontospeicher wurde erreicht (sonst gilt das leere Ersatzprofil). */
  verfuegbar: boolean;
  aendern: (aenderung: ProfilAenderung) => Promise<void>;
  aktivieren: (id: string) => Promise<void>;
  /** Legt ein Profil an und macht es zum aktiven. */
  anlegen: (anzeigename: string) => Promise<void>;
  bildSetzen: (datei: Blob) => Promise<void>;
  bildEntfernen: () => Promise<void>;
}

/**
 * Das Suite-Profil dieses PCs (ADR-0024): lädt `konto_status`, hört auf `konto-geaendert`
 * (auch Änderungen anderer Apps) und lädt das Profilbild als Blob.
 */
export function useKonto(): KontoAnsicht {
  const [aktiv, setAktiv] = useState<KontoProfil>(ERSATZ_PROFIL);
  const [weitere, setWeitere] = useState<KontoProfil[]>([]);
  const [verfuegbar, setVerfuegbar] = useState(false);
  // Gleiches Bild nicht erneut als neuen Blob ausgeben (kein Flackern bei jedem Ereignis).
  const bilder = useRef(new Map<string, { b64: string; blob?: Blob }>());
  const lauf = useRef(0);

  const laden = useCallback(async () => {
    const mein = ++lauf.current;
    try {
      const antwort = await commands.kontoStatus();
      const status: KontoStatus | null =
        antwort?.status === "ok" ? (antwort.data ?? null) : null;
      if (!status?.aktiv) {
        if (mein === lauf.current) setVerfuegbar(false);
        return;
      }
      const alle = [status.aktiv, ...(status.weitere ?? [])];
      const profile = await Promise.all(
        alle.map(async (dto) => {
          if (!dto.hatBild) {
            bilder.current.delete(dto.id);
            return zuProfil(dto);
          }
          const res = await commands.kontoBildLesen(dto.id);
          const b64 = res?.status === "ok" ? res.data : null;
          if (!b64) return zuProfil(dto);
          const alt = bilder.current.get(dto.id);
          const blob = alt?.b64 === b64 ? alt.blob : pngBlob(b64);
          bilder.current.set(dto.id, { b64, blob });
          return zuProfil(dto, blob);
        }),
      );
      if (mein !== lauf.current) return;
      const ids = new Set(alle.map((dto) => dto.id));
      for (const id of bilder.current.keys())
        if (!ids.has(id)) bilder.current.delete(id);
      setAktiv(profile[0]);
      setWeitere(profile.slice(1));
      setVerfuegbar(true);
    } catch {
      if (mein === lauf.current) setVerfuegbar(false);
    }
  }, []);

  useEffect(() => {
    void laden();
    let weg = false;
    let ab: (() => void) | undefined;
    listen(KONTO_EVENT, () => void laden())
      .then((fn) => {
        if (weg) fn();
        else ab = fn;
      })
      .catch(() => {});
    // Das Ereignis ist nur ein Hinweis: beim Zurückkehren ins Fenster neu lesen.
    const onFokus = () => void laden();
    window.addEventListener("focus", onFokus);
    window.addEventListener(LOKAL_EVENT, onFokus);
    return () => {
      weg = true;
      ab?.();
      window.removeEventListener("focus", onFokus);
      window.removeEventListener(LOKAL_EVENT, onFokus);
    };
  }, [laden]);

  const aktualisiert = useCallback(() => {
    window.dispatchEvent(new Event(LOKAL_EVENT));
    return laden();
  }, [laden]);

  const pruefen = (
    res: { status: string; error?: unknown } | null | undefined,
  ) => {
    if (res?.status === "error") throw new Error(String(res.error));
  };

  const aendern = useCallback(
    async (aenderung: ProfilAenderung) => {
      pruefen(await commands.kontoAendern(aktiv.id, alsAenderung(aenderung)));
      await aktualisiert();
    },
    [aktiv.id, aktualisiert],
  );

  const aktivieren = useCallback(
    async (id: string) => {
      pruefen(await commands.kontoAktivieren(id));
      await aktualisiert();
    },
    [aktualisiert],
  );

  const anlegen = useCallback(
    async (anzeigename: string) => {
      const neu = await commands.kontoAnlegen(anzeigename);
      pruefen(neu);
      if (neu?.status === "ok" && neu.data) {
        pruefen(await commands.kontoAktivieren(neu.data.id));
      }
      await aktualisiert();
    },
    [aktualisiert],
  );

  const bildSetzen = useCallback(
    async (datei: Blob) => {
      pruefen(
        await commands.kontoBildSetzen(aktiv.id, await dateiZuBase64(datei)),
      );
      await aktualisiert();
    },
    [aktiv.id, aktualisiert],
  );

  const bildEntfernen = useCallback(async () => {
    pruefen(await commands.kontoBildEntfernen(aktiv.id));
    await aktualisiert();
  }, [aktiv.id, aktualisiert]);

  return {
    aktiv,
    weitere,
    status: aktiv.anmeldung ? "angemeldet" : "lokal",
    verfuegbar,
    aendern,
    aktivieren,
    anlegen,
    bildSetzen,
    bildEntfernen,
  };
}
