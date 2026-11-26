// Saves and closes the embedded sign-in page, then forgets the sign-in.
const call = window.__TAURI_INTERNALS__.invoke;
const saved = await call("signin_close");
const again = await call("signin_close");
await call("forget_sign_ins");
return { saved, closingTwiceIsHarmless: again, afterForget: await call("sign_in_status") };
