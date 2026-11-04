import { mount } from '@vue/test-utils';
import { defineComponent } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { describe, expect, it, vi } from 'vitest';
import AppLayout from '../../src/layouts/AppLayout.vue';
import { i18n } from '../../src/i18n';

const pasteField = vi.fn();
const Toolbar = defineComponent({
  setup(_, { expose }) {
    expose({ pasteField, focusField: vi.fn() });
    return () => null;
  },
});

describe('app-wide paste shortcut', () => {
  it('reads the clipboard explicitly outside editors and leaves native paste inside editors', async () => {
    const router = createRouter({
      history: createMemoryHistory(),
      routes: [{ path: '/', name: 'home', component: { template: '<div />' } }],
    });
    await router.push('/');
    const wrapper = mount(AppLayout, {
      attachTo: document.body,
      global: {
        plugins: [router, i18n],
        stubs: { AppToolbar: Toolbar, AppSidebar: true, AppStatusBar: true, TheUpdater: true, DownloadInspector: true },
      },
    });
    try {
      for (const modifier of ['metaKey', 'ctrlKey']) {
        pasteField.mockClear();
        const event = new KeyboardEvent('keydown', { key: 'v', [modifier]: true, bubbles: true, cancelable: true });
        document.body.dispatchEvent(event);
        expect(event.defaultPrevented).toBe(true);
        expect(pasteField).toHaveBeenCalledOnce();
      }
      const editor = document.createElement('textarea');
      document.body.append(editor);
      pasteField.mockClear();
      const event = new KeyboardEvent('keydown', { key: 'v', ctrlKey: true, bubbles: true, cancelable: true });
      editor.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
      expect(pasteField).not.toHaveBeenCalled();
      editor.remove();
    } finally {
      wrapper.unmount();
    }
  });
});
