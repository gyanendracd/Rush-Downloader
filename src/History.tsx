// History: every link the app has downloaded, as a table. The Downloads
// page shows what can be used now; this page remembers the rest too, files
// deleted long ago included.

import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, Search, X } from "lucide-react";
import {
  api,
  deletedAgo,
  displayName,
  formatBytes,
  formatDate,
  isDeleted,
  STATUS_LABEL,
  type Download,
  type DownloadItem,
} from "./api";
import { ItemPicture } from "./Detail";
import { fileCount, FileIcon, useApp } from "./ui";

/** More item rows than this are counted, not drawn. */
const ITEMS_SHOWN = 500;

/** A link without its scheme and "www.", which say nothing in a table. */
function shortLink(link: string): string {
  return /^https?:/i.test(link) ? link.replace(/^https?:\/\/(www\.)?/i, "") : link;
}

/** What became of a download, in a word or two. */
function Outcome({ d }: { d: Download }) {
  if (isDeleted(d)) {
    return (
      <span className="tone-deleted" title="Its file is no longer where it was saved">
        Deleted · {deletedAgo(d)}
      </span>
    );
  }
  if (d.status === "completed") return <span className="tone-ok">Completed</span>;
  if (d.status === "failed") {
    return (
      <span className="tone-err" title={d.error ?? undefined}>
        Failed
      </span>
    );
  }
  return <span>{STATUS_LABEL[d.status]}</span>;
}

/** One link that gave one file: a line of the table. */
function HistoryRow(props: { d: Download; onMenu: (d: Download, x: number, y: number) => void }) {
  const { actions } = useApp();
  const { d } = props;
  return (
    <div
      className={`history-row${isDeleted(d) ? " is-deleted" : ""}`}
      role="row"
      tabIndex={0}
      onClick={() => actions.inspect(d.id)}
      onKeyDown={(e) => {
        if (e.key === "Enter") actions.inspect(d.id);
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        props.onMenu(d, e.clientX, e.clientY);
      }}
    >
      <span className="history-name" role="cell" title={displayName(d)}>
        <FileIcon d={d} />
        <span>{displayName(d)}</span>
      </span>
      <span className="history-link" role="cell" title={d.url}>
        {shortLink(d.url)}
      </span>
      <span className="history-size" role="cell">
        {d.total != null ? formatBytes(d.total) : d.downloaded > 0 ? formatBytes(d.downloaded) : ""}
      </span>
      <span className="history-date" role="cell">
        {formatDate(d.created_at)}
      </span>
      <span className="history-status" role="cell">
        <Outcome d={d} />
      </span>
    </div>
  );
}

/** True once the element has come into view. */
function useSeen<T extends Element>(): [React.RefObject<T | null>, boolean] {
  const place = useRef<T | null>(null);
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    const el = place.current;
    if (seen || !el) return;
    const watch = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) setSeen(true);
      },
      { rootMargin: "200px" },
    );
    watch.observe(el);
    return () => watch.disconnect();
  }, [seen]);
  return [place, seen];
}

const ITEM_OUTCOME: Record<DownloadItem["state"], string> = {
  done: "Downloaded",
  downloading: "Downloading",
  waiting: "Waiting",
  missed: "Not available",
  deleted: "Deleted",
};

/** Everything a profile or playlist download fetched, one line each. */
function HistoryItems({ d }: { d: Download }) {
  const [items, setItems] = useState<DownloadItem[] | null>(null);
  // A long history has many of these: each asks only once it is on screen.
  const [place, seen] = useSeen<HTMLDivElement>();
  useEffect(() => {
    if (!seen) return;
    let stale = false;
    api
      .items(d.id)
      .then((found) => !stale && setItems(found))
      .catch(() => !stale && setItems((had) => had ?? []));
    return () => {
      stale = true;
    };
  }, [seen, d.id, d.status, d.items, d.deleted_at]);

  if (items == null) {
    return (
      <div ref={place} className="history-items is-loading" aria-hidden="true">
        {[64, 52, 58].map((width, i) => (
          <span key={i} className="shimmer" style={{ width: `${width}%` }} />
        ))}
      </div>
    );
  }
  if (items.length === 0) {
    return (
      <p ref={place as React.RefObject<HTMLParagraphElement | null>} className="history-items is-empty">
        {isDeleted(d)
          ? "Its files are gone, and their names were not kept: it was downloaded before History began keeping them."
          : "Nothing has been fetched for it yet."}
      </p>
    );
  }
  return (
    <div ref={place} className="history-items" role="rowgroup" aria-label={`What ${displayName(d)} holds`}>
      {items.slice(0, ITEMS_SHOWN).map((item, i) => (
        <div key={i} className={`history-item${item.state === "deleted" ? " is-deleted" : ""}`} role="row">
          <span className="history-name" role="cell" title={item.title}>
            <span className="history-index">{i + 1}</span>
            <ItemPicture item={item} />
            <span>{item.title}</span>
          </span>
          <span className="history-link" role="cell" title={item.url ?? undefined}>
            {item.url ? shortLink(item.url) : ""}
          </span>
          <span className="history-size" role="cell">
            {item.size != null ? formatBytes(item.size) : ""}
          </span>
          <span className="history-date" role="cell" />
          <span className="history-status" role="cell">
            <span
              className={
                item.state === "done"
                  ? "tone-ok"
                  : item.state === "deleted"
                    ? "tone-deleted"
                    : item.state === "missed"
                      ? "tone-err"
                      : undefined
              }
            >
              {ITEM_OUTCOME[item.state]}
            </span>
          </span>
        </div>
      ))}
      {items.length > ITEMS_SHOWN && <p className="history-more">and {items.length - ITEMS_SHOWN} more</p>}
    </div>
  );
}

/** A profile or a playlist: a small card saying whose or which it is, and
 *  under it every file it fetched, as lines of the same table. */
function HistoryGroup(props: { d: Download; onMenu: (d: Download, x: number, y: number) => void }) {
  const { actions } = useApp();
  const { d } = props;
  const [open, setOpen] = useState(true);
  const kind = /[?&]list=/.test(d.url) ? "Playlist" : "Profile";
  return (
    <section className={`history-group${isDeleted(d) ? " is-deleted" : ""}`} aria-label={displayName(d)}>
      <div
        className={`history-card history-row${isDeleted(d) ? " is-deleted" : ""}`}
        role="row"
        tabIndex={0}
        onClick={() => actions.inspect(d.id)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && e.target === e.currentTarget) actions.inspect(d.id);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          props.onMenu(d, e.clientX, e.clientY);
        }}
      >
        <span className="history-name" role="cell" title={displayName(d)}>
          <FileIcon d={d} />
          <span>{displayName(d)}</span>
          <span className="history-kind">
            {kind} · {fileCount(d.items)}
          </span>
        </span>
        <span className="history-link" role="cell" title={d.url}>
          {shortLink(d.url)}
        </span>
        <span className="history-size" role="cell">
          {d.total != null ? formatBytes(d.total) : d.downloaded > 0 ? formatBytes(d.downloaded) : ""}
        </span>
        <span className="history-date" role="cell">
          {formatDate(d.created_at)}
        </span>
        <span className="history-status" role="cell">
          <Outcome d={d} />
          <button
            type="button"
            className="history-fold"
            aria-expanded={open}
            aria-label={open ? "Hide its items" : "Show its items"}
            title={open ? "Hide its items" : "Show its items"}
            onClick={(e) => {
              e.stopPropagation();
              setOpen((was) => !was);
            }}
          >
            <ChevronDown size={15} strokeWidth={2.25} aria-hidden="true" />
          </button>
        </span>
      </div>
      {open && <HistoryItems d={d} />}
    </section>
  );
}

export function HistoryView(props: { onMenu: (d: Download, x: number, y: number) => void }) {
  const { downloads } = useApp();
  const [query, setQuery] = useState("");
  const newestFirst = useMemo(() => [...downloads].sort((a, b) => b.created_at - a.created_at || b.id - a.id), [downloads]);
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q
      ? newestFirst.filter((d) => displayName(d).toLowerCase().includes(q) || d.url.toLowerCase().includes(q))
      : newestFirst;
  }, [newestFirst, query]);
  const gone = downloads.filter(isDeleted).length;

  return (
    <div className="page page-history">
      <header className="page-head">
        <h1>History</h1>
        <span className="history-count">
          {downloads.length === 1 ? "1 link" : `${downloads.length} links`}
          {gone > 0 ? ` · ${gone} deleted` : ""}
        </span>
        <label className="search">
          <Search size={16} strokeWidth={2} aria-hidden="true" />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search history..."
            aria-label="Search history"
            spellCheck={false}
          />
          {query && (
            <button type="button" className="search-clear" aria-label="Clear search" onClick={() => setQuery("")}>
              <X size={14} strokeWidth={2.25} aria-hidden="true" />
            </button>
          )}
        </label>
      </header>

      {downloads.length === 0 ? (
        <div className="empty">
          <h2>No history yet</h2>
          <p>Every link you download is kept here, even after its file is deleted.</p>
        </div>
      ) : (
        <div className="history" role="table" aria-label="Download history">
          <div className="history-head" role="row">
            <span role="columnheader">Name</span>
            <span role="columnheader">Link</span>
            <span role="columnheader" className="history-size">
              Size
            </span>
            <span role="columnheader">Added</span>
            <span role="columnheader">Status</span>
          </div>
          <div className="history-body" role="rowgroup">
            {shown.length === 0 ? (
              <p className="history-none">Nothing in History matches “{query.trim()}”.</p>
            ) : (
              shown.map((d) =>
                d.profile ? (
                  <HistoryGroup key={d.id} d={d} onMenu={props.onMenu} />
                ) : (
                  <HistoryRow key={d.id} d={d} onMenu={props.onMenu} />
                ),
              )
            )}
          </div>
        </div>
      )}
    </div>
  );
}
