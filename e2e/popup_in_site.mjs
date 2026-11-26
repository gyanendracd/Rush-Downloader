// Makes the page shown inside the Sign in page try to open a new window, the
// way a "Continue with Google" button does. Usage: node popup_in_site.mjs <url to open>
const PORT = process.env.CDP_PORT ?? "9333";
const target = process.argv[2] ?? "https://accounts.google.com/";
const pages = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const site = pages.find((p) => p.type === "page" && !p.url.includes("localhost"));
if (!site) {
  console.log("FAILED: no site page is open");
  process.exit(1);
}
const ws = new WebSocket(site.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = reject;
});
const answer = new Promise((resolve) => (ws.onmessage = (m) => resolve(JSON.parse(m.data))));
ws.send(
  JSON.stringify({
    id: 1,
    method: "Runtime.evaluate",
    params: { expression: `String(window.open(${JSON.stringify(target)}))`, userGesture: true },
  }),
);
const result = await answer;
console.log(JSON.stringify({ from: new URL(site.url).host, windowOpenReturned: result.result?.result?.value }));
ws.close();
