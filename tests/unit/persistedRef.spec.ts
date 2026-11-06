import { beforeEach, describe, expect, it } from 'vitest';
import { nextTick } from 'vue';
import { choiceCodec, flagCodec, persistedRef } from '../../src/helpers/persistedRef';

describe('persistedRef', () => {
  beforeEach(() => localStorage.clear());

  it('uses the fallback when nothing valid is stored, and writes changes back', async () => {
    localStorage.setItem('k.choice', 'bogus');
    const choice = persistedRef('k.choice', choiceCodec(['best', 'audio']), 'best');
    expect(choice.value).toBe('best');
    choice.value = 'audio';
    await nextTick();
    expect(localStorage.getItem('k.choice')).toBe('audio');
  });

  it('reads a stored flag, including a stored false over a true fallback', () => {
    localStorage.setItem('k.flag', '0');
    expect(persistedRef('k.flag', flagCodec, true).value).toBe(false);
  });
});
