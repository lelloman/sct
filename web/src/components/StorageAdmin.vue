<script setup lang="ts">
import { ref } from "vue";
import PoolAdmin from "./PoolAdmin.vue";
import BackendAdmin from "./BackendAdmin.vue";
import AllocationAdmin from "./AllocationAdmin.vue";
import RootAdmin from "./RootAdmin.vue";
const tab = ref("roots");
</script>
<template>
  <h1>Storage administration</h1>
  <p class="muted">Provision storage and control service-wide capacity.</p>
  <nav class="tabs" aria-label="Storage administration sections">
    <button
      v-for="item in ['roots', 'pools', 'backends', 'allocations']"
      :key="item"
      class="quiet"
      :aria-pressed="tab === item"
      @click="tab = item"
    >
      {{ item.charAt(0).toUpperCase() + item.slice(1) }}
    </button>
  </nav>
  <RootAdmin v-if="tab === 'roots'" /><PoolAdmin
    v-else-if="tab === 'pools'"
  /><BackendAdmin v-else-if="tab === 'backends'" /><AllocationAdmin v-else />
</template>
