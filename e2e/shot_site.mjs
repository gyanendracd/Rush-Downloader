// Saves a picture of the page shown inside the Sign in page, as it is drawn
// in its area. Usage: node shot_site.mjs <file.png>
import { writeFileSync } from "node:fs";

const PORT = process.env.CDP_PORT ?? "9333";
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
ws.send(JSON.stringify({ id: 1, method: "Page.captureScreenshot", params: { format: "png" } }));
const result = await answer;
ws.close();
writeFileSync(process.argv[2], Buffer.from(result.result.data, "base64"));
console.log(site.url.slice(0, 60));
