"""latest.json contract: every shipped platform, primary installer per OS (`pnpm run test:tools`: Rust CI + nightly)."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('updater_json', Path(__file__).with_name('updater-json.py'))
updater = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updater)

# The asset names of the beta.1 draft release.
RELEASE = [
    'Nezzel_0.1.0-beta.1_aarch64.app.tar.gz', 'Nezzel_0.1.0-beta.1_x64.app.tar.gz',
    'Nezzel_0.1.0-beta.1_x64-setup.exe', 'Nezzel_0.1.0-beta.1_arm64-setup.exe',
    'Nezzel_0.1.0-beta.1_amd64.AppImage', 'Nezzel_0.1.0-beta.1_aarch64.AppImage',
    'Nezzel_0.1.0-beta.1_amd64.deb', 'Nezzel_0.1.0-beta.1_arm64.deb',
    'Nezzel-0.1.0-beta.1-1.x86_64.rpm', 'Nezzel-0.1.0-beta.1-1.aarch64.rpm',
]


def manifest(names):
    assets = [(name, f'https://api.example/assets/{name}') for name in names]
    return updater.build_manifest(assets, lambda sig: f'sig of {sig}\n', '0.1.0-beta.1', 'notes', 'now')


class UpdaterJsonTests(unittest.TestCase):
    def test_release_assets_map_to_the_tauri_action_keys(self):
        names = RELEASE + [name + '.sig' for name in RELEASE] + ['latest.json', 'Nezzel_0.1.0-beta.1_aarch64.dmg']
        platforms = manifest(names)['platforms']
        self.assertEqual(sorted(platforms), sorted([
            'darwin-aarch64', 'darwin-aarch64-app', 'darwin-x86_64', 'darwin-x86_64-app',
            'linux-aarch64', 'linux-aarch64-appimage', 'linux-aarch64-deb', 'linux-aarch64-rpm',
            'linux-x86_64', 'linux-x86_64-appimage', 'linux-x86_64-deb', 'linux-x86_64-rpm',
            'windows-aarch64', 'windows-aarch64-nsis', 'windows-x86_64', 'windows-x86_64-nsis',
        ]))
        # Primary entries: the .app, NSIS and AppImage, never the .deb/.rpm.
        self.assertTrue(platforms['linux-x86_64']['url'].endswith('_amd64.AppImage'))
        self.assertTrue(platforms['linux-aarch64']['url'].endswith('_aarch64.AppImage'))
        self.assertTrue(platforms['windows-aarch64']['url'].endswith('_arm64-setup.exe'))
        self.assertTrue(platforms['darwin-x86_64']['url'].endswith('_x64.app.tar.gz'))
        self.assertEqual(platforms['darwin-aarch64']['signature'], 'sig of Nezzel_0.1.0-beta.1_aarch64.app.tar.gz.sig')
        self.assertEqual(updater.missing(manifest(names)), [])

    def test_a_failed_leg_or_unsigned_bundle_is_reported_missing(self):
        # The aarch64 AppImage leg failed: deb/rpm alone don't make linux-aarch64.
        names = [n for n in RELEASE if n != 'Nezzel_0.1.0-beta.1_aarch64.AppImage']
        names += [n + '.sig' for n in names]
        # x64 NSIS uploaded without its signature.
        names.remove('Nezzel_0.1.0-beta.1_x64-setup.exe.sig')
        self.assertEqual(updater.missing(manifest(names)), [
            'windows-x86_64', 'windows-x86_64-nsis', 'linux-aarch64', 'linux-aarch64-appimage',
        ])
        # A deb/rpm leg failed while every primary entry is there: still incomplete.
        names = [n for n in RELEASE if not n.endswith(('.deb', '.rpm'))]
        names += [n + '.sig' for n in names]
        self.assertEqual(updater.missing(manifest(names)), [
            'linux-aarch64-deb', 'linux-aarch64-rpm', 'linux-x86_64-deb', 'linux-x86_64-rpm',
        ])
        self.assertEqual(len(updater.REQUIRED), 16)


if __name__ == '__main__':
    unittest.main()
