import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft, Search } from "lucide-react";
import { commands, type PersonDetail, type PersonSummary } from "@/bindings";
import { peopleErrorKey } from "@/lib/meetingPeople";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { Input } from "../../../ui/Input";
import { Select } from "../../../ui/Select";

interface PeopleDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Nach Umbenennen oder Zusammenfuehren: Anzeigen mit Personen neu laden. */
  onChanged?: () => void;
}

/**
 * "Personen verwalten" (M5-P5d): alle bekannten Personen, Umbenennen und
 * Adresse aendern, Zusammenfuehren zweier Eintraege derselben Person. Gleiche
 * E-Mail-Adressen und gleiche Namen erkennt das Backend selbst; hier steht nur
 * die manuelle Pflege.
 */
export const PeopleDialog: React.FC<PeopleDialogProps> = ({
  open,
  onOpenChange,
  onChanged,
}) => {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [people, setPeople] = useState<PersonSummary[]>([]);
  // Ziele fuers Zusammenfuehren: alle Personen, unabhaengig vom Suchtext.
  const [everyone, setEveryone] = useState<PersonSummary[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [detail, setDetail] = useState<PersonDetail | null>(null);
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [mergeTarget, setMergeTarget] = useState("");
  const [confirmMerge, setConfirmMerge] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<{
    kind: "ok" | "error";
    text: string;
  } | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  // Eine spaet eintreffende Liste einer ueberholten Suche verwerfen.
  const requestRef = useRef(0);

  const loadPeople = useCallback(async (q: string) => {
    const request = ++requestRef.current;
    const result = await commands.peopleList(q.trim() === "" ? null : q);
    if (request !== requestRef.current) return;
    if (result.status === "ok") setPeople(result.data ?? []);
    setLoaded(true);
  }, []);

  useEffect(() => {
    if (!open) return;
    const timer = setTimeout(() => void loadPeople(query), query ? 150 : 0);
    return () => clearTimeout(timer);
  }, [open, query, loadPeople]);

  // Beim Schliessen alles zuruecksetzen: naechstes Oeffnen beginnt mit der Liste.
  useEffect(() => {
    if (open) return;
    setQuery("");
    setDetail(null);
    setStatus(null);
    setMergeTarget("");
    setConfirmMerge(false);
    setLoaded(false);
  }, [open]);

  const select = async (id: string) => {
    setStatus(null);
    setMergeTarget("");
    setConfirmMerge(false);
    const [result, all] = await Promise.all([
      commands.peopleGet(id),
      commands.peopleList(null),
    ]);
    if (result.status !== "ok") {
      setStatus({ kind: "error", text: t(peopleErrorKey(result.error)) });
      void loadPeople(query);
      return;
    }
    if (all.status === "ok") setEveryone(all.data ?? []);
    setDetail(result.data);
    setName(result.data.name);
    setEmail(result.data.email ?? "");
  };

  const back = () => {
    setDetail(null);
    setStatus(null);
    setMergeTarget("");
    setConfirmMerge(false);
    void loadPeople(query);
  };

  const save = async () => {
    if (!detail || busy) return;
    setBusy(true);
    setStatus(null);
    const result = await commands.peopleUpdate(detail.id, name, email);
    setBusy(false);
    if (result.status === "error") {
      setStatus({ kind: "error", text: t(peopleErrorKey(result.error)) });
      return;
    }
    setStatus({ kind: "ok", text: t("meetings.people.dialog.saved") });
    onChanged?.();
    const fresh = await commands.peopleGet(detail.id);
    if (fresh.status === "ok") {
      setDetail(fresh.data);
      setName(fresh.data.name);
      setEmail(fresh.data.email ?? "");
    }
  };

  const merge = async () => {
    if (!detail || !mergeTarget || busy) return;
    setBusy(true);
    setStatus(null);
    // `detail` geht in der gewaehlten Person auf.
    const result = await commands.peopleMerge(mergeTarget, detail.id);
    setBusy(false);
    setConfirmMerge(false);
    if (result.status === "error") {
      setStatus({ kind: "error", text: t(peopleErrorKey(result.error)) });
      return;
    }
    onChanged?.();
    const target = mergeTarget;
    setMergeTarget("");
    await select(target);
    setStatus({ kind: "ok", text: t("meetings.people.dialog.merged") });
  };

  const others = everyone.filter((p) => p.id !== detail?.id);
  const targetName = others.find((p) => p.id === mergeTarget)?.name ?? "";

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("meetings.people.dialog.title")}
      description={t("meetings.people.dialog.description")}
      closeLabel={t("meetings.people.dialog.close")}
      initialFocusRef={searchRef}
      contentClassName="max-w-lg"
      footer={
        <Button variant="secondary" onClick={() => onOpenChange(false)}>
          {t("meetings.people.dialog.close")}
        </Button>
      }
    >
      <div className="space-y-3" data-testid="people-dialog">
        {detail === null ? (
          <>
            <div className="relative">
              <Search
                width={14}
                height={14}
                aria-hidden="true"
                className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-text/40"
              />
              <Input
                ref={searchRef}
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={t("meetings.people.dialog.search")}
                aria-label={t("meetings.people.dialog.searchLabel")}
                variant="compact"
                className="w-full pl-8"
                data-testid="people-search"
              />
            </div>
            {loaded && people.length === 0 ? (
              <p className="text-sm text-text/60" data-testid="people-empty">
                {query.trim() === ""
                  ? t("meetings.people.dialog.empty")
                  : t("meetings.people.dialog.noHits")}
              </p>
            ) : (
              <ul
                className="max-h-72 space-y-0.5 overflow-y-auto"
                data-testid="people-list"
              >
                {people.map((p) => (
                  <li key={p.id}>
                    <button
                      type="button"
                      onClick={() => void select(p.id)}
                      data-testid="people-row"
                      data-human-id={p.id}
                      className="flex w-full items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left text-sm hover:bg-mid-gray/15 cursor-pointer"
                    >
                      <span className="min-w-0">
                        <span className="block truncate font-medium">
                          {p.name}
                          {p.is_self && (
                            <span className="ms-1 font-normal text-text/50">
                              ({t("meetings.people.dialog.self")})
                            </span>
                          )}
                        </span>
                        <span className="block truncate text-xs text-text/60">
                          {[p.email, p.company && !p.email ? p.company : null]
                            .filter(Boolean)
                            .join(" · ")}
                        </span>
                      </span>
                      <span className="shrink-0 text-xs text-text/50">
                        {t("meetings.people.dialog.meetings", {
                          count: p.meeting_count,
                        })}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </>
        ) : (
          <div className="space-y-3" data-testid="person-detail">
            <button
              type="button"
              onClick={back}
              className="flex items-center gap-1 text-sm text-text/70 hover:text-text cursor-pointer"
              data-testid="people-back"
            >
              <ArrowLeft width={14} height={14} aria-hidden="true" />
              {t("meetings.people.dialog.back")}
            </button>
            <div className="space-y-2">
              <label className="block space-y-1 text-sm">
                <span className="text-text/70">
                  {t("meetings.people.dialog.name")}
                </span>
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  variant="compact"
                  className="w-full"
                  data-testid="person-name-input"
                />
              </label>
              <label className="block space-y-1 text-sm">
                <span className="text-text/70">
                  {t("meetings.people.dialog.email")}
                </span>
                <Input
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  variant="compact"
                  className="w-full"
                  inputMode="email"
                  data-testid="person-email-input"
                />
                <span className="block text-xs text-text/50">
                  {t("meetings.people.dialog.emailHint")}
                </span>
              </label>
              {detail.other_emails.length > 0 && (
                <p className="text-xs text-text/60">
                  {t("meetings.people.dialog.otherEmails", {
                    list: detail.other_emails.join(", "),
                  })}
                </p>
              )}
              <Button
                size="sm"
                onClick={() => void save()}
                disabled={busy || name.trim() === ""}
                data-testid="person-save"
              >
                {t("meetings.people.dialog.save")}
              </Button>
            </div>
            <div className="space-y-2 border-t border-mid-gray/20 pt-3">
              <p className="text-sm font-medium">
                {t("meetings.people.dialog.mergeTitle")}
              </p>
              <p className="text-xs text-text/60">
                {t("meetings.people.dialog.mergeHint")}
              </p>
              <div
                title={t("meetings.people.dialog.mergeTarget")}
                data-testid="merge-target"
              >
                <Select
                  value={mergeTarget || null}
                  options={others.map((p) => ({
                    value: p.id,
                    label: p.email ? `${p.name} (${p.email})` : p.name,
                  }))}
                  placeholder={t("meetings.people.dialog.mergePick")}
                  isClearable
                  onChange={(value) => {
                    setMergeTarget(value ?? "");
                    setConfirmMerge(false);
                  }}
                />
              </div>
              {confirmMerge && mergeTarget ? (
                <div
                  className="space-y-2 rounded-md border border-yellow-500/40 bg-yellow-500/10 p-2"
                  data-testid="merge-confirm"
                >
                  <p className="text-sm">
                    {t("meetings.people.dialog.mergeConfirm", {
                      gone: detail.name,
                      keep: targetName,
                    })}
                  </p>
                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      variant="danger"
                      onClick={() => void merge()}
                      disabled={busy}
                      data-testid="merge-do"
                    >
                      {t("meetings.people.dialog.mergeDo")}
                    </Button>
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setConfirmMerge(false)}
                    >
                      {t("meetings.people.dialog.cancel")}
                    </Button>
                  </div>
                </div>
              ) : (
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={!mergeTarget || busy}
                  onClick={() => setConfirmMerge(true)}
                  data-testid="merge-start"
                >
                  {t("meetings.people.dialog.mergeAction")}
                </Button>
              )}
            </div>
          </div>
        )}
        {status && (
          <p
            role={status.kind === "error" ? "alert" : "status"}
            className={`text-sm ${
              status.kind === "error" ? "text-red-500" : "text-text/70"
            }`}
            data-testid="people-status"
          >
            {status.text}
          </p>
        )}
      </div>
    </Dialog>
  );
};
