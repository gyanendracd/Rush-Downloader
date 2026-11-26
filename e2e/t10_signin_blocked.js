// After popup_in_site.mjs: the Sign in page explains why nothing opened.
const call = window.__TAURI_INTERNALS__.invoke;
const notice = await waitFor(() => $(".signin-blocked")?.textContent.trim(), 8000, "the notice");
click(byText(".page-signin .page-head .btn", "Save sign-in and close"));
await waitFor(() => $(".signin-sites"), 8000, "the list of sites to come back");
await call("forget_sign_ins");
return { notice };
