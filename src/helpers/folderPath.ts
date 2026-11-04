/** Folder path helpers for display. Paths come from the OS (POSIX or Windows separators). */

function parts(path: string): string[] {
  return path.split(/[\\/]/).filter(Boolean);
}

/** Last two segments joined with " / " ("Downloads / Nezzel"); a root-level folder shows alone. */
export function compactFolderPath(path: string): string {
  return parts(path).slice(-2).join(' / ');
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
