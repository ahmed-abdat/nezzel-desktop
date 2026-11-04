Android Threads parser fixtures copied from `engine/src/test/resources/threads/` on 2026-11-02 (synthetic media, no live tokens).

`linked-inline-video.html` is a reduced 2026-11-02 browser-page capture of `https://www.threads.com/@nasa/post/DXVbTUPl-Qy`: preserves root code/pk/media_type=19 and its own `text_post_app_info.linked_inline_media` identity/type. Caption/CDN URL/dimensions replaced with synthetic values. Related posts/replies removed. This extra shape is newer than the Android parser's 1/2/8 cases.

Browser navigation headers return TargetQuery data; `facebookexternalhit/1.1` returned preview metadata without that preloader in this run. Fetch policy retains Android's browser navigation headers, no cookies, no redirects, 1.5 MiB cap, and the shared DNS-pinned transport.

`duplicate-code.html` derives from Android’s synthetic image fixture with a conflicting repeated `code` key, verifying Android’s strict duplicate-key rejection survives the Rust port.
