/* The browser extension's connection: the prompt to allow one, and
 * Settings, Advanced, Browser extension (src/Bridge.tsx). */
export const bridge = {
  askTitle: "Connect a browser extension?",
  ask: (browser: string) => `The RushDownload extension in ${browser} wants to send downloads to this app.`,
  askNote: "Allow it only if you just pressed Connect in the extension. Once allowed, it can add downloads until you disconnect it here or under Settings, Advanced.",
  allow: "Allow",
  deny: "Don't allow",
  title: "Browser extension",
  hint: "Install the RushDownload extension in Chrome, Edge, Brave or Firefox, then press Connect in it. Which downloads it catches is set in the extension's own options.",
  none: "No browser is connected yet.",
  lastSeen: (when: string) => `last used ${when}`,
  neverSeen: "not used yet",
  connected: (when: string) => `connected ${when}`,
  revoke: "Disconnect",
  revoked: "Disconnected. That browser must connect again to send downloads.",
};
