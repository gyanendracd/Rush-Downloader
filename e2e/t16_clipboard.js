// Reads what the Home box holds and which page is showing. The caller copies
// a link to the clipboard first (see the README); nothing is submitted.
await waitFor(() => $(".nav-item"), 60000, "the app to load");
const want = window.__expect;
const box = await waitFor(
  () => {
    const input = $(".paste input");
    return input && (want == null || input.value === want) ? input : null;
  },
  8000,
  "the Home box to hold the copied link",
).catch(() => $(".paste input"));
return {
  page: $(".nav-item.is-active")?.textContent.trim(),
  box: box ? box.value : null,
  toast: $$(".toast").map((t) => t.textContent.trim()),
};
