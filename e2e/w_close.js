click($$(".window-controls button").find((b) => b.getAttribute("aria-label") === "Close"));
await sleep(300);
return "clicked Close";
