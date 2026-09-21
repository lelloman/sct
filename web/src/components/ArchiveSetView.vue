<script setup lang="ts">
import { computed, ref } from "vue";
import { bytes, segment } from "../api";
import { archivePage } from "../archivePaging";
import type {
  ArchiveCollection,
  ArchiveEntry,
  ArchiveLogicalDisc,
  ArchiveSet,
} from "../generated/api";
import ArchivePager from "./ArchivePager.vue";
import ArchiveDiscView from "./ArchiveDiscView.vue";
const props = defineProps<{
  collection: ArchiveCollection;
  snapshot: ArchiveSet;
}>();
defineEmits<{ select: [snapshot: ArchiveSet] }>();
const base = computed(
  () =>
    `/api/v1/archives/${segment(props.collection.id)}/sets/${segment(props.snapshot.id)}`,
);
const discs = archivePage<ArchiveLogicalDisc>(
  () => `${base.value}/discs`,
  "discs",
);
const entries = archivePage<ArchiveEntry>(
  () => `${base.value}/entries`,
  "entries",
);
const search = ref("");
const disc = ref<ArchiveLogicalDisc | null>(null);
const filtered = computed(() =>
  discs.items.value.filter((item) =>
    `${item.number} ${item.id}`
      .toLowerCase()
      .includes(search.value.toLowerCase()),
  ),
);
</script>
<template>
  <section class="panel" aria-label="Snapshot details">
    <h2>Snapshot {{ snapshot.sequence }}</h2>
    <p>
      Export state: {{ snapshot.state.replaceAll("_", " ") }}
      <span v-if="snapshot.failure"
        >· {{ snapshot.failure.replaceAll("_", " ") }}</span
      >
    </p>
    <h3>Captured sources</h3>
    <ul>
      <li
        v-for="source in snapshot.selections"
        :key="`${source.root_id}:${source.version_id}:${source.path}`"
      >
        <span class="mono">{{ source.path }}</span> · Root
        {{ source.root_id }} · Version {{ source.version_id }}
      </li>
    </ul>
    <p class="muted">
      This is archived metadata. It does not claim the source version is still
      available online.
    </p>
    <h3>Required discs</h3>
    <p>
      Includes new discs and reused payloads on earlier discs. Earlier recovery
      points remain part of the collection even when newer snapshots do not need
      their discs.
    </p>
    <label
      >Find a disc on this page<input
        v-model="search"
        type="search"
        placeholder="Disc number or ID"
    /></label>
    <p v-if="discs.error.value" role="alert" class="notice error">
      {{ discs.error.value }}
    </p>
    <p v-if="discs.busy.value" role="status">Loading required discs…</p>
    <p v-else-if="!filtered.length" class="empty">
      No matching discs on this page.
    </p>
    <div class="root-grid">
      <article v-for="item in filtered" :key="item.id" class="panel">
        <h4>
          <button class="quiet" @click="disc = item">
            Disc {{ item.number }}
          </button>
        </h4>
        <p>
          {{
            item.set_id === snapshot.id
              ? "New in this snapshot"
              : "Earlier disc required by this snapshot"
          }}
        </p>
        <p>
          {{ item.object_count }} payload objects ·
          {{ bytes(item.payload_bytes) }}
        </p>
        <p>Cached ISO: {{ item.artifact?.state ?? "Not created" }}</p>
      </article>
    </div>
    <ArchivePager
      :page="discs.page.value"
      :next="!!discs.next.value"
      :busy="discs.busy.value"
      @back="discs.back"
      @forward="discs.forward"
      @refresh="discs.refresh"
    />
    <ArchiveDiscView
      v-if="disc"
      :key="disc.id"
      :collection-id="collection.id"
      :disc="disc"
      @select="$emit('select', $event)"
    />
    <details>
      <summary>Archived file and directory index</summary>
      <p v-if="entries.error.value" role="alert" class="notice error">
        {{ entries.error.value }}
      </p>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Archived path</th>
              <th>Kind</th>
              <th>Size</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="entry in entries.items.value" :key="entry.path">
              <td class="mono">{{ entry.path }}</td>
              <td>{{ entry.kind }}</td>
              <td>{{ bytes(entry.length) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <ArchivePager
        :page="entries.page.value"
        :next="!!entries.next.value"
        :busy="entries.busy.value"
        @back="entries.back"
        @forward="entries.forward"
        @refresh="entries.refresh"
      />
    </details>
  </section>
</template>
