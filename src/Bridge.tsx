/* The browser extension's connection, seen from the app: the prompt that
 * allows an extension to connect, and Settings › Advanced › Browser
 * extension, which lists connected browsers and disconnects them
 * (src-tauri/src/extension.rs keeps the pairings). */
import { useEffect, useState } from "react";
import { api, type PairedExtension, type PairRequest } from "./api";
import { t } from "./i18n";
import { ago } from "./Tools";
import { Modal, useApp } from "./ui";

/** Asks the user about each extension that wants to connect, one at a time. */
export function PairPrompt() {
  const { actions } = useApp();
  const [asks, setAsks] = useState<PairRequest[]>([]);
  useEffect(() => {
    const stop = api.onExtensionPair((ask) => setAsks((list) => (list.some((a) => a.id === ask.id) ? list : [...list, ask])));
    return () => {
      void stop.then((f) => f());
    };
  }, []);
  const ask = asks[0];
  if (!ask) return null;

  async function answer(allow: boolean) {
    setAsks((list) => list.slice(1));
    try {
      await api.answerExtensionPair(ask.id, allow);
    } catch (e) {
      actions.toast(String(e), "error");
    }
  }

  return (
    <Modal
      title={t.bridge.askTitle}
      onClose={() => answer(false)}
      footer={
        <>
          <span className="spacer" />
          <button type="button" className="btn" data-autofocus onClick={() => answer(false)}>
            {t.bridge.deny}
          </button>
          <button type="button" className="btn btn-primary" onClick={() => answer(true)}>
            {t.bridge.allow}
          </button>
        </>
      }
    >
      <p className="confirm-name">{t.bridge.ask(ask.browser)}</p>
      <p className="confirm-detail mono selectable break">{ask.origin}</p>
      <p className="confirm-detail">{t.bridge.askNote}</p>
    </Modal>
  );
}

/** Settings › Advanced › Browser extension. */
export function ExtensionSettings() {
  const { actions } = useApp();
  const [paired, setPaired] = useState<PairedExtension[] | null>(null);
  useEffect(() => {
    let live = true;
    const load = () => api.pairedExtensions().then((list) => live && setPaired(list)).catch(() => live && setPaired([]));
    void load();
    // a browser that connects while this is open shows up
    const stop = api.onExtensionPair(() => setTimeout(load, 1500));
    return () => {
      live = false;
      void stop.then((f) => f());
    };
  }, []);

  return (
    <>
      <h3>{t.bridge.title}</h3>
      <div className="setting-card">
        {paired == null ? (
          <div className="setting-row" aria-busy="true" />
        ) : paired.length === 0 ? (
          <div className="setting-row">
            <small className="proxy-empty">{t.bridge.none}</small>
          </div>
        ) : (
          paired.map((p) => (
            <div className="setting-row proxy-row" key={p.id}>
              <div className="setting-text">
                <span>{p.browser}</span>
                <small className="mono">{p.origin}</small>
                <small>
                  {t.bridge.connected(ago(p.created_at))} · {p.last_seen ? t.bridge.lastSeen(ago(p.last_seen)) : t.bridge.neverSeen}
                </small>
              </div>
              <div className="setting-control">
                <button
                  type="button"
                  className="btn btn-small"
                  onClick={async () => {
                    setPaired(await api.revokeExtension(p.id));
                    actions.toast(t.bridge.revoked);
                  }}
                >
                  {t.bridge.revoke}
                </button>
              </div>
            </div>
          ))
        )}
      </div>
      <p className="proxy-limits">{t.bridge.hint}</p>
    </>
  );
}
