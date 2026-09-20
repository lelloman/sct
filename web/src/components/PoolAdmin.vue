<script setup lang="ts">
import { onMounted, ref } from "vue";
import type { PoolInspection, PoolPage, CursorToken } from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
const pools = ref<PoolInspection[]>([]);
const cursor = ref<CursorToken | null>(null);
const selected = ref<PoolInspection | null>(null);
const id = ref("");
const capacity = ref(1000000000);
const staging = ref(100000000);
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
  const page = await api<PoolPage>(
    `/api/v1/admin/pools?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
  );
  pools.value = more ? [...pools.value, ...page.pools] : page.pools;
  cursor.value = page.next_cursor;
}
function edit(pool: PoolInspection | null) {
  selected.value = pool;
  id.value = pool?.id ?? "";
  capacity.value = pool?.allocatable_bytes ?? 1000000000;
}
async function save() {
  await run(async () => {
    const pool = selected.value
      ? await api<PoolInspection>(
          `/api/v1/admin/pools/${segment(selected.value.id)}`,
          "PATCH",
          {
            expected_revision: selected.value.revision,
            capacity_bytes: capacity.value,
          },
        )
      : await api<PoolInspection>("/api/v1/admin/pools", "POST", {
          id: id.value,
          capacity_bytes: capacity.value,
          staging_bytes: staging.value,
        });
    edit(pool);
    await load();
    message.value = "Pool saved.";
  });
}
onMounted(() => void run(() => load()));
</script>
<template>
  <section class="panel" :aria-busy="busy">
    <div class="row">
      <h2>Physical pools</h2>
      <button class="quiet" :disabled="busy" @click="run(() => load())">
        Refresh pools
      </button>
    </div>
    <p>
      A pool is one shared physical capacity budget. Backend instances sharing
      the same storage must use the same pool.
    </p>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="message" class="notice success" role="status">{{ message }}</p>
    <div v-if="pools.length" class="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Pool</th>
            <th>Capacity</th>
            <th>Allocated</th>
            <th>Unallocated</th>
            <th>Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="pool in pools" :key="pool.id">
            <td class="mono">{{ pool.id }}</td>
            <td>{{ bytes(pool.allocatable_bytes) }}</td>
            <td>{{ bytes(pool.allocated_bytes) }}</td>
            <td>{{ bytes(pool.allocatable_bytes - pool.allocated_bytes) }}</td>
            <td>
              <button
                class="quiet"
                :disabled="busy"
                :aria-label="`Edit pool ${pool.id}`"
                @click="edit(pool)"
              >
                Edit
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-else-if="!busy">No pools configured.</p>
    <button
      v-if="cursor"
      class="quiet"
      :disabled="busy"
      @click="run(() => load(true))"
    >
      Load more pools
    </button>
    <form class="stack" @submit.prevent="save">
      <h3>{{ selected ? `Edit pool ${selected.id}` : "Create pool" }}</h3>
      <div class="form-grid">
        <label
          >Pool ID<input
            v-model="id"
            required
            :disabled="busy || !!selected" /></label
        ><label
          >Pool capacity in bytes<input
            v-model.number="capacity"
            type="number"
            min="1"
            max="9007199254740991"
            step="1"
            required /></label
        ><label v-if="!selected"
          >Initial staging allocation in bytes<input
            v-model.number="staging"
            type="number"
            min="0"
            :max="capacity"
            step="1"
            required
        /></label>
      </div>
      <p v-if="selected" class="muted">
        Capacity cannot fall below allocated bytes. Manage staging capacity on
        the Allocations tab.
      </p>
      <div class="actions">
        <button :disabled="busy">Save pool</button
        ><button
          v-if="selected"
          type="button"
          class="quiet"
          :disabled="busy"
          @click="edit(null)"
        >
          New pool
        </button>
      </div>
    </form>
  </section>
</template>
