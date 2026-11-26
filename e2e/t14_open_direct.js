// Opens a page in the embedded browser with secure DNS set to window.__secure
// and reports the error, if any.
const call = window.__TAURI_INTERNALS__.invoke;
await waitFor(() => $(".nav-item"), 60000, "the app to load");
await call("signin_close").catch(() => {});
const settings = await call("get_settings");
await call("set_settings", { settings: { ...settings, secure_dns: !!window.__secure } });
await sleep(1500);
const result = await call("signin_open", { url: "https://1.1.1.1/help", x: 200, y: 140, width: 800, height: 500, browse: true }).then(
  () => "opened",
  (e) => "error: " + String(e),
);
await sleep(window.__secure ? 12000 : 3000);
return { secure: !!window.__secure, result };
