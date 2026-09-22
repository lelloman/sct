<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, bytes, errorMessage, segment } from "../api";
import type { ArchiveAccountLimits, ArchiveUsage } from "../generated/api";
const props = defineProps<{ principalId?: string }>();
const limits = ref<ArchiveAccountLimits | null>(null);
const usage = ref<ArchiveUsage | null>(null);
const busy = ref(false);
const error = ref("");
const saved = ref(false);
const path = computed(() =>
  props.principalId
    ? `/api/v1/admin/principals/${segment(props.principalId)}/archive-limits`
    : "/api/v1/archives/limits",
);
async function load() {
  busy.value = true;
  error.value = "";
  try {
    limits.value = await api<ArchiveAccountLimits>(path.value);
    if (!props.principalId)
      usage.value = await api<ArchiveUsage>("/api/v1/archives/usage");
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
async function save() {
  if (!limits.value || !props.principalId) return;
  busy.value = true;
  error.value = "";
  saved.value = false;
  try {
    limits.value = await api<ArchiveAccountLimits>(path.value, "PATCH", {
      expected_revision: limits.value.revision,
      quota_bytes: limits.value.quota_bytes,
      pin_quota_bytes: limits.value.pin_quota_bytes,
      max_active: limits.value.max_active,
      max_capture_records: limits.value.max_capture_records,
      pin_deadline_seconds: limits.value.pin_deadline_seconds,
      cache_ttl_seconds: limits.value.cache_ttl_seconds,
    });
    saved.value = true;
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
onMounted(load);
</script>
<template>
  <details class="panel">
    <summary>Disc archive limits</summary>
    <p v-if="error" role="alert" class="notice error">{{ error }}</p>
    <p v-if="saved" role="status">Archive limits saved.</p>
    <p v-if="usage">
      Cached ISOs {{ bytes(usage.cache_bytes) }} · Reserved
      {{ bytes(usage.reserved_bytes) }} · Active exports {{ usage.active_sets }}
    </p>
    <form v-if="limits" class="stack" @submit.prevent="save">
      <fieldset :disabled="busy || !principalId">
        <legend>Account archive capacity</legend>
        <label
          >ISO cache quota (bytes)<input
            v-model.number="limits.quota_bytes"
            type="number"
            min="0"
            max="9007199254740991"
            step="1"
            required
        /></label>
        <label
          >Source pin quota (bytes)<input
            v-model.number="limits.pin_quota_bytes"
            type="number"
            min="0"
            max="9007199254740991"
            step="1"
            required
        /></label>
        <label
          >Maximum active exports<input
            v-model.number="limits.max_active"
            type="number"
            min="1"
            max="64"
            step="1"
            required
        /></label>
        <label
          >Maximum captured records<input
            v-model.number="limits.max_capture_records"
            type="number"
            min="1"
            max="100000000"
            step="1"
            required
        /></label>
        <label
          >Plan deadline (seconds)<input
            v-model.number="limits.pin_deadline_seconds"
            type="number"
            min="60"
            max="604800"
            step="1"
            required
        /></label>
        <label
          >ISO cache lifetime (seconds)<input
            v-model.number="limits.cache_ttl_seconds"
            type="number"
            min="3600"
            max="2592000"
            step="1"
            required
        /></label>
        <button v-if="principalId">Save archive limits</button>
      </fieldset>
      <p v-if="!principalId" class="muted">
        An administrator manages these limits. Cache expiry removes server ISO
        files, preserving the permanent disc history.
      </p>
      <button type="button" class="quiet" :disabled="busy" @click="load">
        Refresh archive limits
      </button>
    </form>
  </details>
</template>
