<script setup lang="ts">
import { computed, ref } from "vue";
import { bytes, segment, errorMessage } from "../api";
import { archivePage } from "../archivePaging";
import type {
  ArchiveDiscObject,
  ArchiveLogicalDisc,
  ArchivePhysicalCopy,
  ArchiveSet,
} from "../generated/api";
import ArchiveCopyManager from "./ArchiveCopyManager.vue";
import ArchivePager from "./ArchivePager.vue";
const props = defineProps<{ collectionId: string; disc: ArchiveLogicalDisc }>();
defineEmits<{ select: [snapshot: ArchiveSet] }>();
const base = computed(
  () =>
    `/api/v1/archives/${segment(props.collectionId)}/discs/${segment(props.disc.id)}`,
);
const copies = archivePage<ArchivePhysicalCopy>(
  () => `${base.value}/copies`,
  "copies",
);
const objects = archivePage<ArchiveDiscObject>(
  () => `${base.value}/objects`,
  "objects",
);
const required = archivePage<ArchiveSet>(() => `${base.value}/sets`, "sets");
const downloadError = ref("");
const downloading = ref(false);
async function download() {
  downloading.value = true;
  downloadError.value = "";
  try {
    const path = `${base.value}/iso`;
    const response = await fetch(path, {
      method: "HEAD",
      credentials: "same-origin",
    });
    if (!response.ok)
      throw new Error(
        `ISO download is unavailable (${response.status}). Refresh the disc list; the cache may have expired or your access may have changed.`,
      );
    const link = document.createElement("a");
    link.href = path;
    link.download = `disc-${props.disc.number}.iso`;
    link.click();
  } catch (failure) {
    downloadError.value = errorMessage(failure);
  } finally {
    downloading.value = false;
  }
}
function date(value: number) {
  return new Date(value).toLocaleString();
}
</script>
<template>
  <section class="panel" aria-label="Disc details">
    <h3>Disc {{ disc.number }} details</h3>
    <p class="mono">{{ disc.id }}</p>
    <p>
      {{ disc.object_count }} original payload objects ·
      {{ bytes(disc.payload_bytes) }}
    </p>
    <p>
      Cached ISO: {{ disc.artifact?.state ?? "Not created"
      }}<template v-if="disc.artifact">
        · Cache expiry {{ date(disc.artifact.expires_at_ms) }}</template
      >
    </p>
    <p v-if="downloadError" role="alert" class="notice error">
      {{ downloadError }}
    </p>
    <button
      v-if="disc.artifact?.state === 'available'"
      :disabled="downloading"
      @click="download"
    >
      Download ISO
    </button>
    <p v-if="disc.artifact?.state === 'available'" class="muted">
      The download supports byte ranges and an immutable SHA-256 ETag for
      resume. Keep the displayed hash; the program SDK can verify and resume
      interrupted downloads. Downloading does not mark this disc burned.
    </p>
    <details>
      <summary>Content identities</summary>
      <dl>
        <dt>Snapshot index SHA-256</dt>
        <dd class="mono">{{ disc.index_sha256 }}</dd>
        <dt>Recovery bundle SHA-256</dt>
        <dd class="mono">{{ disc.bundle_sha256 }}</dd>
        <dt>Original payload inventory SHA-256</dt>
        <dd class="mono">{{ disc.payload_inventory_sha256 }}</dd>
        <template v-if="disc.artifact?.sha256"
          ><dt>ISO SHA-256</dt>
          <dd class="mono">{{ disc.artifact.sha256 }}</dd></template
        >
      </dl>
    </details>
    <h4>Recovery points requiring this disc</h4>
    <p v-if="required.error.value" role="alert" class="notice error">
      {{ required.error.value }}
    </p>
    <ul>
      <li v-for="snapshot in required.items.value" :key="snapshot.id">
        <button class="quiet" @click="$emit('select', snapshot)">
          Snapshot {{ snapshot.sequence }}
        </button>
        ·
        {{
          snapshot.verified_complete_at_ms
            ? "Historically completed"
            : "Not historically completed"
        }}
      </li>
    </ul>
    <ArchivePager
      :page="required.page.value"
      :next="!!required.next.value"
      :busy="required.busy.value"
      @back="required.back"
      @forward="required.forward"
      @refresh="required.refresh"
    />
    <h4>Physical copies</h4>
    <ArchiveCopyManager
      :collection-id="collectionId"
      :disc="disc"
      @changed="copies.refresh"
    />
    <p class="muted">
      Burn times and read-back results are owner-supplied observations. An ISO
      export or download does not verify a physical disc.
    </p>
    <p v-if="copies.error.value" role="alert" class="notice error">
      {{ copies.error.value }}
    </p>
    <p v-if="!copies.busy.value && !copies.items.value.length">
      No physical copies recorded on this page.
    </p>
    <article v-for="copy in copies.items.value" :key="copy.id" class="panel">
      <h5>{{ copy.label }}</h5>
      <ArchiveCopyManager
        :collection-id="collectionId"
        :disc="disc"
        :copy="copy"
        @changed="copies.refresh"
      />
      <p>
        Current health: <strong>{{ copy.health.replaceAll("_", " ") }}</strong>
      </p>
      <p>Location: {{ copy.location || "Not recorded" }}</p>
      <p>
        Reported burn:
        {{
          copy.reported_burned_at_ms
            ? date(copy.reported_burned_at_ms)
            : "Not recorded"
        }}
      </p>
      <p>Latest read-back report: {{ copy.latest_report_id ?? "None" }}</p>
    </article>
    <ArchivePager
      :page="copies.page.value"
      :next="!!copies.next.value"
      :busy="copies.busy.value"
      @back="copies.back"
      @forward="copies.forward"
      @refresh="copies.refresh"
    />
    <details>
      <summary>Original payload inventory</summary>
      <p v-if="objects.error.value" role="alert" class="notice error">
        {{ objects.error.value }}
      </p>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Source root</th>
              <th>Object SHA-256</th>
              <th>Size</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="object in objects.items.value"
              :key="`${object.root_id}:${object.digest}`"
            >
              <td>{{ object.root_id }}</td>
              <td class="mono">{{ object.digest }}</td>
              <td>{{ bytes(object.length) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <ArchivePager
        :page="objects.page.value"
        :next="!!objects.next.value"
        :busy="objects.busy.value"
        @back="objects.back"
        @forward="objects.forward"
        @refresh="objects.refresh"
      />
    </details>
  </section>
</template>
