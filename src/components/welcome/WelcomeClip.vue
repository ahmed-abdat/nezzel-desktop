<template>
  <figure class="wc" data-testid="welcome-clip">
    <video
      v-if="src"
      ref="video"
      class="wc-media"
      :src="src"
      :poster="poster"
      :aria-label="t('welcome.clip.alt')"
      muted
      loop
      playsinline
      preload="auto"
      disablepictureinpicture
      @timeupdate="onTime"
      @play="playing = true"
      @pause="playing = false"
    />
    <img v-else-if="poster" class="wc-media" :src="poster" :alt="t('welcome.clip.alt')">
    <!-- Moving pictures need a way to stop them (WCAG 2.2.2); with reduced motion the clip starts paused. -->
    <Button v-if="src" variant="outline" size="icon-xs" class="wc-toggle" :aria-label="playing ? t('welcome.clip.pause') : t('welcome.clip.play')" data-testid="welcome-clip-toggle" @click="toggle">
      <component :is="playing ? Pause : Play" aria-hidden="true" />
    </Button>
  </figure>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Pause, Play } from '@lucide/vue';
import { Button } from '../ui/button';
import { localeDirection } from '../../i18n';

/**
 * A real recording of the app (copy a link, the card shows, Save), one file per text direction and theme:
 * `assets/welcome/{ltr|rtl}-{light|dark}.mp4` with a still of the same name as poster.
 */
const CLIPS = import.meta.glob<string>('../../assets/welcome/*.mp4', { eager: true, query: '?url', import: 'default' });
const STILLS = import.meta.glob<string>('../../assets/welcome/*.{webp,png,gif}', { eager: true, query: '?url', import: 'default' });
/** Seconds at which each step of the recording starts: copy, card, save. */
const STEP_AT = [0, 1.3, 4.2];

const emit = defineEmits<{ step: [step: number] }>();
const { t, locale } = useI18n();

const systemDark = ref(false);
const reducedMotion = ref(false);
/** The theme the app shows now: the `data-theme` the app set on <body>, else the system's. */
const appTheme = ref<string | null>(null);
const dark = computed(() => (appTheme.value ? appTheme.value === 'dark' : systemDark.value));

function pick(files: Record<string, string>): string | undefined {
  const byName = new Map(Object.entries(files).map(([path, url]) => [path.split('/').pop()?.replace(/\.\w+$/, ''), url]));
  const dir = localeDirection(locale.value);
  const theme = dark.value ? 'dark' : 'light';
  return byName.get(`${dir}-${theme}`) ?? byName.get(`${dir}-light`) ?? byName.get('ltr-light') ?? byName.values().next().value;
}
const src = computed(() => pick(CLIPS));
const poster = computed(() => pick(STILLS));

const video = ref<HTMLVideoElement>();
const playing = ref(false);

function onTime() {
  const at = video.value?.currentTime ?? 0;
  emit('step', STEP_AT.reduce((step, start, i) => (at >= start ? i + 1 : step), 0));
}
function play() {
  // Autoplay can be refused (no user gesture yet): the poster and the play button stay.
  void video.value?.play().catch(() => undefined);
}
function toggle() {
  if (playing.value) video.value?.pause();
  else play();
}

let media: { dark: MediaQueryList; motion: MediaQueryList } | undefined;
function readMedia() {
  systemDark.value = !!media?.dark.matches;
  reducedMotion.value = !!media?.motion.matches;
}
const themeWatch = new MutationObserver(() => {
  appTheme.value = document.body.getAttribute('data-theme');
});
onMounted(() => {
  media = { dark: window.matchMedia('(prefers-color-scheme: dark)'), motion: window.matchMedia('(prefers-reduced-motion: reduce)') };
  readMedia();
  appTheme.value = document.body.getAttribute('data-theme');
  themeWatch.observe(document.body, { attributes: true, attributeFilter: ['data-theme'] });
  media.dark.addEventListener('change', readMedia);
  media.motion.addEventListener('change', readMedia);
  if (!reducedMotion.value) play();
});
onBeforeUnmount(() => {
  themeWatch.disconnect();
  media?.dark.removeEventListener('change', readMedia);
  media?.motion.removeEventListener('change', readMedia);
});
// Another language or theme is another file: keep it running if it was.
watch(src, () => {
  if (playing.value) requestAnimationFrame(play);
});

defineExpose({ play });
</script>

<style scoped>
/* The clip is as large as the pane allows, in its own 14:9 shape: the pane is the size container, so a short
   window shrinks the clip instead of pushing what is under it out of view. */
.wc { position: relative; margin: 0; min-height: 0; min-width: 0; container-type: size; display: flex; align-items: center; justify-content: center; }
.wc-media { display: block; width: min(100cqw, 100cqh * 14 / 9); aspect-ratio: 14 / 9; height: auto; border-radius: 8px; border: 1px solid var(--hairline); background: var(--card); }
.wc-toggle { position: absolute; inset-block-end: calc(50% - min(50cqh, 50cqw * 9 / 14) + 8px); inset-inline-end: calc(50% - min(50cqw, 50cqh * 14 / 9) + 8px); opacity: 0; transition: opacity 150ms ease-out; }
.wc:hover .wc-toggle, .wc-toggle:focus { opacity: 1; }
@media (prefers-reduced-motion: reduce) {
  .wc-toggle { opacity: 1; transition: none; }
}
</style>
