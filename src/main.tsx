import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/inter";
import { api } from "./api";
import App from "./App";

async function start() {
  // Outside Tauri (plain `npm run dev` in a browser) there is no backend;
  // development builds fall back to sample data so the interface can be worked on.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { mockApi } = await import("./mock");
    Object.assign(api, mockApi);
  }
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

start();
