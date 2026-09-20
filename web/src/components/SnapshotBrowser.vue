<script setup lang="ts">
import { onMounted, ref } from "vue";
import type {
  RootInspection,
  EntryInspection,
  EntryPage,
  DirectoryCursor,
  FileVersion,
  VersionPage,
  CursorToken,
  DirectoryEntry,
  SnapshotEntryPage,
} from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
const props = defineProps<{ root: RootInspection }>();
const directory = ref<EntryInspection | null>(null);
const ancestors = ref<EntryInspection[]>([]);
const entries = ref<EntryInspection[]>([]);
const nextDirectory = ref<DirectoryCursor | null>(null);
const showHidden = ref(false);
const history = ref<EntryInspection | null>(null);
const versions = ref<FileVersion[]>([]);
const nextVersion = ref<CursorToken | null>(null);
const snapshot = ref<FileVersion | null>(null);
const snapshotName = ref("");
const snapshotPath = ref("/");
const snapshotEntries = ref<DirectoryEntry[]>([]);
const nextSnapshot = ref<CursorToken | null>(null);
const busy = ref(false);
const error = ref("");
const base = `/api/v1/roots/${segment(props.root.id)}`;
function child(path: string, name: string) {
  return `${path === "/" ? "" : path}/${name}`;
}
function download(version: string) {
  return `${base}/versions/${segment(version)}/content`;
}
function snapshotDownload(name: string) {
  return `${base}/snapshots/${segment(snapshot.value!.id)}/content?path=${segment(child(snapshotPath.value, name))}`;
}
async function run(work: () => Promise<void>) {
  busy.value = true;
  error.value = "";
  try {
    await work();
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
async function listing(more = false) {
  if (!directory.value) return;
  const query = new URLSearchParams({
    directory_id: directory.value.id,
    limit: "100",
    include_hidden: String(showHidden.value),
  });
  if (more && nextDirectory.value) {
    query.set("cursor", nextDirectory.value.token);
    query.set("directory_revision", nextDirectory.value.directory_revision);
  }
  const result = await api<EntryPage>(`${base}/entries?${query}`);
  entries.value = more ? [...entries.value, ...result.items] : result.items;
  nextDirectory.value = result.next_cursor;
}
async function openDirectory(entry: EntryInspection, level?: number) {
  await run(async () => {
    snapshot.value = null;
    history.value = null;
    directory.value = entry;
    ancestors.value =
      level === undefined
        ? [...ancestors.value, entry]
        : ancestors.value.slice(0, level + 1);
    entries.value = [];
    nextDirectory.value = null;
    if (entry.current_version_id) {
      const page = await api<VersionPage>(
        `${base}/entries/${segment(entry.id)}/versions`,
      );
      history.value = entry;
      versions.value = page.versions;
      nextVersion.value = page.next_cursor;
      const current = page.versions.find(
        (version) => version.id === entry.current_version_id,
      );
      if (current) {
        snapshot.value = current;
        snapshotName.value = entry.name;
        snapshotEntries.value = [];
        nextSnapshot.value = null;
        await snapshotListing("/");
        return;
      }
    }
    await listing();
  });
}
async function leaveSnapshot() {
  const level = ancestors.value.length - 2;
  if (level >= 0) await openDirectory(ancestors.value[level]!, level);
  else if (directory.value) await openDirectory(directory.value, 0);
}
async function loadHistory(entry: EntryInspection, more = false) {
  await run(async () => {
    history.value = entry;
    const result = await api<VersionPage>(
      `${base}/entries/${segment(entry.id)}/versions${more && nextVersion.value ? `?cursor=${segment(nextVersion.value)}` : ""}`,
    );
    versions.value = more
      ? [...versions.value, ...result.versions]
      : result.versions;
    nextVersion.value = result.next_cursor;
  });
}
async function snapshotListing(path: string, more = false) {
  if (!snapshot.value) return;
  const query = new URLSearchParams({ path, limit: "100" });
  if (more && nextSnapshot.value) query.set("cursor", nextSnapshot.value);
  const result = await api<SnapshotEntryPage>(
    `${base}/snapshots/${segment(snapshot.value.id)}/entries?${query}`,
  );
  snapshotPath.value = path;
  snapshotEntries.value = more
    ? [...snapshotEntries.value, ...result.entries]
    : result.entries;
  nextSnapshot.value = result.next_cursor;
}
async function openSnapshot(version: FileVersion) {
  await run(async () => {
    snapshot.value = version;
    snapshotName.value = history.value?.name ?? "Snapshot";
    snapshotEntries.value = [];
    nextSnapshot.value = null;
    await snapshotListing("/");
  });
}
onMounted(
  () =>
    void run(async () => {
      const root = await api<EntryInspection>(`${base}/resolve?path=%2F`);
      await openDirectory(root);
    }),
);
</script>
<template>
  <section :aria-busy="busy">
    <p v-if="error" class="notice error" role="alert">
      {{ error }}
      <button
        class="quiet"
        :disabled="busy"
        @click="
          run(() => (snapshot ? snapshotListing(snapshotPath) : listing()))
        "
      >
        Refresh
      </button>
    </p>
    <div class="row">
      <nav aria-label="Directory path" class="breadcrumbs">
        <button
          v-for="(entry, index) in ancestors"
          :key="entry.id"
          class="quiet"
          :disabled="busy"
          @click="openDirectory(entry, index)"
        >
          {{ index === 0 ? root.display_name : entry.name }}
        </button>
      </nav>
      <label v-if="!snapshot" class="check"
        ><input
          v-model="showHidden"
          type="checkbox"
          :disabled="busy"
          @change="run(() => listing())"
        />Show hidden entries</label
      >
    </div>
    <template v-if="snapshot">
      <div class="page-heading">
        <div>
          <h2>{{ snapshotName }} · version {{ snapshot.commit_sequence }}</h2>
          <p class="mono">{{ snapshotPath }}</p>
        </div>
        <button class="quiet" :disabled="busy" @click="leaveSnapshot">
          Back to containing directory
        </button>
      </div>
      <button
        v-if="snapshotPath !== '/'"
        class="quiet"
        :disabled="busy"
        @click="
          run(() =>
            snapshotListing(
              snapshotPath.slice(0, snapshotPath.lastIndexOf('/')) || '/',
            ),
          )
        "
      >
        Parent directory
      </button>
      <p v-if="!snapshotEntries.length && !busy" class="empty">
        This snapshot directory is empty.
      </p>
      <div v-else class="table-scroll panel">
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th>Type</th>
              <th>Size</th>
              <th>Action</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="entry in snapshotEntries" :key="entry.id">
              <td>
                <button
                  v-if="entry.kind === 'directory'"
                  class="quiet"
                  :disabled="busy"
                  @click="
                    run(() => snapshotListing(child(snapshotPath, entry.name)))
                  "
                >
                  {{ entry.name }}</button
                ><span v-else>{{ entry.name }}</span>
              </td>
              <td>{{ entry.kind }}</td>
              <td>{{ bytes(entry.length) }}</td>
              <td>
                <a
                  v-if="entry.kind === 'file'"
                  :href="snapshotDownload(entry.name)"
                  :download="entry.name"
                  >Download {{ entry.name }}</a
                >
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <button
        v-if="nextSnapshot"
        class="quiet"
        :disabled="busy"
        @click="run(() => snapshotListing(snapshotPath, true))"
      >
        Load more snapshot entries
      </button>
    </template>
    <template v-else>
      <div v-if="directory?.current_version_id" class="notice">
        <span>This directory has atomic snapshots.</span>
        <button class="quiet" :disabled="busy" @click="loadHistory(directory)">
          Snapshot history
        </button>
      </div>
      <p v-if="!entries.length && !busy" class="empty">
        This directory is empty.
      </p>
      <div v-else class="table-scroll panel">
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th>Type</th>
              <th>Visibility</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="entry in entries" :key="entry.id">
              <td>
                <button
                  v-if="entry.kind === 'directory'"
                  class="quiet"
                  :disabled="busy"
                  @click="openDirectory(entry)"
                >
                  {{ entry.name }}</button
                ><span v-else>{{ entry.name }}</span>
              </td>
              <td>{{ entry.kind }}</td>
              <td>{{ entry.hidden ? "Hidden" : "Visible" }}</td>
              <td>
                <div class="actions">
                  <button
                    v-if="entry.current_version_id"
                    class="quiet"
                    :disabled="busy"
                    @click="loadHistory(entry)"
                  >
                    History of {{ entry.name }}</button
                  ><a
                    v-if="entry.kind === 'file' && entry.current_version_id"
                    :href="download(entry.current_version_id)"
                    :download="entry.name"
                    >Download {{ entry.name }}</a
                  >
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <button
        v-if="nextDirectory"
        class="quiet"
        :disabled="busy"
        @click="run(() => listing(true))"
      >
        Load more entries
      </button>
    </template>
    <p v-if="busy" role="status">Loading saved content…</p>
    <section v-if="history" class="panel" aria-label="Version history">
      <div class="row">
        <h2>History: {{ history.name }}</h2>
        <button class="quiet" @click="history = null">Close history</button>
      </div>
      <p v-if="!versions.length && !busy">
        No retained versions are available.
      </p>
      <div v-else class="table-scroll">
        <table>
          <thead>
            <tr>
              <th>Version</th>
              <th>Size</th>
              <th>Action</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="version in versions" :key="version.id">
              <td>
                {{ version.commit_sequence }}
                <span
                  v-if="version.id === history.current_version_id"
                  class="badge"
                  >Current</span
                >
              </td>
              <td>{{ bytes(version.length) }}</td>
              <td>
                <a
                  v-if="history.kind === 'file'"
                  :href="download(version.id)"
                  :download="history.name"
                  >Download version {{ version.commit_sequence }}</a
                ><button
                  v-else
                  class="quiet"
                  :disabled="busy"
                  @click="openSnapshot(version)"
                >
                  Browse version {{ version.commit_sequence }}
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <button
        v-if="nextVersion"
        class="quiet"
        :disabled="busy"
        @click="loadHistory(history, true)"
      >
        Load more versions
      </button>
    </section>
  </section>
</template>
