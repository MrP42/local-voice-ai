import React, { useMemo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import { linkSlideTags, SLIDE_LINK } from "@/lib/meetingSlides";

/** D5: Folienbelege (`[F7]`) eines Protokolls als anklickbare Marken. */
export interface SlideRefs {
  /** Gibt es die Folie? Nur dann wird aus `[F7]` ein Link. */
  known: (slideNumber: number) => boolean;
  onOpen: (slideNumber: number) => void;
}

interface MarkdownContentProps {
  markdown: string;
  slideRefs?: SlideRefs;
}

const allowedElements = [
  "a",
  "blockquote",
  "br",
  "code",
  "del",
  "em",
  "h1",
  "h2",
  "h3",
  "hr",
  "img",
  "input",
  "li",
  "ol",
  "p",
  "pre",
  "strong",
  "table",
  "tbody",
  "td",
  "th",
  "thead",
  "tr",
  "ul",
];

const isSafeUrl = (url: string) => {
  try {
    const parsed = new URL(url);
    return ["http:", "https:", "mailto:"].includes(parsed.protocol);
  } catch {
    return false;
  }
};

const openSafeUrl = async (url: string) => {
  if (!isSafeUrl(url)) return;

  try {
    await openUrl(url);
  } catch (error) {
    console.error("Failed to open release note link:", error);
  }
};

const isSafeImageSrc = (src: string) => {
  if (!src.startsWith("/release-notes/")) return false;
  if (src.includes("\\") || src.includes("..")) return false;

  return true;
};

/** Der Text eines React-Teilbaums (nur Zeichenketten und deren Kinder). */
const textOf = (node: React.ReactNode): string => {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (React.isValidElement<{ children?: React.ReactNode }>(node)) {
    return textOf(node.props.children);
  }
  return "";
};

const ExternalLink: React.FC<{
  href?: string;
  children?: React.ReactNode;
}> = ({ children, href }) => {
  if (!href || !isSafeUrl(href)) {
    return <>{children}</>;
  }

  return (
    <a
      href={href}
      rel="noreferrer"
      onClick={(event) => {
        event.preventDefault();
        void openSafeUrl(href);
      }}
      className="text-logo-primary underline decoration-logo-primary/40 underline-offset-2 hover:decoration-logo-primary"
    >
      {children}
    </a>
  );
};

const components: Components = {
  h1: ({ children }) => (
    <h3 className="text-base font-semibold leading-snug text-text">
      {children}
    </h3>
  ),
  h2: ({ children }) => (
    <h3 className="text-[15px] font-semibold leading-snug text-text">
      {children}
    </h3>
  ),
  h3: ({ children }) => (
    <h3 className="text-sm font-semibold leading-snug text-text">{children}</h3>
  ),
  p: ({ children }) => (
    <p className="text-sm leading-relaxed text-text/80">{children}</p>
  ),
  ul: ({ children, className }) => {
    const isTaskList = className?.includes("contains-task-list");

    return (
      <ul
        className={`space-y-1 text-sm leading-relaxed text-text/80 ${
          isTaskList ? "ps-0" : "list-disc ps-5"
        }`}
      >
        {children}
      </ul>
    );
  },
  li: ({ children, className }) => {
    const isTaskListItem = className?.includes("task-list-item");
    // Ein Aufgaben-Kontrollkaestchen hat keine Beschriftung ausser dem Text daneben: der
    // Aufgabentext wird sein Name (axe `label`). `input` kennt seine Nachbarn nicht, deshalb
    // gibt der Eintrag ihn weiter.
    const label = isTaskListItem ? textOf(children).trim() : "";

    return (
      <li
        className={`${isTaskListItem ? "list-none" : "pl-1"} marker:text-text/50`}
      >
        {isTaskListItem && label
          ? React.Children.map(children, (child) =>
              React.isValidElement<{ type?: string }>(child) &&
              child.props.type === "checkbox"
                ? React.cloneElement(
                    child as React.ReactElement<Record<string, unknown>>,
                    { "aria-label": label },
                  )
                : child,
            )
          : children}
      </li>
    );
  },
  input: ({ checked, type, ...rest }) => {
    if (type !== "checkbox") return null;
    const label = (rest as { "aria-label"?: string })["aria-label"];

    return (
      <input
        type="checkbox"
        checked={Boolean(checked)}
        disabled
        readOnly
        aria-label={label}
        className="me-2 h-3.5 w-3.5 align-middle accent-logo-primary"
      />
    );
  },
  ol: ({ children }) => (
    <ol className="list-decimal space-y-1 ps-5 text-sm leading-relaxed text-text/80">
      {children}
    </ol>
  ),
  del: ({ children }) => (
    <del className="text-text/60 line-through">{children}</del>
  ),
  br: () => <br />,
  hr: () => <hr className="border-mid-gray/20" />,
  img: ({ alt, src }) => {
    if (!src || !isSafeImageSrc(src)) return null;

    return (
      <img
        src={src}
        alt={alt ?? ""}
        loading="lazy"
        decoding="async"
        className="mx-auto block max-h-72 max-w-full object-contain"
      />
    );
  },
  table: ({ children }) => (
    <div className="overflow-x-auto">
      <table className="w-full border-collapse text-left text-sm leading-relaxed text-text/80">
        {children}
      </table>
    </div>
  ),
  thead: ({ children }) => (
    <thead className="border-b border-mid-gray/30 text-text">{children}</thead>
  ),
  tbody: ({ children }) => (
    <tbody className="divide-y divide-mid-gray/20">{children}</tbody>
  ),
  tr: ({ children }) => <tr>{children}</tr>,
  th: ({ children }) => (
    <th className="px-2 py-1.5 font-semibold">{children}</th>
  ),
  td: ({ children }) => <td className="px-2 py-1.5 align-top">{children}</td>,
  blockquote: ({ children }) => (
    <blockquote className="border-s-2 border-logo-primary/50 ps-3 text-sm leading-relaxed text-text/70">
      {children}
    </blockquote>
  ),
  code: ({ children, className }) => {
    const isBlock = className?.startsWith("language-");

    if (isBlock) {
      return (
        <code className="block whitespace-pre font-mono text-xs">
          {children}
        </code>
      );
    }

    return (
      <code className="rounded bg-mid-gray/10 px-1 py-0.5 font-mono text-[0.85em]">
        {children}
      </code>
    );
  },
  pre: ({ children }) => (
    <pre className="overflow-x-auto rounded-md bg-mid-gray/10 p-3 text-xs leading-relaxed text-text/80">
      {children}
    </pre>
  ),
  a: ({ children, href }) => (
    <ExternalLink href={href}>{children}</ExternalLink>
  ),
};

export const MarkdownContent: React.FC<MarkdownContentProps> = ({
  markdown,
  slideRefs,
}) => {
  const merged = useMemo<Components>(() => {
    if (!slideRefs) return components;
    return {
      ...components,
      a: (props) => {
        const match = SLIDE_LINK.exec(props.href ?? "");
        if (!match) {
          return (
            <ExternalLink href={props.href}>{props.children}</ExternalLink>
          );
        }
        const number = Number(match[1]);
        return (
          <button
            type="button"
            data-testid="slide-ref"
            data-slide-ref={number}
            onClick={() => slideRefs.onOpen(number)}
            className="mx-0.5 inline-flex cursor-pointer items-center rounded-md border border-logo-primary/50 bg-logo-primary/10 px-1 align-baseline text-[11px] font-medium leading-4 tabular-nums text-text hover:bg-logo-primary/25 focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60"
          >
            {props.children}
          </button>
        );
      },
    };
  }, [slideRefs]);
  const source = useMemo(
    () => (slideRefs ? linkSlideTags(markdown, slideRefs.known) : markdown),
    [markdown, slideRefs],
  );
  return (
    <div className="space-y-3">
      <ReactMarkdown
        allowedElements={allowedElements}
        components={merged}
        remarkPlugins={[remarkGfm]}
        skipHtml
      >
        {source}
      </ReactMarkdown>
    </div>
  );
};
