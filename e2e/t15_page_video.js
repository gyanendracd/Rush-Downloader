// The Browser page on a page the site downloader understands: a "Full video"
// row with the page's title and resolutions appears above the caught files,
// small caught files are marked as preview clips, and the full video downloads.
// Needs e2e/video_page.py on port 8766.
await waitFor(() => $(".nav-item"), 60000, "the app to load");
click(byText(".nav-item", "Home"));
await sleep(300);
click(byText(".nav-item", "Browser"));
const box = await waitFor(() => $(".browser-bar input"), 5000, "the address box");
type(box, "http://127.0.0.1:8766/watch/demo");
$(".browser-bar").dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
const full = await waitFor(() => $(".browser-page-video:not(.is-caught)"), 40000, "the full video row");
await sleep(6000);
const before = {
  fullVideo: $(".browser-name", full).textContent.trim(),
  resolutions: $$("select option", full).map((o) => o.textContent.trim()),
  rows: $$(".browser-found .browser-page-video").length,
  caughtListShown: $$(".browser-found li, .browser-page-video.is-caught").length > 0,
};
click($(".btn", full));
await waitFor(() => byText(".browser-page-video .btn", "In downloads"), 8000, "the download to start");
const call = window.__TAURI_INTERNALS__.invoke;
let done = null;
for (let i = 0; i < 300 && !done; i++) {
  const list = await call("list_downloads");
  const item = list[list.length - 1];
  if (item && item.status === "failed") return { ...before, failed: item.error };
  if (item && item.status === "completed") done = item;
  else await sleep(200);
}
click(byText(".nav-item", "Home"));
return { ...before, downloaded: done && { filename: done.filename, bytes: done.downloaded } };
