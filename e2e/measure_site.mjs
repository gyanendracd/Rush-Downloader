// Compares the area the Sign in page gives a site with what the site's page
// itself sees, and whether the site's content overflows it.
const PORT = process.env.CDP_PORT ?? "9333";
const pages = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
async function ask(page, expression) {
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = reject;
  });
  const answer = new Promise((resolve) => (ws.onmessage = (m) => resolve(JSON.parse(m.data))));
  ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression, returnByValue: true } }));
  const result = await answer;
  ws.close();
  return result.result?.result?.value;
}
const app = pages.find((p) => p.type === "page" && p.url.includes("localhost"));
const site = pages.find((p) => p.type === "page" && !p.url.includes("localhost"));
const stage = await ask(
  app,
  `(() => { const b = document.querySelector(".signin-stage").getBoundingClientRect(); return { x: Math.round(b.left), y: Math.round(b.top), width: Math.round(b.width), height: Math.round(b.height), window: innerWidth + "x" + innerHeight, scale: devicePixelRatio }; })()`,
);
const seen = site
  ? await ask(
      site,
      `(() => { const d = document.documentElement; return { host: location.host, width: innerWidth, height: innerHeight, scale: devicePixelRatio, contentWidth: d.scrollWidth, contentHeight: d.scrollHeight, scrollsSideways: d.scrollWidth > innerWidth + 1, scrollsDown: d.scrollHeight > innerHeight + 1 }; })()`,
    )
  : null;
console.log(JSON.stringify({ stage, site: seen }, null, 2));
