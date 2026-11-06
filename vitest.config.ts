import { defineConfig } from 'vitest/config';
import vue from '@vitejs/plugin-vue';
import { fileURLToPath, URL } from 'node:url';

// Pure specs avoid the DOM and Tauri setup used by component/store tests.
const nodeSpecs = [
  'tests/unit/captureChoices.spec.ts',
  'tests/unit/captureFailure.spec.ts',
  'tests/unit/cookieBrowsers.spec.ts',
  'tests/unit/datePickerValue.spec.ts',
  'tests/unit/downloadSelection.spec.ts',
  'tests/unit/errorOutcome.spec.ts',
  'tests/unit/fileCategory.spec.ts',
  'tests/unit/folderPath.spec.ts',
  'tests/unit/formats.spec.ts',
  'tests/unit/inputFilters.spec.ts',
  'tests/unit/listFilter.spec.ts',
  'tests/unit/localeNumber.spec.ts',
  'tests/unit/mediaKind.spec.ts',
  'tests/unit/paneScroll.spec.ts',
  'tests/unit/partialDownload.spec.ts',
  'tests/unit/playlistNumbering.spec.ts',
  'tests/unit/playlistSelection.spec.ts',
  'tests/unit/proxyUrl.spec.ts',
  'tests/unit/quickChoice.spec.ts',
  'tests/unit/resolutionSelection.spec.ts',
  'tests/unit/siteName.spec.ts',
  'tests/unit/skippedDiagnostics.spec.ts',
  'tests/unit/subtitlePreferences.spec.ts',
  'tests/unit/subtitleSettings.spec.ts',
  'tests/unit/units.spec.ts',
  'tests/unit/urlImport.spec.ts',
  'tests/unit/useDuration.spec.ts',
  'tests/unit/useLinkify.spec.ts',
  'tests/unit/useMediaResolutionSelection.spec.ts',
];

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  test: {
    // Reuse workers while keeping each spec in its own VM context.
    pool: 'vmThreads',
    maxWorkers: 2,
    coverage: {
      enabled: process.env.CI === 'true',
      provider: 'v8',
      include: ['src/**/*.{ts,vue}'],
      reportsDirectory: 'coverage/units',
      reporter: ['html', 'lcovonly', 'text'],
    },
    globals: true,
    projects: [
      {
        extends: true,
        test: { name: 'node', environment: 'node', include: nodeSpecs },
      },
      {
        extends: true,
        test: {
          name: 'dom',
          environment: 'jsdom',
          setupFiles: ['./vitest.setup.ts'],
          include: ['tests/unit/**/*.ts'],
          exclude: nodeSpecs,
        },
      },
    ],
  },
});
