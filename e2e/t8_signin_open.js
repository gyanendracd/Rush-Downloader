// Opens the Sign in page and shows a public page that sets a cookie without
// any login inside it. Nothing is typed into that page.
const call = window.__TAURI_INTERNALS__.invoke;
await waitFor(() => $(".nav-item"), 60000, "the app to load");
await call("forget_sign_ins");
click(byText(".nav-item", "Sign in"));
const stage = await waitFor(() => $(".signin-stage"), 5000, "the Sign in page");
const box = stage.getBoundingClientRect();
await call("signin_open", { url: "https://github.com/login", x: box.left, y: box.top, width: box.width, height: box.height });
await sleep(6000);
return {
  before: await call("sign_in_status"),
  stage: { x: Math.round(box.left), y: Math.round(box.top), width: Math.round(box.width), height: Math.round(box.height) },
  sites: $$(".signin-sites .btn").map((b) => b.textContent.trim()),
};
