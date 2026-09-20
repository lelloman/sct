<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import type {
  RootInspection,
  EffectivePolicy,
  EntryInspection,
  ChangePreview,
  PreviewVersion,
  PreviewVersionPage,
  CursorToken,
  LifecycleJob,
  RetentionMode,
  PolicyScope,
  PolicyChange,
} from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
const props = defineProps<{ root: RootInspection }>();
const path = ref("");
const loadedPath = ref("");
const scope = ref<PolicyScope>({ kind: "root", root_id: props.root.id });
const effective = ref<EffectivePolicy | null>(null);
const mode = ref<RetentionMode>("strict");
const keep = ref(1);
const inherit = ref(false);
const preview = ref<ChangePreview | null>(null);
const versions = ref<PreviewVersion[]>([]);
const cursor = ref<CursorToken | null>(null);
const job = ref<LifecycleJob | null>(null);
const acknowledge = ref(false);
const busy = ref(false);
const error = ref("");
const base = `/api/v1/roots/${segment(props.root.id)}`;
let applyKey = "";
watch([mode, keep, inherit, path], () => {
  preview.value = null;
  acknowledge.value = false;
  versions.value = [];
});
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
async function load() {
  preview.value = null;
  job.value = null;
  acknowledge.value = false;
  effective.value = null;
  scope.value = path.value
    ? {
        kind: "entry",
        root_id: props.root.id,
        entry_id: (
          await api<EntryInspection>(
            `${base}/resolve?path=${segment(path.value)}`,
          )
        ).id,
      }
    : { kind: "root", root_id: props.root.id };
  const query = new URLSearchParams({
    kind: scope.value.kind,
    root_id: props.root.id,
  });
  if (scope.value.kind === "entry") query.set("entry_id", scope.value.entry_id);
  effective.value = await api<EffectivePolicy>(`/api/v1/policies?${query}`);
  loadedPath.value = path.value;
  mode.value = effective.value.policy.mode;
  keep.value = effective.value.policy.keep_versions;
  inherit.value = false;
}
async function affected(more = false) {
  const page = await api<PreviewVersionPage>(
    `${base}/policy-previews/${segment(preview.value!.id)}/versions?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
  );
  versions.value = more ? [...versions.value, ...page.versions] : page.versions;
  cursor.value = page.next_cursor;
}
async function advance() {
  if (!preview.value) return;
  // A bounded batch keeps very large previews interruptible and resumable.
  for (let i = 0; i < 16 && preview.value.state === "planning"; i++)
    preview.value = await api<ChangePreview>(
      `${base}/policy-previews/${segment(preview.value.id)}/advance`,
      "POST",
    );
  if (preview.value.state === "ready") await affected();
}
async function createPreview() {
  if (!effective.value) return;
  await run(async () => {
    const root = await api<RootInspection>(base);
    const change: PolicyChange =
      inherit.value && scope.value.kind === "entry"
        ? {
            kind: "inherit",
            scope: scope.value,
            expected_policy_revision: effective.value!.revision,
          }
        : {
            kind: "set",
            scope: scope.value,
            expected_policy_revision: effective.value!.revision,
            policy: { mode: mode.value, keep_versions: keep.value },
          };
    preview.value = await api<ChangePreview>(
      `${base}/policy-previews`,
      "POST",
      { expected_root_revision: root.revision, change },
    );
    acknowledge.value = false;
    versions.value = [];
    cursor.value = null;
    applyKey = crypto.randomUUID();
    await advance();
  });
}
async function apply() {
  if (!preview.value || !acknowledge.value) return;
  await run(async () => {
    job.value = await api<LifecycleJob>(`${base}/policy-jobs`, "POST", {
      idempotency_key: applyKey,
      preview_id: preview.value!.id,
      expected_root_revision: preview.value!.root_revision,
      acknowledge_retention_effect: true,
    });
    preview.value = null;
    await progress();
  });
}
async function resume() {
  if (!job.value) return;
  await run(async () => {
    job.value = await api<LifecycleJob>(
      `${base}/jobs/${segment(job.value!.id)}/resume`,
      "POST",
    );
    await progress();
  });
}
async function progress() {
  if (!job.value) return;
  for (
    let i = 0;
    i < 8 && !["completed", "blocked"].includes(job.value.state);
    i++
  )
    job.value = await api<LifecycleJob>(
      `${base}/jobs/${segment(job.value.id)}/advance`,
      "POST",
    );
}
onMounted(() => void run(load));
</script>
<template>
  <section class="panel" :aria-busy="busy">
    <h2>Retention policy</h2>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <form class="stack" @submit.prevent="run(load)">
      <label
        >Policy path<input
          v-model="path"
          placeholder="Leave empty for root defaults"
          :disabled="busy"
      /></label>
      <div><button class="quiet" :disabled="busy">Load policy</button></div>
    </form>
    <template v-if="effective"
      ><p>
        Loaded policy:
        {{ scope.kind === "root" ? "Root defaults" : scope.entry_id }} ·
        {{ effective.inherited ? "Inherited" : "Explicit" }} ·
        {{ effective.versioning }}
      </p>
      <form class="stack" @submit.prevent="createPreview">
        <label v-if="scope.kind === 'entry'" class="check"
          ><input v-model="inherit" type="checkbox" :disabled="busy" />Inherit
          parent policy</label
        >
        <div v-if="!inherit" class="form-grid">
          <label
            >Retention mode<select v-model="mode" :disabled="busy">
              <option value="strict">Strict — protect retained versions</option>
              <option value="relaxed">Relaxed — permit capacity pruning</option>
            </select></label
          ><label
            >Versions to retain<input
              v-model.number="keep"
              type="number"
              min="1"
              max="9007199254740991"
              step="1"
              required
              :disabled="busy"
          /></label>
        </div>
        <p>
          Policy changes may retire existing versions. Review their effects
          before applying.
        </p>
        <div>
          <button
            :disabled="busy || effective.transitioning || path !== loadedPath"
          >
            Preview policy change
          </button>
        </div>
      </form>
    </template>
    <section v-if="preview" class="notice" aria-label="Policy preview">
      <h3>Policy preview · {{ preview.state }}</h3>
      <p>
        Versions to retire: {{ preview.versions_to_retire }}. Versions losing
        protection: {{ preview.versions_losing_protection }}. Estimated payload:
        {{ bytes(preview.estimated_payload_bytes) }}.
      </p>
      <p class="muted">
        Expires {{ new Date(Number(preview.expires_at_ms)).toLocaleString() }}.
        Concurrent changes require a fresh preview.
      </p>
      <button
        v-if="preview.state === 'planning'"
        class="quiet"
        :disabled="busy"
        @click="run(advance)"
      >
        Continue preview
      </button>
      <template v-if="preview.state === 'ready'"
        ><div v-if="versions.length" class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>Version</th>
                <th>Effect</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="version in versions" :key="version.version_id">
                <td class="mono">{{ version.version_id }}</td>
                <td>{{ version.retire ? "Retire" : "Lose protection" }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <button
          v-if="cursor"
          class="quiet"
          :disabled="busy"
          @click="run(() => affected(true))"
        >
          More affected versions
        </button>
        <label class="check"
          ><input v-model="acknowledge" type="checkbox" />I acknowledge the
          retention effects shown above</label
        ><button :disabled="busy || !acknowledge" @click="apply">
          Apply reviewed policy
        </button>
      </template>
    </section>
    <section v-if="job" class="notice" role="status">
      <h3>Policy job · {{ job.state }}</h3>
      <p>
        {{ job.phase }} · {{ job.processed }} processed<span v-if="job.failure">
          · {{ job.failure }}</span
        >
      </p>
      <button
        v-if="job.state !== 'completed' && job.state !== 'blocked'"
        class="quiet"
        :disabled="busy"
        @click="run(progress)"
      >
        Continue policy job</button
      ><button
        v-if="job.state === 'blocked'"
        class="quiet"
        :disabled="busy"
        @click="resume"
      >
        Resume policy job</button
      ><button class="quiet" :disabled="busy" @click="run(load)">
        Reload policy
      </button>
    </section>
  </section>
</template>
