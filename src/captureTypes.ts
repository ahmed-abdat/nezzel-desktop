export interface CaptureDetails {
  thumbnail: string | null;
  mediaKind: string | null;
  photos: number | null;
  duration: number | null;
  uploader: string;
  views: number | null;
  bestHeight: number | null;
  bestExt: string;
  size: number | null;
  /** The link has sound (separate or muxed): the Audio pick applies. */
  hasAudio: boolean;
  audioSize: number | null;
  /** The Smaller pick's height (≤720 below the best, else the next lower); null = one quality only. */
  smallHeight: number | null;
  smallSize: number | null;
}

export interface CaptureOffer {
  id: number;
  url: string;
  site: string;
  title: string;
  details: CaptureDetails;
  failed: boolean;
  /** Why the read failed (see helpers/captureFailure); null while reading or after success. */
  reason: string | null;
  resolving: boolean;
  feed: boolean;
}

export const emptyDetails = (): CaptureDetails => ({
  thumbnail: null, mediaKind: null, photos: null, duration: null, uploader: '', views: null, bestHeight: null, bestExt: '', size: null,
  hasAudio: false, audioSize: null, smallHeight: null, smallSize: null,
});
