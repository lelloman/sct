<script setup lang="ts">
import { onMounted, ref } from "vue";
import type { RootInspection } from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
import SnapshotBrowser from "./SnapshotBrowser.vue";
const props = defineProps<{ rootId: string }>();
const root = ref<RootInspection | null>(null);
const error = ref("");
const loading = ref(true);
async function load() {
  loading.value = true;
  error.value = "";
  try {
    root.value = await api<RootInspection>(
      `/api/v1/roots/${segment(props.rootId)}`,
    );
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    loading.value = false;
  }
}
onMounted(() => void load());
</script>
<template>
  <a href="#/roots">← All storage roots</a>
  <p v-if="error" class="notice error" role="alert">
    {{ error }} <button class="quiet" @click="load">Retry</button>
  </p>
  <p v-if="loading" role="status">Loading storage root…</p>
  <template v-else-if="root">
    <div class="page-heading">
      <div>
        <p class="eyebrow">Saved content</p>
        <h1>{{ root.display_name }}</h1>
        <p class="muted">
          {{ root.mount_path }} · Quota {{ bytes(root.quota_bytes) }} ·
          {{ root.state }}
        </p>
      </div>
    </div>
    <SnapshotBrowser
      v-if="root.capabilities.includes('list_read')"
      :root="root"
    />
    <p v-else class="notice">
      Your assigned permissions do not include browsing this root's snapshots.
    </p>
  </template>
</template>
