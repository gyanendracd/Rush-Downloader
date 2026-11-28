import { plural } from "../format";

/* Who posted a video, on Home's video card (src/Poster.tsx). */
export const poster = {
  region: "Posted by",
  downloadProfile: "Download whole profile",
  downloadProfileTitle: (name: string, site: string) => `Download everything ${name} has posted on ${site}, into a folder of its own`,
  /** The site, after the handle: "@someone · Instagram". */
  on: (site: string) => site,
  /** An Instagram post or reel on Home (src/InstaPost.tsx): whose it is beside the post. */
  post: {
    region: "Instagram post found at this link",
    /** Over the likes and comments the post has had. */
    thisPost: "This post",
    comments: (n: number) => plural(n, { one: "comment", other: "comments" }),
    views: (n: number) => plural(n, { one: "view", other: "views" }),
  },
};
