<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import NotBuiltYet from '@/components/NotBuiltYet.vue'
import PageHeader from '@/components/PageHeader.vue'

const { t } = useI18n()
</script>

<template>
  <div class="p-6 lg:p-8">
    <PageHeader :title="t('nav.profiles')" />
    <NotBuiltYet icon="profiles" :description="t('placeholder.profiles')" />
  </div>
</template>
