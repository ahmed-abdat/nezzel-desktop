import { describe, expect, it } from 'vitest';
import { fileCategory, revealsInFolder } from '../../src/helpers/fileCategory';

describe('fileCategory', () => {
  it.each([
    ['https://www.dropbox.com/s/abc/report.pdf?dl=0', undefined, 'document'],
    ['https://files.example/%D8%AA%D9%82%D8%B1%D9%8A%D8%B1.zip?token=secret', undefined, 'archive'],
    ['report.pdf', undefined, 'document'],
    ['Notes.DOCX', undefined, 'document'],
    ['a.odt', undefined, 'document'],
    ['a.txt', undefined, 'document'],
    ['a.rtf', undefined, 'document'],
    ['data.xlsx', undefined, 'spreadsheet'],
    ['data.csv', undefined, 'spreadsheet'],
    ['deck.pptx', undefined, 'presentation'],
    ['deck.key', undefined, 'presentation'],
    ['backup.tar.gz', undefined, 'archive'],
    ['x.zip', undefined, 'archive'],
    ['x.7z', undefined, 'archive'],
    ['setup.exe', undefined, 'installer'],
    ['App.dmg', undefined, 'installer'],
    ['tool.AppImage', undefined, 'installer'],
    ['app.apk', undefined, 'installer'],
    ['ubuntu.iso', undefined, 'diskImage'],
    ['disk.img', undefined, 'diskImage'],
    ['config.json', undefined, 'data'],
    ['feed.xml', undefined, 'data'],
    ['/Users/me/Files/تقرير.pdf', undefined, 'document'],
    ['C:\\Users\\me\\a.zip', undefined, 'archive'],
    ['mystery.bin', undefined, 'other'],
    ['noextension', undefined, 'other'],
    ['.hidden', undefined, 'other'],
    [undefined, undefined, 'other'],
    ['download', 'application/pdf', 'document'],
    ['download', 'application/zip; charset=binary', 'archive'],
    ['download', 'application/x-apple-diskimage', 'installer'],
    ['download', 'application/octet-stream', 'other'],
    ['file.pdf', 'application/zip', 'document'],
  ] as const)('%s %s → %s', (name, mime, expected) => {
    expect(fileCategory(name, mime)).toBe(expected);
  });
});

describe('revealsInFolder', () => {
  it.each([['/a/setup.exe', true], ['/a/x.zip', true], ['/a/x.pdf', false], [undefined, false]] as const)('%s → %s', (path, expected) => {
    expect(revealsInFolder(path)).toBe(expected);
  });
});
