<script setup lang="ts">
import { onMounted, ref } from "vue";
import type {
  AllocationInspection,
  AllocationManagement,
  AllocationPage,
  CursorToken,
} from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
const allocations = ref<AllocationInspection[]>([]);
const cursor = ref<CursorToken | null>(null);
const selected = ref<AllocationManagement | null>(null);
const pool = ref("");
const root = ref("");
const capacity = ref(1000000);
const busy = ref(false);
const error = ref("");
const message = ref("");
async function run(work: () => Promise<void>) {
  busy.value = true;
  error.value = "";
  message.value = "";
  try {
    await work();
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
async function load(more = false) {
  const page = await api<AllocationPage>(
    `/api/v1/admin/allocations?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
  );
  allocations.value = more ? [...allocations.value, ...page.items] : page.items;
  cursor.value = page.next_cursor;
}
async function edit(allocation: AllocationInspection) {
  await run(async () => {
    selected.value = await api<AllocationManagement>(
      `/api/v1/admin/allocations/${segment(allocation.id)}/management`,
    );
    capacity.value = selected.value.inspection.capacity_bytes;
  });
}
async function save() {
  await run(async () => {
    selected.value = selected.value
      ? await api<AllocationManagement>(
          `/api/v1/admin/allocations/${segment(selected.value.inspection.id)}`,
          "PUT",
          {
            expected_revision: selected.value.revision,
            capacity_bytes: capacity.value,
          },
        )
      : await api<AllocationManagement>("/api/v1/admin/allocations", "POST", {
          pool_id: pool.value,
          root_id: root.value,
          capacity_bytes: capacity.value,
        });
    await load();
    message.value = "Allocation saved.";
  });
}
onMounted(() => void run(() => load()));
</script>
<template>
  <section class="panel" :aria-busy="busy">
    <div class="row">
      <h2>Capacity allocations</h2>
      <button class="quiet" :disabled="busy" @click="run(() => load())">
        Refresh allocations
      </button>
    </div>
    <p>
      Reservations and uncertain cleanup remain charged. An allocation cannot
      shrink below its outstanding obligations.
    </p>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="message" class="notice success" role="status">{{ message }}</p>
    <div v-if="allocations.length" class="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Pool / owner</th>
            <th>Capacity</th>
            <th>Reserved</th>
            <th>Occupied</th>
            <th>Pending cleanup</th>
            <th>Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="allocation in allocations" :key="allocation.id">
            <td>
              <span class="mono">{{ allocation.pool_id }}</span
              ><br /><span class="mono">{{
                allocation.owner.kind === "root"
                  ? allocation.owner.root_id
                  : "Server staging"
              }}</span>
            </td>
            <td>{{ bytes(allocation.capacity_bytes) }}</td>
            <td>{{ bytes(allocation.reserved_bytes) }}</td>
            <td>{{ bytes(allocation.occupied_bytes) }}</td>
            <td>{{ bytes(allocation.pending_cleanup_bytes) }}</td>
            <td>
              <button
                class="quiet"
                :disabled="busy"
                :aria-label="`Edit allocation ${allocation.id}`"
                @click="edit(allocation)"
              >
                Edit
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-else-if="!busy">No allocations.</p>
    <button
      v-if="cursor"
      class="quiet"
      :disabled="busy"
      @click="run(() => load(true))"
    >
      Load more allocations
    </button>
    <form class="stack" @submit.prevent="save">
      <h3>{{ selected ? "Resize allocation" : "Create root allocation" }}</h3>
      <p v-if="selected" class="mono">{{ selected.inspection.id }}</p>
      <div class="form-grid">
        <template v-if="!selected"
          ><label>Allocation pool ID<input v-model="pool" required /></label
          ><label
            >Allocation root ID<input
              v-model="root"
              required /></label></template
        ><label
          >Allocation capacity in bytes<input
            v-model.number="capacity"
            type="number"
            min="0"
            max="9007199254740991"
            step="1"
            required
        /></label>
      </div>
      <div class="actions">
        <button :disabled="busy">Save allocation</button
        ><button
          v-if="selected"
          type="button"
          class="quiet"
          :disabled="busy"
          @click="selected = null"
        >
          New root allocation
        </button>
      </div>
    </form>
  </section>
</template>
