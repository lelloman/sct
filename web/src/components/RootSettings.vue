<script setup lang="ts">
import { onMounted, ref } from "vue";
import type { RootInspection, RootManagement } from "../generated/api";
import { api, bytes, errorMessage, segment } from "../api";
const props = defineProps<{ root: RootInspection }>();
const emit = defineEmits<{ updated: [] }>();
const management = ref<RootManagement | null>(null);
const name = ref(props.root.display_name as string);
const quota = ref(props.root.quota_bytes);
const interval = ref<number | "">("");
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
async function load() {
  const m = await api<RootManagement>(
    `/api/v1/roots/${segment(props.root.id)}/management`,
  );
  management.value = m;
  quota.value = m.quota_bytes;
  interval.value = m.expected_upload_interval_seconds ?? "";
  const root = await api<RootInspection>(
    `/api/v1/roots/${segment(props.root.id)}`,
  );
  name.value = root.display_name;
}
async function save() {
  if (!management.value) return;
  await run(async () => {
    management.value = await api<RootManagement>(
      `/api/v1/roots/${segment(props.root.id)}/management`,
      "PATCH",
      {
        expected_revision: management.value!.revision,
        display_name: name.value,
        quota_bytes: quota.value,
        expected_upload_interval_seconds:
          interval.value === "" ? null : interval.value,
      },
    );
    message.value = "Root settings saved.";
    emit("updated");
  });
}
onMounted(() => void run(load));
</script>
<template>
  <section class="panel" :aria-busy="busy">
    <div class="row">
      <h2>Root settings</h2>
      <button class="quiet" :disabled="busy" @click="run(load)">
        Reload settings
      </button>
    </div>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="message" class="notice success" role="status">{{ message }}</p>
    <form v-if="management" class="stack" @submit.prevent="save">
      <label
        >Display name<input v-model="name" required maxlength="255" /></label
      ><label
        >Quota in bytes<input
          v-model.number="quota"
          type="number"
          min="0"
          step="1"
          :max="management.quota_ceiling_bytes"
          required
      /></label>
      <p class="muted">
        Administrator ceiling: {{ bytes(management.quota_ceiling_bytes) }}.
        Allocated storage must also cover this quota.
      </p>
      <label
        >Expected backup interval in seconds<input
          v-model.number="interval"
          type="number"
          min="60"
          max="31536000"
          step="1"
          placeholder="No overdue monitoring"
      /></label>
      <p class="muted">
        Optional. Your program controls scheduling; a successful backup commit
        refreshes this interval.
      </p>
      <div><button :disabled="busy">Save root settings</button></div>
    </form>
  </section>
</template>
