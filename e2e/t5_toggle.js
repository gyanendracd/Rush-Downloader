// Test 5: the details popup has one button that switches between Pause and Resume.
const out = {};
const BASE = "http://127.0.0.1:8765";
const media = { max_height: null, audio_only: false };
const footer = () => $$(".dialog-foot .btn").map((b) => b.textContent.trim());
const status = async () => (await invoke("list_downloads")).find((d) => d.filename === "big-video.mkv")?.status;

nav("Downloads");
for (const d of await invoke("list_downloads")) {
  if (d.filename.startsWith("big-video")) await invoke("remove_download", { id: d.id, deleteFile: true });
}
await sleep(400);
await invoke("add_download", { request: { url: `${BASE}/files/big-video.mkv`, media } });
await waitFor(() => row("big-video.mkv")?.classList.contains("row-downloading"), 15000, "big file to start");

click(row("big-video.mkv"));
await waitFor(() => $(".dialog"), 3000, "details popup");
out.popupWidth = Math.round($(".dialog").getBoundingClientRect().width);
out.windowWidth = window.innerWidth;
out.whileDownloading = footer();

click(byText(".dialog-foot .btn", "Pause"));
await waitFor(() => footer().includes("Resume"), 8000, "the button to become Resume");
out.afterPause = { buttons: footer(), status: await status() };

click(byText(".dialog-foot .btn", "Resume"));
await waitFor(() => footer().includes("Pause"), 15000, "the button to become Pause again");
out.afterResume = { buttons: footer(), status: await status() };

// tidy up through the in-app confirmation
click(byText(".dialog-foot .btn", "Cancel"));
await waitFor(() => $$(".dialog").length === 2, 3000, "confirmation");
click(byText(".dialog-foot .btn", "Cancel download"));
await waitFor(() => !row("big-video.mkv"), 8000, "row to disappear");

// a failed download offers Retry in the same place
await invoke("add_download", { request: { url: `${BASE}/files/missing.bin`, media } });
await waitFor(() => row("missing.bin")?.classList.contains("row-failed"), 15000, "missing file to fail");
click(row("missing.bin"));
await waitFor(() => $(".dialog"), 3000, "details popup for failed");
out.whenFailed = footer();
click(byText(".dialog-foot .btn", "Cancel"));
await waitFor(() => $$(".dialog").length === 2, 3000, "confirmation for failed");
click(byText(".dialog-foot .btn", "Cancel download"));
await waitFor(() => !row("missing.bin"), 8000, "failed row to disappear");

out.errors = errors();
return out;
