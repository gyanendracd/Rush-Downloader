// A playlist link typed on Home (window.__link): the card shows the
// playlist's picture, its name and how many videos it holds. Needs the
// internet; nothing is downloaded and nothing is submitted.
await waitFor(() => $(".nav-item"), 60000, "the app to load");
click(byText(".nav-item", "Home"));
const box = await waitFor(() => $(".paste input"), 5000, "the Home box");
type(box, window.__link);
const card = await waitFor(() => $(".video-card.profile-card"), 15000, "the playlist card");
const early = $(".video-meta", card).textContent.trim();
await waitFor(() => /\d+ videos?/.test($(".video-card.profile-card .video-meta")?.textContent ?? ""), 60000, "the playlist's details");
const shown = $(".video-card.profile-card");
const picture = $("img.video-thumb", shown);
if (picture && !picture.complete) await sleep(4000);
const result = {
  whileLooking: early,
  titleLength: $(".video-title", shown).textContent.trim().length,
  titleIsNotThePlaceholder: $(".video-title", shown).textContent.trim() !== "YouTube playlist",
  details: $(".video-meta", shown).textContent.trim().replace(/· [^·]+ ·/, "· (channel) ·"),
  pictureShown: !!picture && picture.naturalWidth > 0,
  pictureSize: picture ? `${picture.naturalWidth}x${picture.naturalHeight}` : null,
};
type(box, "");
return result;
