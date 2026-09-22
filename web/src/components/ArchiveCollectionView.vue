<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { api, bytes, errorMessage, segment } from "../api";
import { archivePage } from "../archivePaging";
import type { ArchiveCollection, ArchiveSet } from "../generated/api";
import ArchivePlanForm from "./ArchivePlanForm.vue";
import ArchivePager from "./ArchivePager.vue";
import ArchiveSetView from "./ArchiveSetView.vue";
const props = defineProps<{ collectionId: string }>();
const base = computed(() => `/api/v1/archives/${segment(props.collectionId)}`);
const collection = ref<ArchiveCollection | null>(null);
const completed = ref<ArchiveSet | null>(null);
const selected = ref<ArchiveSet | null>(null);
const sets = archivePage<ArchiveSet>(() => `${base.value}/sets`, "sets");
const label = ref("");
const capacity = ref(0);
const busy = ref(false);
const error = ref("");
let active = true;
onUnmounted(() => {
  active = false;
});
async function load() {
  busy.value = true;
  error.value = "";
  try {
    const item = await api<ArchiveCollection>(base.value);
    if (!active) return;
    collection.value = item;
    label.value = item.label;
    capacity.value = item.media_capacity_bytes;
    completed.value = null;
    if (item.last_completed_set_id) {
      const last = await api<ArchiveSet>(
        `${base.value}/sets/${segment(item.last_completed_set_id)}`,
      );
      if (active) completed.value = last;
    }
  } catch (failure) {
    if (active) error.value = errorMessage(failure);
  } finally {
    if (active) busy.value = false;
  }
}
async function save() {
  if (!collection.value) return;
  busy.value = true;
  error.value = "";
  try {
    const item = await api<ArchiveCollection>(base.value, "PATCH", {
      expected_revision: collection.value.revision,
      label: label.value,
      media_capacity_bytes: capacity.value,
    });
    if (active) collection.value = item;
  } catch (failure) {
    if (active) error.value = errorMessage(failure);
  } finally {
    if (active) busy.value = false;
  }
}
onMounted(load);
function date(value: number) {
  return new Date(value).toLocaleString();
}
</script>
<template>
  <section>
    <a href="#/archives">All disc collections</a>
    <p v-if="error" role="alert" class="notice error">{{ error }}</p>
    <p v-if="!collection && busy" role="status">Loading collection…</p>
    <template v-if="collection">
      <h1>{{ collection.label }}</h1>
      <p>
        Next disc: <strong>{{ collection.next_disc_number }}</strong> · Capacity
        {{ bytes(collection.media_capacity_bytes) }}
      </p>
      <p class="muted">
        Collection {{ collection.id }} · Owner {{ collection.owner }}
      </p>
      <p v-if="collection.reconciliation_required" class="notice">
        Recovered history: appends are blocked. Create a new collection for
        future exports; preserve these discs.
      </p>
      <p v-if="completed">
        Latest historically completed recovery point:
        <button class="quiet" @click="selected = completed">
          Snapshot {{ completed.sequence }}
        </button>
        · {{ date(completed.verified_complete_at_ms!) }}
      </p>
      <p v-else-if="!busy">
        No historically completed recovery point is recorded.
      </p>
      <p class="muted">
        Historical completion records successful read-back at that time. Check
        physical copies for current damage or loss. Cached ISO availability is
        separate.
      </p>
      <details class="panel">
        <summary>Collection settings</summary>
        <form class="stack" @submit.prevent="save">
          <label
            >Collection label<input v-model="label" maxlength="255" required
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
          <p class="muted">Capacity changes apply to future plans.</p>
          <div class="row">
            <button :disabled="busy">Save collection settings</button
            ><button type="button" class="quiet" :disabled="busy" @click="load">
              Reload settings
            </button>
          </div>
        </form>
      </details>
      <ArchivePlanForm
        :collection="collection"
        @created="
          selected = $event;
          void sets.refresh();
          void load();
        "
      />
      <h2>Recovery-point history</h2>
      <p v-if="sets.error.value" role="alert" class="notice error">
        {{ sets.error.value }}
      </p>
      <p v-if="sets.busy.value" role="status">Loading recovery points…</p>
      <p v-else-if="!sets.items.value.length" class="empty">
        No recovery points on this page.
      </p>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Snapshot</th>
              <th>Created</th>
              <th>Export state</th>
              <th>Historical read-back completion</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="set in sets.items.value" :key="set.id">
              <td>
                <button class="quiet" @click="selected = set">
                  Snapshot {{ set.sequence }}
                </button>
              </td>
              <td>{{ date(set.created_at_ms) }}</td>
              <td>{{ set.state.replaceAll("_", " ") }}</td>
              <td>
                {{
                  set.verified_complete_at_ms
                    ? date(set.verified_complete_at_ms)
                    : "Not completed"
                }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <ArchivePager
        :page="sets.page.value"
        :next="!!sets.next.value"
        :busy="sets.busy.value"
        @back="sets.back"
        @forward="sets.forward"
        @refresh="sets.refresh"
      />
      <ArchiveSetView
        v-if="selected"
        :key="selected.id"
        :collection="collection"
        :snapshot="selected"
        @select="selected = $event"
        @updated="
          selected = $event;
          void sets.refresh();
          void load();
        "
      />
    </template>
  </section>
</template>
