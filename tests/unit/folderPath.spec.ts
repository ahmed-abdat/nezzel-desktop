import { describe, expect, it } from 'vitest';
import { compactFolderPath, homeRelativePath, parentFolder, shortFolderPath } from '../../src/helpers/folderPath';

describe('folder paths for display', () => {
  it.each([
    ['/Users/ahmed/Downloads/Nezzel', '~/Downloads/Nezzel'],
    ['/home/ahmed/Downloads/Nezzel', '~/Downloads/Nezzel'],
    ['C:\\Users\\Ahmed\\Downloads\\Nezzel', '~\\Downloads\\Nezzel'],
    ['/Volumes/Media/Clips', '/Volumes/Media/Clips'],
    ['/Users/ahmedx', '~'],
  ])('home-relative %s', (input, expected) => {
    expect(homeRelativePath(input)).toBe(expected);
  });

  it('middle-truncates long paths, keeping both ends', () => {
    const short = shortFolderPath('/Users/a/Movies/Series/Very long series name/Season 2/Clips', 30);
    expect(short).toHaveLength(30);
    expect(short.startsWith('~/Movies')).toBe(true);
    expect(short.endsWith('Clips')).toBe(true);
    expect(short).toContain('…');
  });

  it('compacts to the last two folders and finds the parent', () => {
    expect(compactFolderPath('/Users/a/Downloads/Nezzel')).toBe('Downloads / Nezzel');
    expect(parentFolder('/Users/a/Downloads/Nezzel/')).toBe('/Users/a/Downloads');
    expect(parentFolder('C:\\Users\\a\\Downloads\\Nezzel')).toBe('C:\\Users\\a\\Downloads');
    expect(parentFolder('/Nezzel')).toBeNull();
  });
});
