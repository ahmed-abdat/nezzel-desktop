import { canOpenFile } from './mediaKind';

export type FileCategory = 'document' | 'spreadsheet' | 'presentation' | 'archive' | 'installer' | 'diskImage' | 'data' | 'other';

const BY_EXTENSION: Record<string, FileCategory> = {};
const add = (category: FileCategory, extensions: string) => {
  for (const extension of extensions.split(' ')) BY_EXTENSION[extension] = category;
};
add('document', 'pdf doc docx odt txt rtf');
add('spreadsheet', 'xls xlsx csv ods');
add('presentation', 'ppt pptx key odp');
add('archive', 'zip rar 7z tar gz tgz xz bz2');
add('installer', 'exe msi dmg pkg deb rpm appimage apk');
add('diskImage', 'iso img');
add('data', 'json xml');

const BY_MIME: Record<string, FileCategory> = {
  'application/pdf': 'document',
  'application/msword': 'document',
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document': 'document',
  'application/vnd.oasis.opendocument.text': 'document',
  'application/rtf': 'document',
  'text/plain': 'document',
  'application/vnd.ms-excel': 'spreadsheet',
  'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet': 'spreadsheet',
  'application/vnd.oasis.opendocument.spreadsheet': 'spreadsheet',
  'text/csv': 'spreadsheet',
  'application/vnd.ms-powerpoint': 'presentation',
  'application/vnd.openxmlformats-officedocument.presentationml.presentation': 'presentation',
  'application/vnd.oasis.opendocument.presentation': 'presentation',
  'application/zip': 'archive',
  'application/x-rar-compressed': 'archive',
  'application/vnd.rar': 'archive',
  'application/x-7z-compressed': 'archive',
  'application/x-tar': 'archive',
  'application/gzip': 'archive',
  'application/x-xz': 'archive',
  'application/x-msdownload': 'installer',
  'application/vnd.microsoft.portable-executable': 'installer',
  'application/x-msi': 'installer',
  'application/x-apple-diskimage': 'installer',
  'application/vnd.android.package-archive': 'installer',
  'application/vnd.debian.binary-package': 'installer',
  'application/x-iso9660-image': 'diskImage',
  'application/json': 'data',
  'application/xml': 'data',
  'text/xml': 'data',
};

/** Category of a downloaded file: the extension decides, the MIME type fills in when the name has none we know. */
export function fileCategory(name: string | undefined, mime?: string): FileCategory {
  let path = name ?? '';
  try {
    if (/^https?:\/\//i.test(path)) path = decodeURIComponent(new URL(path).pathname);
  } catch { /* Keep malformed names as generic files. */ }
  const base = path.split(/[\\/]/).pop() ?? '';
  const dot = base.lastIndexOf('.');
  const byExtension = dot > 0 || (dot === 0 && base.length > 1) ? BY_EXTENSION[base.slice(dot + 1).toLowerCase()] : undefined;
  if (byExtension) return byExtension;
  return BY_MIME[(mime ?? '').split(';')[0].trim().toLowerCase()] ?? 'other';
}

/** Files that run or mount (installers, disk images, archives, unknown types) reveal their folder instead of opening. */
export function revealsInFolder(path: string | undefined): boolean {
  return !!path && !canOpenFile(path);
}

/** Host only (no scheme, path or query), so a signed download link never shows in the UI. */
export function hostOnly(url: string | undefined): string {
  try {
    return new URL(url ?? '').hostname.replace(/^www\./, '');
  } catch {
    return '';
  }
}
