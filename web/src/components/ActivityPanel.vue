<script setup lang="ts">
import { onMounted, ref } from "vue";
import type {
  RootCapacityUsage,
  TransferActivity,
  TransferActivityPage,
  JobActivity,
  JobActivityPage,
  AuditEvent,
  AuditPage,
  CursorToken,
  RetryGuidance,
} from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
const props = defineProps<{ rootId?: string }>();
const tab = ref("jobs");
const usage = ref<RootCapacityUsage | null>(null);
const sessions = ref<TransferActivity[]>([]);
const jobs = ref<JobActivity[]>([]);
const events = ref<AuditEvent[]>([]);
const cursor = ref<CursorToken | null>(null);
const busy = ref(false);
const error = ref("");
const base = props.rootId
  ? `/api/v1/roots/${segment(props.rootId)}`
  : "/api/v1/admin";
function retryLabel(retry: RetryGuidance) {
  switch (retry.kind) {
    case "never":
      return "No automatic retry";
    case "after":
      return `Retry after ${retry.delay_ms} ms`;
    case "refresh":
      return "Refresh the changed state";
    case "reconcile_upload":
      return "Program must reconcile the transfer";
  }
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
async function load(more = false) {
  const query = `?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`;
  if (tab.value === "sessions") {
    const page = await api<TransferActivityPage>(`${base}/sessions${query}`);
    sessions.value = more
      ? [...sessions.value, ...page.sessions]
      : page.sessions;
    cursor.value = page.next_cursor;
  } else if (tab.value === "jobs") {
    const page = await api<JobActivityPage>(`${base}/jobs${query}`);
    jobs.value = more ? [...jobs.value, ...page.jobs] : page.jobs;
    cursor.value = page.next_cursor;
  } else {
    const page = await api<AuditPage>(`${base}/audit${query}`);
    events.value = more ? [...events.value, ...page.events] : page.events;
    cursor.value = page.next_cursor;
  }
  if (props.rootId && !more)
    usage.value = await api<RootCapacityUsage>(`${base}/usage`);
}
async function select(value: string) {
  tab.value = value;
  cursor.value = null;
  await run(() => load());
}
onMounted(() => void run(() => load()));
</script>
<template>
  <section :aria-busy="busy">
    <section v-if="usage" class="panel" aria-label="Storage usage">
      <h2>Storage usage</h2>
      <div class="form-grid">
        <dl>
          <dt>Logical retained</dt>
          <dd>{{ bytes(usage.logical_retained_bytes) }}</dd>
          <dt>Unique retained payload</dt>
          <dd>{{ bytes(usage.unique_retained_bytes) }}</dd>
        </dl>
        <dl>
          <dt>Physical occupied</dt>
          <dd>{{ bytes(usage.physical_occupied_bytes) }}</dd>
          <dt>Staging occupied</dt>
          <dd>{{ bytes(usage.staging_occupied_bytes) }}</dd>
        </dl>
        <dl>
          <dt>Reserved</dt>
          <dd>{{ bytes(usage.reserved_bytes) }}</dd>
          <dt>Pending cleanup</dt>
          <dd>{{ bytes(usage.pending_cleanup_bytes) }}</dd>
        </dl>
      </div>
      <p class="muted">
        Pending cleanup remains charged until deletion is confirmed. Physical
        usage includes copies and storage metadata.
      </p>
    </section>
    <section class="panel">
      <div class="row">
        <h2>Activity</h2>
        <button class="quiet" :disabled="busy" @click="run(() => load())">
          Refresh activity
        </button>
      </div>
      <nav class="tabs" aria-label="Activity sections">
        <button
          class="quiet"
          :disabled="busy"
          :aria-pressed="tab === 'jobs'"
          @click="select('jobs')"
        >
          Jobs</button
        ><button
          v-if="rootId"
          class="quiet"
          :disabled="busy"
          :aria-pressed="tab === 'sessions'"
          @click="select('sessions')"
        >
          Transfer sessions</button
        ><button
          class="quiet"
          :disabled="busy"
          :aria-pressed="tab === 'audit'"
          @click="select('audit')"
        >
          Audit events
        </button>
      </nav>
      <p v-if="error" class="notice error" role="alert">{{ error }}</p>
      <p v-if="busy" role="status">Loading activity…</p>
      <template v-if="tab === 'jobs'"
        ><p v-if="!jobs.length && !busy">No jobs.</p>
        <div v-else class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>Job</th>
                <th>Root</th>
                <th>State / phase</th>
                <th>Progress</th>
                <th>Failure / retry</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="job in jobs" :key="job.operation.id">
                <td>
                  {{ job.operation.kind }}<br /><span class="mono">{{
                    job.operation.id
                  }}</span>
                </td>
                <td>
                  <a :href="`#/roots/${segment(job.operation.root_id)}`">{{
                    job.operation.root_id
                  }}</a>
                </td>
                <td>{{ job.operation.state }} · {{ job.phase }}</td>
                <td>
                  {{ job.operation.completed_units
                  }}<span v-if="job.operation.total_units !== null">
                    / {{ job.operation.total_units }}</span
                  >
                </td>
                <td>
                  {{ job.operation.failure_code ?? "None" }}<br />{{
                    retryLabel(job.operation.retry)
                  }}
                </td>
              </tr>
            </tbody>
          </table>
        </div></template
      >
      <template v-else-if="tab === 'sessions'"
        ><p v-if="!sessions.length && !busy">No transfer sessions.</p>
        <div v-else class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>Session</th>
                <th>Account</th>
                <th>State / phase</th>
                <th>Admission</th>
                <th>Expires</th>
                <th>Failure</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="session in sessions" :key="session.id">
                <td>
                  {{ session.kind }}<br /><span class="mono">{{
                    session.id
                  }}</span>
                </td>
                <td>{{ session.principal_id }}</td>
                <td>{{ session.state }} · {{ session.phase }}</td>
                <td>{{ session.admission ?? "—" }}</td>
                <td>
                  {{ new Date(Number(session.expires_at_ms)).toLocaleString() }}
                </td>
                <td>{{ session.failure ?? "None" }}</td>
              </tr>
            </tbody>
          </table>
        </div></template
      >
      <template v-else
        ><p v-if="!events.length && !busy">No audit events.</p>
        <div v-else class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>Time</th>
                <th>Account</th>
                <th>Event</th>
                <th>Details</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="event in events" :key="event.id">
                <td>
                  {{ new Date(Number(event.created_at_ms)).toLocaleString() }}
                </td>
                <td>{{ event.principal_id ?? "System" }}</td>
                <td>{{ event.event }}</td>
                <td>
                  <details>
                    <summary>View details</summary>
                    <pre class="mono">{{
                      JSON.stringify(event.details, null, 2)
                    }}</pre>
                  </details>
                </td>
              </tr>
            </tbody>
          </table>
        </div></template
      >
      <button
        v-if="cursor"
        class="quiet"
        :disabled="busy"
        @click="run(() => load(true))"
      >
        Load more activity
      </button>
    </section>
  </section>
</template>
