import { createRouter, createWebHistory, type RouteLocation } from 'vue-router';
import HomeView from './views/app/HomeView.vue';
import FullLayout from './layouts/FullLayout.vue';
import AppLayout from './layouts/AppLayout.vue';

const routes = [
  {
    path: '/install',
    name: 'install',
    component: FullLayout,
    children: [
      {
        path: '',
        name: 'install.index',
        component: () => import('./views/full/InstallView.vue'),
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
        component: () => import('./views/app/SettingsView.vue'),
        meta: { index: 1 },
        children: [
          {
            path: '',
            name: 'settings.general',
            component: () => import('./views/app/settings/SettingsGeneralTab.vue'),
            meta: { index: 0 },
          },
          {
            path: 'logins',
            name: 'settings.logins',
            component: () => import('./views/app/settings/SettingsLoginsTab.vue'),
            meta: { index: 1 },
          },
          { path: 'about', name: 'settings.about', component: () => import('./views/app/settings/SettingsAboutTab.vue'), meta: { index: 1 } },
          {
            path: 'advanced',
            name: 'settings.advanced',
            component: () => import('./views/app/settings/SettingsAdvancedTab.vue'),
            meta: { index: 1 },
          },
        ],
      },
      {
        path: 'location',
        name: 'location',
        component: () => import('./views/app/LocationView.vue'),
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
        component: () => import('./views/app/SubtitleView.vue'),
        meta: { index: 1 },
      },
      {
        path: 'input-filters',
        name: 'input-filters',
        component: () => import('./views/app/InputFiltersView.vue'),
        meta: { index: 1 },
      },
      {
        path: 'group/:groupId',
        name: 'group.metadata',
        component: () => import('./views/app/MediaView.vue'),
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
        component: () => import('./views/app/MediaPreferencesView.vue'),
        props: true,
        meta: { index: 1, requiresGroup: true },
        children: [
          {
            path: '',
            name: 'preferences.quality',
            component: () => import('./components/media-view/TheDownloadPreferences.vue'),
            props: true,
            meta: { index: 0 },
          },
          {
            path: 'network',
            name: 'preferences.network',
            component: () => import('./components/media-view/TheNetworkPreferences.vue'),
            props: true,
            meta: { index: 1 },
          },
          {
            path: 'output',
            name: 'preferences.output',
            component: () => import('./components/media-view/TheOutputPreferences.vue'),
            props: true,
            meta: { index: 1 },
          },
          {
            path: 'subtitles',
            name: 'preferences.subtitles',
            component: () => import('./components/media-view/TheSubtitlePreferences.vue'),
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
