// A page whose video is a stream (e2e/video_page.py with a stream folder,
// /stream/demo): the playlist comes from an address that does not look like
// one, and a small piece of the stream is fetched as an .mp4. The Browser page
// must show ONE row whose list holds the stream's qualities and nothing else,
// and the chosen quality must download as a whole video.
const call = window.__TAURI_INTERNALS__.invoke;
await waitFor(() => $(".nav-item"), 60000, "the app to load");
click(byText(".nav-item", "Home"));
await sleep(300);
click(byText(".nav-item", "Browser"));
const box = await waitFor(() => $(".browser-bar input"), 5000, "the address box");
type(box, "http://127.0.0.1:8766/stream/demo");
$(".browser-bar").dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
const caughtRow = await waitFor(() => $(".browser-page-video.is-caught"), 20000, "the caught video row");
await waitFor(() => $$("select option", caughtRow).some((o) => /^\d+p$/.test(o.textContent.trim())), 15000, "the stream's qualities");
await sleep(3000);
const qualities = $$("select option", caughtRow).map((o) => o.textContent.trim());
const before = (await call("list_downloads")).length;
// pick the smaller quality, to see that the pick is what downloads
const select = $("select", caughtRow);
const small = $$("option", select).find((o) => o.textContent.trim() === (window.__pick ?? "480p"));
choose(select, small.value);
await sleep(300);
click($(".browser-page-video.is-caught .btn"));
let done = null;
for (let i = 0; i < 400 && !done; i++) {
  const list = await call("list_downloads");
  const item = list[list.length - 1];
  if (list.length > before && item.status === "failed") return { qualities, failed: item.error };
  if (list.length > before && item.status === "completed") done = item;
  else await sleep(250);
}
click(byText(".nav-item", "Home"));
return {
  rows: 1,
  qualities,
  downloaded: done && { filename: done.filename, bytes: done.downloaded, path: done.path },
};
