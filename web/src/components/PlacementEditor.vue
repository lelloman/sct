<script setup lang="ts">
import { ref, watch } from "vue";
import type { Placement, BackendId, ReadCandidate } from "../generated/api";
const props = defineProps<{ modelValue: Placement }>();
const emit = defineEmits<{ "update:modelValue": [value: Placement] }>();
function clone(value: Placement): Placement {
  return JSON.parse(JSON.stringify(value)) as Placement;
}
const draft = ref(clone(props.modelValue));
const layout = ref(draft.value.kind);
watch(
  () => props.modelValue,
  (value) => {
    draft.value = clone(value);
    layout.value = value.kind;
  },
);
function changed() {
  emit("update:modelValue", clone(draft.value));
}
function candidate(id: BackendId): ReadCandidate {
  return {
    backend_id: id,
    connection_timeout_ms: 5000,
    initial_response_timeout_ms: 10000,
    idle_timeout_ms: 30000,
    throughput_window_ms: 10000,
    minimum_bytes_per_second: 1,
    startup_allowance_ms: 10000,
  };
}
function switchKind() {
  const first =
    draft.value.kind === "single"
      ? draft.value.backend_id
      : draft.value.backend_ids[0];
  const candidates = draft.value.read_policy.candidates;
  const primary =
    candidates.find((c) => c.backend_id === first) ?? candidate(first);
  const read_policy = {
    ...draft.value.read_policy,
    candidates:
      layout.value === "single"
        ? [primary]
        : [primary, candidate("" as BackendId)],
  };
  draft.value =
    layout.value === "single"
      ? { kind: "single", backend_id: first, read_policy }
      : { kind: "mirror", backend_ids: [first, "" as BackendId], read_policy };
  changed();
}
function updateBackend(index: number, event: Event) {
  const value = (event.target as HTMLInputElement).value as BackendId;
  const old =
    draft.value.kind === "single"
      ? draft.value.backend_id
      : draft.value.backend_ids[index];
  if (draft.value.kind === "single") draft.value.backend_id = value;
  else draft.value.backend_ids[index] = value;
  draft.value.read_policy.candidates = draft.value.read_policy.candidates.map(
    (c) => (c.backend_id === old ? { ...c, backend_id: value } : c),
  );
  changed();
}
function move(index: number) {
  const candidates = draft.value.read_policy.candidates;
  [candidates[index - 1], candidates[index]] = [
    candidates[index]!,
    candidates[index - 1]!,
  ];
  changed();
}
</script>
<template>
  <fieldset class="stack">
    <legend>Placement and read policy</legend>
    <p>
      Changing destinations requires an unpopulated root and sufficient explicit
      allocations in each pool. Read priority and timeouts can be adjusted for
      existing placements.
    </p>
    <label
      >Placement type<select v-model="layout" @change="switchKind">
        <option value="single">Single backend</option>
        <option value="mirror">Two-copy mirror</option>
      </select></label
    >
    <div class="form-grid">
      <label
        >Placement first backend ID<input
          :value="
            draft.kind === 'single' ? draft.backend_id : draft.backend_ids[0]
          "
          required
          @change="updateBackend(0, $event)" /></label
      ><label v-if="draft.kind === 'mirror'"
        >Placement second backend ID<input
          :value="draft.backend_ids[1]"
          required
          @change="updateBackend(1, $event)"
      /></label>
    </div>
    <fieldset
      v-for="(item, index) in draft.read_policy.candidates"
      :key="index"
      class="stack"
    >
      <legend>Read priority {{ index + 1 }} · {{ item.backend_id }}</legend>
      <button v-if="index > 0" type="button" class="quiet" @click="move(index)">
        Move read candidate earlier
      </button>
      <div class="form-grid">
        <label
          >Connection timeout in milliseconds<input
            v-model.number="item.connection_timeout_ms"
            type="number"
            min="1"
            max="9007199254740991"
            step="1"
            required
            @change="changed" /></label
        ><label
          >Initial response timeout in milliseconds<input
            v-model.number="item.initial_response_timeout_ms"
            type="number"
            min="1"
            max="9007199254740991"
            step="1"
            required
            @change="changed" /></label
        ><label
          >Idle timeout in milliseconds<input
            v-model.number="item.idle_timeout_ms"
            type="number"
            min="1"
            max="9007199254740991"
            step="1"
            required
            @change="changed" /></label
        ><label
          >Throughput window in milliseconds<input
            v-model.number="item.throughput_window_ms"
            type="number"
            min="1"
            max="9007199254740991"
            step="1"
            required
            @change="changed" /></label
        ><label
          >Minimum bytes per second<input
            v-model.number="item.minimum_bytes_per_second"
            type="number"
            min="1"
            max="9007199254740991"
            step="1"
            required
            @change="changed" /></label
        ><label
          >Startup allowance in milliseconds<input
            v-model.number="item.startup_allowance_ms"
            type="number"
            min="0"
            max="9007199254740991"
            step="1"
            required
            @change="changed"
        /></label>
      </div>
    </fieldset>
    <label
      >Overall read deadline in milliseconds (optional)<input
        :value="draft.read_policy.overall_deadline_ms ?? ''"
        type="number"
        min="1"
        max="9007199254740991"
        step="1"
        @change="
          draft.read_policy.overall_deadline_ms =
            ($event.target as HTMLInputElement).value === ''
              ? null
              : Number(($event.target as HTMLInputElement).value);
          changed();
        "
    /></label>
  </fieldset>
</template>
