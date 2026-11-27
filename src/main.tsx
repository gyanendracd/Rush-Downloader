import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/inter";
import { api } from "./api";
import App from "./App";
import { savedLocale, setLocale } from "./i18n";

async function start() {
  // Outside Tauri (plain `npm run dev` in a browser) there is no backend;
  // development builds fall back to sample data so the interface can be worked on.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { mockApi } = await import("./mock");
    Object.assign(api, mockApi);
  }
  // The language picked in Settings, or the system's on first run; its text
  // is loaded before the first paint so nothing shows in English first.
  await setLocale(savedLocale()).catch(() => setLocale("en"));
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

start();
