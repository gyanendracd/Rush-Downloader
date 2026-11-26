click($$(".window-controls button").find((b) => b.getAttribute("aria-label") === "Minimize"));
await sleep(900);
return "clicked Minimize";
