import { mount } from '@vue/test-utils';
import { createMemoryHistory, createRouter } from 'vue-router';
import { defineComponent, nextTick, reactive } from 'vue';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushPromises } from '@vue/test-utils';
import SettingsView from '../../src/views/app/SettingsView.vue';
import SettingsShortcuts from '../../src/components/settings/SettingsShortcuts.vue';
import SettingsGeneralTab from '../../src/views/app/settings/SettingsGeneralTab.vue';
import SettingsLoginsTab from '../../src/views/app/settings/SettingsLoginsTab.vue';
import { pickOption } from '../utils/selectField';
import SettingsAboutTab from '../../src/views/app/settings/SettingsAboutTab.vue';
import SettingsAdvancedTab from '../../src/views/app/settings/SettingsAdvancedTab.vue';
import { i18n } from '../../src/i18n';
import { defaultSettings, type Settings } from '../../src/tauri/types/config';
import { TranscodePolicy } from '../../src/tauri/types/media';

const patch = vi.fn();
const reset = vi.fn();
const showToast = vi.fn();
const setTheme = vi.fn();
const openInternalPath = vi.fn();

const settingsState = reactive<Settings>(structuredClone(defaultSettings));

vi.mock('../../src/stores/settings', () => ({
  useSettingsStore: () => ({
    settings: settingsState,
    patch,
    reset,
  }),
}));

vi.mock('../../src/stores/toast', () => ({
  useToastStore: () => ({
    showToast,
  }),
}));

vi.mock('../../src/composables/useTheme', () => ({
  useTheme: () => ({
    setTheme,
  }),
}));

vi.mock('../../src/composables/useOpener', () => ({
  useOpener: () => ({
    openInternalPath,
  }),
}));

const capturePlatform = reactive({ session: 'other', backgroundClipboard: true, desktop: 'gnome' });
vi.mock('../../src/composables/useCapturePlatform', () => ({
  useCapturePlatform: () => capturePlatform,
}));

function createRoot() {
  return defineComponent({
    template: '<router-view />',
  });
}

function createBlankView() {
  return defineComponent({
    template: '<div />',
  });
}

function createSettingsRouter() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: '/',
        name: 'home',
        component: createBlankView(),
      },
      {
        path: '/location',
        name: 'location',
        component: createBlankView(),
      },
      {
        path: '/subtitles',
        name: 'subtitles',
        component: createBlankView(),
      },
      {
        path: '/input-filters',
        name: 'input-filters',
        component: createBlankView(),
      },
      {
        path: '/authentication',
        name: 'authentication',
        component: createBlankView(),
      },
      {
        path: '/',
        component: createRoot(),
        children: [
          {
            path: 'settings',
            component: SettingsView,
            children: [
              { path: '', name: 'settings.general', component: SettingsGeneralTab },
              { path: 'logins', name: 'settings.logins', component: SettingsLoginsTab },
              { path: 'advanced', name: 'settings.advanced', component: SettingsAdvancedTab },
              { path: 'about', name: 'settings.about', component: SettingsAboutTab },
            ],
          },
        ],
      },
    ],
  });
  return router;
}

describe('SettingsView', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('explains foreground clipboard pickup only when background reads are unavailable', async () => {
    capturePlatform.backgroundClipboard = false;
    const wrapper = mount(SettingsGeneralTab, {
      props: { modelValue: structuredClone(defaultSettings) }, global: { plugins: [i18n] },
    });
    expect(wrapper.text()).toContain('On this desktop, copied links are picked up when Nezzel\'s window is in front.');
    capturePlatform.backgroundClipboard = true;
    await nextTick();
    expect(wrapper.text()).not.toContain('On this desktop, copied links are picked up');
    wrapper.unmount();
  });

  it('keeps the Wayland system command available when app-managed shortcuts are off', async () => {
    capturePlatform.session = 'wayland';
    const modelValue = structuredClone(defaultSettings);
    modelValue.input.globalShortcuts = false;
    const wrapper = mount(SettingsShortcuts, { props: { modelValue }, global: { plugins: [i18n] } });
    await flushPromises();
    expect(wrapper.get('[data-testid="shortcut-wayland"]').text()).toContain('/usr/bin/nezzel --capture');
    expect(wrapper.get('[data-testid="copy-shortcut-command"]').attributes('disabled')).toBeUndefined();
    wrapper.unmount();
    capturePlatform.session = 'other';
  });

  it('shows General, Logins and Advanced and marks the current one', async () => {
    const router = createSettingsRouter();
    await router.push('/settings');
    await router.isReady();
    const wrapper = mount(createRoot(), { global: { plugins: [router, i18n] } });
    await nextTick();

    const tabs = wrapper.findAll('nav a');
    expect(tabs.map(tab => tab.text())).toEqual(['General', 'Logins', 'Advanced', 'About']);
    expect(tabs[0].attributes('aria-current')).toBe('page');

    await router.push('/settings/logins');
    await nextTick();
    const updated = wrapper.findAll('nav a');
    expect(updated[0].attributes('aria-current')).toBeUndefined();
    expect(updated[1].attributes('aria-current')).toBe('page');
  });

  it('saves a change once, and reset replaces the same draft without saving it back', async () => {
    vi.useFakeTimers();
    patch.mockReset();
    reset.mockReset();
    showToast.mockReset();
    setTheme.mockReset();
    Object.assign(settingsState, structuredClone(defaultSettings));

    patch.mockImplementation(async (draft: Settings) => {
      Object.assign(settingsState, JSON.parse(JSON.stringify(draft)) as Settings);
    });
    reset.mockImplementation(async () => {
      const next = structuredClone(defaultSettings);
      Object.assign(settingsState, next);
      return next;
    });

    const router = createSettingsRouter();
    await router.push('/settings/advanced');
    await router.isReady();
    const wrapper = mount(createRoot(), { global: { plugins: [router, i18n] } });
    await nextTick();

    await wrapper.get('#out-keep-video').trigger('click');
    await vi.advanceTimersByTimeAsync(1000);
    await flushPromises();

    expect(patch).toHaveBeenCalledTimes(1);
    expect(patch.mock.calls[0][0].output.video.policy).toBe(TranscodePolicy.never);

    const resetButton = wrapper.findAll('button').find(button => button.text().includes('Reset'));
    expect(resetButton).toBeTruthy();
    await resetButton!.trigger('click');
    await flushPromises();
    await vi.advanceTimersByTimeAsync(1000);

    expect(reset).toHaveBeenCalledTimes(1);
    expect(patch).toHaveBeenCalledTimes(1);
    expect(showToast).toHaveBeenCalledWith('Reset settings to defaults', { style: 'success' });
    expect(settingsState.output.video.policy).toBe(defaultSettings.output.video.policy);
  });
  it('reports a failed autosave, keeps the draft, and never claims it was saved', async () => {
    vi.useFakeTimers();
    patch.mockReset();
    showToast.mockReset();
    Object.assign(settingsState, structuredClone(defaultSettings));
    patch.mockRejectedValueOnce(new Error('disk full'));
    const router = createSettingsRouter();
    await router.push('/settings/advanced');
    await router.isReady();
    const wrapper = mount(createRoot(), { global: { plugins: [router, i18n] } });
    await wrapper.get('#out-keep-video').trigger('click');
    await vi.advanceTimersByTimeAsync(500);
    expect(showToast).toHaveBeenCalledWith('Couldn\'t save settings. Your changes are still here', { style: 'error' });
    expect(wrapper.get('[role="status"]').text()).toBe('');
    // The failed draft can be retried without losing the user's input.
    patch.mockImplementation(async (draft: Settings) => Object.assign(settingsState, JSON.parse(JSON.stringify(draft))));
    await wrapper.findComponent(SettingsView).vm.$.exposed!.flush();
    await nextTick();
    expect(settingsState.output.video.policy).toBe(TranscodePolicy.never);
    expect(wrapper.get('[role="status"]').text()).toBe('Saved');
    await wrapper.get('#out-keep-video').trigger('click');
    expect(wrapper.get('[role="status"]').text()).toBe('');
    wrapper.unmount();
  });
  it('saves a Logins change as the full Settings snapshot (config_set replaces, it does not merge auth)', async () => {
    vi.useFakeTimers();
    patch.mockReset();
    Object.assign(settingsState, structuredClone(defaultSettings));
    patch.mockImplementation(async (draft: Settings) => Object.assign(settingsState, JSON.parse(JSON.stringify(draft))));
    const router = createSettingsRouter();
    await router.push('/settings/logins');
    await router.isReady();
    const wrapper = mount(createRoot(), { global: { plugins: [router, i18n] } });
    await pickOption(wrapper, 'pref-login-policy', 'never');
    await vi.advanceTimersByTimeAsync(1000);
    await flushPromises();

    expect(patch).toHaveBeenCalledTimes(1);
    expect(patch.mock.calls[0][0]).toEqual({ ...defaultSettings, auth: { ...defaultSettings.auth, cookieBrowser: 'none' } });
    wrapper.unmount();
  });
});

describe('legacy login preferences', () => {
  it.each([
    ['automatic', false, 'ask'], ['chrome', false, 'ask'],
    ['chrome', true, 'always'], ['none', false, 'never'], ['none', true, 'never'], ['', true, 'never'],
  ])('reads %s / %s as %s and writes compatible settings', async (cookieBrowser, always, expected) => {
    const settings = structuredClone(defaultSettings);
    settings.auth.cookieBrowser = String(cookieBrowser);
    settings.auth.alwaysUseBrowserLogins = Boolean(always);
    const wrapper = mount(SettingsLoginsTab, { props: { modelValue: settings }, global: { plugins: [i18n] } });
    const policy = wrapper.findAllComponents({ name: 'SelectField' }).find(c => c.props('id') === 'pref-login-policy')!;
    expect(policy.props('modelValue')).toBe(expected);
    await pickOption(wrapper, 'pref-login-policy', 'always');
    expect(settings.auth.cookieBrowser).toBe(cookieBrowser === 'none' || cookieBrowser === '' ? 'automatic' : cookieBrowser);
    expect(settings.auth.alwaysUseBrowserLogins).toBe(true);
    await pickOption(wrapper, 'pref-cookie-browser', 'firefox');
    await pickOption(wrapper, 'pref-login-policy', 'never');
    expect(settings.auth.cookieBrowser).toBe('none');
    expect(settings.auth.alwaysUseBrowserLogins).toBe(false);
    await pickOption(wrapper, 'pref-login-policy', 'ask');
    expect(settings.auth.cookieBrowser).toBe('firefox');
    expect(settings.auth.alwaysUseBrowserLogins).toBe(false);
    expect(settings.auth.cookieFile).toBe(defaultSettings.auth.cookieFile);
    wrapper.unmount();
  });
});
