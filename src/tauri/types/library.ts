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

/** Rust returns native absolute paths; normalize Windows separators/case for session dedupe. */
export function libraryPathKey(path: string): string {
  let normalized = path.replace(/\\/g, '/');
  if (normalized.startsWith('//?/UNC/')) normalized = `//${normalized.slice(8)}`;
  else if (normalized.startsWith('//?/')) normalized = normalized.slice(4);
  return /^(?:[a-z]:\/|\/\/)/i.test(normalized) ? normalized.toLowerCase() : path;
}
