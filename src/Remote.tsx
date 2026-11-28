/* FTP and SFTP downloads: the sign-in part of the add dialog, and the
 * server key a download was refused for, to trust from its details
 * (crates/engine/src/remote.rs). */
import { useState } from "react";
import { ShieldAlert } from "lucide-react";
import { api, type Download, type RemoteOptions } from "./api";
import { t } from "./i18n";
import { useApp } from "./ui";

export const BLANK_REMOTE: RemoteOptions = { username: "", password: null, key_file: null, active_mode: false, connect_timeout: null };

/** The sign-in fields, shown for an FTP or SFTP link. */
export function RemoteField(props: { kind: "ftp" | "sftp"; value: RemoteOptions; onChange: (v: RemoteOptions) => void }) {
  const v = props.value;
  const set = (patch: Partial<RemoteOptions>) => props.onChange({ ...v, ...patch });
  const r = t.remote;
  return (
    <fieldset className="remote-field">
      <legend>{r.title}</legend>
      <div className="request-pair">
        <label className="field">
          <span>{r.username}</span>
          <input type="text" value={v.username} autoComplete="off" spellCheck={false} placeholder={props.kind === "ftp" ? r.optional : ""} onChange={(e) => set({ username: e.target.value })} />
        </label>
        <label className="field">
          <span>{v.key_file ? r.passphrase : r.password}</span>
          <input type="password" value={v.password ?? ""} autoComplete="new-password" placeholder={v.key_file ? r.optional : ""} onChange={(e) => set({ password: e.target.value || null })} />
        </label>
      </div>
      {props.kind === "sftp" && (
        <div className="field">
          <span>{r.keyFile}</span>
          <div className="field-row">
            <input type="text" readOnly value={v.key_file ?? r.keyNone} aria-label={r.keyFile} />
            <button
              type="button"
              className="btn"
              onClick={async () => {
                const picked = await api.pickAnyFile(r.pickKeyTitle);
                if (picked) set({ key_file: picked });
              }}
            >
              {r.chooseKey}
            </button>
          </div>
          {v.key_file && (
            <button type="button" className="link request-clear" onClick={() => set({ key_file: null })}>
              {r.clearKey}
            </button>
          )}
        </div>
      )}
      {props.kind === "ftp" && (
        <label className="check-row">
          <input type="checkbox" checked={v.active_mode} onChange={(e) => set({ active_mode: e.target.checked })} />
          <span>
            {r.activeMode}
            <small>{r.activeHint}</small>
          </span>
        </label>
      )}
      <small className="form-note">{props.kind === "ftp" ? r.ftpHint : r.sftpHint} {r.passwordStore}</small>
    </fieldset>
  );
}

/** A refused SFTP server key, with the way to trust it. */
export function ServerKeyPanel({ d }: { d: Download }) {
  const { actions } = useApp();
  const [busy, setBusy] = useState(false);
  const key = d.server_key;
  if (!key) return null;
  const r = t.remote;
  const where = key.port === 22 ? key.host : `${key.host}:${key.port}`;
  return (
    <section className={`server-key${key.changed ? " is-changed" : ""}`} aria-label={key.changed ? r.keyChangedTitle : r.keyTitle}>
      <h3>
        <ShieldAlert size={15} strokeWidth={2.2} aria-hidden="true" /> {key.changed ? r.keyChangedTitle : r.keyTitle}
      </h3>
      <p>{key.changed ? r.keyChangedExplain(where) : r.keyExplain(where)}</p>
      <dl className="checksum-sums">
        <div>
          <dt>{r.fingerprint}</dt>
          <dd className="selectable break mono">{key.fingerprint}</dd>
        </div>
      </dl>
      <button
        type="button"
        className={`btn btn-small ${key.changed ? "btn-danger-solid" : "btn-primary"}`}
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          try {
            await api.trustServerKey(d.id);
          } catch (e) {
            actions.toast(String(e), "error");
            setBusy(false);
          }
        }}
      >
        {key.changed ? r.trustNew : r.trust}
      </button>
    </section>
  );
}
