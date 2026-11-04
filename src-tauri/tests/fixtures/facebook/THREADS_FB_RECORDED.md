2026-11-02 fixture additions (no cookies):

- `share-video-hop.json`: response status/location for `https://www.facebook.com/share/v/1DJM2xDCFg/`, crawler UA. Redirect tracking parameters removed; preserves the accepted public video path/id. The test verifies the destination page is not requested.
- `fb-government-photo.html`: reduced browser Relay root for `https://www.facebook.com/governostp/posts/830881617425000`, Photo id `830881584091670`.
- `fb-ministry-photo.html`: reduced browser Relay root for `https://www.facebook.com/mic.gov.sl/posts/616512838936732`, Photo id `616512792270070`.

Photo fixtures preserve the actual post/media ids, media kind and contradictory crawler `og:type=video.other`; CDN URLs use a synthetic hostname/path. Comments, actors, reactions, tracking and other page data removed. These are photo posts in current authoritative Story attachments despite the matrix's expected video labels.
