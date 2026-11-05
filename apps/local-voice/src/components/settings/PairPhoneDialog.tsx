import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  AlertTriangle,
  CheckCircle2,
  Clock,
  Lock,
  Smartphone,
} from "lucide-react";
import { commands, type PairView } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";

/** Touchziele ≥ 44 px auf Mobilbreite und bei Touch-Bedienung (Codex-Abnahme #292). */
const TOUCH = "max-sm:min-h-[44px] pointer-coarse:min-h-[44px]";

/** Abfragetakt; der Hub drosselt offene Abfragen auf 120/min je Netz. */
const POLL_MS = 1500;
const FINAL = new Set(["connected", "unconfirmed", "expired", "aborted"]);

type Local = { stage: "starting" } | { stage: "start_failed"; reason: string };
type Shown = PairView | Local;

const isView = (s: Shown): s is PairView => "remaining_s" in s;

const mmss = (s: number) =>
  `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;

/** SVG des Kerns als Bild: kein HTML aus dem Backend im DOM. */
const svgSrc = (svg: string) =>
  `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Geräteliste neu laden (nach Verbinden oder Abmelden). */
  onChanged: () => void;
}

/**
 * „Handy verbinden“ (local-voice-ai#289, Spec 2026-11-05 Abschnitt 8): QR anzeigen,
 * Vergleichscode bestätigen, Bestätigung des iPhones abwarten. Der Ablauf und alle
 * Prüfungen laufen im Rust-Kern (`sync/pairing.rs`); hier nur Anzeige und Takt.
 */
export const PairPhoneDialog: React.FC<Props> = ({
  open,
  onOpenChange,
  onChanged,
}) => {
  const { t } = useTranslation();
  const [shown, setShown] = useState<Shown>({ stage: "starting" });
  const [deadline, setDeadline] = useState(0);
  const [now, setNow] = useState(Date.now());
  const [busy, setBusy] = useState(false);
  const yesRef = useRef<HTMLButtonElement>(null);
  const alive = useRef(false);
  const [confirmRevoke, setConfirmRevoke] = useState(false);
  const [chainNames, setChainNames] = useState<string[]>([]);

  // Rückruf über eine Referenz: ein neuer `onChanged` je Darstellung darf den
  // Start-Effekt nicht erneut auslösen (sonst entstünde jedes Mal ein neuer Code).
  const onChangedRef = useRef(onChanged);
  onChangedRef.current = onChanged;

  const apply = useCallback((v: PairView) => {
    if (!alive.current) return;
    setShown(v);
    if (v.remaining_s > 0) setDeadline(Date.now() + v.remaining_s * 1000);
    if (v.stage === "connected") onChangedRef.current();
  }, []);

  const start = useCallback(async () => {
    setShown({ stage: "starting" });
    setConfirmRevoke(false);
    const r = await commands.pairStart();
    if (!alive.current) {
      // Während des Starts geschlossen: die eben angelegte Kopplung gleich verwerfen.
      if (r.status === "ok") void commands.pairCancel();
      return;
    }
    if (r.status === "ok") apply(r.data);
    else setShown({ stage: "start_failed", reason: r.error });
  }, [apply]);

  // Stufe für das Aufräumen beim Schließen oder Aushängen (z. B. Abmeldung,
  // Seitenwechsel): eine laufende Kopplung darf nicht im Kern liegen bleiben.
  const stageRef = useRef<string>("starting");
  stageRef.current = shown.stage;

  useEffect(() => {
    alive.current = open;
    if (open) void start();
    return () => {
      alive.current = false;
      if (
        open &&
        !FINAL.has(stageRef.current) &&
        stageRef.current !== "start_failed"
      ) {
        void commands.pairCancel();
      }
    };
  }, [open, start]);

  // Takt: Automat im Kern weiterführen, bis ein Endzustand erreicht ist. Eine
  // Abfrage nach der anderen (setTimeout-Kette, `tick` zählt abgeschlossene).
  // Beim Vergleichscode ruht der Takt, damit keine späte Antwort die Freigabe
  // überholt; dort zählt nur der Ablauf der Frist.
  const stage = shown.stage;
  const [tick, setTick] = useState(0);
  const polling =
    open && isView(shown) && !FINAL.has(stage) && stage !== "sas" && !busy;
  useEffect(() => {
    if (!polling) return;
    const id = window.setTimeout(async () => {
      const r = await commands.pairPoll();
      if (r.status === "ok") apply(r.data);
      setTick((n) => n + 1);
    }, POLL_MS);
    return () => window.clearTimeout(id);
  }, [polling, tick, apply]);

  useEffect(() => {
    if (!open) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [open]);

  const remaining = Math.max(0, Math.ceil((deadline - now) / 1000));

  // Frist beim Vergleichscode abgelaufen: Kern meldet „abgelaufen“ und verwirft.
  useEffect(() => {
    if (stage !== "sas" || remaining > 0 || busy) return;
    void commands.pairPoll().then((r) => r.status === "ok" && apply(r.data));
  }, [stage, remaining, busy, apply]);

  /** Schließen bricht eine laufende Kopplung ab. Ist sie schon abgeholt, zeigt der
   *  Kern „nicht bestätigt“ mit dem Angebot, das Gerät abzumelden. */
  const close = async () => {
    if (isView(shown) && !FINAL.has(shown.stage)) {
      const r = await commands.pairCancel();
      if (r.status === "ok" && r.data.stage === "unconfirmed") {
        apply(r.data);
        return;
      }
    }
    onOpenChange(false);
  };

  const approve = async () => {
    setBusy(true);
    const r = await commands.pairApprove();
    setBusy(false);
    if (r.status === "ok") apply(r.data);
    else setShown({ stage: "start_failed", reason: r.error });
  };

  const reject = async () => {
    setBusy(true);
    const r = await commands.pairCancel();
    setBusy(false);
    if (r.status === "ok" && r.data.stage === "unconfirmed") apply(r.data);
    else onOpenChange(false);
  };

  // Vor dem Abmelden nennen, welche Geräte mit abgemeldet werden (Spec 5a Regel 10).
  const askRevoke = async () => {
    if (!isView(shown) || !shown.suspect) return;
    const ids = shown.suspect.revokes_with ?? [];
    const r = ids.length > 0 ? await commands.syncDevices() : null;
    const byId = new Map(
      (r?.status === "ok" ? r.data : []).map((d) => [d.id, d.name]),
    );
    setChainNames(ids.map((id) => byId.get(id) ?? `#${id}`));
    setConfirmRevoke(true);
  };

  const revokeSuspect = async () => {
    if (!isView(shown) || !shown.suspect) return;
    setBusy(true);
    const r = await commands.syncRevokeDevice(shown.suspect.id);
    setBusy(false);
    if (r.status === "ok") {
      toast.success(t("pairing.revoked"));
      onChanged();
      onOpenChange(false);
    } else {
      toast.error(t("pairing.revokeFailed"), { description: r.error });
    }
  };

  useEffect(() => {
    if (stage === "sas") yesRef.current?.focus();
  }, [stage]);

  const name =
    (isView(shown) && (shown.suspect?.name || shown.device_name)) ||
    t("pairing.unknownDevice");

  const title =
    stage === "sas"
      ? t("pairing.confirmTitle")
      : stage === "connected"
        ? t("pairing.connectedTitle")
        : stage === "aborted"
          ? t("pairing.abortedTitle")
          : stage === "unconfirmed"
            ? t("pairing.unconfirmedTitle")
            : t("pairing.title");

  const newCode = (
    <Button className={TOUCH} variant="primary" onClick={() => void start()}>
      {t("pairing.newCode")}
    </Button>
  );
  const closeBtn = (label = t("pairing.close")) => (
    <Button className={TOUCH} variant="secondary" onClick={() => void close()}>
      {label}
    </Button>
  );

  let body: React.ReactNode = null;
  let footer: React.ReactNode = null;

  if (stage === "starting") {
    body = (
      <p className="text-sm text-text/70" role="status">
        …
      </p>
    );
    footer = closeBtn(t("pairing.cancel"));
  } else if (stage === "start_failed" && !isView(shown)) {
    body = (
      <p className="flex gap-2 text-sm" role="alert">
        <AlertTriangle
          className="shrink-0 text-orange-500"
          width={18}
          height={18}
          aria-hidden
        />
        <span>
          {t("pairing.startFailed")}
          <br />
          <span className="text-text/60">{shown.reason}</span>
        </span>
      </p>
    );
    footer = (
      <>
        {closeBtn()}
        <Button
          className={TOUCH}
          variant="primary"
          onClick={() => void start()}
        >
          {t("pairing.retry")}
        </Button>
      </>
    );
  } else if (isView(shown) && (stage === "qr" || stage === "network")) {
    body = (
      <div
        className="flex flex-col sm:flex-row gap-5 items-center sm:items-start"
        data-testid="pair-qr"
      >
        {shown.qr_svg && (
          <img
            src={svgSrc(shown.qr_svg)}
            alt={t("pairing.qrAlt")}
            width={240}
            height={240}
            className="bg-white p-2 rounded-md shrink-0"
            style={{ imageRendering: "pixelated" }}
          />
        )}
        <div className="space-y-3 text-sm">
          <p className="flex gap-2 text-text/80">
            <Lock
              className="shrink-0 mt-0.5"
              width={16}
              height={16}
              aria-hidden
            />
            {t("pairing.securityNote")}
          </p>
          <ol className="list-decimal pl-5 space-y-1">
            <li>{t("pairing.step1")}</li>
            <li>{t("pairing.step2")}</li>
            <li>{t("pairing.step3")}</li>
          </ol>
          <p className="flex items-center gap-2" aria-live="off">
            <Clock width={16} height={16} aria-hidden />
            {t("pairing.validFor")}{" "}
            <strong className="tabular-nums">{mmss(remaining)}</strong>
          </p>
          {shown.device_name && (
            <p role="status" className="text-text/80">
              {t("pairing.detected", { name: shown.device_name })}
            </p>
          )}
          {stage === "network" && (
            <p role="status" className="text-orange-500">
              {t("pairing.networkHint")}
            </p>
          )}
        </div>
      </div>
    );
    footer = closeBtn(t("pairing.cancel"));
  } else if (isView(shown) && stage === "sas") {
    const sas = shown.sas ?? "";
    body = (
      <div className="space-y-4 text-sm" data-testid="pair-sas">
        <div className="flex gap-3 items-start">
          <Smartphone className="shrink-0" width={28} height={28} aria-hidden />
          <div>
            <p>
              <strong>{shown.device_name}</strong> {t("pairing.wantsToConnect")}
            </p>
            <p className="text-text/60">{t("pairing.nameByDevice")}</p>
          </div>
        </div>
        <p className="flex gap-2 text-text/80">
          <AlertTriangle
            className="shrink-0 mt-0.5 text-orange-500"
            width={16}
            height={16}
            aria-hidden
          />
          {t("pairing.sasWarning")}
        </p>
        <p
          className="text-4xl font-semibold tracking-widest tabular-nums text-center"
          aria-label={`${t("pairing.sasLabel")}: ${sas.split("").join(" ")}`}
        >
          {sas.slice(0, 3)} {sas.slice(3)}
        </p>
        <p className="text-center font-medium">{t("pairing.sasQuestion")}</p>
      </div>
    );
    footer = (
      <>
        <Button
          className={TOUCH}
          variant="secondary"
          onClick={() => void reject()}
          disabled={busy}
        >
          {t("pairing.no")}
        </Button>
        <Button
          className={TOUCH}
          ref={yesRef}
          variant="primary"
          onClick={() => void approve()}
          disabled={busy}
        >
          {t("pairing.yes")}
        </Button>
      </>
    );
  } else if (stage === "wait_confirm") {
    body = (
      <p className="text-sm" role="status">
        {t("pairing.waitConfirm")}
      </p>
    );
    footer = closeBtn(t("pairing.cancel"));
  } else if (stage === "connected") {
    body = (
      <div className="space-y-2 text-sm" role="status">
        <p className="flex gap-2 items-center">
          <CheckCircle2
            className="text-green-600"
            width={20}
            height={20}
            aria-hidden
          />
          {t("pairing.connected", { name })}
        </p>
        <p className="text-text/60">{t("pairing.connectedHint")}</p>
      </div>
    );
    footer = (
      <Button
        className={TOUCH}
        variant="primary"
        onClick={() => onOpenChange(false)}
      >
        {t("pairing.done")}
      </Button>
    );
  } else if (stage === "expired") {
    body = (
      <p className="flex gap-2 text-sm" role="status">
        <Clock className="shrink-0" width={18} height={18} aria-hidden />
        {t("pairing.expired")}
      </p>
    );
    footer = (
      <>
        {closeBtn()}
        {newCode}
      </>
    );
  } else if (isView(shown) && stage === "aborted") {
    body = (
      <div className="space-y-2 text-sm" role="alert">
        <p className="flex gap-2">
          <AlertTriangle
            className="shrink-0 text-orange-500"
            width={18}
            height={18}
            aria-hidden
          />
          {shown.reason}
        </p>
        <p className="text-text/60">{t("pairing.abortedHint")}</p>
      </div>
    );
    footer = (
      <>
        {closeBtn()}
        {newCode}
      </>
    );
  } else if (isView(shown) && stage === "unconfirmed") {
    body = (
      <div className="space-y-2 text-sm" role="alert">
        <p className="flex gap-2">
          <AlertTriangle
            className="shrink-0 text-orange-500"
            width={18}
            height={18}
            aria-hidden
          />
          {shown.suspect ? t("pairing.unconfirmed", { name }) : shown.reason}
        </p>
        <p className="text-text/60">{t("pairing.unconfirmedHint")}</p>
        {confirmRevoke && shown.suspect && (
          <div className="rounded-md border border-orange-500/40 p-3 space-y-1">
            <p className="font-medium">
              {t("pairing.revokeTitle", { name: shown.suspect.name })}
            </p>
            {chainNames.length > 0 && (
              <>
                <p>{t("pairing.revokeAlso")}</p>
                <ul className="list-disc pl-5">
                  {chainNames.map((n) => (
                    <li key={n} className="font-semibold text-text">
                      {n}
                    </li>
                  ))}
                </ul>
              </>
            )}
            <p>{t("pairing.revokeText")}</p>
          </div>
        )}
      </div>
    );
    footer = (
      <>
        <Button
          className={TOUCH}
          variant="secondary"
          onClick={() => onOpenChange(false)}
        >
          {t("pairing.keep")}
        </Button>
        {shown.suspect && (
          <Button
            className={TOUCH}
            variant="danger"
            onClick={() => void (confirmRevoke ? revokeSuspect() : askRevoke())}
            disabled={busy}
            data-testid="pair-revoke-suspect"
          >
            {confirmRevoke ? t("pairing.logout") : t("pairing.revokeDevice")}
          </Button>
        )}
      </>
    );
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => (o ? onOpenChange(true) : void close())}
      title={title}
      closeLabel={t("pairing.close")}
      className="max-w-2xl"
      contentFades={false}
      closeOnBackdrop={false}
      footer={footer}
    >
      <div data-testid="pair-dialog" data-stage={stage}>
        {body}
      </div>
    </Dialog>
  );
};
