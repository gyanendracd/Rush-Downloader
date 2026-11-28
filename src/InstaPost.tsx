/* An Instagram post or reel on Home: whose it is on the left (their picture,
 * name, @username, bio, the links in their bio, their posts, followers and
 * following, and the likes and comments this post has had), the post itself
 * on the right (its picture, caption, length and the choice of what to save).
 *
 * The profile is read as any visitor sees it, the same way Home reads a
 * pasted profile link (profile_face, then profile_stats with its numbers);
 * the post's own numbers come with the post (MediaInfo.likes, .comments). */
import { useEffect, useState } from "react";
import { Globe, Heart, MessageCircle, Play, Users } from "lucide-react";
import { api, formatDuration, qualityOptions, type MediaInfo, type ProfileFace, type ProfileRef } from "./api";
import { formatLocale, num, t } from "./i18n";
import { useApp } from "./ui";

/** "94.4K", as a profile's following is shown. */
const short = (n: number) =>
  new Intl.NumberFormat(formatLocale(), { notation: "compact", maximumFractionDigits: 1 }).format(n);

/** A link as the card shows it: "site.com/path". */
const linkText = (link: string) => link.replace(/^https?:\/\/(www\.)?/i, "").replace(/\/$/, "");

/** Both answers about a profile, as one: the site's own numbers first, its page otherwise. */
function merge(face: ProfileFace | null, stats: ProfileFace | null): ProfileFace | null {
  if (!face && !stats) return null;
  return {
    name: stats?.name ?? face?.name ?? null,
    picture: face?.picture ?? stats?.picture ?? null,
    about: stats?.about ?? face?.about ?? null,
    banner: null,
    posts: stats?.posts ?? face?.posts ?? null,
    media: null,
    followers: stats?.followers ?? face?.followers ?? null,
    following: stats?.following ?? face?.following ?? null,
    likes: null,
    rounded: stats?.followers != null || stats?.posts != null ? false : !!face?.rounded,
    joined: null,
    username: stats?.username ?? face?.username ?? null,
    links: [...(stats?.links ?? []), ...(face?.links ?? [])].filter(
      (link, i, all) => all.findIndex((l) => l.toLowerCase() === link.toLowerCase()) === i,
    ),
  };
}

/** The username in an Instagram profile link ("natgeo"). */
function usernameIn(link: string | null | undefined): string | null {
  const match = /instagram\.com\/([A-Za-z0-9._]{1,30})\/?$/i.exec(link?.trim() ?? "");
  return match ? match[1] : null;
}

export function InstaPostCard(props: {
  video: MediaInfo;
  choice: string;
  onChoice: (value: string) => void;
}) {
  const { actions } = useApp();
  const { video } = props;
  const url = video.uploader_url ?? null;
  const [ref, setRef] = useState<ProfileRef | null>(null);
  const [face, setFace] = useState<ProfileFace | null>(null);
  const [stats, setStats] = useState<ProfileFace | null>(null);
  const [faceLoading, setFaceLoading] = useState(false);
  const [statsLoading, setStatsLoading] = useState(false);
  const [pictureFailed, setPictureFailed] = useState<string | null>(null);
  const [thumbFailed, setThumbFailed] = useState(false);
  const [busy, setBusy] = useState(false);

  // Each part arrives on its own: whose it is first, their numbers a moment later.
  useEffect(() => {
    setRef(null);
    setFace(null);
    setStats(null);
    setThumbFailed(false);
    if (!url) return;
    let stale = false;
    setFaceLoading(true);
    setStatsLoading(true);
    api
      .profileOf(url)
      .then((found) => !stale && setRef(found))
      .catch(() => {});
    api
      .profileFace(url)
      .then((found) => !stale && setFace(found))
      .catch(() => {})
      .finally(() => !stale && setFaceLoading(false));
    api
      .profileStats(url)
      .then((found) => !stale && setStats(found))
      .catch(() => {})
      .finally(() => !stale && setStatsLoading(false));
    return () => {
      stale = true;
    };
  }, [url]);

  const who = merge(face, stats);
  const username = who?.username || ref?.name || usernameIn(url);
  const name = who?.name || video.uploader || username || "Instagram";
  const handle = username ? `@${username}` : null;
  const picture = who?.picture && pictureFailed !== who.picture ? who.picture : null;
  const waiting = faceLoading || statsLoading;
  const exact = (n: number) => (who?.rounded ? short(n) : num(n));

  const numbers: [string, string][] = who
    ? ([
        who.posts != null ? [exact(who.posts), t.views.home.numbers.posts(who.posts)] : null,
        who.followers != null ? [short(who.followers), t.views.home.numbers.followers(who.followers)] : null,
        who.following != null ? [short(who.following), t.views.home.numbers.following(who.following)] : null,
      ].filter(Boolean) as [string, string][])
    : [];
  const links = (who?.links ?? []).slice(0, 6);

  /** The post's own numbers. */
  const postNumbers = [
    video.likes != null ? { icon: Heart, figure: short(video.likes), what: t.views.home.numbers.likes(video.likes) } : null,
    video.comments != null
      ? { icon: MessageCircle, figure: short(video.comments), what: t.poster.post.comments(video.comments) }
      : null,
    video.views != null ? { icon: Play, figure: short(video.views), what: t.poster.post.views(video.views) } : null,
  ].filter(Boolean) as { icon: typeof Heart; figure: string; what: string }[];

  /** The caption's first lines, else the title yt-dlp gave ("Video by @someone"). */
  const caption = video.caption?.trim() || video.title;
  const portrait = !!video.width && !!video.height && video.height > video.width;
  const ratio = video.width && video.height ? `${video.width} / ${video.height}` : portrait ? "9 / 16" : "1 / 1";

  const choices = [
    { value: "best", label: t.views.quality.best },
    ...video.heights.map((h) => ({
      value: String(h),
      label: h === 2160 ? t.views.quality.fourK(h) : t.common.quality.height(String(h)),
    })),
    { value: "audio", label: t.views.quality.audioOnly },
  ];

  return (
    <section className="video-card ig-card" aria-label={t.poster.post.region}>
      {/* whose post it is */}
      <aside className="ig-profile" aria-label={t.poster.region}>
        <div className="ig-head">
          {picture ? (
            <img
              className="ig-face is-arriving"
              src={picture}
              alt=""
              referrerPolicy="no-referrer"
              onError={() => setPictureFailed(picture)}
            />
          ) : waiting ? (
            <span className="ig-face shimmer" aria-hidden="true" />
          ) : (
            <span className="ig-face ig-badge" aria-hidden="true">
              <Users size={22} strokeWidth={2} />
            </span>
          )}
          <div className="ig-who">
            <strong className="ig-name" title={name}>{name}</strong>
            {handle && (
              <button
                type="button"
                className="ig-handle"
                title={t.views.home.openLink(url ?? "")}
                onClick={() => url && api.openWebLink(url).catch((e) => actions.toast(String(e), "error"))}
              >
                {handle}
              </button>
            )}
          </div>
        </div>

        {numbers.length > 0 ? (
          <dl className="ig-numbers is-arriving">
            {numbers.map(([figure, what]) => (
              <div key={what}>
                <dt>{figure}</dt>
                <dd>{what}</dd>
              </div>
            ))}
          </dl>
        ) : (
          waiting && (
            <div className="ig-numbers" aria-hidden="true">
              {[0, 1, 2].map((i) => (
                <span key={i} className="shimmer shimmer-number" style={{ width: "100%" }} />
              ))}
            </div>
          )
        )}

        {who?.about ? (
          <p className="profile-about ig-about is-arriving" title={who.about}>
            {who.about}
          </p>
        ) : (
          waiting && (
            <div className="shimmer-lines" aria-hidden="true">
              <span className="shimmer shimmer-about" />
            </div>
          )
        )}

        {links.length > 0 && (
          <ul className="profile-links is-arriving" aria-label={t.views.home.links}>
            {links.map((link) => (
              <li key={link}>
                <button
                  type="button"
                  className="profile-link"
                  title={t.views.home.openLink(link)}
                  onClick={() => api.openWebLink(link).catch((e) => actions.toast(String(e), "error"))}
                >
                  <Globe size={12} strokeWidth={2} aria-hidden="true" />
                  <span>{linkText(link)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}

        {postNumbers.length > 0 && (
          <div className="ig-post-numbers">
            <span className="ig-label">{t.poster.post.thisPost}</span>
            <p>
              {postNumbers.map(({ icon: Icon, figure, what }) => (
                <span key={what} title={`${figure} ${what}`}>
                  <Icon size={13} strokeWidth={2.2} aria-hidden="true" />
                  <strong>{figure}</strong> {what}
                </span>
              ))}
            </p>
          </div>
        )}

        {ref && url && (
          <button
            type="button"
            className="btn btn-small ig-whole"
            disabled={busy}
            title={t.poster.downloadProfileTitle(handle ?? name, ref.site)}
            onClick={async () => {
              setBusy(true);
              try {
                await actions.submitLink(url, qualityOptions(props.choice));
              } finally {
                setBusy(false);
              }
            }}
          >
            {t.poster.downloadProfile}
          </button>
        )}
      </aside>

      {/* the post itself */}
      <div className="ig-media">
        {video.thumbnail && !thumbFailed ? (
          <img
            className={`ig-thumb${portrait ? " is-portrait" : ""}`}
            style={{ aspectRatio: ratio }}
            src={video.thumbnail}
            alt=""
            referrerPolicy="no-referrer"
            onError={() => setThumbFailed(true)}
          />
        ) : (
          <span className={`ig-thumb video-thumb-empty${portrait ? " is-portrait" : ""}`} style={{ aspectRatio: ratio }} aria-hidden="true" />
        )}
        <div className="ig-media-body">
          <h2 className="ig-caption" title={caption}>
            {caption}
          </h2>
          <p className="video-meta">
            {[
              video.site,
              video.duration ? formatDuration(video.duration) : null,
              video.width && video.height ? `${video.width} × ${video.height}` : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
          <div className="video-choices" role="radiogroup" aria-label={t.views.home.resolution}>
            {choices.map((option) => (
              <button
                key={option.value}
                type="button"
                role="radio"
                aria-checked={props.choice === option.value}
                className={`video-choice${props.choice === option.value ? " is-on" : ""}`}
                onClick={() => props.onChoice(option.value)}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>
      </div>
    </section>
  );
}
