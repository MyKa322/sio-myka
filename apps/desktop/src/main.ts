import { createApp } from 'vue'
import { createPinia } from 'pinia'
import { VueQueryPlugin } from '@tanstack/vue-query'

import App from './App.vue'
import router from './router'
import { i18n } from './i18n'
import './styles.css'

createApp(App)
  .use(createPinia())
  .use(router)
  .use(i18n)
  .use(VueQueryPlugin, {
    queryClientConfig: {
      defaultOptions: {
        queries: {
          // System state changes slowly and the window is usually left open; refetching
          // on every focus would spawn PowerShell repeatedly for the activation check.
          refetchOnWindowFocus: false,
          staleTime: 30_000,
          retry: 1,
        },
      },
    },
  })
  .mount('#app')
