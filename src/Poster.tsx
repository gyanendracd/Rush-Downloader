/* Who posted the video on Home's video card: their picture, name and
 * handle beside the video's own thumbnail, and the way to download
 * everything they have posted (the app's usual profile download). The
 * engine says who posted it (MediaInfo.uploader_url); profile_of says
 * whether the app can download their whole profile. */
import { useEffect, useState } from "react";
import { Users } from "lucide-react";
import { api, qualityOptions, type MediaInfo, type ProfileFace, type ProfileRef } from "./api";
import { t } from "./i18n";
import { useApp } from "./ui";

export function PosterRow(props: { video: MediaInfo; quality: string }) {
  const { actions } = useApp();
  const url = props.video.uploader_url ?? null;
  /** Their profile, when the app can download all of it. */
  const [who, setWho] = useState<ProfileRef | null>(null);
  const [face, setFace] = useState<ProfileFace | null>(null);
  const [pictureFailed, setPictureFailed] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setWho(null);
    setFace(null);
    setPictureFailed(false);
    if (!url) return;
    let stale = false;
    api
      .profileOf(url)
      .then((found) => {
        if (stale || !found) return;
        setWho(found);
        api
          .profileFace(url)
          .then((f) => !stale && setFace(f))
          .catch(() => {});
      })
      .catch(() => {});
    return () => {
      stale = true;
    };
  }, [url]);

  if (!url || (!props.video.uploader && !who)) return null;
  const name = face?.name || props.video.uploader || who?.name || "";
  const handle = who ? `@${who.name}` : null;
  const site = who?.site ?? props.video.site ?? "";

  return (
    <div className="poster" aria-label={t.poster.region}>
      {face?.picture && !pictureFailed ? (
        <img className="poster-face" src={face.picture} alt="" referrerPolicy="no-referrer" onError={() => setPictureFailed(true)} />
      ) : (
        <span className="poster-face poster-badge" aria-hidden="true">
          <Users size={16} strokeWidth={2} />
        </span>
      )}
      <span className="poster-text">
        <strong title={name}>{name}</strong>
        <span>{[handle && handle !== `@${name}` ? handle : null, site ? t.poster.on(site) : null].filter(Boolean).join(" · ")}</span>
      </span>
      {who && (
        <button
          type="button"
          className="btn btn-small"
          disabled={busy}
          title={t.poster.downloadProfileTitle(handle ?? name, site)}
          onClick={async () => {
            setBusy(true);
            try {
              await actions.submitLink(url, qualityOptions(props.quality));
            } finally {
              setBusy(false);
            }
          }}
        >
          {t.poster.downloadProfile}
        </button>
      )}
    </div>
  );
}
