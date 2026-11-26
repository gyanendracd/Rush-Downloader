// Opens a site on the Sign in page through its button and leaves it open.
// Run popup_in_site.mjs next, then t10_signin_blocked.js.
await waitFor(() => $(".nav-item"), 60000, "the app to load");
click(byText(".nav-item", "Sign in"));
await waitFor(() => $(".signin-sites"), 5000, "the list of sites");
click(byText(".signin-sites .btn", "Reddit"));
await waitFor(() => $(".page-signin.is-open .page-head"), 8000, "the site to open");
await sleep(5000);
return { open: $(".page-signin.is-open .page-head").textContent.trim(), noticeYet: !!$(".signin-blocked") };
