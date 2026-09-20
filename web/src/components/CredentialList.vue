<script setup lang="ts">
import { onMounted, ref } from "vue";
import type {
  CredentialInspection,
  CredentialPage,
  CursorToken,
  IssuedToken,
} from "../generated/api";
import { api, errorMessage, segment } from "../api";
const props = defineProps<{ principalId: string }>();
const credentials = ref<CredentialInspection[]>([]);
const cursor = ref<CursorToken | null>(null);
const issued = ref<IssuedToken | null>(null);
const revokeId = ref<string | null>(null);
const busy = ref(false);
const error = ref("");
const base = `/api/v1/principals/${segment(props.principalId)}/credentials`;
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
  const page = await api<CredentialPage>(
    `${base}?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
  );
  credentials.value = more
    ? [...credentials.value, ...page.credentials]
    : page.credentials;
  cursor.value = page.next_cursor;
}
async function issue() {
  await run(async () => {
    issued.value = await api<IssuedToken>(base, "POST");
    await load();
  });
}
async function revoke() {
  if (!revokeId.value) return;
  await run(async () => {
    await api<void>(
      `/api/v1/credentials/${segment(revokeId.value!)}`,
      "DELETE",
    );
    revokeId.value = null;
    await load();
  });
}
onMounted(() => void run(() => load()));
</script>
<template>
  <section class="panel" aria-label="Credentials" :aria-busy="busy">
    <div class="row">
      <h2>Credentials</h2>
      <button class="quiet" :disabled="busy" @click="run(() => load())">
        Refresh credentials
      </button>
    </div>
    <p>
      Program credentials authorize access to this account. Revoking a session
      signs that browser out.
    </p>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <div v-if="issued" class="notice success" role="status">
      <h3>Save this credential now</h3>
      <p>The secret is shown once. Keep it in your program's secret store.</p>
      <label
        >New credential secret<input
          :value="issued.secret"
          readonly
          autocomplete="off"
          spellcheck="false"
      /></label>
      <p class="mono">Credential ID: {{ issued.id }}</p>
      <button class="quiet" @click="issued = null">
        I have saved the credential
      </button>
    </div>
    <button :disabled="busy || !!issued" @click="issue">
      Issue program credential
    </button>
    <div v-if="revokeId" class="notice" role="alert">
      <p>
        Revoke credential <span class="mono">{{ revokeId }}</span
        >? Programs or sessions using it will lose access immediately.
      </p>
      <div class="actions">
        <button class="danger" :disabled="busy" @click="revoke">
          Confirm revocation</button
        ><button class="quiet" :disabled="busy" @click="revokeId = null">
          Cancel
        </button>
      </div>
    </div>
    <p v-if="busy" role="status">Loading credentials…</p>
    <p v-if="!credentials.length && !busy" class="empty">No credentials.</p>
    <div v-else class="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Credential</th>
            <th>Kind</th>
            <th>Status</th>
            <th>Expires</th>
            <th>Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="credential in credentials" :key="credential.id">
            <td class="mono">{{ credential.id }}</td>
            <td>{{ credential.kind }}</td>
            <td>{{ credential.revoked ? "Revoked" : "Active" }}</td>
            <td>
              {{
                credential.expires_at_ms
                  ? new Date(Number(credential.expires_at_ms)).toLocaleString()
                  : "No expiry"
              }}
            </td>
            <td>
              <button
                v-if="!credential.revoked"
                class="quiet"
                :disabled="busy"
                :aria-label="`Revoke ${credential.id}`"
                @click="revokeId = credential.id"
              >
                Revoke
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
      Load more credentials
    </button>
  </section>
</template>
