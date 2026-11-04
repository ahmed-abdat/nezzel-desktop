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
}

export interface CaptureOffer {
  id: number;
  url: string;
  site: string;
  title: string;
  details: CaptureDetails;
  failed: boolean;
  resolving: boolean;
  feed: boolean;
}

export const emptyDetails = (): CaptureDetails => ({
  thumbnail: null, mediaKind: null, photos: null, duration: null, uploader: '', views: null, bestHeight: null, bestExt: '', size: null,
});
