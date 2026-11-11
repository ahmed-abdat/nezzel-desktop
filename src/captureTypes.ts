import type { CaptureChoice } from '@/generated/contract';

export interface CaptureDetails {
  title?: string | null;
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
  /** The copied-link card's format menu (Rust `CardChoice`): heights (+60 fps tiers), then audio bitrates. */
  choices?: CardChoice[];
}

/** One format the link offers, as plain facts (never a raw format id): what picking it sends, and its size. */
export interface CardChoice {
  /** A height (+ high frame rate tier) or an audio bitrate (none: the sound inside the video). */
  choice: Extract<CaptureChoice, { kind: 'video' | 'audio' }>;
  size: number | null;
}

export interface CaptureOffer {
  remoteImages: boolean;
  id: number;
  url: string;
  site: string;
  title: string;
  /** Preview completed, including an untitled result or a failed read. */
  read: boolean;
  details: CaptureDetails;
  failed: boolean;
  /** Why the read failed (see helpers/captureFailure); null while reading or after success. */
  reason: string | null;
  resolving: boolean;
  feed: boolean;
}

export const emptyDetails = (): CaptureDetails => ({
  thumbnail: null, mediaKind: null, photos: null, duration: null, uploader: '', views: null, bestHeight: null, bestExt: '', size: null,
  hasAudio: false, audioSize: null, smallHeight: null, smallSize: null, choices: [],
});
