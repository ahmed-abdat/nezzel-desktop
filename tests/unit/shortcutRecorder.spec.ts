import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { nextTick } from 'vue';
import ShortcutRecorder from '../../src/components/settings/ShortcutRecorder.vue';
import { i18n } from '../../src/i18n';
import {
  DEFAULT_CAPTURE_SHORTCUT,
  FALLBACK_CAPTURE_SHORTCUT,
  heldKeys,
  isReservedShortcut,
  recordKey,
  sameShortcut,
  shortcutKeys,
  shortcutLabel,
} from '../../src/helpers/shortcut';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve()) }));

const key = (code: string, mods: Partial<Record<'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey', boolean>> = {}, keyName?: string) => ({
  key: keyName ?? (code.startsWith('Key') ? code.slice(3).toLowerCase() : code),
  code,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe('shortcut accelerators', () => {
  it.each([
    ['CommandOrControl+Shift+D', true, ['⇧', '⌘', 'D']], // Apple menu order ⌃⌥⇧⌘
    ['CommandOrControl+Shift+D', false, ['Ctrl', 'Shift', 'D']],
    [DEFAULT_CAPTURE_SHORTCUT, true, ['⌥', '⌘', 'N']],
    [DEFAULT_CAPTURE_SHORTCUT, false, ['Ctrl', 'Alt', 'N']],
    [FALLBACK_CAPTURE_SHORTCUT, true, ['⌥', '⇧', '⌘', 'D']],
    [FALLBACK_CAPTURE_SHORTCUT, false, ['Ctrl', 'Alt', 'Shift', 'D']],
    ['CmdOrCtrl+Enter', true, ['⌘', '↩']],
    ['CommandOrControl+Shift+Enter', false, ['Ctrl', 'Shift', 'Enter']],
    ['', true, []],
  ])('renders %s (mac=%s) as keycaps', (accelerator, mac, caps) => {
    expect(shortcutKeys(accelerator, mac)).toEqual(caps);
  });

  it('never shows the raw accelerator in labels', () => {
    expect(shortcutLabel('CommandOrControl+Shift+D', true)).toBe('⇧⌘D');
    expect(shortcutLabel('CommandOrControl+Shift+D', false)).toBe('Ctrl+Shift+D');
  });

  it('records the primary modifier as CommandOrControl on every OS', () => {
    expect(recordKey(key('KeyK', { metaKey: true, shiftKey: true }), true)).toEqual({ kind: 'ok', accelerator: 'CommandOrControl+Shift+K' });
    expect(recordKey(key('KeyK', { ctrlKey: true, shiftKey: true }), false)).toEqual({ kind: 'ok', accelerator: 'CommandOrControl+Shift+K' });
    expect(recordKey(key('Digit5', { altKey: true }), false)).toEqual({ kind: 'ok', accelerator: 'Alt+5' });
  });

  it('rejects bare keys and Shift-only combos, waits on lone modifiers', () => {
    expect(recordKey(key('KeyK'), true).kind).toBe('needsModifier');
    expect(recordKey(key('KeyK', { shiftKey: true }), false).kind).toBe('needsModifier');
    expect(recordKey(key('MetaLeft', { metaKey: true }, 'Meta'), true).kind).toBe('pending');
  });

  it('Esc cancels and Backspace clears', () => {
    expect(recordKey(key('Escape', {}, 'Escape'), true).kind).toBe('cancel');
    expect(recordKey(key('Backspace', {}, 'Backspace'), true).kind).toBe('clear');
  });

  it('flags OS-owned shortcuts per platform', () => {
    expect(isReservedShortcut('CommandOrControl+Q', true)).toBe(true);
    expect(isReservedShortcut('CommandOrControl+Shift+4', true)).toBe(true);
    expect(isReservedShortcut('Alt+F4', false)).toBe(true);
    expect(isReservedShortcut('Super+E', false)).toBe(true);
    expect(isReservedShortcut('CommandOrControl+Shift+D', false)).toBe(true);
    expect(isReservedShortcut(DEFAULT_CAPTURE_SHORTCUT, true)).toBe(false);
    expect(isReservedShortcut(DEFAULT_CAPTURE_SHORTCUT, false)).toBe(false);
    expect(isReservedShortcut(FALLBACK_CAPTURE_SHORTCUT, true)).toBe(false);
    expect(isReservedShortcut(FALLBACK_CAPTURE_SHORTCUT, false)).toBe(false);
  });

  it.each([
    [{ metaKey: true, shiftKey: true }, true, ['⇧', '⌘']],
    [{ ctrlKey: true, altKey: true }, true, ['⌃', '⌥']],
    [{ ctrlKey: true, shiftKey: true }, false, ['Ctrl', 'Shift']],
    [{ metaKey: true }, false, ['Win']],
    [{}, true, []],
  ])('shows held modifiers %o (mac=%s) as %o', (mods, mac, caps) => {
    expect(heldKeys(key('ShiftLeft', mods, 'Shift'), mac)).toEqual(caps);
  });

  it('compares shortcuts regardless of alias and order', () => {
    expect(sameShortcut('Shift+CmdOrCtrl+d', 'CommandOrControl+Shift+D')).toBe(true);
    expect(sameShortcut('CommandOrControl+D', 'CommandOrControl+Shift+D')).toBe(false);
  });
});

function mountRecorder(props: Record<string, unknown> = {}) {
  return mount(ShortcutRecorder, {
    props: {
      'modelValue': DEFAULT_CAPTURE_SHORTCUT,
      'mac': true,
      'label': 'Quick download',
      'defaultValue': DEFAULT_CAPTURE_SHORTCUT,
      'onUpdate:modelValue': () => undefined,
      ...props,
    },
    global: { plugins: [i18n] },
  });
}
const lastValue = (wrapper: ReturnType<typeof mountRecorder>) => wrapper.emitted('update:modelValue')?.at(-1)?.[0];

describe('ShortcutRecorder', () => {
  it('shows keycaps, records a new combo on click', async () => {
    const wrapper = mountRecorder();
    const control = wrapper.get('[data-testid="shortcut-control"]');
    expect(control.findAll('kbd').map(cap => cap.text())).toEqual(['⌥', '⌘', 'N']);
    expect(wrapper.text()).not.toContain('CommandOrControl');

    await control.trigger('click');
    expect(wrapper.text()).toContain(i18n.global.t('shortcut.press'));
    await control.trigger('keydown', key('KeyK', { metaKey: true, altKey: true }));

    expect(lastValue(wrapper)).toBe('CommandOrControl+Alt+K');
  });

  it('takes focus on click, so keys reach it where a click does not focus buttons (macOS WebKit)', async () => {
    const wrapper = mount(ShortcutRecorder, {
      attachTo: document.body,
      props: { 'modelValue': DEFAULT_CAPTURE_SHORTCUT, 'mac': true, 'label': 'Quick download', 'onUpdate:modelValue': () => undefined },
      global: { plugins: [i18n] },
    });
    try {
      const control = wrapper.get('[data-testid="shortcut-control"]');
      await control.trigger('click'); // jsdom, like WebKit, does not focus a button on click
      expect(document.activeElement).toBe(control.element);
      (control.element as HTMLElement).blur();
      await nextTick();
      expect(wrapper.text()).not.toContain(i18n.global.t('shortcut.press'));
    } finally {
      wrapper.unmount();
    }
  });

  it('lets a plain Tab leave while recording instead of trapping keyboard focus', async () => {
    const wrapper = mountRecorder();
    const control = wrapper.get('[data-testid="shortcut-control"]');
    await control.trigger('click');
    const tab = new KeyboardEvent('keydown', { key: 'Tab', code: 'Tab', cancelable: true, bubbles: true });
    control.element.dispatchEvent(tab);
    expect(tab.defaultPrevented).toBe(false);
    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
  });

  it('shows held modifiers live and explains how to record', async () => {
    const wrapper = mountRecorder();
    const control = wrapper.get('[data-testid="shortcut-control"]');
    await control.trigger('click');
    expect(wrapper.text()).toContain(i18n.global.t('shortcut.recordingMac'));
    await control.trigger('keydown', key('MetaLeft', { metaKey: true }, 'Meta'));
    expect(wrapper.get('[data-testid="shortcut-held"]').findAll('kbd').map(cap => cap.text())).toEqual(['⌘']);
    await control.trigger('keyup', key('MetaLeft', {}, 'Meta'));
    expect(wrapper.find('[data-testid="shortcut-held"]').exists()).toBe(false);
  });

  it('pauses the OS-wide shortcut while listening and restores it after', async () => {
    vi.mocked(invoke).mockClear();
    const wrapper = mountRecorder();
    const control = wrapper.get('[data-testid="shortcut-control"]');
    await control.trigger('click');
    expect(invoke).toHaveBeenLastCalledWith('shortcuts_pause', { paused: true });
    await control.trigger('keydown', key('Escape', {}, 'Escape'));
    expect(invoke).toHaveBeenLastCalledWith('shortcuts_pause', { paused: false });
  });

  it('Esc cancels recording without changing the shortcut', async () => {
    const wrapper = mountRecorder();
    const control = wrapper.get('[data-testid="shortcut-control"]');
    await control.trigger('click');
    await control.trigger('keydown', key('Escape', {}, 'Escape'));

    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
    expect(wrapper.text()).not.toContain(i18n.global.t('shortcut.press'));
  });

  it('rejects a bare key with an inline hint', async () => {
    const wrapper = mountRecorder();
    const control = wrapper.get('[data-testid="shortcut-control"]');
    await control.trigger('click');
    await control.trigger('keydown', key('KeyK'));

    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
    expect(wrapper.text()).toContain(i18n.global.t('shortcut.needsModifierMac'));
  });

  it('refuses a combination the validator rejects and keeps the old one', async () => {
    const wrapper = mountRecorder({ validate: (value: string) => (value === 'CommandOrControl+Q' ? 'macOS uses this shortcut. Try another.' : null) });
    const control = wrapper.get('[data-testid="shortcut-control"]');
    await control.trigger('click');
    await control.trigger('keydown', key('KeyQ', { metaKey: true }));

    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
    expect(wrapper.text()).toContain('macOS uses this shortcut');
  });

  it('rolls back to the previous shortcut when the OS refuses it', async () => {
    const wrapper = mountRecorder({ status: { seq: 3, failed: false } });
    const control = wrapper.get('[data-testid="shortcut-control"]');
    await control.trigger('click');
    await control.trigger('keydown', key('KeyK', { metaKey: true, shiftKey: true }));
    expect(lastValue(wrapper)).toBe('CommandOrControl+Shift+K');

    await wrapper.setProps({ modelValue: 'CommandOrControl+Shift+K', status: { seq: 4, failed: true } });
    await nextTick();

    expect(lastValue(wrapper)).toBe(DEFAULT_CAPTURE_SHORTCUT);
    expect(wrapper.text()).toContain(i18n.global.t('shortcut.taken'));
  });

  it('reset restores the default and only shows when not default', async () => {
    const wrapper = mountRecorder({ modelValue: 'CommandOrControl+Alt+K' });
    await wrapper.get('[data-testid="shortcut-reset"]').trigger('click');
    expect(lastValue(wrapper)).toBe(DEFAULT_CAPTURE_SHORTCUT);

    const atDefault = mountRecorder();
    expect(atDefault.find('[data-testid="shortcut-reset"]').exists()).toBe(false);
  });

  it('Backspace clears (turns the shortcut off)', async () => {
    const wrapper = mountRecorder();
    const control = wrapper.get('[data-testid="shortcut-control"]');
    await control.trigger('click');
    await control.trigger('keydown', key('Backspace', {}, 'Backspace'));
    expect(lastValue(wrapper)).toBe('');
  });
});
