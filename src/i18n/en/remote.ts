/* FTP and SFTP: the sign-in part of the add dialog, and a server key to
 * trust in a download's details (src/Remote.tsx). */
export const remote = {
  title: "Sign-in",
  ftpHint: "Leave the user name empty to sign in anonymously. A user name and password written in the link are used too.",
  sftpHint: "SFTP needs a user name, and a password or a private key file.",
  username: "User name",
  password: "Password",
  passphrase: "Key passphrase",
  optional: "Optional",
  keyFile: "Private key file",
  keyNone: "None: sign in with the password",
  chooseKey: "Choose key file",
  pickKeyTitle: "Choose your private key file",
  clearKey: "Use the password instead",
  activeMode: "Active mode (the server connects back for data)",
  activeHint: "Passive mode, the usual, works behind most routers; use active only if the server asks for it.",
  passwordStore: "The password is kept encrypted, never in the app's list or in the link.",
  keyTitle: "This server is not known yet",
  keyChangedTitle: "This server's key has changed",
  keyExplain: (host: string) => `The SFTP server ${host} proved who it is with the key below. Check the fingerprint with the server's owner before trusting it.`,
  keyChangedExplain: (host: string) =>
    `The SFTP server ${host} now shows a different key from the one trusted before. This happens when a server is reinstalled, but it can also mean someone is listening in. Trust the new key only if you know why it changed.`,
  fingerprint: "Fingerprint",
  trust: "Trust this server and retry",
  trustNew: "Trust the new key and retry",
};
