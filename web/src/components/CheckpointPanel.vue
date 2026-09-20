<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import type {
  CheckpointInspection,
  CheckpointPage,
  CheckpointDescriptor,
  CheckpointObject,
  CheckpointInventoryPage,
  CursorToken,
} from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
const checkpoints = ref<CheckpointInspection[]>([]);
const cursor = ref<CursorToken | null>(null);
const selected = ref<CheckpointInspection | null>(null);
const descriptor = ref<CheckpointDescriptor | null>(null);
const objects = ref<CheckpointObject[]>([]);
const inventoryCursor = ref<CursorToken | null>(null);
const inventoryLoaded = ref(false);
const createAcknowledged = ref(false);
const releaseAcknowledged = ref(false);
const busy = ref(false);
const error = ref("");
let createKey = crypto.randomUUID();
let stopped = false;
let timer: number | undefined;
const base = "/api/v1/admin/checkpoints";
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
async function load(more = false) {
  const page = await api<CheckpointPage>(
    `${base}?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
  );
  checkpoints.value = more
    ? [...checkpoints.value, ...page.checkpoints]
    : page.checkpoints;
  cursor.value = page.next_cursor;
}
function schedule() {
  if (timer) window.clearTimeout(timer);
  if (
    !stopped &&
    selected.value &&
    ["fencing", "exporting"].includes(selected.value.state)
  )
    timer = window.setTimeout(() => void run(refresh), 2000);
}
async function refresh() {
  if (!selected.value || stopped) return;
  const id = selected.value.id;
  const checkpoint = await api<CheckpointInspection>(`${base}/${segment(id)}`);
  if (stopped || selected.value?.id !== id) return;
  selected.value = checkpoint;
  if (checkpoint.state === "exported")
    descriptor.value = await api<CheckpointDescriptor>(
      `${base}/${segment(id)}/descriptor`,
    );
  await load();
  schedule();
}
async function inspect(checkpoint: CheckpointInspection) {
  if (timer) window.clearTimeout(timer);
  selected.value = checkpoint;
  descriptor.value = null;
  objects.value = [];
  inventoryCursor.value = null;
  inventoryLoaded.value = false;
  releaseAcknowledged.value = false;
  await run(refresh);
}
async function create() {
  if (!createAcknowledged.value) return;
  await run(async () => {
    selected.value = await api<CheckpointInspection>(base, "POST", {
      idempotency_key: createKey,
    });
    createKey = crypto.randomUUID();
    createAcknowledged.value = false;
    releaseAcknowledged.value = false;
    descriptor.value = null;
    objects.value = [];
    inventoryCursor.value = null;
    inventoryLoaded.value = false;
    schedule();
  });
}
async function inventory(more = false) {
  const page = await api<CheckpointInventoryPage>(
    `${base}/${segment(selected.value!.id)}/inventory?limit=100${more && inventoryCursor.value ? `&cursor=${segment(inventoryCursor.value)}` : ""}`,
  );
  objects.value = more ? [...objects.value, ...page.objects] : page.objects;
  inventoryCursor.value = page.next_cursor;
  inventoryLoaded.value = true;
}
async function release() {
  if (!selected.value || !releaseAcknowledged.value) return;
  await run(async () => {
    selected.value = await api<CheckpointInspection>(
      `${base}/${segment(selected.value!.id)}/abandon`,
      "POST",
      {
        expected_revision: selected.value!.revision,
        acknowledge_release: true,
      },
    );
    releaseAcknowledged.value = false;
    descriptor.value = null;
    schedule();
    await load();
  });
}
function artifact(name: string) {
  return `${base}/${segment(selected.value!.id)}/artifacts/${segment(name)}`;
}
onMounted(() => void run(() => load()));
onUnmounted(() => {
  stopped = true;
  if (timer) window.clearTimeout(timer);
});
</script>
<template>
  <section class="panel" :aria-busy="busy">
    <div class="row">
      <h2>Recovery checkpoints</h2>
      <button class="quiet" :disabled="busy" @click="run(() => load())">
        Refresh checkpoints
      </button>
    </div>
    <p>
      A checkpoint exports a consistent catalog and pins the exact payload
      generations it references. The export alone is not an independent backup.
    </p>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <form class="stack" @submit.prevent="create">
      <p>
        Creation briefly fences catalog writes. Pinned objects remain charged
        until you explicitly release the checkpoint, including after a failed
        export.
      </p>
      <label class="check"
        ><input v-model="createAcknowledged" type="checkbox" />I acknowledge the
        write pause and retained storage pins</label
      >
      <div>
        <button
          :disabled="
            busy ||
            !createAcknowledged ||
            (!!selected && ['fencing', 'exporting'].includes(selected.state))
          "
        >
          Create checkpoint
        </button>
      </div>
    </form>
    <p v-if="!checkpoints.length && !busy">No checkpoints.</p>
    <div v-else class="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Checkpoint</th>
            <th>Created</th>
            <th>State</th>
            <th>Pinned bytes</th>
            <th>Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="checkpoint in checkpoints" :key="checkpoint.id">
            <td class="mono">{{ checkpoint.id }}</td>
            <td>
              {{ new Date(Number(checkpoint.created_at_ms)).toLocaleString() }}
            </td>
            <td>{{ checkpoint.state }}</td>
            <td>{{ bytes(checkpoint.pinned_bytes) }}</td>
            <td>
              <button
                class="quiet"
                :disabled="busy"
                :aria-label="`Inspect checkpoint ${checkpoint.id}`"
                @click="inspect(checkpoint)"
              >
                Inspect
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <button
      v-if="cursor"
      class="quiet"
      :disabled="busy"
      @click="run(() => load(true))"
    >
      Load more checkpoints
    </button>
    <section v-if="selected" class="notice" aria-label="Selected checkpoint">
      <div class="row">
        <h3>Checkpoint · {{ selected.state }}</h3>
        <button class="quiet" :disabled="busy" @click="run(refresh)">
          Refresh selected checkpoint
        </button>
      </div>
      <p class="mono">{{ selected.id }}</p>
      <p>
        Generation watermark: {{ selected.generation_watermark }} · Pinned
        bytes: {{ bytes(selected.pinned_bytes) }}
      </p>
      <p v-if="selected.failure" class="error notice">
        {{ selected.failure }}. Pins remain protected until explicit release.
      </p>
      <template v-if="descriptor"
        ><h3>Catalog-only export</h3>
        <p>
          Download all three artifacts into one package directory. An operator
          must copy the inventory's payloads to independent storage, then run
          <code>sct-server verify-checkpoint PACKAGE PAYLOADS</code>. Recover
          only into an empty isolated catalog and payload destination with
          <code>recover-checkpoint</code>.
        </p>
        <p>
          No payload verification or recovery has been performed by this
          browser.
        </p>
        <div class="actions">
          <a
            v-for="name in [
              'catalog.jsonl',
              'inventory.jsonl',
              'descriptor.json',
            ]"
            :key="name"
            :href="artifact(name)"
            :download="name"
            >Download {{ name }}</a
          >
        </div>
        <details>
          <summary>Artifact checksums</summary>
          <dl>
            <dt>Catalog SHA-256</dt>
            <dd class="mono">{{ descriptor.catalog_digest }}</dd>
            <dt>Inventory SHA-256</dt>
            <dd class="mono">{{ descriptor.inventory_digest }}</dd>
          </dl>
        </details></template
      >
      <button class="quiet" :disabled="busy" @click="run(() => inventory())">
        Inspect pinned inventory
      </button>
      <p v-if="inventoryLoaded && !objects.length">
        No captured objects in this inventory.
      </p>
      <div v-if="objects.length" class="table-scroll">
        <table>
          <thead>
            <tr>
              <th>Root / backend</th>
              <th>Digest / generation</th>
              <th>Bytes</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="object in objects"
              :key="`${object.root_id}/${object.backend_id}/${object.digest}/${object.generation}`"
            >
              <td class="mono">
                {{ object.root_id }}<br />{{ object.backend_id }}
              </td>
              <td class="mono">
                {{ object.digest }}<br />{{ object.generation }}
              </td>
              <td>{{ bytes(object.bytes) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <button
        v-if="inventoryCursor"
        class="quiet"
        :disabled="busy"
        @click="run(() => inventory(true))"
      >
        Load more inventory
      </button>
      <form
        v-if="selected.state !== 'released'"
        class="stack"
        @submit.prevent="release"
      >
        <h3>Release checkpoint pins</h3>
        <p>
          Releasing permits cleanup of these generations and ends export access.
          Finish and independently verify your payload copy first if you intend
          to retain this recovery package.
        </p>
        <label class="check"
          ><input v-model="releaseAcknowledged" type="checkbox" />I acknowledge
          that releasing pins can make this checkpoint unrecoverable</label
        >
        <div>
          <button class="danger" :disabled="busy || !releaseAcknowledged">
            Release checkpoint
          </button>
        </div>
      </form>
    </section>
  </section>
</template>
