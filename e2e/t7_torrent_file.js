// Test 7: a .torrent file on this computer, through the single link box:
// the torrent dialog opens with its files, the save folder can be seen,
// the selection gates the Download button, and the download really starts.
// Set window.__torrentPath before running (see README), e.g. through t7_path.js.
const out = {};
const path = window.__torrentPath;
if (!path) throw new Error("window.__torrentPath is not set");
const dir = (await invoke("get_settings")).download_dir;

nav("Home");
await waitFor(() => $(".paste input"), 5000, "the paste box");
type($(".paste input"), `"${path}"`);
await sleep(150);
out.boxAcceptsQuotedPath = !$(".paste .btn").disabled;
$(".paste").requestSubmit();

await waitFor(() => $(".dialog"), 5000, "the torrent dialog");
out.dialogTitle = $(".dialog-head h2").textContent;
await waitFor(() => $(".torrent-name"), 100000, "the torrent's file list");
out.torrentName = $(".torrent-name").textContent;
out.saveTo = $(".dialog .field-row input").value === dir ? "the default download folder" : $(".dialog .field-row input").value;
out.files = $$(".tree-row").map((r) => `${$(".tree-name", r).textContent} | ${$(".tree-size", r).textContent}`);
out.summary = $(".tree-summary").textContent;

// Unticking everything must block the download.
click(byText(".tree-head .link", "Select none"));
await sleep(200);
out.noneSelected = { downloadDisabled: $(".dialog-foot .btn-primary").disabled, hint: $(".dialog .field-error")?.textContent ?? null, summary: $(".tree-summary").textContent };
click(byText(".tree-head .link", "Select all"));
await sleep(200);
out.allSelected = { downloadDisabled: $(".dialog-foot .btn-primary").disabled };

click($(".dialog-foot .btn-primary"));
await waitFor(() => !$(".dialog"), 8000, "the dialog to close");
await waitFor(() => $(".nav-item.is-active")?.getAttribute("aria-label") === "Downloads", 5000, "the Downloads page");
await waitFor(() => rows().length > 0, 8000, "the torrent's row");
let item = null;
await waitFor(async () => false, 1, "").catch(() => {});
for (let i = 0; i < 600; i++) {
  item = (await invoke("list_downloads")).find((d) => d.torrent);
  if (item && (item.status === "failed" || item.downloaded > 2 * 1024 * 1024)) break;
  await sleep(200);
}
out.started = item && {
  status: item.status,
  error: item.error,
  name: item.filename,
  downloadedMB: Math.round(item.downloaded / 1048576),
  totalGB: item.total ? +(item.total / 1073741824).toFixed(2) : null,
  insideDownloadFolder: item.path.startsWith(dir),
};
out.row = rows()[0];

// Cancel it through the app's own confirmation, which must remove its folder.
click(row(item.filename));
await waitFor(() => $(".dialog"), 3000, "details popup");
out.detailSource = $$(".dialog .fact").length;
click(byText(".dialog-foot .btn", "Cancel"));
await waitFor(() => $$(".dialog").length === 2, 3000, "confirmation");
click(byText(".dialog-foot .btn", "Cancel download"));
await waitFor(() => rows().length === 0, 10000, "the row to disappear");
out.removedFromList = (await invoke("list_downloads")).length === 0;
out.folderPath = item.path;

out.errors = errors();
return out;
