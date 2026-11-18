# rc-012 recorded fixtures

`gallery-recorded-*.json` are trimmed from the Opus triage recordings under
`rc-012` retest scratchpad `work/{ig,x,b,p}.json` (gallery-dl 1.32.19).
Descriptions, author profiles, unrelated metadata and Instagram CDN query signatures
are omitted; record types, delegated URLs, video_url, post IDs and positions are retained.
`ytdlp-recorded-vimeo.json` retains the identity fields from `tri/vimeo.json`
(yt-dlp 2026.10.26). Stream tests repeat the recorded entry with playlist position
metadata to exercise the cap without network calls.
