import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { FilePlus2, Presentation } from "lucide-react";
import {
  commands,
  events,
  type DocTypeInfo,
  type IntegrationView,
} from "@/bindings";
import { IconAction } from "../../../ui/IconAction";
import { PresenterSendDialog } from "./PresenterSendDialog";
import { ActionMenu, type ActionMenuItem } from "../../../ui/ActionMenu";
import type { SlideRefs } from "../../../whats-new/MarkdownContent";
import { MinutesView } from "../MinutesView";
import { ProvenanceArea } from "../variants/ProvenanceArea";
import {
  PROTOCOL_KEY,
  readDocType,
  typesToShow,
  writeDocType,
} from "./docTypes";

interface DocumentsPanelProps {
  meetingId: string;
  meetingTitle: string;
  slideRefs?: SlideRefs;
  onOpenSlide?: (id: string) => void;
  /** Der Typ "Follow-up-Mail" erzeugt nichts selbst, er öffnet den vorhandenen Dialog. */
  onFollowup: () => void;
}

/**
 * Reiter "Dokumente" (Entwurf 2026-10-13): oben die Typleiste (Protokoll und alle
 * Dokumente, die es zu dieser Besprechung schon gibt, plus der gewählte Typ), daneben
 * "Neues Dokument" mit allen Typen des Katalogs samt Zielgruppe und Länge. Darunter
 * dieselbe Ansicht wie beim Protokoll (`MinutesView`), nur mit anderem `kind`.
 */
export const DocumentsPanel: React.FC<DocumentsPanelProps> = ({
  meetingId,
  meetingTitle,
  slideRefs,
  onOpenSlide,
  onFollowup,
}) => {
  const { t } = useTranslation();
  const [types, setTypes] = useState<DocTypeInfo[]>([]);
  const [kinds, setKinds] = useState<string[]>([]);
  const [selected, setSelected] = useState<string>(readDocType);
  // Etappe 4a: eingeschaltete Presenter-Integrationen (Knopf nur, wenn es eine gibt).
  const [presenters, setPresenters] = useState<IntegrationView[]>([]);
  const [sendOpen, setSendOpen] = useState(false);

  useEffect(() => {
    void commands.meetingsDocumentTypes().then((result) => {
      if (result.status === "ok") setTypes(result.data ?? []);
    });
    void commands.integrationsList().then((result) => {
      if (result.status === "ok")
        setPresenters(
          (result.data ?? []).filter(
            (v) => v.integration.kind === "presenter" && v.integration.enabled,
          ),
        );
    });
  }, []);

  const loadKinds = useCallback(async () => {
    const result = await commands.meetingsGetDocuments(meetingId);
    if (result.status === "ok")
      setKinds((result.data ?? []).map((d) => d.kind));
  }, [meetingId]);

  useEffect(() => {
    void loadKinds();
    const un = events.documentEvent.listen((event) => {
      if (
        event.payload.meeting_id === meetingId &&
        event.payload.kind === "done"
      )
        void loadKinds();
    });
    return () => {
      void un.then((f) => f());
    };
  }, [meetingId, loadKinds]);

  const choose = (key: string) => {
    setSelected(key);
    writeDocType(key);
    // Auch ohne Ereignis (Erzeugen im selben Reiter) zeigt die Leiste danach jedes
    // vorhandene Dokument.
    void loadKinds();
  };

  const current = types.find((type) => type.key === selected);
  // Ein gespeicherter Schlüssel, den es nicht (mehr) gibt, fällt aufs Protokoll zurück.
  const active = current && current.engine !== "followup" ? current : undefined;
  const activeKey = active?.key ?? PROTOCOL_KEY;
  const shown = useMemo(
    () => typesToShow(types, kinds, activeKey),
    [types, kinds, activeKey],
  );

  const items: ActionMenuItem[] = types.map((type) => ({
    id: type.key,
    label: type.title,
    icon: FilePlus2,
    title: `${type.audience} · ${type.length}`,
    trailing: <span className="text-xs text-text/50">{type.length}</span>,
    onSelect: () =>
      type.engine === "followup" ? onFollowup() : choose(type.key),
    testId: `doc-type-${type.key}`,
  }));

  const bar = (
    <div
      role="group"
      aria-label={t("meetings.documents.typesLabel")}
      className="flex flex-wrap items-center gap-1.5"
      data-testid="doc-type-bar"
    >
      {shown.map((type) => {
        const on = type.key === activeKey;
        return (
          <button
            key={type.key}
            type="button"
            aria-pressed={on}
            title={`${type.audience} · ${type.length}`}
            data-testid={`doc-chip-${type.key}`}
            onClick={() => choose(type.key)}
            className={`rounded-full border px-2.5 py-0.5 text-xs transition-colors cursor-pointer ${
              on
                ? "border-logo-primary bg-logo-primary/15 text-text"
                : "border-mid-gray/30 text-text/70 hover:border-logo-primary/60"
            }`}
          >
            {type.title}
          </button>
        );
      })}
      {presenters.length > 0 && (
        <IconAction
          size="sm"
          icon={Presentation}
          label={t("meetings.presenter.button")}
          description={t("meetings.presenter.description")}
          testId="presenter-send"
          onClick={() => setSendOpen(true)}
        />
      )}
      {types.length > 0 && (
        <ActionMenu
          trigger={{
            icon: FilePlus2,
            label: t("meetings.documents.new"),
            description: t("meetings.documents.newHint"),
            testId: "doc-new",
          }}
          menuLabel={t("meetings.documents.new")}
          widthClass="w-80"
          items={items}
        />
      )}
    </div>
  );

  // Dokumente, die es zu dieser Besprechung gibt (fuer den Sendedialog).
  const existing = types.filter(
    (type) => type.engine !== "followup" && kinds.includes(type.kind),
  );

  return (
    <div data-testid="documents-panel">
      <PresenterSendDialog
        open={sendOpen}
        onOpenChange={setSendOpen}
        meetingId={meetingId}
        targets={presenters}
        available={existing}
      />
      <ProvenanceArea
        key={activeKey}
        subject={{
          type: "document",
          meetingId,
          docKind: active?.kind ?? "minutes",
        }}
        testId={
          activeKey === PROTOCOL_KEY
            ? "prov-area-minutes"
            : `prov-area-${activeKey}`
        }
        onOpenSlide={onOpenSlide}
      >
        <MinutesView
          key={activeKey}
          meetingId={meetingId}
          meetingTitle={meetingTitle}
          slideRefs={slideRefs}
          docType={active && active.engine === "document" ? active : undefined}
          toolbarLead={bar}
        />
      </ProvenanceArea>
    </div>
  );
};
