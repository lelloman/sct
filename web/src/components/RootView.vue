<script setup lang="ts">
import { onMounted, ref } from "vue";
import type { RootInspection, RootManagement } from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
import SnapshotBrowser from "./SnapshotBrowser.vue";
import RootSettings from "./RootSettings.vue";
import PolicyEditor from "./PolicyEditor.vue";
import GrantEditor from "./GrantEditor.vue";
import ActivityPanel from "./ActivityPanel.vue";
import MaintenancePanel from "./MaintenancePanel.vue";
const tab = ref("snapshots");
const props = defineProps<{ rootId: string; administrator: boolean }>();
const root = ref<RootInspection | null>(null);
const management = ref<RootManagement | null>(null);
const freshness = {
  not_configured: "Overdue monitoring off",
  never_committed: "No successful backup yet",
  on_time: "Backup on time",
  overdue: "Backup overdue",
};
const error = ref("");
const loading = ref(true);
async function load() {
  loading.value = true;
  error.value = "";
  try {
    root.value = await api<RootInspection>(
      `/api/v1/roots/${segment(props.rootId)}`,
    );
    management.value = await api<RootManagement>(
      `/api/v1/roots/${segment(props.rootId)}/management`,
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
    management.value = await api<RootManagement>(
      `/api/v1/roots/${segment(props.rootId)}/management`,
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
    <p
      v-if="management"
      class="notice"
      :class="{ error: management.backup_freshness === 'overdue' }"
    >
      <strong>{{ freshness[management.backup_freshness] }}</strong>
      <span v-if="management.last_commit_at_ms">
        · Last successful commit
        {{
          new Date(Number(management.last_commit_at_ms)).toLocaleString()
        }}</span
      >
      <span v-if="management.next_expected_at_ms">
        · Expected by
        {{
          new Date(Number(management.next_expected_at_ms)).toLocaleString()
        }}</span
      >
    </p>
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
      <button
        v-if="root.capabilities.includes('list_read')"
        class="quiet"
        :aria-pressed="tab === 'activity'"
        @click="tab = 'activity'"
      >
        Activity
      </button>
      <button
        v-if="root.capabilities.includes('list_read')"
        class="quiet"
        :aria-pressed="tab === 'integrity'"
        @click="tab = 'integrity'"
      >
        Integrity
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
    <ActivityPanel
      v-else-if="tab === 'activity' && root.capabilities.includes('list_read')"
      :root-id="root.id"
    />
    <MaintenancePanel
      v-else-if="tab === 'integrity' && root.capabilities.includes('list_read')"
      :root-id="root.id"
      :administrator="administrator"
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
