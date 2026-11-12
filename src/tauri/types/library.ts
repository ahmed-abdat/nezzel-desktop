import type { SourceMediaKind, TrackType } from './media';

export type LibraryDisplay = {
  duration?: number | null;
  height?: number | null;
  fps?: number | null;
  trackType?: TrackType | null;
  attachmentSummary?: { photos: number; videos: number } | null;
};

export type LibraryRecord = {
  id: string;
  paths: string[];
  url: string;
  title: string;
  site?: string | null;
  kind: SourceMediaKind;
  size: number;
  completedAt: number;
  thumbnail?: string | null;
  display?: LibraryDisplay;
  errorCode?: string | null;
};
