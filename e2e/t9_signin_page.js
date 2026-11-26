// The Sign in page through its own buttons: a site opens inside the page,
// "Save sign-in and close" returns to the list of sites. Nobody signs in.
const call = window.__TAURI_INTERNALS__.invoke;
await waitFor(() => $(".nav-item"), 60000, "the app to load");
click(byText(".nav-item", "Home"));
await sleep(300);
click(byText(".nav-item", "Sign in"));
await waitFor(() => $(".signin-sites"), 5000, "the list of sites");
click(byText(".signin-sites .btn", "X"));
const bar = await waitFor(() => $(".page-signin.is-open .page-head"), 8000, "the site to open");
const barText = bar.textContent.trim();
await sleep(5000);
const box = $(".signin-stage").getBoundingClientRect();
click(byText(".page-signin .page-head .btn", "Save sign-in and close"));
await waitFor(() => $(".signin-sites"), 8000, "the list of sites to come back");
await sleep(500);
const status = $(".signin-status").textContent.trim();
// Leaving the page with a site open must close it too.
click(byText(".signin-sites .btn", "Reddit"));
await waitFor(() => $(".page-signin.is-open .page-head"), 8000, "the second site to open");
click(byText(".nav-item", "Home"));
await sleep(1500);
await call("forget_sign_ins");
return {
  barText,
  stage: { x: Math.round(box.left), y: Math.round(box.top), width: Math.round(box.width), height: Math.round(box.height) },
  status,
  afterForget: await call("sign_in_status"),
};
