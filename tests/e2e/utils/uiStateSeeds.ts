/**
 * Browser-side seeds for the UI state gallery (tests/e2e/ui-states.spec.ts).
 * Loaded into the running dev app with `import('/tests/e2e/utils/uiStateSeeds.ts')`, so it shares
 * the app's Pinia stores and i18n. Each seed puts the app in one state through the same store
 * calls the Tauri listeners make; nothing here touches engine logic.
 */
import { i18n, applyDocumentLocale, type Locale } from '../../../src/i18n';
import { applyTheme } from '../../../src/composables/useTheme';
import { MediaState, useMediaStateStore } from '../../../src/stores/media/state';
import { useMediaGroupStore } from '../../../src/stores/media/group';
import { useMediaProgressStore } from '../../../src/stores/media/progress';
import { useMediaDestinationStore } from '../../../src/stores/media/destination';
import { useMediaDiagnosticsStore } from '../../../src/stores/media/diagnostics';
import { useMediaOptionsStore } from '../../../src/stores/media/options';
import { useLoginStore } from '../../../src/stores/login';
import { useDownloadSheetStore } from '../../../src/stores/downloadSheet';
import { useToastStore } from '../../../src/stores/toast';
import { useDragDropStore } from '../../../src/stores/dragDrop';
import { useUpdaterStore } from '../../../src/stores/updater';
import { useSettingsStore } from '../../../src/stores/settings';
import { useDownloadSelection } from '../../../src/composables/useDownloadSelection';
import { ProgressCategory, ProgressStage } from '../../../src/tauri/types/progress';
import { TrackType, type MediaAttachment, type MediaFormat, type SourceMediaKind } from '../../../src/tauri/types/media';
import type { Group } from '../../../src/tauri/types/group';
import type { LoginCandidate } from '../../../src/tauri/types/login';
import type { Router } from 'vue-router';

const THUMB = '/src/assets/placeholder.png';
const VIDEO_FORMATS: MediaFormat[] = [
  { id: 'f1080', height: 1080, fps: 30, audioCodecs: [], videoCodecs: [] },
  { id: 'f720', height: 720, fps: 30, audioCodecs: [], videoCodecs: [] },
  { id: 'f360', height: 360, fps: 30, audioCodecs: [], videoCodecs: [] },
  { id: 'a128', abr: 128, audioCodecs: [], videoCodecs: [] },
];
const AUDIO_FORMATS: MediaFormat[] = [
  { id: 'a160', abr: 160, audioCodecs: [], videoCodecs: [] },
  { id: 'a128', abr: 128, audioCodecs: [], videoCodecs: [] },
];
const CANDIDATES: LoginCandidate[] = [
  { id: 'chrome:/p/Default', browser: 'chrome', profile: 'Person 1', signedIn: true },
  { id: 'firefox:/p/x1.default-release', browser: 'firefox', profile: 'default-release', signedIn: false },
];

let counter = 0;
const nextId = (prefix: string) => `${prefix}-${++counter}`;

export function appearance(locale: Locale, theme: 'light' | 'dark') {
  i18n.global.locale.value = locale;
  applyDocumentLocale(locale);
  applyTheme(theme);
}

function router(): Router {
  const el = document.querySelector('#app') as (Element & { __vue_app__?: { config: { globalProperties: { $router: Router } } } }) | null;
  const r = el?.__vue_app__?.config.globalProperties.$router;
  if (!r) throw new Error('router not mounted');
  return r;
}

export interface RowSeed {
  title?: string;
  url?: string;
  state: MediaState;
  kind?: SourceMediaKind;
  extractor?: string;
  thumbnail?: string | null;
  formats?: MediaFormat[];
  filesize?: number;
  attachments?: MediaAttachment[];
  height?: number;
  audio?: boolean;
  percent?: number;
  speedBps?: number;
  etaSecs?: number;
  stage?: ProgressStage;
  destination?: string;
  code?: string;
  message?: string;
  total?: number;
  processed?: number;
  combined?: boolean;
  entries?: number;
}

/** One list row, built the way media_add + progress/destination/diagnostic events build it. */
export function addRow(seed: RowSeed): { groupId: string; itemId: string } {
  const groupId = nextId('g');
  const itemId = nextId('i');
  const url = seed.url ?? `https://www.youtube.com/watch?v=${itemId}`;
  const formats = seed.formats ?? (seed.kind === 'image' || seed.kind === 'carousel' || seed.kind === 'file' ? [] : VIDEO_FORMATS);
  const entries = seed.entries ? Array.from({ length: seed.entries }, (_, index) => ({ index, videoUrl: `${url}&i=${index + 1}` })) : undefined;
  const thumbnail = seed.thumbnail === null ? undefined : seed.thumbnail ?? THUMB;
  const group: Group = {
    id: groupId,
    total: seed.total ?? seed.entries ?? 1,
    processed: seed.processed ?? 0,
    errored: seed.state === MediaState.error ? 1 : 0,
    isCombined: seed.combined ?? false,
    url,
    title: seed.title,
    thumbnail,
    extractor: seed.extractor ?? 'youtube',
    mediaKind: seed.kind,
    attachments: seed.attachments,
    audioCodecs: ['aac'],
    videoCodecs: [],
    formats,
    filesize: seed.filesize ?? 0,
    duration: 754,
    entries,
    playlistCount: seed.entries,
    items: {
      [itemId]: {
        id: itemId, isLeader: true, url, title: seed.title, thumbnail, mediaKind: seed.kind, attachments: seed.attachments,
        audioCodecs: ['aac'], formats, filesize: seed.filesize ?? 0, entries, playlistCount: seed.entries,
      },
    },
  };
  useMediaGroupStore().createGroup(group);
  useMediaStateStore().setState(itemId, seed.state);
  if (seed.height || seed.audio) {
    useMediaOptionsStore().setOptions(groupId, seed.audio ? { trackType: TrackType.audio, abr: 128 } : { trackType: TrackType.both, height: seed.height });
  }
  const progress = useMediaProgressStore();
  if (seed.percent != null || seed.stage) {
    progress.processMediaProgressStagePayload({ id: itemId, groupId, stage: seed.stage ?? ProgressStage.downloading });
  }
  if (seed.percent != null) {
    progress.processMediaProgressPayload({ id: itemId, groupId, category: ProgressCategory.video, percentage: seed.percent, speedBps: seed.speedBps, etaSecs: seed.etaSecs });
  }
  if (seed.destination) useMediaDestinationStore().addDestination(groupId, { path: seed.destination, confidence: 1 });
  if (seed.code) {
    useMediaDiagnosticsStore().processMediaDiagnosticPayload({
      id: itemId, groupId, level: 'error', code: seed.code, component: 'runner',
      message: seed.message ?? 'ERROR: something went wrong', raw: seed.message ?? 'ERROR: something went wrong', timestamp: Date.now(),
    });
  }
  return { groupId, itemId };
}

const TITLES = [
  'How to make bread at home — full recipe',
  'تعلم البرمجة من الصفر: الدرس الأول',
  'Lo-fi beats to study to',
  'رحلة إلى نواكشوط ٢٠٢٦',
];
const title = (i: number) => TITLES[i % TITLES.length];

const done = (extra: Partial<RowSeed> = {}): RowSeed => ({ title: title(0), state: MediaState.done, height: 1080, destination: '/Users/me/Downloads/Nezzel/video.mp4', ...extra });
const failed = (code: string, extra: Partial<RowSeed> = {}): RowSeed => ({ title: title(1), state: MediaState.error, code, ...extra });

function mixedList() {
  addRow({ title: title(0), state: MediaState.downloading, height: 1080, percent: 42, speedBps: 3_400_000, etaSecs: 75 });
  addRow({ title: title(1), state: MediaState.configure, height: 720 });
  addRow(failed('geoBlocked'));
  addRow(done({ title: title(2), audio: true, kind: 'audio', destination: '/Users/me/Downloads/Nezzel/beats.m4a' }));
  addRow(done({ title: title(3) }));
}

function loginRow(state: MediaState, host = 'www.instagram.com') {
  return addRow({ title: 'Instagram post by @nezzel', url: `https://${host}/p/abc123/`, extractor: 'instagram', state, kind: 'video' });
}

function loginRequest(ids: { groupId: string; itemId: string }, host = 'www.instagram.com') {
  const requestId = nextId('r');
  useLoginStore().receive({ requestId, id: ids.itemId, groupId: ids.groupId, host, reason: 'signInRequired', status: 'needed', candidates: CANDIDATES });
  return requestId;
}

/** Every gallery state. A seed may return the route to show. */
export const seeds: Record<string, () => unknown> = {
  'list-empty': () => undefined,
  'list-search-empty': () => mixedList(),
  'list-filter-all': () => mixedList(),
  'list-filter-active': () => (mixedList(), router().push({ name: 'home', query: { status: 'active' } })),
  'list-filter-done': () => (mixedList(), router().push({ name: 'home', query: { status: 'done' } })),
  'list-filter-failed': () => (mixedList(), router().push({ name: 'home', query: { status: 'failed' } })),
  'list-cleared': () => {
    addRow(done({ title: title(2) }));
    addRow(done({ title: title(3) }));
  },

  'row-reading': () => addRow({ url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', state: MediaState.fetching, thumbnail: null }),
  'row-reading-slow': () => addRow({ url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', state: MediaState.fetching, thumbnail: null }),
  'row-ready': () => addRow({ title: title(0), state: MediaState.configure, height: 1080 }),
  'row-downloading': () => addRow({ title: title(1), state: MediaState.downloading, height: 1080, percent: 42.4, speedBps: 3_400_000, etaSecs: 75 }),
  'row-paused': () => addRow({ title: title(0), state: MediaState.paused, height: 1080, percent: 61 }),
  'row-resuming': () => {
    const { itemId } = addRow({ title: title(0), state: MediaState.paused, height: 1080 });
    setTimeout(() => useMediaStateStore().setState(itemId, MediaState.downloading), 50);
  },
  'row-merging': () => addRow({ title: title(1), state: MediaState.downloading, height: 1080, stage: ProgressStage.merging }),
  'row-done-video': () => addRow(done()),
  'row-done-audio': () => addRow(done({ title: title(2), kind: 'audio', audio: true, destination: '/Users/me/Downloads/Nezzel/beats.m4a' })),
  'row-done-photo': () => addRow(done({ title: 'Sunset at the beach', kind: 'image', extractor: 'instagram', attachments: [{ id: 'p1', kind: 'image', url: 'https://example.com/1.jpg', thumbnail: THUMB }], destination: '/Users/me/Downloads/Nezzel/sunset.jpg' })),
  'row-done-carousel': () => addRow(done({ title: 'صور من حفل التخرج', kind: 'carousel', extractor: 'instagram', thumbnail: null, attachments: [1, 2, 3, 4].map(n => ({ id: `p${n}`, kind: 'image' as const, url: `https://example.com/${n}.jpg`, thumbnail: THUMB })), destination: '/Users/me/Downloads/Nezzel/graduation' })),
  'row-done-file': () => addRow(done({ title: 'annual-report-2026.pdf', url: 'https://example.com/annual-report-2026.pdf', kind: 'file', extractor: 'generic', thumbnail: null, filesize: 2_400_000, destination: '/Users/me/Downloads/Nezzel/annual-report-2026.pdf' })),
  'row-done-missing': () => addRow(done({ destination: undefined })),

  'row-login-pending': () => {
    const ids = loginRow(MediaState.fetching);
    loginRequest(ids);
  },
  'row-login-accepted': () => {
    const ids = loginRow(MediaState.downloading);
    useLoginStore().receiveUsing({ id: ids.itemId, groupId: ids.groupId, host: 'www.instagram.com', candidate: CANDIDATES[0] });
    useMediaProgressStore().processMediaProgressStagePayload({ id: ids.itemId, groupId: ids.groupId, stage: ProgressStage.downloading });
    useMediaProgressStore().processMediaProgressPayload({ id: ids.itemId, groupId: ids.groupId, category: ProgressCategory.video, percentage: 12, speedBps: 900_000, etaSecs: 30 });
  },
  'row-login-refused': () => {
    const ids = loginRow(MediaState.error);
    useMediaDiagnosticsStore().processMediaDiagnosticPayload({ id: ids.itemId, groupId: ids.groupId, level: 'error', code: 'signInRequired', component: 'runner', message: 'login required', raw: '', timestamp: Date.now() });
  },
  'row-login-exhausted': () => {
    const ids = loginRow(MediaState.error);
    useLoginStore().receiveUsing({ id: ids.itemId, groupId: ids.groupId, host: 'www.instagram.com', candidate: CANDIDATES[0] });
    useLoginStore().receive({ requestId: nextId('r'), id: ids.itemId, groupId: ids.groupId, host: 'www.instagram.com', reason: 'signInRequired', status: 'exhausted', candidates: CANDIDATES });
  },
  'row-login-dismissed': async () => {
    const ids = loginRow(MediaState.fetching);
    const requestId = loginRequest(ids);
    await useLoginStore().dismiss(requestId);
    useMediaStateStore().setState(ids.itemId, MediaState.error);
  },
  'row-login-expired': () => {
    const ids = loginRow(MediaState.fetching);
    useLoginStore().expire(loginRequest(ids));
    useMediaStateStore().setState(ids.itemId, MediaState.error);
  },
  'row-login-timeout': () => addRow(failed('loginConsentTimeout', { title: 'Instagram post by @nezzel', url: 'https://www.instagram.com/p/abc123/', extractor: 'instagram' })),
  'row-error-members': () => addRow(failed('membersOnly')),
  'row-error-removed': () => addRow(failed('videoNotFoundOrPrivate')),
  'row-error-drm': () => addRow(failed('drmProtected', { title: 'Movie trailer', extractor: 'netflix', url: 'https://www.netflix.com/watch/1' })),
  'row-error-geo': () => addRow(failed('geoBlocked')),
  'row-error-rate': () => addRow(failed('rateLimited429')),
  'row-error-network': () => addRow(failed('networkNameResolutionFailed')),
  'row-error-noaudio': () => addRow(failed('noAudioTrack', { audio: true })),
  'row-error-notpublic': () => addRow(failed('accessForbidden403', { title: 'report.pdf', url: 'https://example.com/private/report.pdf', kind: 'file', extractor: 'generic', thumbnail: null })),
  'row-error-unsupported': () => addRow(failed('urlNotSupported', { title: undefined, url: 'https://example.com/some/page', extractor: 'generic', thumbnail: null })),
  'row-error-feed': () => addRow(failed('noMediaFound', { title: undefined, url: 'https://www.instagram.com/nezzel/', extractor: 'instagram', thumbnail: null })),
  'row-error-unknown': () => addRow(failed('unknown')),

  'playlist-reading': () => addRow({ title: 'Cooking basics — full course', state: MediaState.fetchingList, total: 24, processed: 9, entries: 24 }),
  'playlist-selection': () => addRow({ title: 'Cooking basics — full course', state: MediaState.playlistSelection, entries: 24 }),
  'multi-select': () => {
    mixedList();
    const selection = useDownloadSelection();
    Object.keys(useMediaGroupStore().groups).slice(0, 3).forEach(id => selection.toggle(id));
  },
  'long-titles': () => {
    addRow({ title: 'شرح مفصل لكيفية تحميل الفيديوهات والصوتيات والصور من جميع المواقع بجودة عالية وبدون إعلانات — الحلقة الأولى من السلسلة الكاملة', state: MediaState.configure, height: 1080 });
    addRow({ title: 'An extraordinarily long English title that keeps going far past any reasonable width to test truncation and tooltips in the row', state: MediaState.downloading, height: 720, percent: 18, speedBps: 12_500_000, etaSecs: 3725 });
    addRow({ title: 'Mixed: مقابلة حصرية with the team behind Nezzel 2.0 (full interview)', state: MediaState.done, height: 1080, destination: '/x.mp4' });
    addRow(failed('geoBlocked', { title: 'عنوان عربي طويل جدا لفيديو محظور في منطقتك ولا يمكن تحميله الآن بسبب قيود الموقع' }));
  },
  'rows-100': () => {
    const states = [MediaState.configure, MediaState.downloading, MediaState.done, MediaState.error, MediaState.paused];
    for (let i = 0; i < 100; i++) {
      const state = states[i % states.length];
      addRow({ title: `${title(i)} #${i + 1}`, state, height: 720, percent: state === MediaState.downloading || state === MediaState.paused ? (i * 7) % 100 : undefined, speedBps: 2_000_000, etaSecs: 40, code: state === MediaState.error ? 'rateLimited429' : undefined, destination: state === MediaState.done ? '/x.mp4' : undefined });
    }
  },

  'sheet-video': () => useDownloadSheetStore().show(addRow({ title: title(0), state: MediaState.configure }).groupId),
  'sheet-audio': () => useDownloadSheetStore().show(addRow({ title: 'Podcast episode 12', kind: 'audio', extractor: 'soundcloud', formats: AUDIO_FORMATS, state: MediaState.configure }).groupId),
  'sheet-playlist': () => useDownloadSheetStore().show(addRow({ title: 'Cooking basics — full course', state: MediaState.configure, entries: 24, combined: true }).groupId),
  'sheet-waiting': () => useDownloadSheetStore().show(addRow({ url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', state: MediaState.fetching, thumbnail: null, formats: [] }).groupId),

  'settings-general': () => router().push({ name: 'settings.general' }),
  'settings-advanced': () => router().push({ name: 'settings.advanced' }),
  'settings-logins-auto': () => router().push({ name: 'settings.logins' }),
  'settings-logins-none': () => {
    useSettingsStore().settings.auth.cookieBrowser = 'none';
    return router().push({ name: 'settings.logins' });
  },
  'settings-logins-browser': () => {
    useSettingsStore().settings.auth.cookieBrowser = 'chrome';
    return router().push({ name: 'settings.logins' });
  },
  'settings-accounts': () => router().push({ name: 'settings.logins' }),
  'settings-about': () => router().push({ name: 'settings.advanced' }),

  'updater-available': () => {
    useUpdaterStore().checkResult = { available: true, currentVersion: '1.0.0', availableVersion: '1.1.0' };
  },
  'updater-downloading': () => {
    const updater = useUpdaterStore();
    updater.checkResult = { available: true, currentVersion: '1.0.0', availableVersion: '1.1.0' };
    updater.isUpdating = true;
    updater.downloadProgress = { received: 31_000_000, total: 52_000_000 };
  },
  'updater-restart': () => {
    const updater = useUpdaterStore();
    updater.checkResult = { available: true, currentVersion: '1.0.0', availableVersion: '1.1.0' };
    updater.isNeedingRestart = true;
  },
  'updater-error': () => {
    const updater = useUpdaterStore();
    updater.checkResult = { available: true, currentVersion: '1.0.0', availableVersion: '1.1.0' };
    updater.lastError = 'network error';
  },
  'toast-success': () => (mixedList(), useToastStore().showToast(i18n.global.t('feedback.started', { n: 3 }), { style: 'success', duration: 0 })),
  'toast-error': () => (mixedList(), useToastStore().showToast(i18n.global.t('feedback.downloadFailed'), { style: 'error', duration: 0 })),
  'toast-undo': () => (mixedList(), useToastStore().showToast(i18n.global.t('feedback.removed', { n: 2 }), { duration: 0, action: { label: i18n.global.t('downloads.list.undo'), run: () => {} } })),
  'drop-overlay': () => {
    mixedList();
    useDragDropStore().isOver = true;
  },
};

export async function seed(name: string) {
  const run = seeds[name];
  if (!run) throw new Error(`unknown state ${name}`);
  await run();
}
