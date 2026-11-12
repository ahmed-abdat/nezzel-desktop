// Generated from src-tauri/src/contract.rs by `cargo test -- --ignored write_contract_ts`. Do not edit.

/** What a capture window asks for (`capture::Action`). */
export const CAPTURE_ACTIONS = ['download', 'options', 'adopt', 'dismiss', 'report'] as const;
export type CaptureAction = (typeof CAPTURE_ACTIONS)[number];

/** What a capture webview may send to `capture_action`. */
export const CAPTURE_WINDOW_ACTIONS = ['download', 'options', 'dismiss', 'report'] as const;
export type CaptureWindowAction = (typeof CAPTURE_WINDOW_ACTIONS)[number];

/** Actions that reach the main window's queue as a claim on an offer. */
export const CAPTURE_CLAIMS = ['download', 'options', 'adopt'] as const;
export type CaptureClaim = (typeof CAPTURE_CLAIMS)[number];

/** Why a capture read failed; each has `capture.fail.<reason>.title/.hint` (`preview::FailReason`). */
export const FAIL_REASONS = ['login', 'unavailable', 'unsupported', 'region', 'protected', 'busy', 'network', 'slow', 'unknown'] as const;
export type FailReason = (typeof FAIL_REASONS)[number];

/** Native labels of the two capture windows. */
export const CAPTURE_WINDOW = { card: 'capture-card', bar: 'capture-bar' } as const;
export type CaptureWindow = (typeof CAPTURE_WINDOW)[keyof typeof CAPTURE_WINDOW];

/** Native label of the close-to-tray note window. */
export const CLOSE_NOTE_WINDOW = 'close-note';

/** Copied-link card size in logical pixels: closed, and the tallest with its format list open. */
export const CAPTURE_CARD_SIZE = { width: 360, height: 160, maxHeight: 480 } as const;

/** Quick bar size in logical pixels as built; the bar then fits its height to its content. */
export const CAPTURE_BAR_SIZE = { width: 620, height: 360 } as const;

/** Card heights offered before the read and for links without their own format list. */
export const GENERIC_HEIGHTS = [1080, 720, 480] as const;

/** Rounded frame rates from here up are their own (high frame rate) format row. */
export const HIGH_FPS_MIN = 48;

/** The Smaller pick: the highest height at or under this. */
export const SMALL_MAX_HEIGHT = 720;

/** A quick pick or card format row (`capture::Choice`): what card rows offer and `capture_action` carries. Rust checks the numbers. */
export type CaptureChoice = { kind: 'best' } | { kind: 'small' } | { kind: 'audio'; kbps?: number } | { kind: 'video'; height: number; fps?: number };

/** How often the main window pulls the capture outbox (`capture_pull`), events or not. */
export const CAPTURE_PULL_MS = 1500;

/** Session kinds the capture copy depends on (`platform::Session`). */
export const CAPTURE_SESSIONS = ['x11', 'wayland', 'other'] as const;
export type CaptureSession = (typeof CAPTURE_SESSIONS)[number];

/** Linux desktop families whose tray words the copy uses (`platform::Desktop`). */
export const LINUX_DESKTOPS = ['gnome', 'kde', 'other'] as const;
export type LinuxDesktop = (typeof LINUX_DESKTOPS)[number];

/** Welcome "Try it" sample link. */
export const TRY_LINK = 'https://www.youtube.com/watch?v=jNQXAC9IVRw';

/** Quick download shortcut, and what is registered when the default is taken. */
export const DEFAULT_CAPTURE_SHORTCUT = 'CommandOrControl+Alt+N';
export const FALLBACK_CAPTURE_SHORTCUT = 'CommandOrControl+Alt+Shift+D';

/** Usage keys the web UI may count; Rust drops any other. */
export const UI_USAGE_KEYS = ['paste', 'pause', 'pause_all', 'retry', 'report.opened', 'funnel.batch'] as const;
export type UiUsageKey = (typeof UI_USAGE_KEYS)[number];

/** Browsers a login can come from (yt-dlp names), in the order Settings lists them. */
export const COOKIE_BROWSERS = ['chrome', 'firefox', 'safari', 'edge', 'brave', 'opera', 'vivaldi', 'chromium', 'whale'] as const;
export type CookieBrowser = (typeof COOKIE_BROWSERS)[number];

/** `auth.cookieBrowser`: find a signed-in profile when a site refuses. */
export const COOKIE_BROWSER_AUTOMATIC = 'automatic';

/** `auth.cookieBrowser`: never use browser logins. */
export const COOKIE_BROWSER_NEVER = 'none';

/** Login reason: every signed-in profile is behind Windows app-bound encryption. */
export const LOGIN_APP_BOUND_REASON = 'browserAppBound';

/** How long the engine keeps a login request. */
export const LOGIN_TTL_MS = 900000;

/** Error codes a login can fix. */
export const LOGIN_CODES = ['signInRequired', 'signInRequiredForBotDetection', 'membersOnly', 'videoPasswordRequired'] as const;
export type LoginCode = (typeof LOGIN_CODES)[number];

/** Error codes the site decides: retrying the same link fails the same way. */
export const PERMANENT_CODES = ['urlNotSupported', 'urlUnsupported', 'unknownUrlType', 'fileNotFile', 'notFound404', 'copyrightClaimed', 'channelUnavailable', 'videoNotFoundOrPrivate', 'playlistPrivateOrMissing', 'geoBlocked', 'drmProtected', 'membersOnly', 'photosOnly', 'noMediaFound', 'fileEmpty'] as const;
export type PermanentCode = (typeof PERMANENT_CODES)[number];

/** Subfolder of the download folder for file-share downloads. */
export const FILES_FOLDER = 'Files';

/** Languages written right to left (primary subtags). */
export const RTL_LANGS = ['ar'] as const;
export type RtlLang = (typeof RTL_LANGS)[number];

/** Privacy switches until the config has loaded (Rust `PrivacySettings::default`). */
export const PRIVACY_DEFAULTS = { sendReports: true, sendUsage: true } as const;

/** Why a report the user sent did not go (`diagnostics::send::SendError`); only `notDelivered` is worth a retry. */
export const SEND_ERRORS = ['notConfigured', 'notDelivered', 'nothingToSend'] as const;
export type SendError = (typeof SEND_ERRORS)[number];

/** Login reasons the engine gives itself; any other `LoginNeeded.reason` is the site's diagnostic code. */
export const LOGIN_REASONS = ['browserAppBound', 'changeProfile', 'browserLoginFailed'] as const;
export type LoginReason = (typeof LOGIN_REASONS)[number];

/** Row events the engine emits to the webview (`events.rs`). */
export const ROW_EVENTS = { mediaAttempts: 'media_attempts', mediaComplete: 'media_complete', mediaDestination: 'media_destination', mediaDiagnostic: 'media_diagnostic', mediaFatal: 'media_fatal', mediaLoginNeeded: 'media_login_needed', mediaLoginUsing: 'media_login_using', mediaLoginNotice: 'media_login_notice', mediaProgress: 'media_progress', mediaProgressStage: 'media_progress_stage', mediaTransport: 'media_transport' } as const;

/** Known sites (`sites.rs`): display name, hosts (the host or a subdomain), yt-dlp extractor keys. */
export const SITES = [
  { key: 'youtube', name: 'YouTube', hosts: ['youtube.com', 'youtu.be', 'youtube-nocookie.com'], extractors: ['youtube', 'youtubetab'] },
  { key: 'vimeo', name: 'Vimeo', hosts: ['vimeo.com'], extractors: ['vimeo'] },
  { key: 'tiktok', name: 'TikTok', hosts: ['tiktok.com'], extractors: ['tiktok'] },
  { key: 'instagram', name: 'Instagram', hosts: ['instagram.com'], extractors: ['instagram'] },
  { key: 'facebook', name: 'Facebook', hosts: ['facebook.com', 'fb.watch'], extractors: ['facebook'] },
  { key: 'x', name: 'X', hosts: ['x.com', 'twitter.com', 'fxtwitter.com', 'vxtwitter.com'], extractors: ['twitter'] },
  { key: 'twitch', name: 'Twitch', hosts: ['twitch.tv'], extractors: ['twitch'] },
  { key: 'soundcloud', name: 'SoundCloud', hosts: ['soundcloud.com'], extractors: ['soundcloud'] },
  { key: 'dailymotion', name: 'Dailymotion', hosts: ['dailymotion.com', 'dai.ly'], extractors: ['dailymotion'] },
  { key: 'reddit', name: 'Reddit', hosts: ['reddit.com', 'redd.it'], extractors: ['reddit'] },
  { key: 'bilibili', name: 'Bilibili', hosts: ['bilibili.com', 'b23.tv'], extractors: [] },
  { key: 'pinterest', name: 'Pinterest', hosts: ['pinterest.com', 'pin.it'], extractors: [] },
  { key: 'ted', name: 'TED', hosts: ['ted.com'], extractors: [] },
  { key: 'bandcamp', name: 'Bandcamp', hosts: ['bandcamp.com'], extractors: [] },
  { key: 'archive', name: 'Internet Archive', hosts: ['archive.org'], extractors: [] },
  { key: 'rumble', name: 'Rumble', hosts: ['rumble.com'], extractors: [] },
  { key: 'odysee', name: 'Odysee', hosts: ['odysee.com'], extractors: [] },
  { key: 'linkedin', name: 'LinkedIn', hosts: ['linkedin.com', 'lnkd.in'], extractors: [] },
  { key: 'threads', name: 'Threads', hosts: ['threads.net', 'threads.com'], extractors: [] },
  { key: 'bluesky', name: 'Bluesky', hosts: ['bsky.app'], extractors: [] },
  { key: 'telegram', name: 'Telegram', hosts: ['t.me', 'telegram.me'], extractors: [] },
  { key: 'snapchat', name: 'Snapchat', hosts: ['snapchat.com'], extractors: [] },
  { key: 'googledrive', name: 'Google Drive', hosts: ['drive.google.com'], extractors: [] },
  { key: 'googledocs', name: 'Google Docs', hosts: ['docs.google.com'], extractors: [] },
  { key: 'dropbox', name: 'Dropbox', hosts: ['dropbox.com'], extractors: [] },
  { key: 'tumblr', name: 'Tumblr', hosts: ['tumblr.com'], extractors: ['tumblr'] },
  { key: 'kick', name: 'Kick', hosts: ['kick.com'], extractors: ['kick:clips'] },
  { key: 'vk', name: 'VK', hosts: ['vk.com', 'vkvideo.ru'], extractors: ['vk'] },
  { key: 'ok', name: 'OK.ru', hosts: ['ok.ru'], extractors: ['odnoklassniki'] },
  { key: 'streamable', name: 'Streamable', hosts: ['streamable.com'], extractors: ['streamable'] },
  { key: 'imgur', name: 'Imgur', hosts: ['imgur.com'], extractors: ['imgur'] },
  { key: '9gag', name: '9GAG', hosts: ['9gag.com'], extractors: ['9gag'] },
  { key: 'likee', name: 'Likee', hosts: ['likee.video'], extractors: ['likee'] },
  { key: 'kwai', name: 'Kwai', hosts: ['kwai.com'], extractors: [] },
  { key: 'flickr', name: 'Flickr', hosts: ['flickr.com', 'flic.kr'], extractors: ['flickr'] },
] as const;
