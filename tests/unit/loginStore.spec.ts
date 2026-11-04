import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { emit } from '@tauri-apps/api/event';
import { LOGIN_TTL_MS, useLoginStore } from '../../src/stores/login';
import { useMediaGroupStore } from '../../src/stores/media/group';
import { MediaState, useMediaStateStore } from '../../src/stores/media/state';
import { useMediaStore } from '../../src/stores/media/media';
import { registerLoginListeners } from '../../src/tauri/listeners/login';
import type { LoginNeeded } from '../../src/tauri/types/login';
import type { Group } from '../../src/tauri/types/group';

const { accept, dismiss, choices } = vi.hoisted(() => ({ accept: vi.fn(), dismiss: vi.fn(), choices: vi.fn() }));
vi.mock('../../src/tauri/listeners/login', async importOriginal => ({
  ...(await importOriginal<object>()),
  acceptBrowserLogin: accept,
  dismissBrowserLogin: dismiss,
  browserLoginChoices: choices,
}));
vi.mock('../../src/tauri/notifications', () => ({ notify: vi.fn(), notifyGroup: vi.fn() }));

const chrome = { id: '147599e2-568e-469f-a8cd-fbf9a79606ca', browser: 'chrome', profile: 'Profile 1', signedIn: false };
const firefox = { id: 'ce6ea606-f7f8-4e7d-8d69-0cab2916d928', browser: 'firefox', profile: 'default-release', signedIn: false };

function request(over: Partial<LoginNeeded> = {}): LoginNeeded {
  return {
    requestId: 'r1', id: 'item1', groupId: 'g1', host: 'www.instagram.com', reason: 'loginRequired',
    status: 'needed', candidates: [chrome, firefox], ...over,
  };
}

function addGroup(id = 'g1', itemId = 'item1') {
  const group = {
    id, url: 'https://www.instagram.com/p/x', title: 'Post', total: 1, processed: 0, errored: 0, isCombined: false,
    audioCodecs: [], videoCodecs: [], audioTracks: [], videoTracks: [], formats: [], filesize: 0,
    items: { [itemId]: { id: itemId, isLeader: true, url: 'https://www.instagram.com/p/x', audioCodecs: [], videoCodecs: [], audioTracks: [], videoTracks: [], formats: [], filesize: 0 } },
  } as unknown as Group;
  useMediaGroupStore().createGroup(group);
  useMediaStateStore().setState(itemId, MediaState.fetching);
}

describe('login store', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    accept.mockReset().mockResolvedValue(undefined);
    dismiss.mockReset().mockResolvedValue(undefined);
    choices.mockReset();
  });
  afterEach(() => vi.useRealTimers());

  it('allows profile choices through isolation while rejecting unknown commands', async () => {
    const previous = window.__TAURI_ISOLATION_HOOK__;
    const warning = vi.spyOn(console, 'warn').mockImplementation(() => {});
    await import('../../src-isolation/main');
    try {
      const payload = { cmd: 'media_login_choices' };
      expect(window.__TAURI_ISOLATION_HOOK__(payload)).toBe(payload);
      expect(() => window.__TAURI_ISOLATION_HOOK__({ cmd: 'unapproved_profile_reader' })).toThrow('Unauthorized command');
    } finally {
      window.__TAURI_ISOLATION_HOOK__ = previous;
      warning.mockRestore();
    }
  });

  it('keeps the actual automatic profile on each row and changing it only opens the picker', async () => {
    const store = useLoginStore();
    store.receiveUsing({ id: 'item1', groupId: 'g1', host: 'instagram.com', candidate: chrome });
    store.receiveUsing({ id: 'item2', groupId: 'g2', host: 'instagram.com', candidate: firefox });
    choices.mockResolvedValue(request({ changeOnly: true }));
    await store.change('g1');
    expect(store.using.g1.candidate.id).toBe(chrome.id);
    expect(store.using.g2.candidate.id).toBe(firefox.id);
    expect(store.cardFor('g1')).toMatchObject({ kind: 'needed', request: { changeOnly: true } });
    await store.dismiss('r1');
    expect(store.cardFor('g1')).toBeNull();
    expect(store.failed.g1).toBeUndefined();
    expect(store.using.g1.candidate.id).toBe(chrome.id);
    store.forgetGroup('g1');
    expect(store.using.g1).toBeUndefined();
  });

  it('keeps the previous refusal until the selected profile is actually attempted', async () => {
    const store = useLoginStore();
    store.receiveUsing({ id: 'item1', groupId: 'g1', host: 'instagram.com', candidate: chrome });
    store.receive(request({ status: 'exhausted', candidates: [] }));
    store.receive(request({ requestId: 'choice', changeOnly: true }));
    await store.accept('choice', firefox.id);
    store.receiveUsing({ id: 'item1', groupId: 'g1', host: 'instagram.com', candidate: firefox, selectedOnly: true });
    expect(store.failed.g1.tried).toBe(chrome.id);
    expect(store.using.g1.candidate.id).toBe(firefox.id);
    store.receiveUsing({ id: 'item1', groupId: 'g1', host: 'instagram.com', candidate: firefox });
    expect(store.failed.g1).toBeUndefined();
  });

  it('attaches a request to its row without guessing an unsigned profile', () => {
    const store = useLoginStore();
    store.receive(request());
    expect(store.cardFor('g1')).toMatchObject({ kind: 'needed', preselect: '', request: { requestId: 'r1' } });
    expect(store.cardFor('other')).toBeNull();
  });

  it('preselects signed-in evidence ahead of the first and previously tried profile', () => {
    const store = useLoginStore();
    const signedIn = { ...firefox, signedIn: true };
    store.tried['www.instagram.com'] = signedIn.id;
    store.receive(request({ candidates: [chrome, signedIn] }));
    expect(store.cardFor('g1')).toMatchObject({ preselect: signedIn.id });
  });

  it('accept sends the chosen browser once, covers waiting siblings on the same host, and clears the card', async () => {
    const store = useLoginStore();
    store.receive(request());
    store.receive(request({ requestId: 'r2', id: 'item2' }));
    store.receive(request({ requestId: 'r3', id: 'item3', host: 'example.com' }));
    await store.accept('r1', chrome.id);
    expect(accept.mock.calls).toEqual([['r1', chrome.id], ['r2', chrome.id]]);
    expect(store.cardFor('g1')).toMatchObject({ request: { requestId: 'r3' } });
    // Without signed-in evidence a later card requires an explicit selection.
    store.forgetGroup('g1');
    store.receive(request({ requestId: 'r5' }));
    expect(store.cardFor('g1')).toMatchObject({ preselect: '' });
  });

  it('dismiss tells the engine, clears the card and leaves the row failed with "needs login"', async () => {
    addGroup();
    const store = useLoginStore();
    store.receive(request());
    await store.dismiss('r1');
    expect(dismiss).toHaveBeenCalledWith('r1');
    expect(store.cardFor('g1')).toEqual({ kind: 'failed', failure: { groupId: 'g1', host: 'www.instagram.com', reason: 'dismissed' } });
    expect(useMediaStateStore().getGroupState('g1')).toBe(MediaState.error);
  });

  it('expires the card after 15 minutes', () => {
    const store = useLoginStore();
    store.receive(request());
    vi.advanceTimersByTime(LOGIN_TTL_MS - 1);
    expect(store.cardFor('g1')?.kind).toBe('needed');
    vi.advanceTimersByTime(1);
    expect(store.cardFor('g1')).toMatchObject({ kind: 'failed', failure: { reason: 'expired' } });
    store.forgetGroup('g1');
    expect(store.cardFor('g1')).toBeNull();
  });

  it('exhausted replaces the card with a plain failure naming the browser that was tried', async () => {
    const store = useLoginStore();
    store.receive(request());
    await store.accept('r1', chrome.id);
    store.receive(request({ requestId: 'r9', status: 'exhausted', candidates: [] }));
    expect(store.cardFor('g1')).toEqual({ kind: 'failed', failure: { groupId: 'g1', host: 'www.instagram.com', reason: 'exhausted', tried: chrome.id, browser: 'chrome' } });
    // The engine's fatal that follows an exhausted request must not wipe the failure.
    store.onFatal({ id: 'item1', groupId: 'g1' });
    expect(store.cardFor('g1')?.kind).toBe('failed');
  });

  it('listens once at app start and forgets a removed row', async () => {
    addGroup();
    await registerLoginListeners();
    await emit('media_login_needed', request());
    const store = useLoginStore();
    expect(store.cardFor('g1')?.kind).toBe('needed');
    await emit('media_fatal', { id: 'item1', groupId: 'g1' });
    expect(store.cardFor('g1')).toBeNull();
    store.receive(request({ requestId: 'r2' }));
    useMediaStore().deleteGroup('g1');
    expect(store.cardFor('g1')).toBeNull();
  });
});
