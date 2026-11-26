// Test 4: cancelling and deleting ask in the app's own dialog, and both answers do what they say.
const out = {};
const BASE = "http://127.0.0.1:8765";
const media = { max_height: null, audio_only: false };
const dir = (await invoke("get_settings")).download_dir;

nav("Downloads");
// Start from a clean, known state for the two files this test uses.
for (const d of await invoke("list_downloads")) {
  if (d.filename.startsWith("big-video") || d.filename.startsWith("report")) {
    await invoke("remove_download", { id: d.id, deleteFile: true });
  }
}
await sleep(500);

await invoke("add_download", { request: { url: `${BASE}/files/big-video.mkv`, media } });
await waitFor(() => row("big-video.mkv")?.classList.contains("row-downloading"), 15000, "big file to start");
await sleep(1500);

// --- open the detail popup and press Cancel
click(row("big-video.mkv"));
await waitFor(() => $(".dialog"), 3000, "detail popup");
out.detailPopupWidth = Math.round($(".dialog").getBoundingClientRect().width);
click(byText(".dialog-foot .btn", "Cancel"));
await waitFor(() => $$(".dialog").length === 2, 3000, "the in-app confirmation");
const confirmBox = $$(".dialog")[1];
out.confirm = {
  title: $("h2", confirmBox).textContent,
  name: $(".confirm-name", confirmBox).textContent,
  detail: $(".confirm-detail", confirmBox).textContent,
  buttons: $$(".dialog-foot .btn", confirmBox).map((b) => b.textContent.trim()),
  focusOnSafeButton: document.activeElement?.textContent.trim(),
  usesAppStyles: getComputedStyle(confirmBox).borderRadius,
};

// "Keep downloading" must change nothing.
click(byText(".dialog-foot .btn", "Keep downloading"));
await waitFor(() => $$(".dialog").length === 1, 3000, "confirmation to close");
await sleep(600);
const kept = (await invoke("list_downloads")).find((d) => d.filename === "big-video.mkv");
out.afterKeep = kept ? kept.status : "gone";

// Escape on the confirmation also means "keep".
click(byText(".dialog-foot .btn", "Cancel"));
await waitFor(() => $$(".dialog").length === 2, 3000, "confirmation again");
$$(".dialog")[1].dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
await waitFor(() => $$(".dialog").length === 1, 3000, "confirmation to close on Escape");
await sleep(400);
out.afterEscape = (await invoke("list_downloads")).some((d) => d.filename === "big-video.mkv") ? "still there" : "gone";

// "Cancel download" removes it and its partial data.
click(byText(".dialog-foot .btn", "Cancel"));
await waitFor(() => $$(".dialog").length === 2, 3000, "confirmation a third time");
click(byText(".dialog-foot .btn", "Cancel download"));
await waitFor(() => !row("big-video.mkv"), 8000, "the row to disappear");
await waitFor(() => $$(".dialog").length === 0, 3000, "both popups to close");
out.afterCancel = (await invoke("list_downloads")).some((d) => d.filename === "big-video.mkv") ? "still listed" : "removed";
out.partialDir = dir;

// --- delete a finished file from disk through the row menu
await invoke("add_download", { request: { url: `${BASE}/files/report.pdf`, media } });
await waitFor(() => row("report.pdf")?.classList.contains("row-completed"), 15000, "report.pdf to complete");
click($(".row-actions .icon-btn", row("report.pdf")));
await waitFor(() => $(".menu"), 3000, "row menu");
click(byText(".menu button", "Delete file from disk"));
await waitFor(() => $(".dialog"), 3000, "delete confirmation");
out.deleteConfirm = {
  title: $(".dialog h2").textContent,
  name: $(".confirm-name").textContent,
  buttons: $$(".dialog-foot .btn").map((b) => b.textContent.trim()),
};
click(byText(".dialog-foot .btn", "Delete file"));
await waitFor(() => !row("report.pdf"), 8000, "report.pdf row to disappear");

out.errors = errors();
return out;
