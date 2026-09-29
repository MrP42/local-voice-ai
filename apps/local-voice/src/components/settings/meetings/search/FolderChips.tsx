import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { Plus } from "lucide-react";
import { commands, type Folder } from "@/bindings";
import { Dialog } from "../../../ui/Dialog";
import { Button } from "../../../ui/Button";
import { Input } from "../../../ui/Input";
import { Alert } from "../../../ui/Alert";
import { chipClass } from "./FilterChips";

/** Fehlercodes der Ordner-Commands in Nutzertext. */
export const folderErrorText = (code: string, t: TFunction) => {
  switch (code) {
    case "folder_name_invalid":
      return t("meetings.folders.errors.nameInvalid");
    case "folder_name_taken":
      return t("meetings.folders.errors.nameTaken");
    case "folder_not_found":
      return t("meetings.folders.errors.notFound");
    default:
      return t("meetings.folders.errors.failed");
  }
};

// ---------------------------------------------------------------------------
// Kontextmenue (auch fuer die Zeilen der Besprechungsliste)
// ---------------------------------------------------------------------------

export interface ContextMenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
}

interface ContextMenuProps {
  x: number;
  y: number;
  label: string;
  items: ContextMenuItem[];
  onClose: () => void;
}

/** Schlichtes Menue an der Mausposition; schliesst bei Klick daneben, Escape,
 *  Scrollen und Fensterwechsel. */
export const ContextMenu: React.FC<ContextMenuProps> = ({
  x,
  y,
  label,
  items,
  onClose,
}) => {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(x, window.innerWidth - rect.width - 4)),
      y: Math.max(4, Math.min(y, window.innerHeight - rect.height - 4)),
    });
    el.querySelector<HTMLElement>("[role=menuitem]")?.focus();
  }, [x, y]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onClose, true);
    window.addEventListener("blur", onClose);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onClose, true);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  return createPortal(
    <div
      ref={ref}
      role="menu"
      aria-label={label}
      className="fixed z-50 min-w-[180px] rounded-md border border-mid-gray/30 bg-background py-1 shadow-lg"
      style={{ left: pos.x, top: pos.y }}
    >
      {items.map((item) => (
        <button
          key={item.label}
          type="button"
          role="menuitem"
          className={`flex w-full min-h-[36px] cursor-pointer items-center px-3 text-start text-sm hover:bg-mid-gray/15 focus-visible:outline-none focus-visible:bg-mid-gray/15 ${
            item.danger ? "text-red-400" : "text-text/80 hover:text-text"
          }`}
          onClick={() => {
            onClose();
            item.onSelect();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>,
    document.body,
  );
};

// ---------------------------------------------------------------------------
// Ordner anlegen / umbenennen
// ---------------------------------------------------------------------------

interface FolderNameDialogProps {
  /** `null` = anlegen, sonst umbenennen. */
  folder: Folder | null;
  open: boolean;
  onClose: () => void;
  onSaved: (folder: Folder) => void;
}

const FolderNameDialog: React.FC<FolderNameDialogProps> = ({
  folder,
  open,
  onClose,
  onSaved,
}) => {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setName(folder?.name ?? "");
      setError(null);
    }
  }, [open, folder]);

  const submit = async () => {
    if (busy) return;
    setBusy(true);
    const result = await commands.meetingFoldersSave(
      folder?.id ?? null,
      name,
      folder?.color ?? null,
    );
    setBusy(false);
    if (result.status === "ok" && result.data) {
      onSaved(result.data);
      onClose();
    } else {
      setError(
        folderErrorText(result.status === "error" ? result.error : "", t),
      );
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title={
        folder
          ? t("meetings.folders.renameTitle")
          : t("meetings.folders.newTitle")
      }
      closeLabel={t("meetings.folders.cancel")}
      initialFocusRef={inputRef as React.RefObject<HTMLElement>}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {t("meetings.folders.cancel")}
          </Button>
          <Button onClick={submit} disabled={busy || name.trim() === ""}>
            {folder ? t("meetings.folders.save") : t("meetings.folders.create")}
          </Button>
        </>
      }
    >
      <form
        className="space-y-2"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <label className="block text-sm text-text/80">
          {t("meetings.folders.nameLabel")}
          <Input
            ref={inputRef}
            value={name}
            maxLength={60}
            onChange={(e) => setName(e.target.value)}
            placeholder={t("meetings.folders.namePlaceholder")}
            className="mt-1 w-full font-normal"
          />
        </label>
        {error && <Alert variant="error">{error}</Alert>}
      </form>
    </Dialog>
  );
};

// ---------------------------------------------------------------------------
// Chips „Alle · <Ordner> · +“
// ---------------------------------------------------------------------------

interface FolderChipsProps {
  folders: Folder[];
  activeId: string | null;
  onSelect: (id: string | null) => void;
  /** Nach Anlegen, Umbenennen oder Loeschen: Ordner neu laden. */
  onChanged: () => void;
}

export const FolderChips: React.FC<FolderChipsProps> = ({
  folders,
  activeId,
  onSelect,
  onChanged,
}) => {
  const { t } = useTranslation();
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    folder: Folder;
  } | null>(null);
  const [nameDialog, setNameDialog] = useState<{
    folder: Folder | null;
  } | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Folder | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    const target = deleteTarget;
    setDeleteTarget(null);
    const result = await commands.meetingFoldersDelete(target.id);
    if (result.status === "error") {
      setDeleteError(folderErrorText(result.error, t));
    } else {
      setDeleteError(null);
      if (activeId === target.id) onSelect(null);
    }
    onChanged();
  };

  return (
    <>
      <div
        className="flex flex-wrap items-center gap-1.5"
        role="group"
        aria-label={t("meetings.folders.label")}
      >
        <button
          type="button"
          aria-pressed={activeId === null}
          className={chipClass(activeId === null)}
          onClick={() => onSelect(null)}
        >
          {t("meetings.folders.all")}
        </button>
        {folders.map((folder) => {
          const active = activeId === folder.id;
          return (
            <button
              key={folder.id}
              type="button"
              aria-pressed={active}
              data-folder-id={folder.id}
              className={chipClass(active)}
              onClick={() => onSelect(active ? null : folder.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                setMenu({ x: e.clientX, y: e.clientY, folder });
              }}
            >
              <span className="truncate max-w-[12rem]">{folder.name}</span>
              <span className="text-text/50">{folder.meeting_count}</span>
            </button>
          );
        })}
        <button
          type="button"
          className={chipClass(false)}
          title={t("meetings.folders.add")}
          aria-label={t("meetings.folders.add")}
          onClick={() => setNameDialog({ folder: null })}
        >
          <Plus width={12} height={12} />
        </button>
      </div>
      {deleteError && <Alert variant="error">{deleteError}</Alert>}

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          label={t("meetings.folders.menu")}
          onClose={() => setMenu(null)}
          items={[
            {
              label: t("meetings.folders.rename"),
              onSelect: () => setNameDialog({ folder: menu.folder }),
            },
            {
              label: t("meetings.folders.delete"),
              danger: true,
              onSelect: () => setDeleteTarget(menu.folder),
            },
          ]}
        />
      )}

      <FolderNameDialog
        open={nameDialog !== null}
        folder={nameDialog?.folder ?? null}
        onClose={() => setNameDialog(null)}
        onSaved={() => onChanged()}
      />

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(o) => {
          if (!o) setDeleteTarget(null);
        }}
        title={t("meetings.folders.deleteTitle")}
        closeLabel={t("meetings.folders.cancel")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setDeleteTarget(null)}>
              {t("meetings.folders.cancel")}
            </Button>
            <Button variant="danger" onClick={confirmDelete}>
              {t("meetings.folders.delete")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("meetings.folders.deleteBody", { name: deleteTarget?.name ?? "" })}
        </p>
      </Dialog>
    </>
  );
};
