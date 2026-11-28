/* Proxies: the Proxy pane of Settings, Network (src/Proxies.tsx) and the
 * connection picker of a new download. */
export const proxy = {
  title: "Proxy",
  defaultConnection: "Default connection",
  defaultHint:
    "What every download uses unless you choose another connection for it. Site downloads (yt-dlp, gallery-dl) and torrents use it too.",
  direct: "Direct (no proxy)",
  /** The default, when it is a direct connection, inside "Default (…)". */
  directShort: "direct",
  none: "No proxies yet. Add one to send downloads through it.",
  add: "Add proxy",
  edit: "Edit",
  isDefault: "Default",
  passwordSaved: "password saved",
  /** A proxy in the list: its kind and address. */
  where: (kind: string, address: string) => `${kind} · ${address}`,
  editor: {
    addTitle: "Add proxy",
    editTitle: "Edit proxy",
    name: "Name",
    namePlaceholder: "Office proxy",
    protocol: "Protocol",
    host: "Address",
    hostPlaceholder: "proxy.example.com or 192.168.1.10",
    port: "Port",
    username: "User name",
    password: "Password",
    optional: "Optional",
    passwordKept: "Saved; type to replace it",
    clearPassword: "Remove the saved password",
    passwordStore: "Passwords are kept in this computer's credential store, never in the app's files.",
    bypass: "Connect directly to",
    bypassHint:
      "One per line: a site (example.com, which includes its subdomains), *.example.com for subdomains only, an address, a range such as 10.0.0.0/8, or <local>. This computer itself is always reached directly.",
    test: "Test connection",
    testing: "Testing…",
    testOk: (status: number, ms: string) => `Connected. The test page answered ${status} in ${ms}.`,
    testFailed: (why: string) => `Could not connect: ${why}`,
    save: "Save",
    delete: "Delete",
    deleteConfirm: (name: string) => `Delete the proxy "${name}"?`,
    deleteDefault: "It is the default connection: downloads that use the default will connect directly.",
    socks4NoPassword: "SOCKS4 has no passwords. Use SOCKS5 for a proxy that asks for one.",
  },
  kinds: {
    http: "HTTP",
    https: "HTTPS",
    socks4: "SOCKS4",
    socks4a: "SOCKS4a",
    socks5: "SOCKS5",
    socks5h: "SOCKS5h",
  },
  kindHints: {
    http: "Plain HTTP proxy.",
    https: "HTTP proxy reached over an encrypted connection.",
    socks4: "Site names are looked up on this computer.",
    socks4a: "Site names are looked up by the proxy.",
    socks5: "Site names are looked up on this computer. Torrents can use it.",
    socks5h: "Site names are looked up by the proxy. Torrents can use it.",
  },
  limits:
    "Torrents can only use a SOCKS5 proxy. Through one, peer discovery that would bypass it (DHT, local discovery, the listening port) is off, and UDP trackers are still contacted directly. The Browser and Sign in pages do not use the proxy.",
  /** The connection picker of a new download. */
  connection: "Connection",
  useDefault: (what: string) => `Default (${what})`,
};
