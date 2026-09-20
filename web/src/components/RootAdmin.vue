<script setup lang="ts">
import { onMounted, ref } from "vue";
import type {
  RootInspection,
  RootPage,
  RootManagement,
  CursorToken,
  Placement,
  BackendId,
} from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
const roots = ref<RootInspection[]>([]);
const cursor = ref<CursorToken | null>(null);
const selected = ref<RootInspection | null>(null);
const management = ref<RootManagement | null>(null);
const owner = ref("");
const name = ref("");
const mount = ref("/");
const quota = ref(1000000);
const enabled = ref(false);
const mirror = ref(false);
const firstBackend = ref("");
const secondBackend = ref("");
const firstPool = ref("");
const secondPool = ref("");
const firstCapacity = ref(1000000);
const secondCapacity = ref(1000000);
const ceiling = ref(1000000);
const minimum = ref(1);
const maximum = ref(100);
const relaxed = ref(false);
const retained = ref(1);
const busy = ref(false);
const error = ref("");
const message = ref("");
let createKey = crypto.randomUUID();
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
  const page = await api<RootPage>(
    `/api/v1/roots?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
  );
  roots.value = more ? [...roots.value, ...page.items] : page.items;
  cursor.value = page.next_cursor;
}
async function edit(root: RootInspection) {
  selected.value = await api<RootInspection>(
    `/api/v1/roots/${segment(root.id)}`,
  );
  management.value = await api<RootManagement>(
    `/api/v1/roots/${segment(root.id)}/management`,
  );
  name.value = selected.value.display_name;
  quota.value = selected.value.quota_bytes;
  enabled.value = selected.value.state === "enabled";
  ceiling.value = management.value.quota_ceiling_bytes;
  minimum.value = management.value.min_versions;
  maximum.value = management.value.max_versions;
  relaxed.value = management.value.allow_relaxed;
}
function newRoot() {
  selected.value = null;
  management.value = null;
  owner.value = "";
  name.value = "";
  mount.value = "/";
  quota.value = 1000000;
  createKey = crypto.randomUUID();
}
async function save() {
  await run(async () => {
    let root: RootInspection;
    if (selected.value)
      root = await api<RootInspection>(
        `/api/v1/roots/${segment(selected.value.id)}`,
        "PATCH",
        {
          expected_revision: selected.value.revision,
          display_name: name.value,
          state: enabled.value ? "enabled" : "disabled",
          placement: selected.value.placement,
          quota_bytes: quota.value,
        },
      );
    else {
      const ids = (
        mirror.value
          ? [firstBackend.value, secondBackend.value]
          : [firstBackend.value]
      ) as BackendId[];
      const read_policy = {
        candidates: ids.map((backend_id) => ({
          backend_id,
          connection_timeout_ms: 5000,
          initial_response_timeout_ms: 10000,
          idle_timeout_ms: 30000,
          throughput_window_ms: 10000,
          minimum_bytes_per_second: 1,
          startup_allowance_ms: 10000,
        })),
        overall_deadline_ms: null,
      };
      const placement: Placement = mirror.value
        ? { kind: "mirror", backend_ids: [ids[0]!, ids[1]!], read_policy }
        : { kind: "single", backend_id: ids[0]!, read_policy };
      const allocations = [
        {
          pool_id: firstPool.value,
          retained_capacity_bytes: firstCapacity.value,
        },
      ];
      if (
        mirror.value &&
        secondPool.value &&
        secondPool.value !== firstPool.value
      )
        allocations.push({
          pool_id: secondPool.value,
          retained_capacity_bytes: secondCapacity.value,
        });
      root = await api<RootInspection>("/api/v1/roots", "POST", {
        idempotency_key: createKey,
        owner_id: owner.value,
        display_name: name.value,
        mount_path: mount.value,
        quota_bytes: quota.value,
        default_policy: { mode: "strict", keep_versions: retained.value },
        placement,
        allocations,
      });
    }
    await edit(root);
    await load();
    message.value = "Root saved.";
  });
}
async function saveCeiling() {
  if (!selected.value || !management.value) return;
  await run(async () => {
    await api<RootManagement>(
      `/api/v1/roots/${segment(selected.value!.id)}/quota-ceiling`,
      "PATCH",
      {
        expected_revision: management.value!.revision,
        quota_ceiling_bytes: ceiling.value,
      },
    );
    await edit(selected.value!);
    await load();
    message.value = "Quota ceiling saved.";
  });
}
async function saveConstraints() {
  if (!selected.value || !management.value) return;
  await run(async () => {
    const root = await api<RootInspection>(
      `/api/v1/admin/roots/${segment(selected.value!.id)}/policy-constraints`,
      "PATCH",
      {
        expected_revision: management.value!.revision,
        min_versions: minimum.value,
        max_versions: maximum.value,
        allow_relaxed: relaxed.value,
      },
    );
    await edit(root);
    message.value = "Policy constraints saved.";
  });
}
onMounted(() => void run(() => load()));
</script>
<template>
  <section class="panel" :aria-busy="busy">
    <div class="row">
      <h2>Service roots</h2>
      <button class="quiet" :disabled="busy" @click="run(() => load())">
        Refresh service roots
      </button>
    </div>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="message" class="notice success" role="status">{{ message }}</p>
    <div v-if="roots.length" class="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Root</th>
            <th>Owner</th>
            <th>State</th>
            <th>Quota</th>
            <th>Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="root in roots" :key="root.id">
            <td>
              <a :href="`#/roots/${segment(root.id)}`">{{
                root.display_name
              }}</a
              ><br /><span class="mono">{{ root.id }}</span>
            </td>
            <td>{{ root.owner_id }}</td>
            <td>{{ root.state }}</td>
            <td>{{ bytes(root.quota_bytes) }}</td>
            <td>
              <button
                class="quiet"
                :disabled="busy"
                :aria-label="`Configure root ${root.display_name}`"
                @click="run(() => edit(root))"
              >
                Configure
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-else-if="!busy">No roots configured.</p>
    <button
      v-if="cursor"
      class="quiet"
      :disabled="busy"
      @click="run(() => load(true))"
    >
      Load more service roots
    </button>
    <form class="stack" @submit.prevent="save">
      <div class="row">
        <h3>
          {{
            selected ? `Configure ${selected.display_name}` : "Provision root"
          }}
        </h3>
        <button
          v-if="selected"
          type="button"
          class="quiet"
          :disabled="busy"
          @click="run(() => edit(selected!))"
        >
          Reload root configuration
        </button>
      </div>
      <div class="form-grid">
        <label
          >Root display name<input
            v-model="name"
            required
            maxlength="255" /></label
        ><label
          >Root quota in bytes<input
            v-model.number="quota"
            type="number"
            min="0"
            max="9007199254740991"
            step="1"
            required /></label
        ><template v-if="!selected"
          ><label>Owner account ID<input v-model="owner" required /></label
          ><label>Account mount path<input v-model="mount" required /></label
          ><label
            >Initial retained versions<input
              v-model.number="retained"
              type="number"
              min="1"
              max="1000000"
              step="1"
              required /></label
        ></template>
      </div>
      <label v-if="selected" class="check"
        ><input
          v-model="enabled"
          type="checkbox"
          :disabled="selected.state === 'recovering'"
        />Enable root</label
      >
      <template v-else
        ><label class="check"
          ><input v-model="mirror" type="checkbox" />Mirror across two backend
          instances</label
        >
        <div class="form-grid">
          <label
            >First backend ID<input v-model="firstBackend" required /></label
          ><label>First pool ID<input v-model="firstPool" required /></label
          ><label
            >First pool retained allocation in bytes<input
              v-model.number="firstCapacity"
              type="number"
              min="1"
              max="9007199254740991"
              step="1"
              required /></label
          ><template v-if="mirror"
            ><label
              >Second backend ID<input
                v-model="secondBackend"
                required /></label
            ><label>Second pool ID<input v-model="secondPool" required /></label
            ><label v-if="secondPool !== firstPool"
              >Second pool retained allocation in bytes<input
                v-model.number="secondCapacity"
                type="number"
                min="1"
                max="9007199254740991"
                step="1"
                required /></label
          ></template>
        </div>
        <p>
          New roots start disabled. Each backend's pool needs an explicit
          allocation. A mirror sharing one pool requires capacity for both
          copies in that pool.
        </p></template
      >
      <p v-if="selected" class="muted">
        Placement: {{ selected.placement.kind }}. Retention and grants are
        managed on the root's page.
      </p>
      <div class="actions">
        <button :disabled="busy || selected?.state === 'recovering'">
          Save root</button
        ><button
          v-if="selected"
          type="button"
          class="quiet"
          :disabled="busy"
          @click="newRoot"
        >
          New root
        </button>
      </div>
    </form>
    <template v-if="selected && management"
      ><form class="stack" @submit.prevent="saveCeiling">
        <h3>Delegated quota ceiling</h3>
        <label
          >Maximum delegated quota in bytes<input
            v-model.number="ceiling"
            type="number"
            :min="management.quota_bytes"
            max="9007199254740991"
            step="1"
            required
        /></label>
        <div><button :disabled="busy">Save quota ceiling</button></div>
      </form>
      <form class="stack" @submit.prevent="saveConstraints">
        <h3>Retention constraints</h3>
        <p>
          These bounds constrain future policy changes. Existing policies are
          unchanged; review a policy change on the root page to alter retained
          versions.
        </p>
        <div class="form-grid">
          <label
            >Minimum retained versions<input
              v-model.number="minimum"
              type="number"
              min="1"
              :max="maximum"
              step="1"
              required /></label
          ><label
            >Maximum retained versions<input
              v-model.number="maximum"
              type="number"
              :min="minimum"
              max="9007199254740991"
              step="1"
              required
          /></label>
        </div>
        <label class="check"
          ><input v-model="relaxed" type="checkbox" />Allow relaxed
          retention</label
        >
        <div><button :disabled="busy">Save retention constraints</button></div>
      </form></template
    >
  </section>
</template>
