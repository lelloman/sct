<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import type {
  BackendConfiguration,
  AdapterDocumentation,
  AvailabilityHint,
  BackendPage,
  CursorToken,
} from "../generated/api";
import { api, errorMessage, segment } from "../api";
const backends = ref<BackendConfiguration[]>([]);
const cursor = ref<CursorToken | null>(null);
const selected = ref<BackendConfiguration | null>(null);
const id = ref("");
const name = ref("");
const kind = ref("local");
const pool = ref("");
const directory = ref("");
const headroom = ref(0);
const endpoint = ref("");
const bucket = ref("");
const region = ref("us-east-1");
const accessEnv = ref("");
const secretEnv = ref("");
const sessionEnv = ref("");
const documentation = ref<AdapterDocumentation | null>(null);
const documentationError = ref("");
const expectedResponse = ref<number | "">("");
const expectedRate = ref<number | "">("");
const expectedAvailability = ref<AvailabilityHint | "">("");
let documentationGeneration = 0;
async function loadDocumentation() {
  const generation = ++documentationGeneration;
  documentation.value = null;
  documentationError.value = "";
  try {
    const value = await api<AdapterDocumentation>(
      `/api/v1/admin/adapters/${segment(kind.value)}`,
    );
    if (generation === documentationGeneration) documentation.value = value;
  } catch (failure) {
    if (generation === documentationGeneration)
      documentationError.value = errorMessage(failure);
  }
}
watch(kind, () => void loadDocumentation());
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
  const page = await api<BackendPage>(
    `/api/v1/admin/backends?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
  );
  backends.value = more ? [...backends.value, ...page.backends] : page.backends;
  cursor.value = page.next_cursor;
}
function edit(backend: BackendConfiguration | null) {
  selected.value = backend;
  id.value = backend?.inspection.id ?? "";
  name.value = backend?.inspection.display_name ?? "";
  kind.value = backend?.inspection.adapter_kind ?? "local";
  pool.value = backend?.pool_id ?? "";
  expectedResponse.value =
    backend?.inspection.hints.expected_initial_response_ms ?? "";
  expectedRate.value =
    backend?.inspection.hints.expected_bytes_per_second ?? "";
  expectedAvailability.value =
    backend?.inspection.hints.expected_availability ?? "";
  const settings = (backend?.settings ?? {}) as Record<string, string>;
  directory.value = settings.directory ?? "";
  headroom.value = Number(settings.headroom_bytes ?? 0);
  endpoint.value = settings.endpoint ?? "";
  bucket.value = settings.bucket ?? "";
  region.value = settings.region ?? "us-east-1";
  accessEnv.value = settings.access_key_env ?? "";
  secretEnv.value = settings.secret_key_env ?? "";
  sessionEnv.value = settings.session_token_env ?? "";
}
async function save() {
  await run(async () => {
    const settings =
      kind.value === "local"
        ? { directory: directory.value, headroom_bytes: headroom.value }
        : {
            endpoint: endpoint.value,
            bucket: bucket.value,
            region: region.value,
            access_key_env: accessEnv.value,
            secret_key_env: secretEnv.value,
            session_token_env: sessionEnv.value || null,
          };
    const backend = await api<BackendConfiguration>(
      `/api/v1/admin/backends/${segment(id.value)}`,
      "PUT",
      {
        expected_revision: selected.value?.revision ?? null,
        pool_id: pool.value,
        settings,
        inspection: {
          id: id.value,
          display_name: name.value,
          adapter_kind: kind.value,
          hints: {
            expected_initial_response_ms:
              expectedResponse.value === "" ? null : expectedResponse.value,
            expected_bytes_per_second:
              expectedRate.value === "" ? null : expectedRate.value,
            expected_availability: expectedAvailability.value || null,
          },
          observed_health:
            selected.value?.inspection.observed_health ?? "unknown",
          last_observed_at_ms:
            selected.value?.inspection.last_observed_at_ms ?? null,
        },
      },
    );
    edit(backend);
    await load();
    message.value = "Backend saved.";
  });
}
onMounted(() => {
  void run(() => load());
  void loadDocumentation();
});
</script>
<template>
  <section class="panel" :aria-busy="busy">
    <div class="row">
      <h2>Backend instances</h2>
      <button class="quiet" :disabled="busy" @click="run(() => load())">
        Refresh backends
      </button>
    </div>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="message" class="notice success" role="status">{{ message }}</p>
    <div v-if="backends.length" class="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Backend</th>
            <th>Adapter</th>
            <th>Pool</th>
            <th>Configured expectations</th>
            <th>Observed health</th>
            <th>Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="backend in backends" :key="backend.inspection.id">
            <td>
              {{ backend.inspection.display_name }}<br /><span class="mono">{{
                backend.inspection.id
              }}</span>
            </td>
            <td>{{ backend.inspection.adapter_kind }}</td>
            <td class="mono">{{ backend.pool_id }}</td>
            <td>
              Initial response:
              {{
                backend.inspection.hints.expected_initial_response_ms ??
                "Unspecified"
              }}
              ms<br />Throughput:
              {{
                backend.inspection.hints.expected_bytes_per_second ??
                "Unspecified"
              }}
              B/s<br />Availability:
              {{
                backend.inspection.hints.expected_availability ?? "Unspecified"
              }}
            </td>
            <td>
              {{ backend.inspection.observed_health
              }}<span v-if="backend.inspection.last_observed_at_ms"
                ><br />{{
                  new Date(
                    Number(backend.inspection.last_observed_at_ms),
                  ).toLocaleString()
                }}</span
              >
            </td>
            <td>
              <button
                class="quiet"
                :disabled="busy"
                :aria-label="`Edit backend ${backend.inspection.id}`"
                @click="edit(backend)"
              >
                Edit
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-else-if="!busy">No backends.</p>
    <button
      v-if="cursor"
      class="quiet"
      :disabled="busy"
      @click="run(() => load(true))"
    >
      Load more backends
    </button>
    <form class="stack" @submit.prevent="save">
      <h3>{{ selected ? "Edit backend" : "Configure backend" }}</h3>
      <div class="form-grid">
        <label
          >Backend ID<input
            v-model="id"
            required
            :disabled="busy || !!selected" /></label
        ><label>Backend display name<input v-model="name" required /></label
        ><label
          >Adapter<select v-model="kind" :disabled="busy || !!selected">
            <option value="local">Local filesystem</option>
            <option value="s3">S3 compatible</option>
            <option v-if="!['local', 's3'].includes(kind)" :value="kind">
              {{ kind }}
            </option>
          </select></label
        ><label
          >Backend pool ID<input
            v-model="pool"
            required
            :disabled="busy || !!selected"
        /></label>
      </div>
      <p>
        Existing destination identity, adapter and pool are fixed. Configure a
        new instance for another destination.
      </p>
      <label v-if="kind === 'local'"
        >Local storage directory<input
          v-model="directory"
          required
          :disabled="busy || !!selected"
      /></label>
      <template v-else-if="kind === 's3'"
        ><div class="form-grid">
          <label
            >S3 endpoint<input
              v-model="endpoint"
              type="url"
              required
              :disabled="busy || !!selected" /></label
          ><label
            >Bucket<input
              v-model="bucket"
              required
              :disabled="busy || !!selected" /></label
          ><label
            >Region<input
              v-model="region"
              required
              :disabled="busy || !!selected"
          /></label>
        </div>
        <p>
          Enter server environment variable names, not credential values. Their
          values stay on the server.
        </p>
        <div class="form-grid">
          <label
            >Access key environment variable<input
              v-model="accessEnv"
              required
              autocomplete="off" /></label
          ><label
            >Secret key environment variable<input
              v-model="secretEnv"
              required
              autocomplete="off" /></label
          ><label
            >Session token environment variable (optional)<input
              v-model="sessionEnv"
              autocomplete="off"
          /></label></div
      ></template>
      <p v-else class="notice">
        This adapter is inspectable here; use its API documentation for
        configuration.
      </p>
      <label v-if="kind === 'local'"
        >Local free-space headroom in bytes<input
          v-model.number="headroom"
          type="number"
          min="0"
          max="9007199254740991"
          step="1"
      /></label>
      <fieldset class="stack">
        <legend>Configured expectations</legend>
        <p class="muted">
          Planning hints, separate from observed health and read timeout policy.
        </p>
        <div class="form-grid">
          <label
            >Expected initial response in milliseconds<input
              v-model.number="expectedResponse"
              type="number"
              min="1"
              max="9007199254740991"
              step="1" /></label
          ><label
            >Expected throughput in bytes per second<input
              v-model.number="expectedRate"
              type="number"
              min="1"
              max="9007199254740991"
              step="1" /></label
          ><label
            >Expected availability<select v-model="expectedAvailability">
              <option value="">Unspecified</option>
              <option value="normally_available">Normally available</option>
              <option value="intermittent">Intermittent</option>
            </select></label
          >
        </div>
      </fieldset>
      <section
        v-if="documentation"
        class="notice"
        aria-label="Adapter documentation"
      >
        <h3>{{ documentation.display_name }}</h3>
        <dl>
          <dt>Acknowledgment</dt>
          <dd>{{ documentation.acknowledgement }}</dd>
          <dt>Consistency</dt>
          <dd>{{ documentation.consistency }}</dd>
          <dt>Durability</dt>
          <dd>{{ documentation.durability }}</dd>
          <dt>Deletion and capacity</dt>
          <dd>{{ documentation.deletion_and_capacity }}</dd>
        </dl>
        <ul>
          <li v-for="limitation in documentation.limitations" :key="limitation">
            {{ limitation }}
          </li>
        </ul>
      </section>
      <p v-if="documentationError" class="notice error">
        {{ documentationError }}
        <button type="button" class="quiet" @click="loadDocumentation">
          Retry adapter documentation
        </button>
      </p>
      <div class="actions">
        <button :disabled="busy || !['local', 's3'].includes(kind)">
          Save backend</button
        ><button
          v-if="selected"
          type="button"
          class="quiet"
          :disabled="busy"
          @click="edit(null)"
        >
          New backend
        </button>
      </div>
    </form>
  </section>
</template>
