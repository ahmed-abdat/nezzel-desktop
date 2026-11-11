import { createRouter, createWebHistory, type RouteLocation } from 'vue-router';
import HomeView from './views/app/HomeView.vue';
import MediaView from './views/app/MediaView.vue';
import SubtitleView from './views/app/SubtitleView.vue';
import FullLayout from './layouts/FullLayout.vue';
import AppLayout from './layouts/AppLayout.vue';
import InstallView from './views/full/InstallView.vue';
import LocationView from './views/app/LocationView.vue';
import SettingsView from './views/app/SettingsView.vue';
import MediaPreferencesView from './views/app/MediaPreferencesView.vue';
import InputFiltersView from './views/app/InputFiltersView.vue';
import TheDownloadPreferences from './components/media-view/TheDownloadPreferences.vue';
import TheNetworkPreferences from './components/media-view/TheNetworkPreferences.vue';
import TheOutputPreferences from './components/media-view/TheOutputPreferences.vue';
import TheSubtitlePreferences from './components/media-view/TheSubtitlePreferences.vue';
import SettingsGeneralTab from './views/app/settings/SettingsGeneralTab.vue';
import SettingsLoginsTab from './views/app/settings/SettingsLoginsTab.vue';
import SettingsAboutTab from './views/app/settings/SettingsAboutTab.vue';
import SettingsAdvancedTab from './views/app/settings/SettingsAdvancedTab.vue';

const routes = [
  {
    path: '/install',
    name: 'install',
    component: FullLayout,
    children: [
      {
        path: '',
        name: 'install.index',
        component: InstallView,
        meta: { index: 0 },
      },
    ],
  },
  {
    path: '/',
    name: 'app',
    component: AppLayout,
    children: [
      {
        path: '',
        name: 'home',
        component: HomeView,
        meta: { index: 0 },
      },
      {
        path: 'finished',
        name: 'finished',
        redirect: { name: 'home', query: { filter: 'done' } },
      },
      {
        path: 'settings',
        name: 'settings',
        component: SettingsView,
        meta: { index: 1 },
        children: [
          {
            path: '',
            name: 'settings.general',
            component: SettingsGeneralTab,
            meta: { index: 0 },
          },
          {
            path: 'logins',
            name: 'settings.logins',
            component: SettingsLoginsTab,
            meta: { index: 1 },
          },
          { path: 'about', name: 'settings.about', component: SettingsAboutTab, meta: { index: 1 } },
          {
            path: 'advanced',
            name: 'settings.advanced',
            component: SettingsAdvancedTab,
            meta: { index: 1 },
          },
        ],
      },
      {
        path: 'location',
        name: 'location',
        component: LocationView,
        meta: { index: 1 },
      },
      {
        // Saved accounts moved into Settings > Logins; old links land there.
        path: 'authentication',
        name: 'authentication',
        redirect: { name: 'settings.logins' },
      },
      {
        path: 'subtitles',
        name: 'subtitles',
        component: SubtitleView,
        meta: { index: 1 },
      },
      {
        path: 'input-filters',
        name: 'input-filters',
        component: InputFiltersView,
        meta: { index: 1 },
      },
      {
        path: 'group/:groupId',
        name: 'group.metadata',
        component: MediaView,
        props: true,
        meta: { index: 1, requiresGroup: true },
      },
      {
        // The Logs tab is gone: technical details live at the bottom of the one detail view.
        path: 'group/:groupId/logs',
        name: 'group.logs',
        redirect: (to: RouteLocation) => ({ name: 'group.metadata', params: to.params }),
      },
      {
        path: 'preferences/:groupId',
        name: 'preferences',
        component: MediaPreferencesView,
        props: true,
        meta: { index: 1, requiresGroup: true },
        children: [
          {
            path: '',
            name: 'preferences.quality',
            component: TheDownloadPreferences,
            props: true,
            meta: { index: 0 },
          },
          {
            path: 'network',
            name: 'preferences.network',
            component: TheNetworkPreferences,
            props: true,
            meta: { index: 1 },
          },
          {
            path: 'output',
            name: 'preferences.output',
            component: TheOutputPreferences,
            props: true,
            meta: { index: 1 },
          },
          {
            path: 'subtitles',
            name: 'preferences.subtitles',
            component: TheSubtitlePreferences,
            props: true,
            meta: { index: 1 },
          },
        ],
      },
    ],
  },
];

const router = createRouter({
  history: createWebHistory(),
  routes,
});

export default router;
