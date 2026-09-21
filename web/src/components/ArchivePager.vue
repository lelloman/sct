<script setup lang="ts">
defineProps<{ page: number; next: boolean; busy: boolean }>();
defineEmits<{ back: []; forward: []; refresh: [] }>();
</script>
<template>
  <div class="row" aria-label="Pagination">
    <button class="quiet" :disabled="busy || page === 1" @click="$emit('back')">
      Previous page
    </button>
    <span>Page {{ page }}</span>
    <button class="quiet" :disabled="busy || !next" @click="$emit('forward')">
      Next page
    </button>
    <button class="quiet" :disabled="busy" @click="$emit('refresh')">
      Refresh list
    </button>
  </div>
</template>
