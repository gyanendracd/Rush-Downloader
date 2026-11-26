// Prints the lines of the page shown inside the app that contain a word.
// Usage: node read_site.mjs <word>
const PORT = process.env.CDP_PORT ?? "9333";
const word = (process.argv[2] ?? "").toLowerCase();
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
ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression: "document.body.innerText", returnByValue: true } }));
const text = String((await answer).result?.result?.value ?? "");
ws.close();
const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
console.log(site.url.slice(0, 50));
lines.forEach((line, i) => {
  if (line.toLowerCase().includes(word)) console.log("  " + line + " | " + (lines[i + 1] ?? ""));
});
