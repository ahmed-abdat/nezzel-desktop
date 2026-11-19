import { Chapter, EntryItem, MediaAttachment, MediaCodec, MediaFormat, MediaItem, MediaTrack, SourceMediaKind } from './media';

export type Group = {
  id: string;
  mediaKind?: SourceMediaKind;
  attachments?: MediaAttachment[];
  savedAttachmentSummary?: { photos: number; videos: number };
  /** Whole seconds SponsorBlock cut out of the saved video. */
  sponsorRemovedSecs?: number;
  total: number;
  processed: number;
  errored: number;
  isCombined: boolean;
  items: Record<string, MediaItem>;
  url: string;
  title?: string;
  thumbnail?: string;
  description?: string;
  uploaderId?: string;
  uploader?: string;
  extractor?: string;
  views?: number;
  comments?: number;
  likes?: number;
  dislikes?: number;
  rating?: number;
  duration?: number;
  audioCodecs: MediaCodec[];
  videoCodecs?: MediaCodec[];
  audioTracks?: MediaTrack[];
  videoTracks?: MediaTrack[];
  formats: MediaFormat[];
  chapters?: Chapter[];
  entries?: EntryItem[];
  filesize: number;
  playlistId?: string;
  playlistTitle?: string;
  playlistCount?: number;
  playlistUploader?: string;
  playlistUploaderId?: string;
  /** Playlist this download came from (id of the playlist's own row); unset for single links. */
  parentId?: string;
  /** Position inside that playlist (0-based entry index). */
  parentIndex?: number;
  fromShortcut?: boolean;
  skipPlaylistSelection?: boolean;
};

/** A playlist shown as one row over its downloads (children carry `parentId`). */
export type PlaylistParent = {
  id: string;
  url?: string;
  title?: string;
  thumbnail?: string;
  uploader?: string;
  extractor?: string;
  /** Entries chosen for download. */
  count: number;
  /** Entries that could not be read and have no row of their own. */
  skipped: number;
};
