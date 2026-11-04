import { DOMWrapper, flushPromises, mount } from '@vue/test-utils';
import { describe, expect, it, beforeEach } from 'vitest';
import PlaylistSelectionModal from '../../src/components/media-card/steps/PlaylistSelectionModal.vue';
import { i18n } from '../../src/i18n';

describe('PlaylistSelectionModal', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  it('opens with a cloned selection and emits a cloned draft on save', async () => {
    const wrapper = mount(PlaylistSelectionModal, {
      global: {
        plugins: [i18n],
      },
      attachTo: document.body,
    });
    // The dialog renders in a portal on <body>, outside the component's own root.
    const body = new DOMWrapper(document.body);

    const original = {
      rows: [{ id: 'single', type: 'single' as const, index: 1 }],
    };

    (wrapper.vm as unknown as { open: (selection: typeof original) => void }).open(original);
    await flushPromises();

    await body.get('input[id^="playlist-row-index-"]').setValue('2');
    await body.findAll('button').find(button => button.text().includes('Use advanced selection'))!.trigger('click');

    expect(wrapper.emitted('apply')).toEqual([[
      { rows: [{ id: 'single', type: 'single', index: 2 }] },
    ]]);
    expect(original.rows[0].index).toBe(1);
  });

  it('disables save when a row becomes invalid', async () => {
    const wrapper = mount(PlaylistSelectionModal, {
      global: {
        plugins: [i18n],
      },
      attachTo: document.body,
    });
    // The dialog renders in a portal on <body>, outside the component's own root.
    const body = new DOMWrapper(document.body);

    (wrapper.vm as unknown as { open: (selection: { rows: never[] }) => void }).open({ rows: [] });
    await flushPromises();

    await body.findAll('button').find(button => button.text().includes('Add range'))!.trigger('click');
    await body.get('input[id^="playlist-row-start-"]').setValue('1');

    const saveButton = body.findAll('button').find(button => button.text().includes('Use advanced selection'));
    expect(saveButton?.attributes('disabled')).toBeDefined();
    expect(body.text()).toContain('Set start and end.');
  });

  it('switches row types and removes rows', async () => {
    const wrapper = mount(PlaylistSelectionModal, {
      global: {
        plugins: [i18n],
      },
      attachTo: document.body,
    });
    // The dialog renders in a portal on <body>, outside the component's own root.
    const body = new DOMWrapper(document.body);

    (wrapper.vm as unknown as { open: (selection: { rows: never[] }) => void }).open({ rows: [] });
    await flushPromises();

    await body.findAll('button').find(button => button.text().includes('Add item'))!.trigger('click');
    expect(body.find('input[id^="playlist-row-index-"]').exists()).toBe(true);

    await body.findAll('button').find(button => button.text() === 'Range')!.trigger('click');
    expect(body.find('input[id^="playlist-row-start-"]').exists()).toBe(true);

    await body.findAll('button').find(button => button.text().includes('Remove'))!.trigger('click');
    expect(body.text()).toContain('No advanced selection configured yet');
  });
});
