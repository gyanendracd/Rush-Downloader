// Opens the site named in window.__site on the Sign in page and leaves it open.
await waitFor(() => $(".nav-item"), 60000, "the app to load");
if ($(".page-signin.is-open .page-head")) {
  click(byText(".page-signin .page-head .btn", "Save sign-in and close"));
  await waitFor(() => $(".signin-sites"), 8000, "the list of sites");
}
click(byText(".nav-item", "Sign in"));
await waitFor(() => $(".signin-sites"), 5000, "the list of sites");
click($$(".signin-sites .btn").find((b) => b.textContent.trim() === window.__site));
await waitFor(() => $(".page-signin.is-open .page-head"), 8000, "the site to open");
await sleep(7000);
return window.__site;
