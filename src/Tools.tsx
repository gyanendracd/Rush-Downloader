/* Settings › Advanced › Engines: the helper programs site downloads need.
 * yt-dlp and gallery-dl are installed, checked and updated by the engine
 * (crates/engine/src/deps.rs); FFmpeg is only looked for. Progress arrives
 * as "tools" events, so the buttons start work and the list follows it. */
import { useEffect, useState } from "react";
import { api, formatBytes, type Settings, type ToolInfo, type ToolStatus } from "./api";
import { formatLocale, t } from "./i18n";

/** "3 hours ago", in the language of the interface. */
export function ago(unixSeconds: number): string {
  const seconds = Math.round(unixSeconds - Date.now() / 1000);
  const steps: [Intl.RelativeTimeFormatUnit, number][] = [
    ["day", 86400],
    ["hour", 3600],
    ["minute", 60],
  ];
  const format = new Intl.RelativeTimeFormat(formatLocale(), { numeric: "auto" });
  for (const [unit, size] of steps) {
    if (Math.abs(seconds) >= size) return format.format(Math.round(seconds / size), unit);
  }
  return format.format(0, "minute");
}

function progressText(tool: ToolInfo): string {
  const busy = tool.busy!;
  const ph = t.tools.phase;
  switch (busy.phase) {
    case "checking":
      return ph.checking;
    case "downloading":
      return busy.total ? ph.downloading(formatBytes(busy.done), formatBytes(busy.total)) : ph.downloadingUnsized(formatBytes(busy.done));
    case "verifying":
      return ph.verifying;
    case "installing":
      return ph.installing;
  }
}

function ToolRow(props: { tool: ToolInfo; onError: (why: string) => void }) {
  const { tool } = props;
  const [starting, setStarting] = useState(false);
  const busy = tool.busy != null || starting;

  async function run(work: () => Promise<unknown>) {
    setStarting(true);
    try {
      await work();
    } catch (e) {
      props.onError(String(e));
    } finally {
      setStarting(false);
    }
  }

  const where =
    tool.source === "managed" ? t.tools.managed : tool.source === "system" && tool.path ? t.tools.system(tool.path) : t.tools.missing;
  const versionLine = [
    tool.version ? t.tools.version(tool.version) : tool.source !== "missing" ? t.tools.unknownVersion : null,
    tool.latest && (tool.update_available ? t.tools.latest(tool.latest) : tool.source !== "missing" ? t.tools.upToDate : null),
    tool.last_checked ? t.tools.checkedAgo(ago(tool.last_checked)) : t.tools.never,
  ]
    .filter(Boolean)
    .join(" · ");

  let action: { label: string; title?: string } | null = null;
  if (tool.can_install) {
    if (tool.source === "missing") action = { label: t.tools.install };
    else if (tool.source === "managed" && tool.update_available && tool.latest) action = { label: t.tools.update(tool.latest) };
    else if (tool.source === "system") action = { label: t.tools.useOwn, title: t.tools.useOwnHint };
  }
  const pct = tool.busy?.total ? Math.min(100, (tool.busy.done / tool.busy.total) * 100) : null;

  return (
    <div className="setting-row tool-row" aria-busy={busy}>
      <div className="setting-text">
        <span>
          {tool.name}
        </span>
        <small>{where}</small>
        <small>{versionLine}</small>
        {!tool.can_install && tool.source === "missing" && <small>{t.tools.noBuild}</small>}
        {tool.busy && (
          <>
            <small role="status">{progressText(tool)}</small>
            <div className={`bar tool-bar${pct == null ? " bar-busy bar-downloading" : ""}`}>
              {pct != null && <div className="fill" style={{ width: `${pct}%` }} />}
            </div>
          </>
        )}
        {tool.error && !tool.busy && <small className="tool-error">{tool.error}</small>}
        <span className="tool-links">
          {tool.notes_url && (
            <button type="button" className="link" onClick={() => run(() => api.openReleaseNotes(tool.name))}>
              {t.tools.notes}
            </button>
          )}
          {tool.previous && tool.source === "managed" && (
            <button type="button" className="link" disabled={busy} onClick={() => run(() => api.rollbackTool(tool.name))}>
              {t.tools.rollback(tool.previous)}
            </button>
          )}
        </span>
      </div>
      {action && (
        <div className="setting-control">
          <button
            type="button"
            className={`btn btn-small${tool.source === "missing" || tool.update_available ? " btn-primary" : ""}`}
            title={action.title}
            disabled={busy}
            onClick={() => run(() => api.installTool(tool.name))}
          >
            {action.label}
          </button>
        </div>
      )}
    </div>
  );
}

export function ToolSettings(props: { settings: Settings; onSettings: (s: Settings) => void; ffmpeg: ToolStatus | null }) {
  const [tools, setTools] = useState<ToolInfo[] | null>(null);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    let live = true;
    api.toolsInfo().then((list) => live && setTools(list)).catch(() => live && setTools([]));
    const stop = api.onTools((list) => live && setTools(list));
    return () => {
      live = false;
      void stop.then((unlisten) => unlisten());
    };
  }, []);

  async function check() {
    setChecking(true);
    setError("");
    try {
      setTools(await api.checkTools());
    } catch (e) {
      setError(String(e));
    } finally {
      setChecking(false);
    }
  }

  return (
    <>
      <h3>{t.tools.title}</h3>
      <div className="setting-card">
        <label className="setting-row toggle">
          <span className="setting-text">
            <span>{t.tools.auto}</span>
            <small>{t.tools.autoHint}</small>
          </span>
          <input
            type="checkbox"
            role="switch"
            checked={props.settings.auto_update_tools}
            onChange={(e) => props.onSettings({ ...props.settings, auto_update_tools: e.target.checked })}
          />
          <span className="toggle-dot" aria-hidden="true" />
        </label>
        {tools == null ? (
          <div className="setting-row" aria-busy="true" />
        ) : (
          tools.map((tool) => <ToolRow key={tool.name} tool={tool} onError={setError} />)
        )}
        <div className="setting-row">
          <div className="setting-text">
            <span>{t.tools.ffmpeg}</span>
            <small>{t.tools.ffmpegHint}</small>
            <small>{props.ffmpeg == null ? "…" : props.ffmpeg.ffmpeg ? t.tools.ffmpegFound : t.tools.ffmpegMissing}</small>
          </div>
        </div>
        <div className="setting-row tool-foot">
          <button type="button" className="btn btn-small" disabled={checking || tools?.some((x) => x.busy)} onClick={check}>
            {checking ? t.tools.checking : t.tools.check}
          </button>
        </div>
      </div>
      {error && (
        <p className="tool-error tool-error-line" role="alert">
          {error}
        </p>
      )}
      <p className="tool-note">{t.tools.verified}</p>
    </>
  );
}
