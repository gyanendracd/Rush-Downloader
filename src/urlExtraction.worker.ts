/* Runs the link finder (urlExtraction.ts) away from the window, so a large
 * file never holds the interface up. Stopping it (Cancel) ends the worker. */
import { extractUrls, type ExtractionSource } from "./urlExtraction";

export type WorkerRequest = { sources: ExtractionSource[] };
export type WorkerReply =
  | { kind: "progress"; done: number; total: number }
  | { kind: "done"; result: ReturnType<typeof extractUrls> }
  | { kind: "error"; message: string };

const scope = self as unknown as {
  onmessage: ((e: MessageEvent<WorkerRequest>) => void) | null;
  postMessage: (reply: WorkerReply) => void;
};

scope.onmessage = (e) => {
  const { sources } = e.data;
  try {
    const result = extractUrls(sources, (done) => scope.postMessage({ kind: "progress", done, total: sources.length }));
    scope.postMessage({ kind: "done", result });
  } catch (err) {
    scope.postMessage({ kind: "error", message: String(err) });
  }
};
