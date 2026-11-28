// The extension's options: which downloads the browser starts go to the app.
const $ = (id) => document.getElementById(id);

function lines(text) {
  return text
    .split(/[\n,]+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

async function load() {
  const s = await rushSettings();
  $("capture").checked = s.capture;
  $("sites").value = s.excludedSites.join("\n");
  $("types").value = s.excludedTypes.join(", ");
  $("min").value = s.minSizeKb || "";
  $("cookies").checked = s.sendCookies;
  const state = await rushPing();
  $("app").textContent = !state.open ? "App is not open" : state.paired ? "Connected" : "Not connected: connect from the toolbar button";
  $("app").className = "app " + (!state.open ? "off" : state.paired ? "on" : "warn");
}

$("save").addEventListener("click", async () => {
  const min = Math.max(0, Math.round(Number($("min").value) || 0));
  await rushSaveSettings({
    capture: $("capture").checked,
    excludedSites: lines($("sites").value).map((s) => s.toLowerCase().replace(/^https?:\/\//, "").split("/")[0]),
    excludedTypes: $("types")
      .value.split(/[\s,;]+/)
      .map((t) => t.trim().toLowerCase().replace(/^\.+/, ""))
      .filter(Boolean),
    minSizeKb: min,
    sendCookies: $("cookies").checked,
  });
  $("said").textContent = "Saved";
  $("said").className = "ok";
  load();
});

load();
