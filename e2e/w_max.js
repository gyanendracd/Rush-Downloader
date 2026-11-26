click($$(".window-controls button").find((b) => b.getAttribute("aria-label") === "Maximize"));
await sleep(900);
return "clicked Maximize";
