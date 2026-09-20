<script setup lang="ts">
import { ref } from "vue";
import ActivityPanel from "./ActivityPanel.vue";
import MaintenancePanel from "./MaintenancePanel.vue";
import CheckpointPanel from "./CheckpointPanel.vue";
const tab = ref("activity");
</script>
<template>
  <h1>Operations</h1>
  <nav class="tabs" aria-label="Operations sections">
    <button
      class="quiet"
      :aria-pressed="tab === 'activity'"
      @click="tab = 'activity'"
    >
      Service activity</button
    ><button
      class="quiet"
      :aria-pressed="tab === 'maintenance'"
      @click="tab = 'maintenance'"
    >
      Maintenance</button
    ><button
      class="quiet"
      :aria-pressed="tab === 'checkpoints'"
      @click="tab = 'checkpoints'"
    >
      Checkpoints
    </button>
  </nav>
  <ActivityPanel v-if="tab === 'activity'" /><MaintenancePanel
    v-else-if="tab === 'maintenance'"
    :administrator="true"
  /><CheckpointPanel v-else />
</template>
