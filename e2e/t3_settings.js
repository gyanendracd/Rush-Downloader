// Test 3: selection fix, settings, page scan, link-file import, About.
const out = {};
const BASE = "http://127.0.0.1:8765";

// --- the fix: closing the detail dialog leaves nothing selected
nav("Downloads");
await waitFor(() => row("clip.mp4"), 5000, "the list");
key("Escape");
click(row("clip.mp4"));
await waitFor(() => $(".dialog"), 3000, "detail dialog");
click($(".dialog-head .icon-btn"));
await waitFor(() => !$(".dialog"), 3000, "dialog to close");
await sleep(150);
out.selectedAfterClosingDetail = rows().filter((r) => r.selected).map((r) => r.name);

// --- settings: General
nav("Settings");
await waitFor(() => $(".settings-nav"), 3000, "settings");
out.sections = $$(".settings-nav button").map((b) => b.textContent.trim());
out.general = {
  toggles: $$(".toggle").map((t) => `${t.textContent.trim()}${$("input", t).disabled ? " [off]" : ""}`),
  folder: $(".input-with-button input").value.slice(-12),
  language: $(".settings-body select").disabled,
};

// --- settings: Network values reach the engine
click(byText(".settings-nav button", "Network"));
await sleep(200);
const numbers = $$(".settings-body input[type=number]");
type(numbers[0], "12");
type(numbers[1], "2");
choose($(".settings-body select"), String(5 * 1024 * 1024));
await sleep(400);
const s1 = await invoke("get_settings");
out.networkSaved = { connections: s1.connections, max_concurrent: s1.max_concurrent, speed_limit: s1.speed_limit };
type(numbers[0], "999");
await sleep(300);
out.connectionsClampedTo = (await invoke("get_settings")).connections;
choose($(".settings-body select"), "0");
type(numbers[0], "4");
type(numbers[1], "3");
await sleep(300);

// --- settings: Downloads (quality + sign-in option exists and is off)
click(byText(".settings-nav button", "Downloads"));
await sleep(200);
const selects = $$(".settings-body select");
out.downloads = {
  qualityOptions: [...selects[0].options].map((o) => o.textContent),
  signIn: [...selects[1].options].map((o) => o.textContent),
  signInValue: selects[1].value,
};

// --- settings: Appearance
click(byText(".settings-nav button", "Appearance"));
await sleep(200);
const themeBefore = document.documentElement.dataset.theme;
click(byText(".segmented button", "Light"));
await sleep(150);
const light = document.documentElement.dataset.theme;
click(byText(".segmented button", "Dark"));
await sleep(150);
const dark = document.documentElement.dataset.theme;
click(byText(".segmented button", "Compact"));
await sleep(150);
const compact = document.documentElement.dataset.density;
click(byText(".segmented button", "Comfortable"));
click(byText(".segmented button", themeBefore === "dark" ? "Dark" : "Light"));
await sleep(150);
out.appearance = { light, dark, compact, restoredTo: document.documentElement.dataset.theme };

// sidebar switch
const sw = $(".nav-item[role=switch]");
const before = sw.getAttribute("aria-checked");
click(sw);
await sleep(150);
out.darkSwitchToggles = sw.getAttribute("aria-checked") !== before;
click(sw);
await sleep(150);

// --- Advanced and Scheduler
click(byText(".settings-nav button", "Advanced"));
await waitFor(() => $(".settings-body .fact dd")?.textContent !== "Checking", 5000, "tool status");
out.advanced = { siteDownloads: $(".settings-body .fact dd").textContent, shortcuts: $$(".shortcuts > div").length };
click(byText(".settings-nav button", "Scheduler"));
await sleep(150);
out.scheduler = $(".settings-body .note").textContent.slice(0, 60);

// --- About
click($$(".nav-item").find((b) => b.getAttribute("aria-label") === "About"));
await waitFor(() => $(".dialog"), 3000, "about dialog");
out.about = Object.fromEntries($$(".dialog .fact").map((f) => [$("dt", f).textContent, $("dd", f).textContent.slice(0, 40)]));
click(byText(".dialog-foot .btn", "Close"));
await waitFor(() => !$(".dialog"), 3000, "about to close");

// --- Add from Browser: scan a page and download one result
nav("Home");
await waitFor(() => $(".welcome-actions"), 3000, "home");
click(byText(".welcome-actions .btn", "Add from Browser"));
await waitFor(() => $(".dialog"), 3000, "scan dialog");
out.scanDialog = { title: $(".dialog-head h2").textContent, button: $(".dialog-foot .btn-primary").textContent.trim() };
type($("#add-form input"), `${BASE}/page.html`);
await sleep(150);
$("#add-form").requestSubmit();
await waitFor(() => $(".found li:not(.skeleton-row)"), 15000, "scan results");
out.scanFound = $$(".found li").map((li) => `${$(".kind", li).textContent} | ${$(".found-name", li).textContent} | ${$(".found-meta", li).textContent}`);
// song.mp3 was downloaded before; a second copy must get its own name
click($(".btn", $$(".found li").find((li) => li.textContent.includes("song.mp3"))));
await waitFor(() => byText(".found-added", "Added"), 5000, "item to be added");
click(byText(".dialog-foot .btn", "Close"));
await waitFor(() => !$(".dialog"), 3000, "scan dialog to close");
nav("Downloads");
await waitFor(() => rows().filter((r) => r.name.startsWith("song")).length === 2 && rows().filter((r) => r.name.startsWith("song")).every((r) => r.status === "completed"), 15000, "second song to complete");
out.songCopies = rows().filter((r) => r.name.startsWith("song")).map((r) => r.name).sort();

// --- Browse Files: the command behind it reads links out of a text file
const listPath = (await invoke("get_settings")).download_dir + "\\links.txt";
out.importLinks = await invoke("import_links", { path: listPath }).catch((e) => "error: " + e);
out.importMissingFile = await invoke("import_links", { path: listPath + ".nope" }).catch((e) => "error: " + String(e).slice(0, 40));

out.errors = errors();
return out;
