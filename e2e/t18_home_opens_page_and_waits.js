// A link pasted on Home that cannot be downloaded from the link alone
// (e2e/video_page.py on port 8766). The app opens the page on the Browser page
// and starts the player by itself, so the video's qualities show up; then it
// WAITS. Nothing may download until the user picks a quality and presses
// Download. `window.__link` picks the page: /click/demo only loads its video
// after "play" is pressed; /stream/demo plays a stream.
const call = window.__TAURI_INTERNALS__.invoke;
await waitFor(() => $(".nav-item"), 60000, "the app to load");
const link = window.__link ?? "http://127.0.0.1:8766/click/demo";
const known = new Set((await call("list_downloads")).map((d) => d.id));
click(byText(".nav-item", "Home"));
const box = await waitFor(() => $(".paste input"), 5000, "the Home box");
type(box, link);
await sleep(300);
$(".paste").dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
const caughtRow = await waitFor(() => $(".browser-page-video.is-caught"), 60000, "the video's row on the Browser page");
// long enough for an automatic download to have started, if there were one
await sleep(12000);
const stillRow = $(".browser-page-video.is-caught");
const fresh = (await call("list_downloads")).filter((d) => !known.has(d.id));
const waiting = {
  link: link.replace("http://127.0.0.1:8766", ""),
  pageNow: $(".nav-item.is-active")?.textContent.trim(),
  qualities: $$("select option", stillRow ?? caughtRow).map((o) => o.textContent.trim()),
  downloadsStartedByItself: fresh.length,
  stillFinding: !!$(".browser-note"),
};
// now the user chooses the last quality in the list and presses Download
const select = $("select", stillRow);
const last = $$("option", select).pop();
choose(select, last.value);
await sleep(300);
click($(".browser-page-video.is-caught .btn"));
let done = null;
for (let i = 0; i < 400 && !done; i++) {
  const mine = (await call("list_downloads")).filter((d) => !known.has(d.id));
  if (mine.some((d) => d.status === "failed")) return { ...waiting, failed: mine.find((d) => d.status === "failed").error };
  done = mine.find((d) => d.status === "completed") ?? null;
  if (!done) await sleep(250);
}
click(byText(".nav-item", "Home"));
return { ...waiting, picked: last.textContent.trim(), downloaded: done && { filename: done.filename, bytes: done.downloaded } };
