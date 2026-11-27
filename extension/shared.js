// Used by both the popup and the background worker: where the app listens,
// and how a link is told to be a video, a sound file or a stream.

// The RushDownload app listens here, on this computer only
// (see src-tauri/src/extension.rs; the two port numbers must match).
const RUSH_APP = "http://127.0.0.1:38951";

const RUSH_MEDIA = /\.(mp4|m4v|webm|mov|mkv|flv|m3u8|mpd|mp3|m4a|aac|ogg|opus|wav|flac)(?:[?#]|$)/i;
const RUSH_AUDIO = ["mp3", "m4a", "aac", "ogg", "opus", "wav", "flac"];
// Pieces of a stream. The stream's playlist is what gets downloaded, not these.
const RUSH_PIECE = /\.(ts|m4s|cmfv|cmfa|m4f|frag)(?:[?#]|$)/i;

/** "video", "audio" or "stream" for an address, by its file type alone; null for anything else. */
function rushKindOf(url) {
  const ext = (RUSH_MEDIA.exec(url) || [])[1];
  if (!ext) return null;
  const lower = ext.toLowerCase();
  if (lower === "m3u8" || lower === "mpd") return "stream";
  return RUSH_AUDIO.includes(lower) ? "audio" : "video";
}

/** Whether the app is open. */
async function rushPing() {
  try {
    const answer = await fetch(RUSH_APP + "/ping", { cache: "no-store", signal: AbortSignal.timeout(1500) });
    return answer.ok && (await answer.json()).app === "RushDownload";
  } catch (_) {
    return false;
  }
}

/**
 * Hands links to the app. Each item is { url, kind, referer?, title? } where
 * kind is "video", "audio", "stream", "embed" or "page".
 * Resolves with { ok, added, pages, failed, error }.
 */
async function rushSend(items) {
  try {
    const answer = await fetch(RUSH_APP + "/add", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ agent: navigator.userAgent, items }),
      signal: AbortSignal.timeout(8000),
    });
    const said = await answer.json().catch(() => ({}));
    if (!answer.ok) return { ok: false, error: said.error || "The app refused those links" };
    return { ok: true, added: said.added || 0, pages: said.pages || 0, failed: said.failed || [] };
  } catch (_) {
    return { ok: false, error: "RushDownload is not open. Open the app, then send again." };
  }
}
