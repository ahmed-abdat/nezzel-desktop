import { ref } from 'vue';

/** Open/closed per playlist row for this session (not saved): a row the user closed stays closed. */
const chosen = ref<Record<string, boolean>>({});

export const isPlaylistOpen = (id: string, fallback: boolean): boolean => chosen.value[id] ?? fallback;

export function setPlaylistOpen(id: string, open: boolean) {
  chosen.value = { ...chosen.value, [id]: open };
}
