/* English text of views: the Home, Downloads, Browser, Sign in and Settings
 * pages, the transfer summary in the sidebar and the About dialog
 * (src/views.tsx). See src/i18n/README for how to add keys. */
import { num, pct, plural } from "../format";

export const views = {
  /** Toasts more than one page shows. */
  downloadStarted: "Download started",
  /** Quality choices for a video. */
  quality: {
    best: "Best",
    audioOnly: "Audio only",
    /** 2160 */
    fourK: (h: number) => `${h}p 4K`,
  },

  browser: {
    /** What a caught file is, at the start of its line: "Stream · every quality". */
    stream: "Stream",
    audio: "Audio",
    video: "Video",
    everyQuality: "every quality",
    previewClip: "preview clip",
    autoplayFailed: "Could not start this video by itself. Press play on the page to see its qualities",
    videoFound: "Video found. Choose a quality and press Download",
    back: "Back",
    forward: "Forward",
    reload: "Reload",
    addressPlaceholder: "Type a web address, open the page and play its video",
    address: "Web address",
    go: "Go",
    popup: (name: string) => `The page tried to open a new window: ${name}`,
    openHere: "Open it here",
    ignore: "Ignore",
    finding:
      "Finding this page's video: starting the player so its qualities show up. Nothing downloads until you press Download.",
    stop: "Stop",
    foundRegion: "Media found on this page",
    fullVideo: "Full video",
    resolution: "Resolution",
    qualityLabel: "Quality",
    inDownloads: "In downloads",
    download: "Download",
    nothingCaught:
      "Nothing caught yet. Play the video on the page and it shows up here. To get another quality, choose it in the page's own player.",
    shownHere: "The page is shown here inside the app.",
    intro:
      "Open the page that has the video and press play. Every video or audio file the page loads is caught and listed with a Download button. Sign-ins made on the Sign in page work here too. Protected streaming services cannot be downloaded this way.",
  },

  signin: {
    googlePopup:
      "Google's sign-in opened in a small window of its own. If Google answers that this browser or app may not be secure, that is Google refusing to sign in inside other apps: sign in with your phone, email or username on this page instead, or import a cookies file.",
    popup: (host: string) => `${host} opened in a small window of its own to finish the sign-in. Close it when it is done.`,
    popupUnknown: "Another site opened in a small window of its own to finish the sign-in. Close it when it is done.",
    saved: "Sign-in saved",
    noneFound: "No sign-in was found",
    imported: "Sign-in imported",
    broughtOver: (n: number, browser: string) =>
      plural(n, { one: `Brought over # sign-in from ${browser}`, other: `Brought over # sign-ins from ${browser}` }),
    upToDate: (browser: string) => `Sign-ins from ${browser} are up to date`,
    forgotten: "Sign-ins forgotten",
    title: "Sign in",
    /** "to <site>, then save", beside the title while a site is open. */
    whereBefore: "to ",
    whereAfter: ", then save",
    saveAndClose: "Save sign-in and close",
    intro:
      "Some sites only show a profile or a video to signed-in visitors. Pick a site, sign in on its own page right here, then save. RushDownload keeps that sign-in on this computer and downloads as you.",
    browsersRegion: "Bring sign-ins over from a browser",
    elsewhereTitle: "Already signed in somewhere else?",
    elsewhereText:
      "Bring the sign-ins over from a browser on this computer, with nothing to type and no file to export. Close that browser first. Every sign-in it holds is copied, not only the ones for video sites.",
    reading: (browser: string) => `Reading ${browser}`,
    importFile: "Import a cookies file",
    browserNote: "Firefox works best. Chrome, Edge, Brave, Opera and Vivaldi lock their sign-ins on Windows, and often refuse.",
    /** Followed by the list of sites, in bold. */
    signedIn: "Signed in:",
    forget: "Forget sign-ins",
    notSignedIn: "Not signed in to any site.",
    shownHere: "The site's page is shown here inside the app.",
    googleNote:
      'Google and YouTube do not allow signing in inside other apps, here included. For those, sign in with your own browser, save that site\'s cookies as a cookies.txt file with a browser extension made for it, and import the file with the button above. On sites that offer "Sign in with Google", use the site\'s own password instead.',
  },

  home: {
    features: {
      fast: "Ultra Fast",
      fastDetail: "Multi-connection",
      safe: "Safe & Secure",
      safeDetail: "Encrypted DNS",
      builtIn: "Built-in",
      builtInDetail: "Browser",
      smart: "Smart",
      smartDetail: "Queue & resume",
    },
    featuresRegion: "What RushDownload does",
    welcome: "Welcome to RushDownload",
    subtitle: "Paste a link, or start a new download.",
    placeholder: "Paste any link: file, video page or torrent",
    linkLabel: "Link to download",
    download: "Download",
    noLinks: "No links were found in that file",
    added: (n: number) => plural(n, { one: "Added # download", other: "Added # downloads" }),

    /** A profile's numbers. The figure is shown in bold before the word. */
    numbers: {
      posts: (n: number) => plural(n, { one: "post", other: "posts" }),
      media: (n: number) => plural(n, { one: "with photos or videos", other: "with photos or videos" }),
      followers: (n: number) => plural(n, { one: "follower", other: "followers" }),
      following: (n: number) => plural(n, { one: "following", other: "following" }),
      /** "Joined Dec 2017" */
      joined: (date: string) => `Joined ${date}`,
    },
    /** What a playlist or profile holds. `plus` is "+" when there are more than counted, else "". */
    holds: {
      videos: (n: number, plus: string) => plural(n, { one: `#${plus} video`, other: `#${plus} videos` }),
      photos: (n: number, plus: string) => plural(n, { one: `#${plus} photo`, other: `#${plus} photos` }),
      total: (time: string) => `${time} total`,
    },
    playlist: "Playlist",
    profileOf: (site: string) => `${site} profile`,

    locked: {
      profileShown: (site: string) =>
        `${site} shows this profile to everyone, but lists its posts only for signed-in visitors.`,
      profileShownThisSite: "This site shows this profile to everyone, but lists its posts only for signed-in visitors.",
      site: (site: string) => `${site} only shows this to signed-in visitors.`,
      thisSite: "This site only shows this to signed-in visitors.",
      goYouTube:
        "Go to the Sign in page and bring your sign-in over from a browser (YouTube does not allow signing in inside other apps).",
      goSite: (site: string) => `Go to the Sign in page and sign in to ${site}.`,
      go: "Go to the Sign in page and sign in.",
      comeBack: "Then come back: your link stays here, ready to download.",
      button: "Go to Sign in",
    },

    findingVideo: "Finding the video on this page",
    lookingPage: "Looking at what this page holds",
    noVideo: "No video could be found on this page by itself.",
    openInBrowser: "Open it on the Browser page",
    caughtRegion: "Video found on this page",
    foundStillLooking: "Found on the page · still looking for more qualities",
    foundOnPage: "Found on the page",
    qualityLabel: "Quality",
    resolution: "Resolution",
    profileRegion: "Profile found at this link",
    videoRegion: "Video found at this link",

    loading: {
      resolutions: "Checking which resolutions it has",
      playlist: "Looking up the playlist",
      counting: "Counting what it holds",
      slow: (seconds: number, site: string) =>
        `Still counting what it holds · ${num(seconds)}s. ${site} can be slow to list posts; Download works without the count.`,
    },

    playlistNote:
      "Download saves every video of this playlist, in order, into a folder named after the playlist, in the quality picked above. A private playlist needs you to be signed in.",
    /** Around the folder's name, shown in bold. */
    profileNoteBefore: "Download saves every photo and video this profile has posted into a folder named ",
    profileNoteAfter: ", sorted into Images and Videos. Some sites only show a profile when you are signed in.",
    profileNoteAfterQuality:
      ", sorted into Images and Videos, in the quality picked above. Some sites only show a profile when you are signed in.",
    /** The folder's name: "john (Instagram)". */
    profileFolder: (name: string, site: string) => `${name} (${site})`,

    or: "or",
    addFromBrowserTitle: "Paste a page address and pick the media found on it",
    addFromBrowser: "Add from Browser",
    browseFilesTitle: "Open a .torrent file, or add every link listed in a text file",
    browseFiles: "Browse Files",
    downloadingRegion: "Downloading now",
    /** "3 downloading · 12 MB/s" */
    downloading: (n: number, speed: string) =>
      plural(n, { one: `# downloading · ${speed}`, other: `# downloading · ${speed}` }),
    checking: (percent: number) => `Checking ${pct(percent)}`,
  },

  downloads: {
    title: "Downloads",
    chips: {
      all: "All",
      running: "Downloading",
      completed: "Completed",
      paused: "Paused",
      failed: "Failed",
      queued: "Queued",
      deleted: "Deleted",
    },
    sorts: {
      newest: "Newest first",
      oldest: "Oldest first",
      name: "Name",
      largest: "Largest first",
      smallest: "Smallest first",
      fastest: "Fastest first",
    },
    types: {
      all: "All Types",
      video: "Video",
      audio: "Audio",
      image: "Images",
      document: "Documents",
      archive: "Archives",
      software: "Programs",
      other: "Others",
    },
    dates: {
      any: "Any time",
      today: "Today",
      week: "Last 7 days",
      month: "Last 30 days",
    },
    sizes: {
      any: "Any size",
      small: "Under 100 MB",
      medium: "100 MB to 1 GB",
      large: "Over 1 GB",
    },
    openFolder: "Open download folder",
    search: "Search downloads",
    hideFilters: "Hide filters",
    showFilters: "Show filters",
    sortBy: "Sort by",
    filterByStatus: "Filter by status",
    type: "Type",
    date: "Date",
    size: "Size",
    searchPlaceholder: "Search downloads...",
    clearSearch: "Clear search",
    bulkRegion: "Actions for selected downloads",
    selected: (n: number) => plural(n, { one: "# selected", other: "# selected" }),
    pause: "Pause",
    resume: "Resume",
    remove: "Remove",
    clearSelection: "Clear selection",
    emptyTitle: "No downloads yet",
    emptyText: "Paste a link on the Home page to start your first download.",
    goHome: "Go to Home",
    noMatchTitle: "Nothing matches",
    noMatchText: "No download fits the current status, filters and search.",
    clearFilters: "Clear filters",
  },

  summary: {
    region: "Transfer summary",
    speed: "Download speed",
    speedValue: (speed: string) => `Download speed ${speed}`,
    checkingTitle: "Checking what was already downloaded before carrying on",
    checking: "Checking data",
    sizeTitle: "Downloaded so far, of everything downloading, waiting or paused",
    percent: (n: number) => pct(n),
    total: "Total",
  },

  settings: {
    title: "Settings",
    sectionsLabel: "Settings sections",
    sections: {
      general: "General",
      downloads: "Downloads",
      network: "Network",
      performance: "Performance",
      scheduler: "Scheduler",
      appearance: "Appearance",
      advanced: "Advanced",
    },
    soon: "Soon",
    notBuilt: "Not built yet",
    folder: "Default Download Location",
    chooseFolder: "Choose folder",
    general: {
      clipWatch: "Put links I copy into the Home box",
      askFolder: "Ask where to save each profile or playlist",
      startup: "Launch on system startup",
      tray: "Minimize to tray",
      notifications: "Show download notifications",
      hardware: "Enable hardware acceleration",
      startupHint:
        "Opens when you sign in to the computer. With the tray on, it waits there.",
      trayHint:
        "Closing the window keeps RushDownload running in the tray, so downloads carry on.",
      notificationsHint:
        "Tells you when a download finishes or fails while the app is in the background.",
      hardwareHint:
        "Draws the window with the graphics card. Turn it off if the window flickers or stays blank. Takes effect after a restart.",
    },
    downloads: {
      folder: "Download folder",
      folderHint: "Where finished files are, in Explorer.",
      openFolder: "Open download folder",
      quality: "Default quality for video pages",
      qualityHint: "Used for links such as YouTube. Direct file links ignore it.",
      torrentApart: "Download a torrent's files side by side",
      torrentApartHint:
        "Each file of a torrent gets its own connections, up to 12 files at once. Turn this off if torrents get slower: some people sharing a torrent accept only one connection from you.",
      cookies: "Use a browser's sign-in",
      cookiesHint:
        "Reads a browser's saved sign-in. Chrome, Edge and Brave lock their sign-ins on Windows, so this usually only works with Firefox. The Sign in page in the sidebar is the reliable way.",
      off: "Off",
      useBrowser: (browser: string) => `Use my ${browser} sign-in`,
    },
    network: {
      dns: "Secure DNS",
      dnsHint:
        "On by default. Site names are looked up at Cloudflare over an encrypted connection, the way a browser's secure DNS setting does, so a network's own DNS cannot send a site to a dead address. A change applies to downloads at once, and to the Browser and Sign in pages after the app is restarted.",
      dnsOff: "Off (use this network's own)",
      connections: "Connections per download",
      connectionsHint:
        "Each file is split across this many connections. Servers that refuse extra connections are detected and fewer are used.",
      concurrent: "Downloads at once",
      concurrentHint: "The rest wait in the queue.",
    },
    performance: {
      maxSpeed: "Max download speed",
      limitedHint: "Shared by every download: together they stay under this speed.",
      unlimitedHint: "Downloads run as fast as the connection and the server allow.",
      unlimited: "Unlimited",
      manual: "Set manually",
    },
    /** "... with Ctrl Shift P and Ctrl Shift R." The keys are drawn between the parts. */
    scheduler: {
      before:
        "Scheduling downloads for a set time is not built yet. Until then, downloads start as soon as a slot is free, and you can pause or resume all of them at once with ",
      and: " and ",
      after: ".",
    },
    appearance: {
      theme: "Theme",
      system: "Match Windows",
      light: "Light",
      dark: "Dark",
      density: "List density",
      comfortable: "Comfortable",
      compact: "Compact",
      progress: "Progress bar",
      progressHint:
        "One bar per connection shows every part of a file filling up on its own. Site videos and torrents are fetched in order from the start, so their bars fill one after another.",
      single: "Single bar",
      threads: "One per connection",
    },
    advanced: {
      siteDownloads: "Site downloads",
      profileDownloads: "Profile downloads",
      checking: "Checking",
      sitesReady: "Ready. yt-dlp and ffmpeg were found.",
      /** `tools`: the missing programs, already joined into a list. */
      missing: (tools: string) => `Not ready. Missing ${tools}.`,
      galleryReady: "Ready. gallery-dl was found.",
      galleryMissing: "Not ready. Missing gallery-dl.",
      shortcuts: "Keyboard shortcuts",
    },
    /** What each keyboard shortcut does. */
    shortcuts: {
      add: "Add a download",
      paste: "Add the copied link",
      search: "Search commands and downloads",
      pages: "Home / Downloads",
      settings: "Settings",
      selectAll: "Select every download",
      toggle: "Pause or resume the selection",
      remove: "Remove the selection from the list",
      pauseAll: "Pause all",
      resumeAll: "Resume all",
      close: "Close the dialog or menu",
    },
  },

  about: {
    title: "About RushDownload",
    close: "Close",
    version: "Version",
    files: "File downloads",
    filesText: "HTTP and HTTPS, split across several connections, with pause and resume.",
    sites: "Site downloads",
    sitesText: "Video pages such as YouTube, Instagram and Vimeo, through yt-dlp and ffmpeg.",
    torrents: "Torrents",
    torrentsText: "Magnet links and links to .torrent files. Uploading stops when a download ends.",
    notBuilt: "Not built yet",
    notBuiltText: "Browser extension and scheduling.",
  },
};
