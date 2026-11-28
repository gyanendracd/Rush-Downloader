// The app shows who asks to connect; this presses Allow.
const prompt = await waitFor(() => byText(".dialog h2", "Connect a browser extension"), 10000, "the prompt to connect");
const dialog = prompt.closest(".dialog");
const said = dialog.textContent;
click(byText(".dialog button", "Allow"));
await sleep(300);
return { prompted: true, mentionsBrowser: said.includes("TestBrowser 1"), mentionsOrigin: said.includes("chrome-extension://rushtestextension") };
