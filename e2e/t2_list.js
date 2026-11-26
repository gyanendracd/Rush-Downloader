// Test 2: everything around the list. Detail dialog, menus, chips, filters, search, sort,
// multi-select, palette and keyboard shortcuts.
const out = {};
const BASE = "http://127.0.0.1:8765";
const media = { max_height: null, audio_only: false };
const add = (path) => invoke("add_download", { request: { url: `${BASE}${path}`, media } });

nav("Downloads");
await add("/files/report.pdf");
await add("/media/clip.mp4");
await add("/media/song.mp3");
await add("/files/does-not-exist.exe");
await waitFor(() => row("report.pdf")?.classList.contains("row-completed"), 15000, "report.pdf to complete");
await waitFor(() => row("does-not-exist")?.classList.contains("row-failed"), 15000, "missing file to fail");
await waitFor(() => row("song.mp3")?.classList.contains("row-completed"), 15000, "song to complete");
await waitFor(() => row("big-video.mkv")?.classList.contains("row-completed"), 60000, "big video to complete");
out.failedRow = rows().find((r) => r.name.includes("does-not-exist"));

// --- chips and their counts
const chips = () => Object.fromEntries($$(".chip").map((c) => [c.childNodes[0].textContent.trim(), Number($(".chip-count", c).textContent)]));
out.chips = chips();
click(byText(".chip", "Failed"));
await sleep(200);
out.failedChipShows = rows().map((r) => r.name);
click(byText(".chip", "Completed"));
await sleep(200);
out.completedChipCount = rows().length;
click(byText(".chip", "All"));
await sleep(200);

// --- detail dialog on a completed download
click(row("big-video.mkv"));
await waitFor(() => $(".dialog"), 5000, "detail dialog");
out.detail = {
  title: $(".detail-title h2").textContent,
  headline: $(".detail-headline").textContent,
  facts: Object.fromEntries($$(".dialog .fact").map((f) => [$("dt", f).textContent, $("dd", f).textContent])),
  footer: $$(".dialog-foot .btn").map((b) => b.textContent.trim()),
};
click(byText(".tabs-line button", "Details"));
await sleep(150);
out.detailsTab = Object.fromEntries($$(".dialog .fact").map((f) => [$("dt", f).textContent, $("dd", f).textContent.slice(0, 60)]));
click(byText(".tabs-line button", "Options"));
await sleep(150);
out.optionsTab = $$(".option-list button").map((b) => b.textContent.trim());
click(byText(".option-list button", "Copy link"));
await sleep(400);
out.copyToast = toasts();
$(".dialog").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
await waitFor(() => !$(".dialog"), 3000, "dialog to close on Escape");

// --- detail dialog on the failed one shows why
click(row("does-not-exist"));
await waitFor(() => $(".dialog"), 5000, "detail dialog for failed");
out.failedDetail = { callout: $(".callout")?.textContent ?? null, footer: $$(".dialog-foot .btn").map((b) => `${b.textContent.trim()}${b.disabled ? " (off)" : ""}`) };
click($(".dialog-head .icon-btn"));
await waitFor(() => !$(".dialog"), 3000, "dialog to close with the X");

// --- the "more" menu
click($(".row-actions .icon-btn", row("report.pdf")));
await waitFor(() => $(".menu"), 3000, "row menu");
out.menuForCompleted = $$(".menu button").map((b) => b.textContent.trim());
click(byText(".menu button", "Remove from list"));
await waitFor(() => !row("report.pdf"), 5000, "report.pdf to leave the list");
out.removedFromList = true;
out.fileKeptOnDisk = (await invoke("list_downloads")).every((d) => d.filename !== "report.pdf");

// --- filters and search
click($$(".page-head .icon-btn")[1]);
await waitFor(() => $(".filters"), 3000, "filter bar");
click(byText(".filters .pill-btn", "Type"));
await waitFor(() => $(".menu"), 3000, "type menu");
out.typeOptions = $$(".menu button").map((b) => b.textContent.trim());
click(byText(".menu button", "Audio"));
await sleep(250);
out.audioOnly = rows().map((r) => r.name);
out.typePillLabel = $$(".filters .pill-btn")[0].textContent.trim();
click($$(".filters .pill-btn")[0]);
await waitFor(() => $(".menu"), 3000, "type menu again");
click(byText(".menu button", "All Types"));
await sleep(200);
click(byText(".filters .pill-btn", "Size"));
await waitFor(() => $(".menu"), 3000, "size menu");
click(byText(".menu button", "Under 100 MB"));
await sleep(250);
out.under100 = rows().map((r) => r.name);
click($$(".filters .pill-btn")[2]);
await waitFor(() => $(".menu"), 3000, "size menu again");
click(byText(".menu button", "Any size"));
await sleep(200);
type($(".search input"), "clip");
await sleep(250);
out.searchClip = rows().map((r) => r.name);
type($(".search input"), "zzzz-nothing");
await sleep(250);
out.emptyState = $(".empty h2")?.textContent ?? null;
click(byText(".empty .btn", "Clear filters"));
await sleep(250);
out.afterClear = rows().length;

// --- sort
click(byText(".page-head .pill-btn", "Sort by"));
await waitFor(() => $(".menu"), 3000, "sort menu");
click(byText(".menu button", "Name"));
await sleep(250);
out.sortedByName = rows().map((r) => r.name);
click(byText(".page-head .pill-btn", "Sort by"));
await waitFor(() => $(".menu"), 3000, "sort menu again");
click(byText(".menu button", "Largest first"));
await sleep(250);
out.sortedLargest = rows().map((r) => r.name);

// --- multi-select with Ctrl and Shift, then the bulk bar
click(row("clip.mp4"), { ctrlKey: true });
click(row("song.mp3"), { ctrlKey: true });
await sleep(200);
out.ctrlSelected = rows().filter((r) => r.selected).map((r) => r.name);
out.bulkBar = $(".bulk strong")?.textContent ?? null;
out.noDialogOnCtrlClick = !$(".dialog");
key("a", { ctrlKey: true });
await sleep(200);
out.ctrlASelectsAll = rows().filter((r) => r.selected).length === rows().length;
key("Escape");
await sleep(200);
out.escapeClears = rows().filter((r) => r.selected).length === 0;

// --- palette and navigation shortcuts
key("k", { ctrlKey: true });
await waitFor(() => $(".palette"), 3000, "palette");
out.paletteCommands = $$(".palette-item").map((b) => $(".palette-label", b).textContent);
type($(".palette-input input"), "song");
await sleep(250);
out.paletteFindsDownload = $$(".palette-item").map((b) => $(".palette-label", b).textContent);
key("k", { ctrlKey: true });
await waitFor(() => !$(".palette"), 3000, "palette to close");
key(",", { ctrlKey: true });
await sleep(250);
out.ctrlComma = $(".nav-item.is-active")?.getAttribute("aria-label");
key("1", { ctrlKey: true });
await sleep(250);
out.ctrl1 = $(".nav-item.is-active")?.getAttribute("aria-label");
key("n", { ctrlKey: true });
await waitFor(() => $(".dialog"), 3000, "add dialog");
out.addDialog = { title: $(".dialog-head h2").textContent, focusIsLinkField: document.activeElement === $("#add-form input") };
$(".dialog").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
await waitFor(() => !$(".dialog"), 3000, "add dialog to close");

out.errors = errors();
return out;
