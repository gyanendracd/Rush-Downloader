/* Settings › Network › Proxy: the saved proxies, the default connection and
 * the dialog that adds, changes, tests and deletes a proxy. Also the
 * connection picker of a new download. The engine keeps the proxies
 * (crates/engine/src/proxy.rs); their passwords go to the system's
 * credential store and never come back to the interface. */
import { useEffect, useRef, useState, type FormEvent } from "react";
import { createPortal } from "react-dom";
import { Plus } from "lucide-react";
import {
  api,
  PROXY_KINDS,
  type PasswordChange,
  type ProxyBook,
  type ProxyChoice,
  type ProxyKind,
  type ProxyProfile,
  type ProxyTest,
} from "./api";
import { num, t } from "./i18n";
import { Modal, Select } from "./ui";

/** The saved proxies, loaded once; null until known. */
export function useProxies(): [ProxyBook | null, (book: ProxyBook) => void] {
  const [book, setBook] = useState<ProxyBook | null>(null);
  useEffect(() => {
    let live = true;
    api
      .proxies()
      .then((b) => live && setBook(b))
      .catch(() => live && setBook({ next_id: 1, default: null, items: [] }));
    return () => {
      live = false;
    };
  }, []);
  return [book, setBook];
}

const address = (p: ProxyProfile) => (p.host.includes(":") ? `[${p.host}]:${p.port}` : `${p.host}:${p.port}`);
const isSocks4 = (kind: ProxyKind) => kind === "socks4" || kind === "socks4a";

export function ProxySettings() {
  const [book, setBook] = useProxies();
  /** The proxy being edited: a saved one, or a new one (id 0). */
  const [editing, setEditing] = useState<ProxyProfile | null>(null);
  const [error, setError] = useState("");

  if (!book) return <div className="setting-card" aria-busy="true" />;

  async function setDefault(value: string) {
    setError("");
    try {
      setBook(await api.setDefaultProxy(value === "direct" ? null : Number(value)));
    } catch (e) {
      setError(String(e));
    }
  }

  const blank: ProxyProfile = { id: 0, name: "", kind: "http", host: "", port: 8080, username: "", has_password: false, bypass: [] };

  return (
    <>
      <h3>{t.proxy.title}</h3>
      <div className="setting-card proxy-card">
        <div className="setting-row">
          <div className="setting-text">
            <span id="set-proxy-default">{t.proxy.defaultConnection}</span>
            <small>{t.proxy.defaultHint}</small>
          </div>
          <div className="setting-control">
            <Select
              aria-labelledby="set-proxy-default"
              value={book.default == null ? "direct" : String(book.default)}
              onChange={setDefault}
              options={[
                { value: "direct", label: t.proxy.direct },
                ...book.items.map((p) => ({ value: String(p.id), label: p.name })),
              ]}
            />
          </div>
        </div>
        {book.items.length === 0 && (
          <div className="setting-row proxy-empty">
            <small>{t.proxy.none}</small>
          </div>
        )}
        {book.items.map((p) => (
          <div className="setting-row proxy-row" key={p.id}>
            <div className="setting-text">
              <span>
                {p.name}
                {book.default === p.id && <span className="soon">{t.proxy.isDefault}</span>}
              </span>
              <small>
                {t.proxy.where(t.proxy.kinds[p.kind], address(p))}
                {p.username && ` · ${p.username}`}
                {p.has_password && ` · ${t.proxy.passwordSaved}`}
              </small>
            </div>
            <div className="setting-control">
              <button type="button" className="btn btn-small" onClick={() => setEditing(p)} aria-label={`${t.proxy.edit} ${p.name}`}>
                {t.proxy.edit}
              </button>
            </div>
          </div>
        ))}
        <div className="setting-row proxy-foot">
          <button type="button" className="btn btn-small" onClick={() => setEditing(blank)}>
            <Plus size={14} strokeWidth={2.2} aria-hidden="true" /> {t.proxy.add}
          </button>
        </div>
      </div>
      {error && <p className="field-error proxy-error" role="alert">{error}</p>}
      <p className="proxy-limits">{t.proxy.limits}</p>
      {editing &&
        // At the top of the page: the Settings panel would clip a dialog inside it.
        createPortal(
        <ProxyEditor
          initial={editing}
          isDefault={book.default === editing.id && editing.id !== 0}
          onClose={() => setEditing(null)}
          onSaved={(next) => {
            setBook(next);
            setEditing(null);
          }}
        />,
          document.body,
        )}
    </>
  );
}

function ProxyEditor(props: {
  initial: ProxyProfile;
  isDefault: boolean;
  onClose: () => void;
  onSaved: (book: ProxyBook) => void;
}) {
  const saved = props.initial.id !== 0;
  const [name, setName] = useState(props.initial.name);
  const [kind, setKind] = useState<ProxyKind>(props.initial.kind);
  const [host, setHost] = useState(props.initial.host);
  const [port, setPort] = useState(String(props.initial.port));
  const [username, setUsername] = useState(props.initial.username);
  /** A new password typed in; empty keeps (or leaves out) the saved one. */
  const [password, setPassword] = useState("");
  const [clearSaved, setClearSaved] = useState(false);
  const [bypass, setBypass] = useState(props.initial.bypass.join("\n"));
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [test, setTest] = useState<ProxyTest | "running" | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  /** The Delete button, given focus back when its question is cancelled
   *  (the buttons that had it are gone, and Escape would stop working). */
  const deleteButton = useRef<HTMLButtonElement>(null);
  const asked = useRef(false);
  useEffect(() => {
    if (confirmDelete) asked.current = true;
    else if (asked.current) deleteButton.current?.focus();
  }, [confirmDelete]);
  const hasSaved = props.initial.has_password && !clearSaved;

  const profile = (): ProxyProfile => ({
    id: props.initial.id,
    name,
    kind,
    host,
    port: Math.round(Number(port)) || 0,
    username,
    has_password: false,
    bypass: bypass.split(/\r?\n/).map((l) => l.trim()).filter(Boolean),
  });
  const change = (): PasswordChange => {
    if (isSocks4(kind)) return props.initial.has_password ? "clear" : "keep";
    if (password) return { set: password };
    return clearSaved ? "clear" : "keep";
  };

  async function save(e: FormEvent) {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      props.onSaved(await api.saveProxy(profile(), change()));
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  }

  async function runTest() {
    setTest("running");
    try {
      setTest(await api.testProxy(profile(), change()));
    } catch (err) {
      setTest({ ok: false, url: "", status: null, millis: 0, error: String(err) });
    }
  }

  async function remove() {
    setBusy(true);
    setError("");
    try {
      props.onSaved(await api.deleteProxy(props.initial.id));
    } catch (err) {
      setError(String(err));
      setConfirmDelete(false);
      setBusy(false);
    }
  }

  const ed = t.proxy.editor;
  const ms = (n: number) => (n < 1000 ? `${num(n)} ms` : `${num(Math.round(n / 100) / 10)} s`);

  return (
    <Modal
      title={saved ? ed.editTitle : ed.addTitle}
      onClose={props.onClose}
      footer={
        confirmDelete ? (
          <>
            <span className="proxy-confirm">
              {ed.deleteConfirm(props.initial.name)} {props.isDefault && ed.deleteDefault}
            </span>
            <span className="spacer" />
            <button type="button" className="btn" data-autofocus onClick={() => setConfirmDelete(false)}>
              {t.dialogs.cancel}
            </button>
            <button type="button" className="btn btn-danger-solid" disabled={busy} onClick={remove}>
              {ed.delete}
            </button>
          </>
        ) : (
          <>
            {saved && (
              <button type="button" className="btn" ref={deleteButton} onClick={() => setConfirmDelete(true)}>
                {ed.delete}
              </button>
            )}
            <button type="button" className="btn" disabled={test === "running"} onClick={runTest}>
              {test === "running" ? ed.testing : ed.test}
            </button>
            <span className="spacer" />
            <button type="button" className="btn" onClick={props.onClose}>
              {t.dialogs.cancel}
            </button>
            <button type="submit" form="proxy-form" className="btn btn-primary" disabled={busy}>
              {ed.save}
            </button>
          </>
        )
      }
    >
      <form id="proxy-form" className="form" onSubmit={save} autoComplete="off">
        <label className="field">
          <span>{ed.name}</span>
          <input data-autofocus type="text" value={name} placeholder={ed.namePlaceholder} maxLength={60} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="field">
          <span id="proxy-kind">{ed.protocol}</span>
          <Select
            aria-labelledby="proxy-kind"
            value={kind}
            onChange={setKind}
            options={PROXY_KINDS.map((k) => ({ value: k, label: t.proxy.kinds[k] }))}
          />
          <small>{t.proxy.kindHints[kind]}</small>
        </label>
        <div className="proxy-address">
          <label className="field">
            <span>{ed.host}</span>
            <input type="text" value={host} placeholder={ed.hostPlaceholder} spellCheck={false} onChange={(e) => setHost(e.target.value)} />
          </label>
          <label className="field">
            <span>{ed.port}</span>
            <input type="number" min={1} max={65535} value={port} onChange={(e) => setPort(e.target.value)} />
          </label>
        </div>
        <div className="proxy-address">
          <label className="field">
            <span>{ed.username}</span>
            <input type="text" value={username} placeholder={ed.optional} spellCheck={false} autoComplete="off" onChange={(e) => setUsername(e.target.value)} />
          </label>
          <label className="field">
            <span>{ed.password}</span>
            <input
              type="password"
              value={password}
              disabled={isSocks4(kind)}
              placeholder={hasSaved ? ed.passwordKept : ed.optional}
              autoComplete="new-password"
              onChange={(e) => setPassword(e.target.value)}
            />
          </label>
        </div>
        {isSocks4(kind) ? (
          <small className="form-note">{ed.socks4NoPassword}</small>
        ) : (
          <small className="form-note">
            {ed.passwordStore}{" "}
            {hasSaved && !password && (
              <button type="button" className="link" onClick={() => setClearSaved(true)}>
                {ed.clearPassword}
              </button>
            )}
          </small>
        )}
        <label className="field">
          <span>{ed.bypass}</span>
          <textarea rows={3} value={bypass} spellCheck={false} placeholder={"example.com\n10.0.0.0/8\n<local>"} onChange={(e) => setBypass(e.target.value)} />
          <small>{ed.bypassHint}</small>
        </label>
        {test && test !== "running" && (
          <p className={`proxy-test ${test.ok ? "is-ok" : "is-bad"}`} role="status">
            {test.ok ? ed.testOk(test.status ?? 0, ms(test.millis)) : ed.testFailed(test.error ?? "")}
          </p>
        )}
        {error && (
          <small className="field-error" role="alert">
            {error}
          </small>
        )}
      </form>
    </Modal>
  );
}

const choiceValue = (c: ProxyChoice | undefined) => (c === undefined || c === "default" ? "default" : c === "direct" ? "direct" : String(c.profile));

/** The connection a new download uses. Shown only once a proxy is saved:
 *  until then every download connects directly and there is nothing to pick. */
export function ConnectionField(props: { value: ProxyChoice; onChange: (choice: ProxyChoice) => void }) {
  const [book] = useProxies();
  if (!book || book.items.length === 0) return null;
  const fallback = book.items.find((p) => p.id === book.default)?.name ?? t.proxy.directShort;
  return (
    <label className="field">
      <span id="add-connection">{t.proxy.connection}</span>
      <Select
        aria-labelledby="add-connection"
        value={choiceValue(props.value)}
        onChange={(v) => props.onChange(v === "default" ? "default" : v === "direct" ? "direct" : { profile: Number(v) })}
        options={[
          { value: "default", label: t.proxy.useDefault(fallback) },
          { value: "direct", label: t.proxy.direct },
          ...book.items.map((p) => ({ value: String(p.id), label: p.name })),
        ]}
      />
    </label>
  );
}
