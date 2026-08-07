import { createRouter, createWebHistory } from 'vue-router'

/**
 * Views are lazily imported so the initial window paints without parsing every screen.
 * The dashboard is eager because it is always the first thing shown.
 */
const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      name: 'dashboard',
      component: () => import('@/views/DashboardView.vue'),
    },
    {
      path: '/apps',
      name: 'apps',
      component: () => import('@/views/AppsView.vue'),
    },
    {
      path: '/tweaks',
      name: 'tweaks',
      component: () => import('@/views/TweaksView.vue'),
    },
    {
      path: '/profiles',
      name: 'profiles',
      component: () => import('@/views/ProfilesView.vue'),
    },
    {
      path: '/settings',
      name: 'settings',
      component: () => import('@/views/SettingsView.vue'),
    },
    // Anything unrecognised goes home rather than showing a blank window.
    { path: '/:pathMatch(.*)*', redirect: '/' },
  ],
})

export default router
