// A link typed on Home whose page the site downloader cannot read
// (e2e/video_page.py with a stream folder, /stream/demo). Home must show the
// same card as for other videos: picture, title and the video's qualities.
// The page is opened out of sight for that. Nothing downloads until a quality
// is picked and Download is pressed; the app stays on Home meanwhile.
const call = window.__TAURI_INTERNALS__.invoke;
await waitFor(() => $(".nav-item"), 60000, "the app to load");
const link = window.__link ?? "http://127.0.0.1:8766/stream/demo";
const known = new Set((await call("list_downloads")).map((d) => d.id));
click(byText(".nav-item", "Home"));
const box = await waitFor(() => $(".paste input"), 5000, "the Home box");
type(box, link);
const card = await waitFor(() => $(".video-card.is-caught"), 60000, "the card for the caught video");
await waitFor(() => $$(".video-choice", $(".video-card.is-caught")).length >= 2, 20000, "the qualities");
// long enough for an automatic download to have started, if there were one
await sleep(8000);
const shown = $(".video-card.is-caught");
const before = {
  pageNow: $(".nav-item.is-active")?.textContent.trim(),
  title: $(".video-title", shown).textContent.trim(),
  picture: $("img.video-thumb", shown)?.getAttribute("src")?.replace("http://127.0.0.1:8766", "") ?? null,
  qualities: $$(".video-choice", shown).map((b) => b.textContent.trim()),
  downloadsStartedByItself: (await call("list_downloads")).filter((d) => !known.has(d.id)).length,
};
// the user picks the smaller quality and presses Download
click($$(".video-choice", shown).find((b) => b.textContent.trim() === "480p"));
await sleep(300);
$(".paste").dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
let done = null;
for (let i = 0; i < 400 && !done; i++) {
  const mine = (await call("list_downloads")).filter((d) => !known.has(d.id));
  if (mine.some((d) => d.status === "failed")) return { ...before, failed: mine.find((d) => d.status === "failed").error };
  done = mine.find((d) => d.status === "completed") ?? null;
  if (!done) await sleep(250);
}
return {
  ...before,
  pageAfterDownload: $(".nav-item.is-active")?.textContent.trim(),
  downloaded: done && { filename: done.filename, bytes: done.downloaded },
};
