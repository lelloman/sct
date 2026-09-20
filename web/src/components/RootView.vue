<script setup lang="ts">
import { onMounted, ref } from "vue";
import type { RootInspection } from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
import SnapshotBrowser from "./SnapshotBrowser.vue";
import RootSettings from "./RootSettings.vue";
import PolicyEditor from "./PolicyEditor.vue";
import GrantEditor from "./GrantEditor.vue";
const tab = ref("snapshots");
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
async function refreshRoot() {
  try {
    root.value = await api<RootInspection>(
      `/api/v1/roots/${segment(props.rootId)}`,
    );
  } catch (failure) {
    error.value = errorMessage(failure);
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
    <nav class="tabs" aria-label="Root sections">
      <button
        class="quiet"
        :aria-pressed="tab === 'snapshots'"
        @click="tab = 'snapshots'"
      >
        Snapshots
      </button>
      <button
        v-if="root.capabilities.includes('manage_root')"
        class="quiet"
        :aria-pressed="tab === 'settings'"
        @click="tab = 'settings'"
      >
        Settings
      </button>
      <button
        v-if="root.capabilities.includes('manage_policy')"
        class="quiet"
        :aria-pressed="tab === 'policy'"
        @click="tab = 'policy'"
      >
        Retention
      </button>
      <button
        v-if="root.capabilities.includes('manage_grants')"
        class="quiet"
        :aria-pressed="tab === 'grants'"
        @click="tab = 'grants'"
      >
        Grants
      </button>
    </nav>
    <RootSettings
      v-if="tab === 'settings' && root.capabilities.includes('manage_root')"
      :root="root"
      @updated="refreshRoot"
    />
    <PolicyEditor
      v-else-if="
        tab === 'policy' && root.capabilities.includes('manage_policy')
      "
      :root="root"
    />
    <GrantEditor
      v-else-if="
        tab === 'grants' && root.capabilities.includes('manage_grants')
      "
      :root="root"
    />
    <SnapshotBrowser
      v-else-if="tab === 'snapshots' && root.capabilities.includes('list_read')"
      :root="root"
    />
    <p v-else class="notice">
      Your assigned permissions do not include browsing this root's snapshots.
    </p>
  </template>
</template>
