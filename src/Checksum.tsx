/* A finished file's checksum: the Integrity part of a download's details,
 * the field of the add dialog, and checking a batch against a sums file.
 * The engine works the sums out (crates/engine/src/checksum.rs); progress
 * and the outcome arrive with the download list. */
import { useState } from "react";
import { ShieldAlert, ShieldCheck, ShieldQuestion } from "lucide-react";
import { api, formatBytes, HASH_ALGOS, type Download, type HashAlgo } from "./api";
import { t } from "./i18n";
import { Select, useApp } from "./ui";

/** Whether a download is one file whose checksum can be worked out. */
export function checkable(d: Download): boolean {
  return !d.torrent && !d.profile && !d.bundle && !d.batch;
}

export function ChecksumField(props: { value: string; onChange: (text: string) => void }) {
  return (
    <label className="field">
      <span>{t.checksum.field}</span>
      <input
        type="text"
        value={props.value}
        placeholder={t.checksum.fieldPlaceholder}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => props.onChange(e.target.value)}
      />
      <small>{t.checksum.fieldHint}</small>
    </label>
  );
}

/** The Integrity part of a finished download's details. */
export function ChecksumPanel({ d }: { d: Download }) {
  const { actions } = useApp();
  const c = d.checksum ?? null;
  const [pasted, setPasted] = useState("");
  const [algorithm, setAlgorithm] = useState<HashAlgo>(c?.algorithm ?? "sha256");
  const [error, setError] = useState("");
  if (d.status !== "completed" || !checkable(d)) return null;

  async function verify(expected: string | null) {
    setError("");
    try {
      await api.verifyChecksum(d.id, expected, algorithm);
      setPasted("");
    } catch (e) {
      setError(String(e));
    }
  }

  const calculating = c?.state === "calculating";
  const tone = c?.state === "verified" ? "ok" : c?.state === "failed" || c?.state === "error" ? "bad" : "plain";
  const Icon = c?.state === "verified" ? ShieldCheck : c?.state === "failed" || c?.state === "error" ? ShieldAlert : ShieldQuestion;

  return (
    <section className="checksum" aria-label={t.checksum.title}>
      <div className="checksum-head">
        <h3>{t.checksum.title}</h3>
        <span className={`checksum-state is-${tone}`} role="status">
          <Icon size={14} strokeWidth={2.2} aria-hidden="true" />
          {c ? `${t.checksum.algorithms[c.algorithm]} · ${t.checksum.states[c.state]}` : t.checksum.states.not_verified}
        </span>
      </div>
      {calculating && c && d.total ? (
        <div className="checksum-progress">
          <div className="bar">
            <div className="fill" style={{ width: `${Math.min(100, (c.done / d.total) * 100)}%` }} />
          </div>
          <small>{t.checksum.progress(formatBytes(c.done), formatBytes(d.total))}</small>
        </div>
      ) : null}
      {c?.state === "failed" && <p className="checksum-note is-bad">{t.checksum.failedNote}</p>}
      {c?.state === "error" && c.error && <p className="checksum-note is-bad">{c.error}</p>}
      {c?.state === "not_verified" && c.actual && !c.expected && <p className="checksum-note">{t.checksum.calculatedOnly}</p>}
      {c && (c.expected || c.actual) && (
        <dl className="checksum-sums">
          {c.expected && (
            <div>
              <dt>{t.checksum.expected}</dt>
              <dd className="selectable break mono">{c.expected}</dd>
            </div>
          )}
          {c.actual && (
            <div>
              <dt>{t.checksum.calculated}</dt>
              <dd className="selectable break mono">{c.actual}</dd>
            </div>
          )}
        </dl>
      )}
      <form
        className="checksum-form"
        onSubmit={(e) => {
          e.preventDefault();
          void verify(pasted.trim() || null);
        }}
      >
        <input
          type="text"
          value={pasted}
          placeholder={t.checksum.paste}
          aria-label={t.checksum.paste}
          spellCheck={false}
          autoComplete="off"
          disabled={calculating}
          onChange={(e) => {
            setPasted(e.target.value);
            setError("");
          }}
        />
        <Select
          aria-label={t.checksum.field}
          value={algorithm}
          onChange={setAlgorithm}
          disabled={calculating}
          options={HASH_ALGOS.map((a) => ({ value: a, label: t.checksum.algorithms[a] }))}
        />
        <button type="submit" className="btn btn-small" disabled={calculating}>
          {pasted.trim() ? t.checksum.verify : c?.actual ? t.checksum.recalculate : t.checksum.calculate}
        </button>
      </form>
      {error && <small className="field-error">{error}</small>}
      {c?.actual && (
        <button type="button" className="link checksum-copy" onClick={() => actions.copy(c.actual!, t.checksum.calculated)}>
          {t.checksum.copy}
        </button>
      )}
    </section>
  );
}

/** Checks a batch's files against a sums file picked from disk. */
export function SumsButton({ id }: { id: number }) {
  const { actions } = useApp();
  return (
    <button
      type="button"
      className="btn btn-small"
      onClick={async () => {
        try {
          const [path] = await api.pickTextFiles();
          if (!path) return;
          const file = await api.readTextFile(path);
          const n = await api.verifyWithSums(id, file.text);
          actions.toast(t.checksum.sumsChecked(n));
        } catch (e) {
          actions.toast(String(e), "error");
        }
      }}
    >
      {t.checksum.sumsFile}
    </button>
  );
}
