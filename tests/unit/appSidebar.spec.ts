import { mount } from '@vue/test-utils';
import { computed } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { i18n } from '../../src/i18n';

vi.mock('../../src/composables/usePlatform', () => ({
  usePlatform: () => ({ isMac: computed(() => true), isWindows: computed(() => false), platform: computed(() => 'macos') }),
}));

const { default: AppSidebar } = await import('../../src/components/shell/AppSidebar.vue');

async function mountSidebar(locale: 'en' | 'ar', collapsed = false) {
  i18n.global.locale.value = locale;
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', name: 'home', component: { render: () => null } },
      { path: '/settings', name: 'settings.general', component: { render: () => null } },
      { path: '/settings/about', name: 'settings.about', component: { render: () => null } },
    ],
  });
  await router.push('/');
  return mount(AppSidebar, { props: { collapsed, canToggle: true }, global: { plugins: [router, i18n], stubs: { Tooltip: { template: '<slot />' } } } });
}

describe('sidebar brand on macOS', () => {
  afterEach(() => {
    i18n.global.locale.value = 'en';
  });

  it('leaves the title strip to the traffic lights in LTR', async () => {
    expect((await mountSidebar('en')).find('img').exists()).toBe(false);
  });

  it('shows the brand in RTL, where the traffic lights sit over the toolbar', async () => {
    expect((await mountSidebar('ar')).find('img').exists()).toBe(true);
  });

  it('shows the maker line only when expanded, linking to About', async () => {
    const byline = (await mountSidebar('ar')).find('[data-testid=sidebar-byline]');
    expect(byline.text()).toContain('Hululsoft');
    expect(byline.attributes('href')).toBe('/settings/about');
    expect((await mountSidebar('ar', true)).find('[data-testid=sidebar-byline]').exists()).toBe(false);
  });
});
