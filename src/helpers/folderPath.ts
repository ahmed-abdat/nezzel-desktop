/** Folder path helpers for display. Paths come from the OS (POSIX or Windows separators). */

const SEPARATORS = /[\\/]/;

/** Non-empty segments of a path, either separator style. */
export function pathParts(path: string): string[] {
  return path.split(SEPARATORS).filter(Boolean);
}

/** Last segment of a path (empty after a trailing separator). */
export function baseName(path: string): string {
  return path.split(SEPARATORS).pop() ?? '';
}

/** Last two segments joined with " / " ("Downloads / Nezzel"); a root-level folder shows alone. */
export function compactFolderPath(path: string, separator = ' / '): string {
  return pathParts(path).slice(-2).join(separator);
}

/** Parent folder of `path`, keeping its separator style; null when there is none. */
export function parentFolder(path: string): string | null {
  const trimmed = path.replace(/[\\/]+$/, '');
  const index = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  if (index <= 0) return null;
  return trimmed.slice(0, index);
}

/** "~/Downloads/Nezzel" style: the user's home folder becomes "~" (macOS, Linux, Windows). */
export function homeRelativePath(path: string): string {
  return path
    .replace(/^\/(Users|home)\/[^/]+(?=\/|$)/, '~')
    .replace(/^[A-Za-z]:\\Users\\[^\\]+(?=\\|$)/, '~');
}

/** Keeps both ends of a long path: "~/Movies/…/Season 2/Clips". */
export function middleTruncate(text: string, max = 36): string {
  if (text.length <= max) return text;
  const keep = max - 1;
  const head = Math.ceil(keep / 2);
  return `${text.slice(0, head)}…${text.slice(text.length - (keep - head))}`;
}

/** Short display form for a folder path. */
export function shortFolderPath(path: string, max = 36): string {
  return middleTruncate(homeRelativePath(path), max);
}

/** Rust returns native absolute paths; normalize Windows separators/case for session dedupe. */
export function libraryPathKey(path: string): string {
  let normalized = path.replace(/\\/g, '/');
  if (normalized.startsWith('//?/UNC/')) normalized = `//${normalized.slice(8)}`;
  else if (normalized.startsWith('//?/')) normalized = normalized.slice(4);
  return /^(?:[a-z]:\/|\/\/)/i.test(normalized) ? normalized.toLowerCase() : path;
}
