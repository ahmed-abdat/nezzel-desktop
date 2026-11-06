import type { SourceMediaKind } from './media';

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
};
