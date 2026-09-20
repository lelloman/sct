<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import type {
  MaintenanceJob,
  MaintenancePage,
  MaintenanceKind,
  IntegrityFinding,
  IntegrityFindingPage,
  CursorToken,
} from "../generated/api";
import { api, errorMessage, segment } from "../api";
const props = defineProps<{ rootId?: string; administrator: boolean }>();
const jobs = ref<MaintenanceJob[]>([]);
const cursor = ref<CursorToken | null>(null);
const root = ref(props.rootId ?? "");
const kind = ref<MaintenanceKind>("integrity_scan");
const acknowledged = ref(false);
const findingsRoot = ref(props.rootId ?? "");
const findings = ref<IntegrityFinding[]>([]);
const findingsCursor = ref<CursorToken | null>(null);
const inspectedRoot = ref("");
const busy = ref(false);
const error = ref("");
const message = ref("");
let startKey = crypto.randomUUID();
watch([root, kind], () => {
  acknowledged.value = false;
  startKey = crypto.randomUUID();
});
const base = props.rootId
  ? `/api/v1/roots/${segment(props.rootId)}/maintenance`
  : "/api/v1/admin/maintenance";
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
  const page = await api<MaintenancePage>(
    `${base}?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
  );
  jobs.value = more ? [...jobs.value, ...page.jobs] : page.jobs;
  cursor.value = page.next_cursor;
}
async function start() {
  if (!acknowledged.value) return;
  await run(async () => {
    await api<MaintenanceJob>(
      kind.value === "mirror_repair"
        ? `/api/v1/admin/roots/${segment(root.value)}/repair`
        : "/api/v1/admin/maintenance",
      "POST",
      { idempotency_key: startKey, kind: kind.value, root_id: root.value },
    );
    startKey = crypto.randomUUID();
    acknowledged.value = false;
    await load();
    message.value = "Maintenance job queued.";
  });
}
async function resume(job: MaintenanceJob) {
  await run(async () => {
    await api<MaintenanceJob>(
      `/api/v1/admin/maintenance/${segment(job.id)}/resume`,
      "POST",
      { expected_revision: job.revision },
    );
    await load();
    message.value = "Maintenance job resumed.";
  });
}
async function inspect(more = false) {
  const target = more ? inspectedRoot.value : findingsRoot.value;
  const page = await api<IntegrityFindingPage>(
    `/api/v1/roots/${segment(target)}/integrity-findings?limit=100${more && findingsCursor.value ? `&cursor=${segment(findingsCursor.value)}` : ""}`,
  );
  inspectedRoot.value = target;
  findings.value = more ? [...findings.value, ...page.findings] : page.findings;
  findingsCursor.value = page.next_cursor;
}
onMounted(() => void run(() => load()));
</script>
<template>
  <section class="panel" :aria-busy="busy">
    <div class="row">
      <h2>Integrity and repair</h2>
      <button class="quiet" :disabled="busy" @click="run(() => load())">
        Refresh maintenance
      </button>
    </div>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="message" class="notice success" role="status">{{ message }}</p>
    <form v-if="administrator" class="stack" @submit.prevent="start">
      <div class="form-grid">
        <label
          >Maintenance root ID<input
            v-model="root"
            required
            :disabled="!!rootId" /></label
        ><label
          >Maintenance operation<select v-model="kind">
            <option value="integrity_scan">Integrity scan</option>
            <option value="mirror_repair">Repair mirrored copies</option>
          </select></label
        >
      </div>
      <p>
        Integrity scans verify saved bytes. Mirror repair requires a verified
        healthy alternate copy and may need staging capacity. Jobs run under
        exclusive root ownership and may temporarily delay backups.
      </p>
      <label class="check"
        ><input v-model="acknowledged" type="checkbox" />I acknowledge this
        maintenance operation</label
      >
      <div>
        <button :disabled="busy || !acknowledged">Start maintenance</button>
      </div>
    </form>
    <p v-if="!jobs.length && !busy">No maintenance jobs.</p>
    <div v-else class="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Job / root</th>
            <th>Operation</th>
            <th>State</th>
            <th>Processed</th>
            <th>Attempts / retry</th>
            <th>Failure</th>
            <th v-if="administrator">Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="job in jobs" :key="job.id">
            <td class="mono">{{ job.id }}<br />{{ job.root_id }}</td>
            <td>{{ job.kind }}</td>
            <td>{{ job.state }}</td>
            <td>{{ job.processed }}</td>
            <td>
              {{ job.attempts
              }}<span v-if="job.next_attempt_at_ms"
                ><br />{{
                  new Date(Number(job.next_attempt_at_ms)).toLocaleString()
                }}</span
              >
            </td>
            <td>{{ job.failure ?? "None" }}</td>
            <td v-if="administrator">
              <button
                v-if="['blocked', 'failed', 'retry_wait'].includes(job.state)"
                class="quiet"
                :disabled="busy"
                :aria-label="`Resume maintenance ${job.id}`"
                @click="resume(job)"
              >
                Resume
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
      Load more maintenance jobs
    </button>
    <form class="stack" @submit.prevent="run(() => inspect())">
      <h3>Integrity findings</h3>
      <label
        >Findings root ID<input
          v-model="findingsRoot"
          required
          :disabled="!!rootId"
      /></label>
      <div>
        <button class="quiet" :disabled="busy">Inspect findings</button>
      </div>
    </form>
    <p v-if="inspectedRoot && !findings.length">
      No integrity findings for this root.
    </p>
    <div v-if="findings.length" class="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Backend / digest</th>
            <th>Finding</th>
            <th>Resolution</th>
            <th>Scan job</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="finding in findings" :key="finding.id">
            <td class="mono">
              {{ finding.backend_id ?? "Catalog" }}<br />{{ finding.digest }}
            </td>
            <td>{{ finding.kind }}</td>
            <td>
              {{ finding.repaired ? "Repaired or resolved" : "Outstanding" }}
            </td>
            <td class="mono">{{ finding.job_id }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <button
      v-if="findingsCursor"
      class="quiet"
      :disabled="busy"
      @click="run(() => inspect(true))"
    >
      Load more findings
    </button>
  </section>
</template>
