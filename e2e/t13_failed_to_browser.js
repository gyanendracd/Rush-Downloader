// A page link that cannot be downloaded from the link alone: its row's menu
// offers "Open on Browser page", which opens that page there, ready to catch
// a playing video. Needs e2e/video_page.py on port 8766.
const call = window.__TAURI_INTERNALS__.invoke;
await waitFor(() => $(".nav-item"), 60000, "the app to load");
const link = "http://127.0.0.1:8766/posts/no-video-here";
await call("add_download", { request: { url: link, media: { max_height: null, audio_only: false } } }).catch(() =>
  call("add_download", { new: { url: link, media: { max_height: null, audio_only: false } } }),
);
click(byText(".nav-item", "Downloads"));
let failed = null;
for (let i = 0; i < 300 && !failed; i++) {
  const list = await call("list_downloads");
  failed = list.find((d) => d.url === link && d.status === "failed");
  if (!failed) await sleep(200);
}
if (!failed) return { problem: "the link did not fail" };
const target = await waitFor(() => $$(".row").find((r) => r.textContent.includes("no-video-here")), 5000, "the failed row");
target.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 400, clientY: 200 }));
const item = await waitFor(() => byText("button, [role=menuitem]", "Open on Browser page"), 5000, "the menu item");
click(item);
const box = await waitFor(() => $(".browser-bar input"), 5000, "the Browser page");
await waitFor(() => $(".signin-stage.is-open"), 8000, "the page to open");
await sleep(1500);
const result = { error: failed.error, address: box.value, pageOpen: !!$(".signin-stage.is-open") };
click(byText(".nav-item", "Home"));
return result;
