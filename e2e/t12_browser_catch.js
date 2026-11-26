// The Browser page on a page the site downloader cannot read (its player is
// built by script; e2e/video_page.py on port 8766): the video is caught as it
// plays and shown as ONE row with its qualities in a list; the chosen one downloads.
await waitFor(() => $(".nav-item"), 60000, "the app to load");
click(byText(".nav-item", "Browser"));
const box = await waitFor(() => $(".browser-bar input"), 5000, "the address box");
type(box, "http://127.0.0.1:8766/play/demo");
$(".browser-bar").dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
const caughtRow = await waitFor(() => $(".browser-page-video.is-caught"), 20000, "the caught video row");
await waitFor(() => $$("select option", caughtRow).length >= 4, 20000, "all four qualities in the list");
await waitFor(() => $$("select option", caughtRow).every((o) => !o.textContent.includes("…")), 15000, "every size to arrive");
const caught = {
  rows: $$(".browser-found .browser-page-video").length,
  shown: $(".browser-name", caughtRow).textContent.trim(),
  chosen: $("select", caughtRow).selectedOptions[0].textContent.trim(),
  qualities: $$("select option", caughtRow).map((o) => o.textContent.trim()),
};
const first = caughtRow;
const stage = $(".signin-stage").getBoundingClientRect();
click($(".btn", first));
await waitFor(() => byText(".browser-found .btn", "In downloads"), 8000, "the download to start");
const call = window.__TAURI_INTERNALS__.invoke;
let done = null;
for (let i = 0; i < 300 && !done; i++) {
  const list = await call("list_downloads");
  const item = list[list.length - 1];
  if (item && item.status === "failed") return { caught, failed: item.error };
  if (item && item.status === "completed") done = item;
  else await sleep(200);
}
click(byText(".nav-item", "Home"));
return {
  caught,
  address: box.value,
  stage: { width: Math.round(stage.width), height: Math.round(stage.height) },
  downloaded: done && { filename: done.filename, bytes: done.downloaded, viaSite: done.media },
};
