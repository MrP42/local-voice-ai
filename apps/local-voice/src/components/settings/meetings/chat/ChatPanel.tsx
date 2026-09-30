import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  Loader2,
  MessageSquare,
  Plus,
  Send,
  Square,
  Trash2,
  X,
} from "lucide-react";
import {
  commands,
  events,
  type ChatMessage as ChatMessageData,
  type ChatScope,
  type ChatStage,
  type ChatThread,
  type Citation,
  type Folder,
  type PostProcessProvider,
  type RecipeCall,
  type RecipeItem,
  type ScopeFilter,
} from "@/bindings";
import { useSettings } from "../../../../hooks/useSettings";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { isLocalProvider } from "../MeetingChatNotice";
import {
  chatErrorCode,
  chatErrorKey,
  missingValues,
  newRequestId,
  recipeFits,
  recipePlaceholders,
  recipeValues,
  slashQuery,
} from "@/lib/meetingChat";
import { ChatMessage } from "./ChatMessage";
import {
  RecipeBar,
  RecipeChip,
  RecipeMenu,
  recipeTitleText,
} from "./RecipeMenu";
import { RecipeManagerDialog } from "./RecipeManagerDialog";
import { ScopeChips } from "./ScopeChips";

/** Wo der Chat steht: Besprechung (Detail), laufende Aufnahme, viele. */
export type ChatMode = "meeting" | "live" | "global";

/** Bestaetigte externe Anbieter (einmal je Anbieter, nur dieser Rechner). */
const CONFIRMED_KEY = "lva.meetingChat.remoteConfirmed";

const readConfirmed = (): string[] => {
  try {
    const raw = localStorage.getItem(CONFIRMED_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.map(String) : [];
  } catch {
    return [];
  }
};

const writeConfirmed = (ids: string[]) => {
  try {
    localStorage.setItem(CONFIRMED_KEY, JSON.stringify(ids));
  } catch {
    // Ohne Speicher wird eben beim naechsten Mal wieder gefragt.
  }
};

interface Pending {
  requestId: string;
  stage: ChatStage;
  round: number;
  text: string;
}

interface Outgoing {
  question: string;
  recipe: RecipeCall | null;
  /** Was im Verlauf als Frage erscheint. */
  display: string;
}

interface ChatPanelProps {
  scope: ChatScope;
  mode: ChatMode;
  /** Ohne Rueckruf kein Schliessen-Knopf (Live-Zeile klappt selbst ein). */
  onClose?: () => void;
  /** Klick auf einen Beleg. */
  onJump?: (citation: Citation) => void;
  /** Nur global: Scope-Chips aendern die Eingrenzung. */
  onScopeChange?: (filter: ScopeFilter) => void;
  /**
   * Recipe sofort ausfuehren (Knoepfe der Live-Zeile, Brief); `nonce` je Klick
   * neu. `values`: Werte der Recipe-Variablen (M5-P5e: Teilnehmende).
   */
  autoRecipe?: {
    id: string;
    nonce: number;
    values?: Record<string, string>;
    /** Was im Verlauf als Frage erscheint (sonst der Titel des Recipes). */
    display?: string;
  } | null;
  /** M5-P5e: einen gespeicherten Verlauf sofort oeffnen (Brief); `nonce` je Klick neu. */
  openThread?: { id: string; nonce: number } | null;
  /** Fuellt die Hoehe des Elternelements (Reiter "Fragen" der Aufnahmen-Seite)
      statt einer eigenen Mindest-/Maximalhoehe; nur die Nachrichten scrollen. */
  fill?: boolean;
}

/**
 * Chat ueber Besprechungen (M4, "Ask"): Verlaeufe je Scope, gestreamte
 * Antwort mit Stufen, Belege als Chips, Abdeckung unter jeder Antwort,
 * Recipes per "/" und Knopfleiste. Ein Bauteil fuer alle drei Orte.
 */
export const ChatPanel: React.FC<ChatPanelProps> = ({
  scope,
  mode,
  onClose,
  onJump,
  onScopeChange,
  autoRecipe,
  openThread: openThreadRequest,
  fill = false,
}) => {
  const { t } = useTranslation();
  const global = mode === "global";
  const scopeKey = JSON.stringify(scope);
  const scopeRef = useRef(scope);
  scopeRef.current = scope;

  const [threads, setThreads] = useState<ChatThread[]>([]);
  const [threadId, setThreadId] = useState<string | null>(null);
  const threadIdRef = useRef<string | null>(null);
  threadIdRef.current = threadId;
  const [messages, setMessages] = useState<ChatMessageData[]>([]);
  const [recipes, setRecipes] = useState<RecipeItem[]>([]);
  const [folders, setFolders] = useState<Folder[]>([]);
  const [input, setInput] = useState("");
  const [recipe, setRecipe] = useState<RecipeItem | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [menuIndex, setMenuIndex] = useState(0);
  const [pending, setPendingState] = useState<Pending | null>(null);
  const pendingRef = useRef<Pending | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const queuedRef = useRef<Outgoing | null>(null);
  const [managerOpen, setManagerOpen] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  // Scope-Wechsel oder Abbruch: eine spaet eintreffende Antwort verwerfen.
  const epochRef = useRef(0);

  const setPending = useCallback((next: Pending | null) => {
    pendingRef.current = next;
    setPendingState(next);
  }, []);

  // Externer Anbieter (wie im Backend: `llm::is_local`).
  const { getSetting } = useSettings();
  const providerId = getSetting("post_process_provider_id");
  const providers = (getSetting("post_process_providers") ??
    []) as PostProcessProvider[];
  const active = providers.find((p) => p.id === providerId);
  const remote = active && !isLocalProvider(active) ? active : null;

  const loadThreads = useCallback(async () => {
    const result = await commands.meetingChatThreads(scopeRef.current);
    if (result.status === "ok") setThreads(result.data ?? []);
  }, []);

  const loadRecipes = useCallback(async () => {
    const result = await commands.chatRecipesList();
    if (result.status === "ok") setRecipes(result.data ?? []);
  }, []);

  useEffect(() => {
    void loadRecipes();
    void commands.meetingFoldersList().then((result) => {
      if (result.status === "ok") setFolders(result.data ?? []);
    });
  }, [loadRecipes]);

  // Neuer Scope: neuer Verlauf, laufende Antwort gilt nicht mehr.
  useEffect(() => {
    epochRef.current += 1;
    const running = pendingRef.current;
    if (running) void commands.meetingChatCancel(running.requestId);
    setPending(null);
    setThreadId(null);
    setMessages([]);
    setError(null);
    setNotice(null);
    void loadThreads();
  }, [scopeKey, loadThreads, setPending]);

  // Panel geschlossen: ein laufender Lauf blockierte sonst den naechsten.
  useEffect(
    () => () => {
      const running = pendingRef.current;
      if (running) void commands.meetingChatCancel(running.requestId);
    },
    [],
  );

  // Deltas, Stufen, Fehler der laufenden Anfrage.
  useEffect(() => {
    const un = events.meetingChatEvent.listen((e) => {
      const payload = e.payload;
      const current = pendingRef.current;
      if (!payload || !current || payload.request_id !== current.requestId)
        return;
      if (payload.kind === "delta") {
        setPending({ ...current, text: current.text + payload.text });
      } else if (payload.kind === "stage") {
        // Runde 2 = Wiederholung: der bisherige Text ist hinfaellig.
        const newRound = payload.round > current.round;
        setPending({
          ...current,
          stage: payload.stage,
          round: payload.round,
          text: newRound ? "" : current.text,
        });
      } else if (payload.kind === "failed" && payload.code !== "cancelled") {
        setError(t(chatErrorKey(payload.code), { code: payload.code }));
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, [setPending, t]);

  // Neue Inhalte: ans Ende scrollen.
  useEffect(() => {
    const list = listRef.current;
    if (list) list.scrollTop = list.scrollHeight;
  }, [messages, pending?.text, pending?.stage]);

  const fitting = useMemo(
    () =>
      recipes.filter((r) =>
        recipeFits(r.spec, { global, live: mode === "live" }),
      ),
    [recipes, global, mode],
  );
  const query = recipe ? null : slashQuery(input);
  const menuItems = useMemo(
    () =>
      query === null
        ? []
        : fitting.filter((r) =>
            recipeTitleText(r).toLowerCase().includes(query),
          ),
    [fitting, query],
  );

  const send = useCallback(
    async (out: Outgoing) => {
      const requestId = newRequestId();
      const epoch = epochRef.current;
      setError(null);
      setNotice(null);
      setPending({ requestId, stage: "searching", round: 1, text: "" });
      const localId = `local-${requestId}`;
      setMessages((prev) => [
        ...prev,
        {
          id: localId,
          role: "user",
          text: out.display,
          citations: [],
          coverage: null,
          not_found: false,
          uncited: false,
          created_at: Math.floor(Date.now() / 1000),
        },
      ]);
      let result: Awaited<ReturnType<typeof commands.meetingChatAsk>>;
      try {
        result = await commands.meetingChatAsk({
          request_id: requestId,
          thread_id: threadIdRef.current,
          scope: scopeRef.current,
          question: out.question,
          recipe: out.recipe,
        });
      } catch (e) {
        result = { status: "error", error: String(e) };
      }
      // Abgebrochen oder Scope gewechselt: diese Antwort gilt nicht mehr.
      if (
        epoch !== epochRef.current ||
        pendingRef.current?.requestId !== requestId
      )
        return;
      setPending(null);
      if (result.status === "error") {
        const code = chatErrorCode(result.error);
        setMessages((prev) => prev.filter((m) => m.id !== localId));
        if (out.recipe === null)
          setInput((cur) => (cur === "" ? out.question : cur));
        if (code === "cancelled") setNotice(t("meetings.chat.cancelled"));
        else setError(t(chatErrorKey(code), { code }));
        return;
      }
      const answer = result.data;
      if (!answer) return;
      setMessages((prev) => [
        ...prev,
        {
          id: answer.message_id,
          role: "assistant",
          text: answer.text ?? "",
          citations: answer.citations ?? [],
          coverage: answer.coverage ?? null,
          not_found: answer.not_found,
          uncited: answer.uncited,
          created_at: Math.floor(Date.now() / 1000),
        },
      ]);
      if (answer.thread_id) setThreadId(answer.thread_id);
      void loadThreads();
    },
    [loadThreads, setPending, t],
  );

  const guardedSend = (out: Outgoing) => {
    if (pendingRef.current) return;
    if (remote && !readConfirmed().includes(remote.id)) {
      queuedRef.current = out;
      setConfirmOpen(true);
      return;
    }
    void send(out);
  };

  const confirmRemote = () => {
    if (remote) writeConfirmed([...readConfirmed(), remote.id]);
    setConfirmOpen(false);
    const out = queuedRef.current;
    queuedRef.current = null;
    if (out) void send(out);
  };

  const runRecipe = (item: RecipeItem, question = "") => {
    const title = recipeTitleText(item);
    guardedSend({
      question,
      recipe: {
        recipe_id: item.id,
        values: recipeValues(item.spec, values),
      },
      display: question ? `${title}\n${question}` : title,
    });
  };

  const pickRecipe = (item: RecipeItem, runIfPlain: boolean) => {
    setError(null);
    if (runIfPlain && recipePlaceholders(item.spec).length === 0) {
      runRecipe(item);
      return;
    }
    setRecipe(item);
    setValues({});
    setInput("");
    setMenuIndex(0);
    inputRef.current?.focus();
  };

  const submit = () => {
    if (pendingRef.current) return;
    const question = input.trim();
    if (recipe) {
      const missing = missingValues(recipe.spec, values);
      if (missing.length > 0) {
        setError(t("meetings.recipes.missing", { label: missing[0].label }));
        return;
      }
      const item = recipe;
      setRecipe(null);
      setInput("");
      runRecipe(item, question);
      setValues({});
      return;
    }
    if (question === "") return;
    setInput("");
    guardedSend({ question, recipe: null, display: question });
  };

  // Knoepfe der Live-Zeile: Recipe direkt ausfuehren.
  // Erst nach allen Mount-Effekten (Scope-Reset, StrictMode-Doppellauf)
  // ausloesen, sonst verwirft der Reset die eben gestellte Frage.
  const handledNonce = useRef<number | null>(null);
  useEffect(() => {
    if (!autoRecipe || handledNonce.current === autoRecipe.nonce) return;
    const { id, nonce, values: given, display } = autoRecipe;
    const timer = setTimeout(() => {
      handledNonce.current = nonce;
      const item = recipes.find((r) => r.id === id);
      guardedSend({
        question: "",
        recipe: { recipe_id: id, values: given ?? {} },
        display: display ?? (item ? recipeTitleText(item) : id),
      });
    }, 0);
    return () => clearTimeout(timer);
    // Nur je Klick (nonce); guardedSend liest Refs und den aktuellen Anbieter.
  }, [autoRecipe?.nonce]);

  const cancel = () => {
    const current = pendingRef.current;
    if (!current) return;
    setPending(null);
    setMessages((prev) =>
      prev.filter((m) => m.id !== `local-${current.requestId}`),
    );
    setNotice(t("meetings.chat.cancelled"));
    void commands.meetingChatCancel(current.requestId);
  };

  const openThread = async (id: string) => {
    if (pendingRef.current) return;
    setThreadId(id);
    setError(null);
    setNotice(null);
    const result = await commands.meetingChatThread(id);
    setMessages(result.status === "ok" ? (result.data ?? []) : []);
  };

  // M5-P5e: gespeicherter Brief. Wie beim Recipe erst nach den Mount-Effekten.
  const handledThreadNonce = useRef<number | null>(null);
  useEffect(() => {
    if (
      !openThreadRequest ||
      handledThreadNonce.current === openThreadRequest.nonce
    )
      return;
    const { id, nonce } = openThreadRequest;
    const timer = setTimeout(() => {
      handledThreadNonce.current = nonce;
      void openThread(id);
    }, 0);
    return () => clearTimeout(timer);
    // Nur je Klick (nonce).
  }, [openThreadRequest?.nonce]);

  const newThread = () => {
    if (pendingRef.current) return;
    setThreadId(null);
    setMessages([]);
    setError(null);
    setNotice(null);
  };

  const deleteThread = async (id: string) => {
    await commands.meetingChatThreadDelete(id);
    if (id === threadIdRef.current) newThread();
    void loadThreads();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (query !== null) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setMenuIndex((i) => Math.min(i + 1, Math.max(0, menuItems.length - 1)));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setMenuIndex((i) => Math.max(0, i - 1));
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setInput("");
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const item = menuItems[Math.min(menuIndex, menuItems.length - 1)];
        if (item) pickRecipe(item, false);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  const scopeLabel = t(`meetings.chat.scope.${mode}`);
  const busy = pending !== null;
  const stageText = pending
    ? pending.round >= 2 && pending.stage !== "answering"
      ? t("meetings.chat.stage.retry")
      : t(`meetings.chat.stage.${pending.stage}`)
    : null;

  return (
    <section
      data-testid="chat-panel"
      aria-label={t("meetings.chat.panelLabel")}
      className={`flex flex-col gap-2 bg-background p-3 ${
        fill
          ? "min-h-0 flex-1 overflow-hidden"
          : "max-h-[80vh] min-h-[22rem] rounded-lg border border-mid-gray/20"
      }`}
    >
      <header className="flex items-center justify-between gap-2">
        <h2 className="flex items-center gap-1.5 text-sm font-semibold">
          <MessageSquare width={14} height={14} aria-hidden="true" />
          {t("meetings.chat.ask")}
          <span className="font-normal text-text/60">· {scopeLabel}</span>
        </h2>
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            aria-label={t("meetings.chat.close")}
            title={t("meetings.chat.close")}
            className="rounded-md p-1 text-text/60 hover:bg-mid-gray/15 hover:text-text cursor-pointer"
          >
            <X width={14} height={14} />
          </button>
        )}
      </header>

      {global && scope.kind === "global" && (
        <ScopeChips
          filter={scope.filter}
          folders={folders}
          onChange={onScopeChange}
        />
      )}

      {remote && (
        <p
          data-testid="chat-remote-bar"
          className="flex items-start gap-1.5 rounded-md border border-yellow-500/40 bg-yellow-500/10 px-2 py-1.5 text-xs text-yellow-700 dark:text-yellow-300"
        >
          <AlertTriangle
            width={12}
            height={12}
            className="mt-0.5 shrink-0"
            aria-hidden="true"
          />
          <span>
            {t("meetings.chat.remote.bar", { provider: remote.label })}
          </span>
        </p>
      )}

      <div className="space-y-1">
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs font-medium uppercase tracking-wide text-mid-gray">
            {t("meetings.chat.threads")}
          </span>
          <button
            type="button"
            onClick={newThread}
            disabled={busy}
            className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-xs text-text/70 hover:bg-mid-gray/15 hover:text-text disabled:opacity-50 cursor-pointer"
          >
            <Plus width={12} height={12} aria-hidden="true" />
            {t("meetings.chat.newThread")}
          </button>
        </div>
        {threads.length > 0 && (
          <ul className="max-h-24 space-y-0.5 overflow-y-auto">
            {threads.map((thread) => (
              <li key={thread.id} className="group flex items-center gap-1">
                <button
                  type="button"
                  onClick={() => void openThread(thread.id)}
                  aria-current={thread.id === threadId ? "true" : undefined}
                  className={`min-w-0 flex-1 truncate rounded px-1.5 py-0.5 text-left text-xs cursor-pointer ${
                    thread.id === threadId
                      ? "bg-logo-primary/20 text-text"
                      : "text-text/70 hover:bg-mid-gray/15"
                  }`}
                >
                  {thread.title || t("meetings.chat.threadUntitled")}
                  <span className="ms-1 text-text/40">
                    {t("meetings.chat.threadCount", {
                      count: thread.message_count,
                    })}
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => void deleteThread(thread.id)}
                  aria-label={t("meetings.chat.deleteThread")}
                  title={t("meetings.chat.deleteThread")}
                  className="rounded p-0.5 text-text/40 opacity-60 hover:text-red-400 group-hover:opacity-100 cursor-pointer"
                >
                  <Trash2 width={12} height={12} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div
        ref={listRef}
        className={`flex-1 space-y-3 overflow-y-auto border-t border-mid-gray/20 pt-2 ${
          fill ? "min-h-0" : "min-h-[6rem]"
        }`}
        aria-live="polite"
      >
        {messages.length === 0 && !pending && (
          <p className="text-xs text-text/50">{t("meetings.chat.empty")}</p>
        )}
        {messages.map((message) => (
          <ChatMessage
            key={message.id}
            message={message}
            global={global}
            onJump={onJump}
          />
        ))}
        {pending && (
          <div className="space-y-1">
            {pending.text !== "" && (
              <p
                data-testid="chat-streaming"
                className="whitespace-pre-wrap break-words text-sm text-text"
              >
                {pending.text}
              </p>
            )}
            <p
              data-testid="chat-stage"
              className="flex items-center gap-1.5 text-xs text-text/60"
            >
              <Loader2
                width={12}
                height={12}
                className="animate-spin"
                aria-hidden="true"
              />
              {stageText}
            </p>
          </div>
        )}
      </div>

      {error && (
        <p role="alert" className="text-xs text-red-500">
          {error}
        </p>
      )}
      {notice && (
        <p data-testid="chat-notice" className="text-xs text-text/60">
          {notice}
        </p>
      )}

      <RecipeBar
        items={fitting}
        disabled={busy}
        onPick={(item) => pickRecipe(item, true)}
        onManage={() => setManagerOpen(true)}
      />

      {recipe && (
        <RecipeChip
          recipe={recipe}
          values={values}
          folders={folders}
          onChange={setValues}
          onRemove={() => {
            setRecipe(null);
            setValues({});
            setError(null);
          }}
          onSubmit={submit}
        />
      )}

      <div className="relative">
        {query !== null && (
          <div className="absolute bottom-full left-0 right-0 z-10 mb-1">
            <RecipeMenu
              items={menuItems}
              activeIndex={Math.min(
                menuIndex,
                Math.max(0, menuItems.length - 1),
              )}
              onPick={(item) => pickRecipe(item, false)}
              onHover={setMenuIndex}
            />
          </div>
        )}
        <div className="flex items-end gap-2">
          <textarea
            ref={inputRef}
            aria-label={t("meetings.chat.inputLabel")}
            placeholder={t("meetings.chat.placeholder")}
            value={input}
            rows={2}
            onChange={(e) => {
              setInput(e.target.value);
              setMenuIndex(0);
            }}
            onKeyDown={onKeyDown}
            className="min-h-[2.5rem] flex-1 resize-none rounded-md border border-mid-gray/40 bg-mid-gray/10 px-2 py-1.5 text-sm focus:border-logo-primary focus:outline-none"
          />
          {busy ? (
            <Button size="sm" variant="secondary" onClick={cancel}>
              <Square width={12} height={12} aria-hidden="true" />
              {t("meetings.chat.cancel")}
            </Button>
          ) : (
            <Button
              size="sm"
              onClick={submit}
              disabled={input.trim() === "" && !recipe}
            >
              <Send width={12} height={12} aria-hidden="true" />
              {t("meetings.chat.send")}
            </Button>
          )}
        </div>
      </div>

      <Dialog
        open={confirmOpen}
        onOpenChange={(open) => {
          if (!open) {
            setConfirmOpen(false);
            queuedRef.current = null;
          }
        }}
        title={t("meetings.chat.remote.confirmTitle", {
          provider: remote?.label ?? "",
        })}
        closeLabel={t("meetings.chat.remote.cancel")}
        footer={
          <>
            <Button
              variant="secondary"
              onClick={() => {
                setConfirmOpen(false);
                queuedRef.current = null;
              }}
            >
              {t("meetings.chat.remote.cancel")}
            </Button>
            <Button onClick={confirmRemote}>
              {t("meetings.chat.remote.confirm")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {t("meetings.chat.remote.confirmBody", {
            provider: remote?.label ?? "",
          })}
        </p>
      </Dialog>

      <RecipeManagerDialog
        open={managerOpen}
        onOpenChange={setManagerOpen}
        onChanged={() => void loadRecipes()}
      />
    </section>
  );
};
