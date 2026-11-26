// A playlist link (window.__link) and a profile link (window.__profile) typed
// on Home: the card says how many videos and photos there are and how long the
// videos run, and a playlist offers a resolution. Settings has the switch for
// asking where to save each one. Needs the internet; nothing is downloaded and
// nothing is submitted.
await waitFor(() => $(".nav-item"), 60000, "the app to load");
click(byText(".nav-item", "Home"));
const box = await waitFor(() => $(".paste input"), 5000, "the Home box");
const meta = () => $(".video-card.profile-card .video-meta")?.textContent.trim() ?? "";

type(box, window.__link);
await waitFor(() => /\d+ videos?/.test(meta()), 60000, "the playlist's details");
const playlistLine = meta().replace(/total · .+ · Playlist$/, "total · (channel) · Playlist");
const chips = () => $$(".video-card.profile-card .video-choice");
// The choice appears once a few of the playlist's videos have been looked up:
// only the resolutions it really has are offered.
const whileChecking = $(".video-card.profile-card [role=status]")?.textContent.trim() ?? null;
await waitFor(() => chips().length > 0, 90000, "the resolutions");
const offered = chips().map((c) => c.textContent.trim());
const usual = chips().find((c) => c.getAttribute("aria-checked") === "true")?.textContent.trim();
click(chips()[chips().length - 2]);
await sleep(200);
const picked = chips().find((c) => c.getAttribute("aria-checked") === "true")?.textContent.trim();

let profileLine = null;
let profileOffersResolution = null;
if (window.__profile) {
  type(box, window.__profile);
  await waitFor(() => / profile/.test(meta()), 15000, "the profile card");
  const whileCounting = meta();
  await waitFor(() => /\d+\+? (photos?|videos?).* profile/.test(meta()), 60000, "the profile's counts");
  profileLine = { whileCounting, counted: meta() };
  profileOffersResolution = chips().length > 0;
}
type(box, "");

click(byText(".nav-item", "Settings"));
const toggle = await waitFor(
  () => $$(".toggle").find((t) => /Ask where to save each profile or playlist/.test(t.textContent)),
  5000,
  "the switch in Settings",
);
const input = $("input", toggle);
const before = input.checked;
click(input);
await sleep(200);
const saved = localStorage.getItem("askfolder");
click(input);
await sleep(200);
return {
  playlistLine,
  whileChecking,
  offered,
  usual,
  picked,
  profileLine,
  profileOffersResolution,
  askSwitch: { startsOff: before === false, savedWhenOn: saved, backTo: localStorage.getItem("askfolder") },
};
