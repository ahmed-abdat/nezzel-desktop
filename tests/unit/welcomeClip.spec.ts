import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import WelcomeClip from '../../src/components/welcome/WelcomeClip.vue';
import { WELCOME_SAMPLE } from '../../src/components/welcome/welcomeSample';
import { i18n } from '../../src/i18n';
import { formatSize } from '../../src/helpers/localeNumber';
import { installTauriMock } from '../utils/tauriMock';

describe('welcome clip sample card', () => {
  it('shows the one sample constant: length, quality, container, size', () => {
    installTauriMock({});
    // Reduced motion: the clip starts still, so no animation clock runs in jsdom.
    vi.stubGlobal('matchMedia', () => ({ matches: true, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
    const wrapper = mount(WelcomeClip, { global: { plugins: [i18n] } });
    const card = wrapper.find('.wc-card').text();
    expect(card).toContain('00:19');
    expect(card).toContain(`${WELCOME_SAMPLE.height}p`);
    expect(card).toContain(WELCOME_SAMPLE.ext);
    expect(card).toContain(formatSize(WELCOME_SAMPLE.bytes, i18n.global.locale.value as string));
    expect(card).not.toContain('360p');
    wrapper.unmount();
    vi.unstubAllGlobals();
  });
});
