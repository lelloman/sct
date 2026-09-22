<script setup lang="ts">
import { computed, ref } from "vue";
import { api, errorMessage, segment } from "../api";
import { archivePage } from "../archivePaging";
import type {
  ArchiveCopyEvent,
  ArchiveLogicalDisc,
  ArchiveMediaVerification,
  ArchivePhysicalCopy,
  ArchiveVerificationReport,
} from "../generated/api";
import ArchivePager from "./ArchivePager.vue";
const props = defineProps<{
  collectionId: string;
  disc: ArchiveLogicalDisc;
  copy?: ArchivePhysicalCopy;
}>();
const emit = defineEmits<{ changed: [] }>();
const label = ref(props.copy?.label ?? "");
const location = ref(props.copy?.location ?? "");
const health = ref<string>("");
const burned = ref("");
const report = ref("");
const busy = ref(false);
const error = ref("");
const message = ref("");
const base = computed(() => `/api/v1/archives/${segment(props.collectionId)}`);
const reports = archivePage<ArchiveVerificationReport>(
  () =>
    props.copy ? `${base.value}/copies/${segment(props.copy.id)}/reports` : "",
  "reports",
);
const events = archivePage<ArchiveCopyEvent>(
  () =>
    props.copy ? `${base.value}/copies/${segment(props.copy.id)}/events` : "",
  "events",
);
async function run(work: () => Promise<void>) {
  busy.value = true;
  error.value = "";
  message.value = "";
  try {
    await work();
    emit("changed");
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
async function save() {
  await run(async () => {
    const reported = burned.value ? new Date(burned.value).getTime() : null;
    if (reported !== null && (!Number.isSafeInteger(reported) || reported < 0))
      throw new Error("Enter a valid burn date and time.");
    if (props.copy) {
      await api(`${base.value}/copies/${segment(props.copy.id)}`, "PATCH", {
        expected_revision: props.copy.revision,
        label: label.value,
        location: location.value,
        health: health.value || null,
        reported_burned_at_ms: reported,
      });
    } else {
      await api(
        `${base.value}/discs/${segment(props.disc.id)}/copies`,
        "POST",
        {
          request_id: crypto.randomUUID(),
          label: label.value,
          location: location.value,
          reported_burned_at_ms: reported,
        },
      );
      label.value = "";
      location.value = "";
      burned.value = "";
    }
    message.value = "Physical-copy observation saved.";
  });
}
async function verify() {
  await run(async () => {
    if (!props.copy) return;
    if (report.value.length > 65536)
      throw new Error(
        "The verification summary exceeds 64 KiB. Paste the summary JSON, not the per-object log.",
      );
    let observation: ArchiveMediaVerification;
    try {
      observation = JSON.parse(report.value) as ArchiveMediaVerification;
    } catch {
      throw new Error("Paste the JSON summary emitted by verify-report.");
    }
    if (
      !observation ||
      observation.collection_id !== props.collectionId ||
      observation.disc_id !== props.disc.id ||
      observation.index_sha256 !== props.disc.index_sha256 ||
      observation.bundle_sha256 !== props.disc.bundle_sha256 ||
      observation.payload_inventory_sha256 !==
        props.disc.payload_inventory_sha256
    )
      throw new Error(
        "This report does not match this collection and original disc inventory.",
      );
    const result = await api<ArchiveVerificationReport>(
      `${base.value}/copies/${segment(props.copy.id)}/reports`,
      "POST",
      {
        ...observation,
        request_id: crypto.randomUUID(),
        expected_revision: props.copy.revision,
      },
    );
    message.value = result.complete_read_back
      ? "Complete read-back observation recorded."
      : "Partial or damaged read-back recorded. This copy does not satisfy verification.";
    report.value = "";
    await reports.refresh();
    await events.refresh();
  });
}
</script>
<template>
  <details class="panel">
    <summary>
      {{ copy ? `Manage copy ${copy.label}` : "Record a physical copy" }}
    </summary>
    <p v-if="error" role="alert" class="notice error">{{ error }}</p>
    <p v-if="message" role="status">{{ message }}</p>
    <form class="stack" @submit.prevent="save">
      <label for="archive-copy-label">Copy label</label>
      <input id="archive-copy-label" v-model="label" maxlength="255" required />
      <label for="archive-copy-location">Physical location</label>
      <input id="archive-copy-location" v-model="location" maxlength="1024" />
      <label for="archive-copy-burned">Reported burn date and time</label>
      <input id="archive-copy-burned" v-model="burned" type="datetime-local" />
      <p v-if="copy" class="muted">
        Leave the date blank to preserve the existing burn observation.
      </p>
      <label v-if="copy" for="archive-copy-health"
        >Current health observation</label
      >
      <select v-if="copy" id="archive-copy-health" v-model="health">
        <option value="">Preserve current health</option>
        <option value="unknown">Unknown</option>
        <option value="damaged">Damaged</option>
        <option value="missing">Missing</option>
      </select>
      <button :disabled="busy">
        {{ copy ? "Save copy details" : "Register physical copy" }}
      </button>
    </form>
    <template v-if="copy">
      <h5>Import full-disc read-back</h5>
      <p>After burning, mount the physical disc and run its recovery tool:</p>
      <pre><code>sct-recover verify-report /media/disc /private/original-disc.sqlite</code></pre>
      <p>
        Paste the single JSON summary from standard output. The per-object
        diagnostic log is separate. This records your observation; it is not
        cryptographic proof of physical possession.
      </p>
      <form class="stack" @submit.prevent="verify">
        <label for="archive-verification-report"
          >Verification summary JSON</label
        >
        <textarea
          id="archive-verification-report"
          v-model="report"
          rows="8"
          maxlength="65536"
          required
          spellcheck="false"
        ></textarea>
        <button :disabled="busy">Record read-back report</button>
      </form>
      <h5>Read-back history</h5>
      <p v-if="reports.error.value" role="alert">{{ reports.error.value }}</p>
      <ul>
        <li v-for="item in reports.items.value" :key="item.id">
          {{ new Date(item.received_at_ms).toLocaleString() }} ·
          {{
            item.complete_read_back
              ? "Complete read-back"
              : "Partial or damaged"
          }}
          · {{ item.observation.verified_objects }}/{{
            item.observation.expected_objects
          }}
          objects · Reported by {{ item.principal_id }}
        </li>
      </ul>
      <ArchivePager
        :page="reports.page.value"
        :next="!!reports.next.value"
        :busy="reports.busy.value"
        @back="reports.back"
        @forward="reports.forward"
        @refresh="reports.refresh"
      />
      <h5>Copy observations</h5>
      <p v-if="events.error.value" role="alert">{{ events.error.value }}</p>
      <ul>
        <li v-for="item in events.items.value" :key="item.id">
          {{ new Date(item.received_at_ms).toLocaleString() }} ·
          {{ item.kind.replaceAll("_", " ") }} ·
          {{ item.observation.health.replaceAll("_", " ") }} ·
          {{ item.observation.label }} · {{ item.observation.location }}
        </li>
      </ul>
      <ArchivePager
        :page="events.page.value"
        :next="!!events.next.value"
        :busy="events.busy.value"
        @back="events.back"
        @forward="events.forward"
        @refresh="events.refresh"
      />
    </template>
  </details>
</template>
