// Test 6: the single link box accepts every kind of link. Nothing is submitted,
// so no torrent or download is started.
const out = {};
nav("Home");
await waitFor(() => $(".paste input"), 5000, "the paste box");
const box = $(".paste input");
const button = $(".paste .btn");
const accepts = async (text) => {
  type(box, text);
  await sleep(120);
  return !button.disabled;
};

out.placeholder = box.placeholder;
out.fileLink = await accepts("https://example.com/files/setup.exe");
out.videoPage = await accepts("https://www.youtube.com/watch?v=abc123");
out.magnetLink = await accepts("magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&dn=Example");
out.torrentFile = await accepts("https://releases.ubuntu.com/24.04/ubuntu-24.04.3-live-server-amd64.iso.torrent");
out.plainText = await accepts("hello there");
out.brokenMagnet = await accepts("magnet:");
type(box, "");

// The add dialog (Ctrl+N) follows the same rule.
key("n", { ctrlKey: true });
await waitFor(() => $(".dialog"), 3000, "add dialog");
type($("#add-form input"), "magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567");
await sleep(150);
out.addDialogAcceptsMagnet = !$(".dialog-foot .btn-primary").disabled;
type($("#add-form input"), "ftp://example.com/file");
await sleep(150);
out.addDialogRejectsOther = $(".dialog-foot .btn-primary").disabled;
out.addDialogHint = $(".field-error")?.textContent.trim() ?? null;
$(".dialog").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
await waitFor(() => !$(".dialog"), 3000, "add dialog to close");

out.inputsOnHome = $$(".page-home input").length;
out.errors = errors();
return out;
