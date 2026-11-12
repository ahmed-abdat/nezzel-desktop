import { flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import LoginNeeded from '../../src/components/downloads/LoginNeeded.vue';
import LoginProfile from '../../src/components/downloads/LoginProfile.vue';
import { i18n } from '../../src/i18n';
import { useLoginStore } from '../../src/stores/login';
import { resetPlatformForTests } from '../../src/composables/usePlatform';
import { Platform } from '../../src/tauri/types/app';
import { installTauriMock } from '../utils/tauriMock';

describe('login picker profile metadata', () => {
  it.each(['browserKeychainDenied', 'browserDpapiFailed', 'browserAppBound', 'browserSafariAccessDenied'])('shows the concrete remedy for %s while choosing another login', (code) => {
    const store = useLoginStore();
    store.receive({ requestId: 'remedy', id: 'item', groupId: 'remedy', host: 'reddit.com', reason: 'signInRequired', status: 'needed', candidates: [{ id: 'firefox', browser: 'firefox', profile: 'main', signedIn: false }] });
    const wrapper = mount(LoginNeeded, { props: { card: store.cardFor('remedy')!, diagnosticCode: code }, global: { plugins: [i18n], stubs: { RouterLink: true } } });
    try {
      expect(wrapper.text()).toContain(i18n.global.t(`errors.runner.${code}.action`));
    } finally {
      wrapper.unmount();
      store.forgetGroup('remedy');
    }
  });

  it.each(['en', 'ar', 'fr'] as const)('uses the locale direction around LTR browser and host runs in %s', async (locale) => {
    const oldLocale = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const store = useLoginStore();
    store.receive({ requestId: 'direction', id: 'item', groupId: 'direction', host: 'www.instagram.com', reason: 'signInRequired', status: 'needed', candidates: [
      { id: 'chrome-token', browser: 'chrome', profile: 'Person 1', signedIn: true },
      { id: 'firefox-token', browser: 'firefox', profile: 'Default', signedIn: false },
    ] });
    const wrapper = mount(LoginNeeded, { props: { card: store.cardFor('direction')! }, global: { plugins: [i18n], stubs: { RouterLink: true } } });
    try {
      await flushPromises();
      const value = wrapper.get('[role=combobox] > span');
      expect(value.attributes('dir')).toBe(locale === 'ar' ? 'rtl' : 'ltr');
      expect(value.findAll('bdi[dir=ltr]').map(run => run.text())).toEqual(['Chrome (Person 1)', 'www.instagram.com']);
    } finally {
      wrapper.unmount();
      store.forgetGroup('direction');
      i18n.global.locale.value = oldLocale;
    }
  });
  it.each(['en', 'ar', 'fr'] as const)('keeps the login reason and engine Details reachable in %s', async (locale) => {
    const oldLocale = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const store = useLoginStore();
    const raw = 'ERROR: [Reddit] Account authentication is required. Use --cookies';
    try {
      for (const { platform, reason, blockedBrowser, status, expected } of [
        { platform: Platform.Windows, reason: 'signInRequired', status: 'exhausted', expected: i18n.global.t('downloads.login.noBrowserWindows') },
        { platform: Platform.Windows, reason: 'browserAppBound', status: 'exhausted', expected: i18n.global.t('downloads.login.windowsAppBound') },
        { platform: Platform.Windows, reason: 'browserAppBound', blockedBrowser: 'edge', status: 'exhausted', expected: i18n.global.t('downloads.login.windowsAppBoundBrowser', { browser: locale === 'ar' ? '\u2068Edge\u2069' : 'Edge' }) },
        { platform: Platform.Windows, reason: 'browserAppBound', blockedBrowser: 'chrome', status: 'needed', expected: i18n.global.t('downloads.login.windowsAppBoundBrowser', { browser: locale === 'ar' ? '\u2068Chrome\u2069' : 'Chrome' }) },
        { platform: Platform.Linux, reason: 'signInRequired', status: 'exhausted', expected: i18n.global.t('downloads.login.noBrowser') },
        { platform: Platform.MacOS, reason: 'signInRequired', status: 'exhausted', expected: i18n.global.t('downloads.login.noBrowser') },
      ] as const) {
        installTauriMock({ get_platform: () => platform });
        resetPlatformForTests(platform);
        store.receive({ requestId: 'details', id: 'item', groupId: 'details', host: 'reddit.com', reason, blockedBrowser, status,
          candidates: status === 'needed' ? [{ id: 'firefox-token', browser: 'firefox', profile: 'Default', signedIn: false }] : [] });
        const wrapper = mount(LoginNeeded, { props: { card: store.cardFor('details')!, details: raw }, global: { plugins: [i18n], stubs: { RouterLink: true } } });
        try {
          await flushPromises();
          expect(wrapper.text()).toContain(expected);
          const details = wrapper.findAll('button').find(button => button.text() === i18n.global.t('downloads.row.details'))!;
          expect(details.attributes('aria-expanded')).toBe('false');
          await details.trigger('click');
          expect(details.attributes('aria-expanded')).toBe('true');
          expect(wrapper.get('[data-testid="login-details"]').text()).toBe(raw);
          expect(wrapper.get('[data-testid="login-details"]').attributes('dir')).toBe('ltr');
        } finally {
          wrapper.unmount();
          store.forgetGroup('details');
        }
      }
    } finally {
      resetPlatformForTests();
      i18n.global.locale.value = oldLocale;
    }
  });

  it('keeps a dismissed prompt clear even when the previous cookie read failed', () => {
    const wrapper = mount(LoginNeeded, { props: { card: { kind: 'failed', failure: { groupId: 'dismissed', host: 'reddit.com', reason: 'dismissed' } }, diagnosticCode: 'browserDpapiFailed' }, global: { plugins: [i18n], stubs: { RouterLink: true } } });
    try {
      expect(wrapper.text()).toContain(i18n.global.t('downloads.login.skipped', { site: 'Reddit' }));
      expect(wrapper.text()).not.toContain(i18n.global.t('errors.runner.browserDpapiFailed.action'));
    } finally { wrapper.unmount(); }
  });

  // What helps comes first: settings when a retry can't work yet, the prompt when it was only closed.
  it.each([
    { reason: 'exhausted', first: 'downloads.login.settings', retry: 'downloads.login.tryAnother' },
    { reason: 'dismissed', first: 'downloads.login.again', retry: 'downloads.login.again' },
    { reason: 'expired', first: 'downloads.login.again', retry: 'downloads.login.again' },
  ] as const)('a failed login ($reason) leads with the action that helps and keeps retry', async ({ reason, first, retry }) => {
    const wrapper = mount(LoginNeeded, {
      props: { card: { kind: 'failed', failure: { groupId: 'g', host: 'instagram.com', reason } } },
      global: { plugins: [i18n], stubs: { RouterLink: { template: '<a><slot /></a>' } } },
    });
    const actions = wrapper.get('[data-testid="login-failed"]').findAll('a, button').map(el => el.text());
    expect(actions[0]).toBe(i18n.global.t(first));
    expect(actions).toContain(i18n.global.t('downloads.login.settings'));
    await wrapper.findAll('button').find(b => b.text() === i18n.global.t(retry))!.trigger('click');
    expect(wrapper.emitted('retry')).toHaveLength(1);
    wrapper.unmount();
  });

  it('names the refused browser from metadata while its candidate id stays opaque', () => {
    const wrapper = mount(LoginNeeded, {
      props: { card: { kind: 'failed', failure: { groupId: 'opaque', host: 'instagram.com', reason: 'exhausted', tried: '147599e2-568e-469f-a8cd-fbf9a79606ca', browser: 'chrome' } } },
      global: { plugins: [i18n], stubs: { RouterLink: true } },
    });
    expect(wrapper.text()).toContain(i18n.global.t('downloads.login.refusedBrowser', { site: 'Instagram', browser: 'Chrome' }));
    expect(wrapper.text()).not.toContain('147599e2');
    wrapper.unmount();
  });

  it.each(['en', 'ar', 'fr'] as const)('shows the profile used on the row and a change action in %s', async (locale) => {
    const oldLocale = i18n.global.locale.value;
    i18n.global.locale.value = locale;
    const store = useLoginStore();
    store.receiveUsing({ id: 'item', groupId: 'used', host: 'instagram.com', candidate: { id: 'chrome:/private/Default', browser: 'chrome', profile: 'Ahmed', signedIn: true } });
    const wrapper = mount(LoginProfile, { props: { groupId: 'used' }, global: { plugins: [i18n], stubs: { RouterLink: true } } });
    await flushPromises();
    expect(wrapper.get('bdi[dir=ltr]').text()).toBe('Chrome · Ahmed');
    expect(wrapper.text()).toContain(i18n.global.t('downloads.login.usingProfile', { profile: 'Chrome · Ahmed' }));
    expect(wrapper.find('button').text()).toBe(i18n.global.t('downloads.login.changeProfile'));
    expect(wrapper.text()).not.toContain('/private');
    wrapper.unmount();
    store.forgetGroup('used');
    i18n.global.locale.value = oldLocale;
  });

  it.each([true, false])('hides the used-profile claim while consent is pending (showPicker=%s)', async (showPicker) => {
    const store = useLoginStore();
    store.receiveUsing({ id: 'item', groupId: 'pending-used', host: 'reddit.com', candidate: { id: 'chrome-token', browser: 'chrome', profile: 'Ahmed', signedIn: true } });
    const wrapper = mount(LoginProfile, { props: { groupId: 'pending-used', showPicker }, global: { plugins: [i18n], stubs: { RouterLink: true } } });
    expect(wrapper.text()).toContain('Chrome · Ahmed');
    store.receive({ requestId: 'pending-consent', id: 'item', groupId: 'pending-used', host: 'reddit.com', reason: 'signInRequired', status: 'needed', candidates: [{ id: 'safari-token', browser: 'safari', profile: 'Safari', signedIn: false }] });
    await flushPromises();
    expect(wrapper.text()).not.toContain(i18n.global.t('downloads.login.usingProfile', { profile: 'Chrome · Ahmed' }));
    wrapper.unmount();
    store.forgetGroup('pending-used');
  });

  it('explains Windows app-bound cookies and accepts an explicit click on the sole Firefox choice', async () => {
    const store = useLoginStore();
    store.receive({ requestId: 'blocked', id: 'item', groupId: 'blocked', host: 'instagram.com', reason: 'browserAppBound', status: 'needed',
      candidates: [{ id: 'firefox:/profile', browser: 'firefox', profile: 'Ahmed', signedIn: false }] });
    const wrapper = mount(LoginNeeded, { props: { card: store.cardFor('blocked')! }, global: { plugins: [i18n], stubs: { RouterLink: true } } });
    expect(wrapper.text()).toContain(i18n.global.t('downloads.login.windowsAppBound'));
    expect(wrapper.text()).toContain('Firefox (Ahmed)');
    const accept = vi.spyOn(store, 'accept').mockResolvedValue(undefined);
    const use = wrapper.findAll('button').find(button => button.text() === i18n.global.t('downloads.login.use'))!;
    expect(use.attributes('disabled')).toBeUndefined();
    await use.trigger('click');
    expect(accept).toHaveBeenCalledWith('blocked', 'firefox:/profile');
    accept.mockRestore();
    wrapper.unmount();
    store.forgetGroup('blocked');
  });

  it('preselects the first ranked profile and marks signed-in choices for the host', async () => {
    const store = useLoginStore();
    store.receive({
      requestId: 'profiles', id: 'item', groupId: 'profiles', host: 'instagram.com', reason: 'signInRequired', status: 'needed',
      candidates: [
        { id: 'chrome:/p/Profile 5', browser: 'chrome', profile: 'MBI (groupmbi and cacmauritania)', signedIn: true },
        { id: 'chrome:/p/Default', browser: 'chrome', profile: 'Ahmed', signedIn: false },
      ],
    });
    const card = store.cardFor('profiles')!;
    const wrapper = mount(LoginNeeded, { props: { card }, global: { plugins: [i18n], stubs: { RouterLink: true } } });
    await flushPromises();
    const select = wrapper.findComponent({ name: 'SelectField' });
    expect(select.props('modelValue')).toBe('chrome:/p/Profile 5');
    expect(select.props('options')).toEqual([
      { value: 'chrome:/p/Profile 5', label: 'Chrome (MBI (groupmbi and cacmauritania)) · logged in to instagram.com' },
      { value: 'chrome:/p/Default', label: 'Chrome (Ahmed)' },
    ]);
    wrapper.unmount();
    store.forgetGroup('profiles');
  });
});
