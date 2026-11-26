// Test 1: the main path. Paste a link on Home, watch it finish, then pause and resume a big one.
const out = {};
const BASE = "http://127.0.0.1:8765";

await waitFor(() => $(".sidebar"), 15000, "the app to render");
out.isTauri = "__TAURI_INTERNALS__" in window;
out.settings = await invoke("get_settings");
out.windowControls = $$(".window-controls button").map((b) => b.getAttribute("aria-label"));

// --- Home: paste a small file link and press Download
nav("Home");
await waitFor(() => $(".paste input"), 5000, "the paste box");
out.downloadDisabledWhenEmpty = $(".paste .btn").disabled;
type($(".paste input"), "not a link");
await sleep(100);
out.downloadDisabledForBadText = $(".paste .btn").disabled;
type($(".paste input"), `${BASE}/files/small-archive.zip`);
await sleep(100);
out.downloadEnabledForLink = !$(".paste .btn").disabled;
$(".paste").requestSubmit();

// It should take us to Downloads and finish.
await waitFor(() => row("small-archive.zip"), 8000, "the new row on the Downloads page");
out.wentToDownloads = $(".nav-item.is-active")?.getAttribute("aria-label");
await waitFor(() => row("small-archive.zip")?.classList.contains("row-completed"), 20000, "small file to complete");
out.smallDone = rows().find((r) => r.name.includes("small-archive"));

// --- A big, slow file: pause, check it stops, resume, check it moves again
await invoke("add_download", { request: { url: `${BASE}/files/big-video.mkv`, media: { max_height: null, audio_only: false } } });
await waitFor(() => row("big-video.mkv")?.classList.contains("row-downloading"), 15000, "big file to start");
await sleep(2500);
const live = (await invoke("list_downloads")).find((d) => d.filename === "big-video.mkv");
out.bigRunning = { status: live.status, connections: live.active_connections, segments: live.segments.length, speed: live.speed, downloaded: live.downloaded };
out.bigRowWhileRunning = rows().find((r) => r.name.includes("big-video"));
out.hasProgressBar = !!$(".bar .fill", row("big-video.mkv"));
out.etaShown = $(".row-eta", row("big-video.mkv"))?.textContent ?? null;

// Pause with the round button in the row.
click($(".round", row("big-video.mkv")));
await waitFor(() => row("big-video.mkv")?.classList.contains("row-paused"), 8000, "row to show paused");
await sleep(1500);
const p1 = (await invoke("list_downloads")).find((d) => d.filename === "big-video.mkv");
await sleep(2000);
const p2 = (await invoke("list_downloads")).find((d) => d.filename === "big-video.mkv");
out.pause = { status: p2.status, stoppedGrowing: p1.downloaded === p2.downloaded, downloaded: p2.downloaded };
out.pausedRow = rows().find((r) => r.name.includes("big-video"));
out.roundLabelWhenPaused = $(".round", row("big-video.mkv")).getAttribute("aria-label");

// Resume with the same button.
click($(".round", row("big-video.mkv")));
await waitFor(() => row("big-video.mkv")?.classList.contains("row-downloading"), 15000, "row to resume");
await sleep(2500);
const r1 = (await invoke("list_downloads")).find((d) => d.filename === "big-video.mkv");
out.resume = { status: r1.status, continuedFromWhereItStopped: r1.downloaded > p2.downloaded, downloaded: r1.downloaded };

out.errors = errors();
return out;
