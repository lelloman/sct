<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { api, bytes, errorMessage, segment } from "../api";
import type { ArchiveSet } from "../generated/api";
const props = defineProps<{ snapshot: ArchiveSet }>();
const emit = defineEmits<{ updated: [snapshot: ArchiveSet] }>();
const busy = ref(false);
const error = ref("");
const active = computed(() =>
  ["capturing", "planning", "awaiting_confirmation", "building"].includes(
    props.snapshot.state,
  ),
);
const base = computed(
  () =>
    `/api/v1/archives/${segment(props.snapshot.collection_id)}/sets/${segment(props.snapshot.id)}`,
);
let mounted = true;
let timer: number | undefined;
async function refresh() {
  try {
    const result = await api<ArchiveSet>(base.value);
    if (mounted) {
      emit("updated", result);
      error.value = "";
    }
  } catch (failure) {
    if (mounted) error.value = errorMessage(failure);
  }
}
async function poll() {
  if (active.value && !busy.value) await refresh();
  if (mounted) timer = window.setTimeout(() => void poll(), 3000);
}
onMounted(() => {
  timer = window.setTimeout(() => void poll(), 3000);
});
onUnmounted(() => {
  mounted = false;
  window.clearTimeout(timer);
});
async function action(name: "confirm" | "cancel") {
  busy.value = true;
  error.value = "";
  try {
    const result = await api<ArchiveSet>(`${base.value}/${name}`, "POST", {
      expected_revision: props.snapshot.revision,
      request_id: crypto.randomUUID(),
    });
    if (mounted) emit("updated", result);
  } catch (failure) {
    if (mounted) error.value = errorMessage(failure);
  } finally {
    if (mounted) busy.value = false;
  }
}
</script>
<template>
  <section aria-label="Export progress">
    <p v-if="error" role="alert" class="notice error">{{ error }}</p>
    <p role="status">
      {{ snapshot.state.replaceAll("_", " ") }} ·
      {{ snapshot.processed_entries }} entries captured ·
      {{ bytes(snapshot.processed_bytes) }}
    </p>
    <p v-if="snapshot.failure" class="notice error">
      {{ errorMessage(new Error(snapshot.failure.replaceAll("_", " "))) }}
    </p>
    <p v-if="active">
      Plan expires {{ new Date(snapshot.expires_at_ms).toLocaleString() }}.
      Source pins and temporary capacity are released after completion or
      cancellation.
    </p>
    <template v-if="snapshot.plan">
      <h3>Disc-set preview</h3>
      <dl>
        <dt>New payload</dt>
        <dd>{{ bytes(snapshot.plan.new_payload_bytes) }}</dd>
        <dt>Reused payload</dt>
        <dd>{{ bytes(snapshot.plan.reused_payload_bytes) }}</dd>
        <dt>New discs</dt>
        <dd>{{ snapshot.plan.new_disc_count }}</dd>
        <dt>Required earlier discs</dt>
        <dd>{{ snapshot.plan.required_earlier_disc_count }}</dd>
        <dt>Complete index repeated on every new disc</dt>
        <dd>{{ bytes(snapshot.plan.index_bytes_per_disc) }}</dd>
        <dt>Recovery software repeated on every new disc</dt>
        <dd>{{ bytes(snapshot.plan.bundle_bytes_per_disc) }}</dd>
        <dt>Filesystem reserve per disc</dt>
        <dd>{{ bytes(snapshot.plan.filesystem_reserve_per_disc) }}</dd>
        <dt>Total ISO budget</dt>
        <dd>{{ bytes(snapshot.plan.total_iso_budget_bytes) }}</dd>
      </dl>
      <p>
        Review the required-disc list and each disc's payload assignment below
        before confirming. Preserve all earlier discs for their historical
        snapshots.
      </p>
    </template>
    <div class="row">
      <button
        v-if="snapshot.state === 'awaiting_confirmation'"
        :disabled="busy"
        @click="action('confirm')"
      >
        Confirm and generate ISOs</button
      ><button
        v-if="active"
        class="quiet"
        :disabled="busy"
        @click="action('cancel')"
      >
        Cancel this export</button
      ><button class="quiet" :disabled="busy" @click="refresh">
        Refresh export status
      </button>
    </div>
    <p v-if="snapshot.state === 'ready'">
      Images are ready. Downloading an ISO does not record a burn or verify a
      physical copy.
    </p>
    <details>
      <summary>Recover with surviving discs</summary>
      <p>
        Each surviving disc contains the full index and software for its own
        snapshot. Choose a surviving disc from the newest recovery point you
        want, then use its recovery tool to ingest all available discs required
        by that snapshot.
      </p>
      <p>
        If required payloads are missing, complete snapshot recovery is
        unavailable. The offline tool identifies exact missing objects and can
        restore complete files with <code>restore --partial</code>; it never
        silently substitutes older files. A disc from an older set can instead
        bootstrap that older snapshot.
      </p>
    </details>
  </section>
</template>
