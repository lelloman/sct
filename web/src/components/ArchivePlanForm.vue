<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { api, bytes, errorMessage, segment } from "../api";
import { archivePage } from "../archivePaging";
import type {
  ArchiveCollection,
  ArchiveSelection,
  ArchiveServiceStatus,
  ArchiveSet,
  EntryInspection,
  FileVersion,
  RootInspection,
  VersionPage,
} from "../generated/api";
import ArchivePager from "./ArchivePager.vue";
const props = defineProps<{ collection: ArchiveCollection }>();
const emit = defineEmits<{ created: [snapshot: ArchiveSet] }>();
const roots = archivePage<RootInspection>(() => "/api/v1/roots", "items");
const status = ref<ArchiveServiceStatus | null>(null);
const root = ref("");
const path = ref("/");
const destination = ref("/");
const versions = ref<FileVersion[]>([]);
const version = ref("");
const versionCursor = ref<string | null>(null);
const entry = ref<EntryInspection | null>(null);
const selections = ref<ArchiveSelection[]>([]);
const busy = ref(false);
const error = ref("");
const enabled = computed(
  () => status.value?.configured && !props.collection.reconciliation_required,
);
watch(root, () => {
  versions.value = [];
  version.value = "";
  entry.value = null;
});
onMounted(async () => {
  try {
    status.value = await api<ArchiveServiceStatus>("/api/v1/archives/status");
  } catch (failure) {
    error.value = errorMessage(failure);
  }
});
async function lookup(more = false) {
  busy.value = true;
  error.value = "";
  try {
    const base = `/api/v1/roots/${segment(root.value)}`;
    if (!more)
      entry.value = await api<EntryInspection>(
        `${base}/resolve?path=${segment(path.value)}`,
      );
    if (!entry.value) return;
    const result = await api<VersionPage>(
      `${base}/entries/${segment(entry.value.id)}/versions?limit=20${more && versionCursor.value ? `&cursor=${segment(versionCursor.value)}` : ""}`,
    );
    versions.value = result.versions;
    versionCursor.value = result.next_cursor;
    version.value = "";
    if (!more) destination.value = path.value;
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
function add() {
  const selected = versions.value.find((v) => v.id === version.value);
  const selectedRoot = roots.items.value.find((r) => r.id === root.value);
  if (!selected || !selectedRoot) return;
  selections.value.push({
    root_id: selectedRoot.id,
    version_id: selected.id,
    path: destination.value as ArchiveSelection["path"],
  });
  versions.value = [];
  version.value = "";
}
async function plan() {
  busy.value = true;
  error.value = "";
  try {
    const current = await api<ArchiveCollection>(
      `/api/v1/archives/${segment(props.collection.id)}`,
    );
    const result = await api<ArchiveSet>(
      `/api/v1/archives/${segment(current.id)}/sets`,
      "POST",
      {
        request_id: crypto.randomUUID(),
        expected_revision: current.revision,
        selections: selections.value,
        media_capacity_bytes: current.media_capacity_bytes,
      },
    );
    selections.value = [];
    emit("created", result);
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
</script>
<template>
  <details class="panel">
    <summary>Plan the next disc set</summary>
    <p v-if="status && !status.configured" class="notice">
      ISO export is not configured. An administrator must configure export
      storage and the recovery bundle.
    </p>
    <p v-if="status?.configured" class="muted">
      Recovery software: {{ status.bundle_version }} · {{ status.platform }}
    </p>
    <p v-if="error || roots.error.value" role="alert" class="notice error">
      {{ error || roots.error.value }}
    </p>
    <fieldset :disabled="busy || !enabled">
      <legend>Choose saved versions</legend>
      <label for="archive-source-root">Source root</label>
      <select id="archive-source-root" v-model="root">
        <option value="">Select a root</option>
        <option
          v-for="item in roots.items.value"
          :key="item.id"
          :value="item.id"
        >
          {{ item.display_name }} ({{ item.mount_path }})
        </option>
      </select>
      <ArchivePager
        :page="roots.page.value"
        :next="!!roots.next.value"
        :busy="roots.busy.value"
        @back="roots.back"
        @forward="roots.forward"
        @refresh="roots.refresh"
      />
      <form class="stack" @submit.prevent="lookup()">
        <label for="archive-source-path">Source path</label>
        <input
          id="archive-source-path"
          v-model="path"
          required
          placeholder="/backups"
        />
        <button :disabled="!root">Find saved versions</button>
      </form>
      <form v-if="versions.length" class="stack" @submit.prevent="add">
        <label for="archive-source-version">Saved version</label>
        <select id="archive-source-version" v-model="version" required>
          <option value="">Choose a version</option>
          <option v-for="item in versions" :key="item.id" :value="item.id">
            {{ item.id }} · {{ bytes(item.length) }}
          </option>
        </select>
        <label for="archive-destination-path"
          >Path in the archived snapshot</label
        >
        <input
          id="archive-destination-path"
          v-model="destination"
          required
          placeholder="/backups"
        />
        <button>Add source to plan</button>
      </form>
      <button v-if="versionCursor" class="quiet" @click="lookup(true)">
        Next saved-version page
      </button>
      <p v-if="entry && !versions.length && !versionCursor">
        No saved versions on this page.
      </p>
      <ul>
        <li v-for="(source, index) in selections" :key="index">
          {{ source.path }} · {{ source.root_id }} · {{ source.version_id }}
          <button class="quiet" @click="selections.splice(index, 1)">
            Remove source {{ index + 1 }}
          </button>
        </li>
      </ul>
      <p>
        Capacity {{ bytes(collection.media_capacity_bytes) }} per disc. Preview
        reserves source data while you review new payloads, reused discs, and
        the recovery software repeated on every disc.
      </p>
      <button :disabled="!selections.length" @click="plan">
        Create preview
      </button>
    </fieldset>
  </details>
</template>
