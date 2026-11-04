import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyDocumentLocale, getDefaultLocale, type Locale } from '../../src/i18n';
import { i18n } from '../../src/i18n';
import { mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { defineComponent, h, ref } from 'vue';
import { useDiagnostic } from '../../src/composables/useDiagnostic';
import en from '../../src/locales/en.json';

describe('locale direction', () => {
  it('falls back to English for a new diagnostic key in an older locale', () => {
    i18n.global.locale.value = 'es';
    const component = defineComponent({
      setup() {
        const { diagnosticDisplay } = useDiagnostic(ref({
          id: 'image', groupId: 'images', level: 'error', code: 'imageEngineUnavailable',
          message: '', raw: '', component: 'gallery-dl', timestamp: 1,
        }));
        return () => h('p', diagnosticDisplay.value.message);
      },
    });
    const wrapper = mount(component, { global: { plugins: [i18n, createPinia()] } });
    try {
      expect(wrapper.text()).toBe(en.errors.runner.imageEngineUnavailable.message);
    } finally {
      wrapper.unmount();
      i18n.global.locale.value = 'en';
    }
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it.each([
    [['ar-MR'], 'ar'],
    [['ar'], 'ar'],
    [['fr-FR', 'en'], 'fr'],
    [['xx'], 'en'],
  ])('picks %j -> %s from the system languages', (languages, expected) => {
    vi.stubGlobal('navigator', { languages, language: languages[0] });
    expect(getDefaultLocale()).toBe(expected);
  });

  it.each([
    ['ar', 'rtl'],
    ['en', 'ltr'],
    ['fr', 'ltr'],
  ] as [Locale, string][])('sets lang and dir on <html> for %s', (locale, dir) => {
    applyDocumentLocale(locale);
    expect(document.documentElement.getAttribute('lang')).toBe(locale);
    expect(document.documentElement.getAttribute('dir')).toBe(dir);
  });
});
