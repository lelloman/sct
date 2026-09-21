<script setup lang="ts">
import { ref } from "vue";
import { api, bytes, errorMessage, segment } from "../api";
import { archivePage } from "../archivePaging";
import type { ArchiveCollection } from "../generated/api";
import ArchivePager from "./ArchivePager.vue";
import ArchiveCollectionView from "./ArchiveCollectionView.vue";
defineProps<{ collectionId: string | null }>();
const collections = archivePage<ArchiveCollection>(
  () => "/api/v1/archives",
  "collections",
);
const label = ref("");
const capacity = ref(25_000_000_000);
const busy = ref(false);
const error = ref("");
async function create() {
  busy.value = true;
  error.value = "";
  try {
    const collection = await api<ArchiveCollection>(
      "/api/v1/archives",
      "POST",
      {
        request_id: crypto.randomUUID(),
        label: label.value,
        media_capacity_bytes: capacity.value,
      },
    );
    label.value = "";
    await collections.refresh();
    location.hash = `/archives/${segment(collection.id)}`;
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
</script>
<template>
  <ArchiveCollectionView
    v-if="collectionId"
    :key="collectionId"
    :collection-id="collectionId"
  />
  <section v-else>
    <h1>Disc archives</h1>
    <p>
      Permanent collections of recovery discs. Each disc carries its snapshot
      index and recovery software.
    </p>
    <p
      v-if="error || collections.error.value"
      role="alert"
      class="notice error"
    >
      {{ error || collections.error.value }}
    </p>
    <details class="panel">
      <summary>Create a collection</summary>
      <form class="stack" @submit.prevent="create">
        <label
          >Collection label<input v-model="label" required maxlength="255"
        /></label>
        <label
          >Usable disc capacity (bytes)<input
            v-model.number="capacity"
            type="number"
            min="1"
            max="9007199254740991"
            step="1"
            required
        /></label>
        <p class="muted">
          Use the capacity reported for your media. Every disc reserves space
          for its complete index and recovery software.
        </p>
        <button :disabled="busy">Create collection</button>
      </form>
    </details>
    <p v-if="collections.busy.value" role="status">Loading collections…</p>
    <p v-else-if="!collections.items.value.length" class="empty">
      No disc collections on this page.
    </p>
    <div class="root-grid">
      <article
        v-for="collection in collections.items.value"
        :key="collection.id"
        class="panel"
      >
        <h2>
          <a :href="`#/archives/${segment(collection.id)}`">{{
            collection.label
          }}</a>
        </h2>
        <p>
          Next disc: {{ collection.next_disc_number }} ·
          {{ bytes(collection.media_capacity_bytes) }} per disc
        </p>
        <p class="muted">Owner: {{ collection.owner }}</p>
        <p v-if="collection.reconciliation_required" class="notice">
          Recovered history: new appends are blocked. Create a new collection
          for future discs.
        </p>
      </article>
    </div>
    <ArchivePager
      :page="collections.page.value"
      :next="!!collections.next.value"
      :busy="collections.busy.value"
      @back="collections.back"
      @forward="collections.forward"
      @refresh="collections.refresh"
    />
  </section>
</template>
