// Settings › Advanced lists the connected browser; Disconnect removes it.
click(byText(".nav button, nav button, button", "Settings"));
await sleep(400);
click(await waitFor(() => byText("[role=tab]", "Advanced"), 5000, "the Advanced tab"));
const entry = await waitFor(() => $$(".setting-row").find((r) => r.textContent.includes("TestBrowser 1")), 5000, "the connected browser");
const listed = entry.textContent.includes("chrome-extension://rushtestextension");
click(byText(".setting-row button", "Disconnect"));
await waitFor(() => !$$(".setting-row").some((r) => r.textContent.includes("TestBrowser 1")), 5000, "the browser to go");
return { listed, disconnected: true };
