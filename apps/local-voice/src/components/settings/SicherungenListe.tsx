import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { ArchiveRestore, FolderOpen, Settings2, Trash2 } from "lucide-react";
import { commands, type Sicherung } from "@/bindings";
import { formatBytes } from "@/lib/library/remove";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";

const ANLAESSE = [
  "anmeldung",
  "abmeldung",
  "kollision",
  "freigabe",
  "schutz",
  "vor_wiederherstellen",
] as const;

interface Props {
  /** Ändert sich, wenn eine neue Sicherung entstanden sein kann. */
  stand: unknown;
}

/**
 * Sicherungen aus Anmeldung, Abmeldung und Übernahme (Spec 2026-11-08, U8).
 * Wiederherstellen überschreibt nie: Fehlendes kommt zurück, Abweichendes
 * wird zusätzlicher Eintrag oder bleibt in der Sicherung. Alles
 * Wiederhergestellte bleibt nur auf diesem Gerät.
 */
export const SicherungenListe: React.FC<Props> = ({ stand }) => {
  const { t, i18n } = useTranslation();
  const [liste, setListe] = useState<Sicherung[]>([]);
  const [laeuft, setLaeuft] = useState<string | null>(null);
  const [loeschen, setLoeschen] = useState<Sicherung | null>(null);

  const laden = useCallback(async () => {
    const r = await commands.syncSicherungen();
    if (r?.status === "ok") setListe(r.data);
  }, []);

  useEffect(() => {
    void laden();
  }, [laden, stand]);

  if (liste.length === 0) return null;

  const datum = (ms: number) =>
    new Date(ms).toLocaleString(i18n.language, {
      dateStyle: "medium",
      timeStyle: "short",
    });
  const anlass = (a: string) =>
    (ANLAESSE as readonly string[]).includes(a)
      ? t(`sync.sicherungen.anlass.${a}`)
      : a;

  const wiederherstellen = async (s: Sicherung) => {
    setLaeuft(s.name);
    const r = await commands.syncSicherungWiederherstellen(s.name);
    setLaeuft(null);
    if (r.status === "error") {
      toast.error(t("sync.sicherungen.fehler"), { description: r.error });
      return;
    }
    const b = r.data;
    const teile = [
      t("sync.sicherungen.zurueck", { count: b.zurueck }),
      b.zusaetzlich > 0 &&
        t("sync.sicherungen.zusaetzlich", { count: b.zusaetzlich }),
      b.gleich > 0 && t("sync.sicherungen.gleich", { count: b.gleich }),
      b.fortschritte_ergaenzt > 0 &&
        t("sync.sicherungen.fortschritte", { count: b.fortschritte_ergaenzt }),
      b.fortschritte_abweichend > 0 &&
        t("sync.sicherungen.fortschritteAbweichend", {
          count: b.fortschritte_abweichend,
        }),
      b.in_sicherung.length > 0 &&
        t("sync.sicherungen.inSicherung", {
          count: b.in_sicherung.length,
          liste: b.in_sicherung.join(", "),
        }),
    ].filter(Boolean);
    toast.success(t("sync.sicherungen.wiederhergestellt"), {
      description: `${teile.join(" · ")}. ${t("sync.sicherungen.nurLokal")}`,
      duration: 12000,
      action:
        b.in_sicherung.length > 0
          ? {
              label: t("sync.sicherungen.ordner"),
              onClick: () => void revealItemInDir(b.ordner).catch(() => {}),
            }
          : undefined,
    });
  };

  const einstellungen = async (s: Sicherung) => {
    setLaeuft(s.name);
    const r = await commands.syncSicherungEinstellungen(s.name);
    setLaeuft(null);
    if (r.status === "error") {
      toast.error(t("sync.sicherungen.fehler"), { description: r.error });
      return;
    }
    toast.success(t("sync.sicherungen.einstellungenZurueck"));
    void laden();
  };

  const loeschenBestaetigt = async () => {
    const s = loeschen;
    if (!s) return;
    setLaeuft(s.name);
    const r = await commands.syncSicherungLoeschen(s.name);
    setLaeuft(null);
    setLoeschen(null);
    if (r.status === "error") {
      toast.error(t("sync.sicherungen.fehler"), { description: r.error });
      return;
    }
    void laden();
  };

  return (
    <div className="px-4 py-3 space-y-2 text-sm" data-testid="sync-sicherungen">
      <div>
        <p className="font-medium">{t("sync.sicherungen.titel")}</p>
        <p className="text-text-muted">{t("sync.sicherungen.hinweis")}</p>
      </div>
      <ul className="divide-y divide-mid-gray/15 rounded-md border border-mid-gray/20">
        {liste.map((s) => (
          <li
            key={s.name}
            className="flex flex-wrap items-center gap-x-3 gap-y-2 px-3 py-2"
          >
            <div className="min-w-0 flex-1">
              <p className="truncate">
                {datum(s.erstellt_ms)} · {anlass(s.anlass)}
              </p>
              <p className="truncate text-text-muted">
                {[
                  s.konto,
                  t("sync.anmeldung.seiten", { count: s.seiten }),
                  t("sync.anmeldung.buecher", { count: s.buecher }),
                  formatBytes(s.bytes, i18n.language),
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-1">
              <Button
                variant="secondary"
                size="sm"
                className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
                onClick={() => void wiederherstellen(s)}
                disabled={!!laeuft}
              >
                <ArchiveRestore width={14} height={14} aria-hidden="true" />
                {t("sync.sicherungen.wiederherstellen")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
                onClick={() => void einstellungen(s)}
                disabled={!!laeuft}
                title={t("sync.sicherungen.einstellungenHinweis")}
              >
                <Settings2 width={14} height={14} aria-hidden="true" />
                {t("sync.sicherungen.einstellungen")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
                onClick={() => void revealItemInDir(s.ordner).catch(() => {})}
                aria-label={t("sync.sicherungen.ordner")}
                title={t("sync.sicherungen.ordner")}
              >
                <FolderOpen width={14} height={14} aria-hidden="true" />
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
                onClick={() => setLoeschen(s)}
                disabled={!!laeuft}
                aria-label={t("sync.sicherungen.loeschen")}
                title={t("sync.sicherungen.loeschen")}
              >
                <Trash2 width={14} height={14} aria-hidden="true" />
              </Button>
            </div>
          </li>
        ))}
      </ul>
      <Dialog
        open={!!loeschen}
        onOpenChange={(o) => !o && setLoeschen(null)}
        title={t("sync.sicherungen.loeschenTitel")}
        description={
          loeschen
            ? t("sync.sicherungen.loeschenText", {
                datum: datum(loeschen.erstellt_ms),
              })
            : undefined
        }
        closeLabel={t("sync.abmelden.abbrechen")}
        className="max-w-md"
        footer={
          <>
            <Button
              variant="danger"
              onClick={() => void loeschenBestaetigt()}
              disabled={!!laeuft}
            >
              {t("sync.sicherungen.loeschen")}
            </Button>
            <Button variant="secondary" onClick={() => setLoeschen(null)}>
              {t("sync.abmelden.abbrechen")}
            </Button>
          </>
        }
      >
        {null}
      </Dialog>
    </div>
  );
};
