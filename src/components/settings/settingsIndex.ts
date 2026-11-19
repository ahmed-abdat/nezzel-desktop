/**
 * Every searchable row in Settings: its `search-id` (also the `data-setting-id` anchor on the row),
 * the tab it lives on, its section heading, and the i18n keys of its label and hint, exactly as the
 * row template passes them. `tests/unit/settingsIndex.spec.ts` checks this list against the tab
 * templates both ways, so a new row cannot be added without being findable.
 *
 * Synonyms live in the locale files: `settings.search.synonyms.<id>`, a comma-separated list.
 */
export type SettingsTab = 'general' | 'logins' | 'advanced' | 'about';

export interface SettingsIndexEntry {
  id: string;
  tab: SettingsTab;
  /** i18n key of the section heading (or the disclosure label for rows outside a section). */
  section?: string;
  label: string;
  /** macOS wording of the label, when it differs. */
  labelMac?: string;
  hint?: string;
  /** Row exists only on macOS. */
  macOnly?: boolean;
  /** Row shows only in some states (e.g. a custom command): when it is not on screen, reveal this row instead. */
  parent?: string;
}

export const SETTINGS_TABS: readonly { tab: SettingsTab; route: string; label: string }[] = [
  { tab: 'general', route: 'settings.general', label: 'prefs.tabs.general' },
  { tab: 'logins', route: 'settings.logins', label: 'prefs.tabs.logins' },
  { tab: 'advanced', route: 'settings.advanced', label: 'prefs.tabs.advanced' },
  { tab: 'about', route: 'settings.about', label: 'about.title' },
];

const general = (section: string, rows: Omit<SettingsIndexEntry, 'tab' | 'section'>[]) => rows.map(row => ({ ...row, tab: 'general' as const, section }));
const advanced = (section: string, rows: Omit<SettingsIndexEntry, 'tab' | 'section'>[]) => rows.map(row => ({ ...row, tab: 'advanced' as const, section }));

export const SETTINGS_INDEX: readonly SettingsIndexEntry[] = [
  ...general('prefs.sections.downloads', [
    { id: 'saveFolder', label: 'prefs.general.folder', hint: 'prefs.general.folderHint' },
    { id: 'defaultChoice', label: 'prefs.general.choice', hint: 'prefs.general.choiceHint' },
    { id: 'askEach', label: 'prefs.general.askEach', hint: 'prefs.general.askEachHint' },
  ]),
  ...general('prefs.sections.quick', [
    { id: 'clipboardFill', label: 'prefs.general.clip', hint: 'prefs.general.clipHint' },
    { id: 'clipboardCard', label: 'prefs.general.card', hint: 'prefs.general.cardHint' },
    { id: 'cardKeepOpen', label: 'prefs.general.cardKeepOpen' },
    { id: 'cardPosition', label: 'prefs.general.cardPosition' },
    { id: 'tray', label: 'prefs.general.tray', hint: 'prefs.general.trayHint' },
  ]),
  ...general('prefs.sections.appearance', [
    { id: 'language', label: 'prefs.general.language' },
    { id: 'theme', label: 'prefs.general.theme' },
  ]),

  { id: 'autoLogins', tab: 'logins', label: 'prefs.logins.autoLogins', hint: 'prefs.logins.autoLoginsHint' },
  { id: 'cookieBrowser', tab: 'logins', section: 'prefs.tabs.advanced', label: 'prefs.logins.browser', hint: 'prefs.logins.browserHint' },
  { id: 'cookieFile', tab: 'logins', section: 'prefs.sections.otherLogins', label: 'prefs.logins.file', hint: 'prefs.logins.fileHint' },

  ...advanced('prefs.adv.files.title', [
    { id: 'videoFormat', label: 'prefs.adv.files.videoFormat', hint: 'prefs.adv.files.videoFormatHint' },
    { id: 'audioFormat', label: 'prefs.adv.files.audioFormat', hint: 'prefs.adv.files.audioFormatHint' },
    { id: 'coverImage', label: 'prefs.adv.files.cover', hint: 'prefs.adv.files.coverHint' },
    { id: 'metadata', label: 'prefs.adv.files.metadata', hint: 'prefs.adv.files.metadataHint' },
    { id: 'folders', label: 'prefs.adv.files.folders', hint: 'prefs.adv.files.foldersHint' },
    { id: 'subtitles', label: 'prefs.adv.files.subtitles', hint: 'prefs.adv.files.subtitlesHint' },
    { id: 'linkFilters', label: 'prefs.adv.files.filters', hint: 'prefs.adv.files.filtersHint' },
    { id: 'keepOriginalCodec', label: 'prefs.adv.files.keepOriginalCodec', hint: 'prefs.adv.files.keepOriginalCodecHint', macOnly: true },
    { id: 'saveCover', label: 'prefs.adv.files.saveCover', hint: 'prefs.adv.files.saveCoverHint' },
    { id: 'keepVideo', label: 'prefs.adv.files.keepVideo', hint: 'prefs.adv.files.keepVideoHint' },
    { id: 'keepAudio', label: 'prefs.adv.files.keepAudio', hint: 'prefs.adv.files.keepAudioHint' },
    { id: 'videoAfter', label: 'prefs.adv.files.videoAfter', hint: 'prefs.adv.files.videoAfterHint' },
    { id: 'videoCommandMode', label: 'prefs.adv.files.customMode', hint: 'prefs.adv.files.customModeHint', parent: 'videoAfter' },
    { id: 'videoCommand', label: 'prefs.adv.files.ffmpegArgs', hint: 'prefs.adv.files.ffmpegArgsHint', parent: 'videoAfter' },
    { id: 'audioAfter', label: 'prefs.adv.files.audioAfter', hint: 'prefs.adv.files.audioAfterHint' },
    { id: 'audioCommand', label: 'prefs.adv.files.ffmpegArgs', hint: 'prefs.adv.files.ffmpegArgsHint', parent: 'audioAfter' },
  ]),
  ...advanced('prefs.adv.keys.title', [
    { id: 'globalShortcuts', label: 'prefs.adv.keys.globalOn', hint: 'prefs.adv.keys.globalOnHint' },
    { id: 'quickShortcut', label: 'prefs.adv.keys.quick', hint: 'prefs.adv.keys.defaultIs' },
    { id: 'pasteShortcut', label: 'prefs.adv.keys.paste', hint: 'prefs.adv.keys.pasteHint' },
    { id: 'downloadAllShortcut', label: 'prefs.adv.keys.downloadAll', hint: 'prefs.adv.keys.defaultIs' },
    { id: 'addAndDownloadShortcut', label: 'prefs.adv.keys.addAndDownload', hint: 'prefs.adv.keys.addAndDownloadHint' },
  ]),
  ...advanced('settings.network.legend', [
    { id: 'loginsLink', label: 'prefs.adv.net.logins', hint: 'prefs.adv.net.loginsHint' },
    { id: 'proxy', label: 'prefs.adv.net.proxy', hint: 'prefs.adv.net.proxyHint' },
    { id: 'impersonate', label: 'prefs.adv.net.impersonate', hint: 'prefs.adv.net.impersonateHint' },
  ]),
  ...advanced('settings.performance.legend', [
    { id: 'parallelDownloads', label: 'prefs.adv.perf.parallel', hint: 'prefs.adv.perf.parallelHint' },
    { id: 'pieces', label: 'prefs.adv.perf.pieces', hint: 'prefs.adv.perf.piecesHint' },
    { id: 'keepAwake', label: 'reliability.keepAwake', hint: 'reliability.keepAwakeHint' },
    { id: 'showSizes', label: 'prefs.adv.perf.size', hint: 'prefs.adv.perf.sizeHint' },
    { id: 'aria2', label: 'prefs.adv.perf.aria2', hint: 'prefs.adv.perf.aria2Hint' },
  ]),
  ...advanced('settings.sponsorBlock.legend', [
    { id: 'sponsorBlock', label: 'prefs.adv.sb.remove', hint: 'prefs.adv.sb.removeHint' },
    { id: 'sponsorBlockParts', label: 'prefs.adv.sb.choose' },
    { id: 'sponsorBlockPrecise', label: 'prefs.adv.sb.precise', hint: 'prefs.adv.sb.preciseHint' },
    { id: 'sponsorBlockServer', label: 'prefs.adv.sb.server', hint: 'prefs.adv.sb.serverHint' },
  ]),
  ...advanced('prefs.adv.app.title', [
    { id: 'notifications', label: 'prefs.adv.app.notify', hint: 'prefs.adv.app.notifyHint' },
    { id: 'autoStart', label: 'settings.system.autoStart.label', hint: 'settings.system.autoStart.hint' },
    { id: 'updateApp', label: 'prefs.adv.app.updateApp', hint: 'prefs.adv.app.updateAppHint' },
    { id: 'updateTools', label: 'prefs.adv.app.updateTools', hint: 'prefs.adv.app.updateToolsHint' },
    { id: 'sendReports', label: 'report.setting.label', hint: 'report.setting.help' },
    { id: 'sendUsage', label: 'usage.setting.label', hint: 'usage.setting.help' },
    { id: 'startMinimised', label: 'settings.system.autoStartMinimised.label.windows', labelMac: 'settings.system.autoStartMinimised.label.mac' },
    { id: 'closeToTray', label: 'prefs.adv.app.closeToTray', hint: 'prefs.adv.app.closeToTrayHint' },
    { id: 'notificationKinds', label: 'prefs.adv.app.notifyWhich' },
  ]),
  ...advanced('prefs.adv.dev.title', [
    { id: 'extractorArgs', label: 'prefs.adv.dev.extractor', hint: 'prefs.adv.dev.extractorHint' },
  ]),

  { id: 'version', tab: 'about', label: 'about.version' },
  { id: 'reportProblem', tab: 'about', label: 'report.title', hint: 'report.aboutHelp' },
  { id: 'licenses', tab: 'about', label: 'about.poweredBy', hint: 'about.credit' },
];

/** Translation lookups the search needs; `locale` omitted = the current UI language. */
export interface IndexTranslator {
  text: (key: string, locale?: string) => string;
  has: (key: string, locale: string) => boolean;
  locale: string;
  isMac: boolean;
}

/** Labels and hints carry placeholders ("Default: {keys}"): search the words around them. */
export const NO_VALUES = { keys: '', version: '' };
const SYNONYM_LOCALES = ['en', 'ar', 'fr'];
const splitList = (text: string) => text.split(/[,،]/).map(part => part.trim()).filter(Boolean);

export function entryLabel(entry: SettingsIndexEntry, tr: Pick<IndexTranslator, 'text' | 'isMac'>): string {
  return tr.text(tr.isMac && entry.labelMac ? entry.labelMac : entry.label).replace(/\s*[:：]\s*$/, '');
}

/** One search document per row on this platform: current-language label/hint/section, synonyms in every language. */
export function buildSearchDocs(entries: readonly SettingsIndexEntry[], tr: IndexTranslator) {
  return entries.filter(entry => !entry.macOnly || tr.isMac).map((entry) => {
    const key = `settings.search.synonyms.${entry.id}`;
    const synonyms = SYNONYM_LOCALES.flatMap(lang => (tr.has(key, lang) ? splitList(tr.text(key, lang)) : []));
    // People often know the English name of a setting even with the app in Arabic or French.
    if (tr.locale !== 'en') synonyms.push(tr.text(entry.label, 'en'));
    return {
      id: entry.id,
      label: entryLabel(entry, tr),
      hint: entry.hint ? tr.text(entry.hint) : undefined,
      section: entry.section ? tr.text(entry.section) : undefined,
      synonyms,
    };
  });
}
