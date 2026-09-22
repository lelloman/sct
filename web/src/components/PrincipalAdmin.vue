<script setup lang="ts">
import ArchiveLimits from "./ArchiveLimits.vue";
import { onMounted, ref } from "vue";
import type {
  PrincipalInspection,
  PrincipalPage,
  CursorToken,
} from "../generated/api";
import { api, errorMessage, segment } from "../api";
import CredentialList from "./CredentialList.vue";
const principals = ref<PrincipalInspection[]>([]);
const cursor = ref<CursorToken | null>(null);
const selected = ref<PrincipalInspection | null>(null);
const newId = ref("");
const disabled = ref(false);
const administrator = ref(false);
const acknowledged = ref(false);
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
async function load(more = false) {
  const page = await api<PrincipalPage>(
    `/api/v1/principals?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
  );
  principals.value = more
    ? [...principals.value, ...page.principals]
    : page.principals;
  cursor.value = page.next_cursor;
}
function select(principal: PrincipalInspection) {
  selected.value = principal;
  disabled.value = principal.disabled;
  administrator.value = principal.administrator;
  acknowledged.value = false;
}
async function create() {
  await run(async () => {
    const principal = await api<PrincipalInspection>(
      "/api/v1/principals",
      "POST",
      { id: newId.value },
    );
    newId.value = "";
    await load();
    select(principal);
    message.value = "Account created.";
  });
}
async function save() {
  if (!selected.value || !acknowledged.value) return;
  await run(async () => {
    const principal = await api<PrincipalInspection>(
      `/api/v1/principals/${segment(selected.value!.id)}`,
      "PATCH",
      {
        expected_revision: selected.value!.revision,
        disabled: disabled.value,
        administrator: administrator.value,
      },
    );
    select(principal);
    await load();
    message.value = "Account updated.";
  });
}
async function refreshSelected() {
  if (!selected.value) return;
  await run(async () =>
    select(
      await api<PrincipalInspection>(
        `/api/v1/principals/${segment(selected.value!.id)}`,
      ),
    ),
  );
}
onMounted(() => void run(() => load()));
</script>
<template>
  <h1>Accounts</h1>
  <p v-if="error" class="notice error" role="alert">{{ error }}</p>
  <p v-if="message" class="notice success" role="status">{{ message }}</p>
  <form class="panel stack" @submit.prevent="create">
    <h2>Create account</h2>
    <label
      >New account ID<input
        v-model="newId"
        required
        autocomplete="off"
        maxlength="128"
    /></label>
    <div><button :disabled="busy">Create account</button></div>
  </form>
  <section class="panel" :aria-busy="busy">
    <div class="row">
      <h2>Service accounts</h2>
      <button class="quiet" :disabled="busy" @click="run(() => load())">
        Refresh accounts
      </button>
    </div>
    <p v-if="!principals.length && !busy" class="empty">No accounts.</p>
    <div v-else class="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Account</th>
            <th>Role</th>
            <th>Status</th>
            <th>Action</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="principal in principals" :key="principal.id">
            <td class="mono">{{ principal.id }}</td>
            <td>
              {{
                principal.administrator ? "Administrator" : "Regular account"
              }}
            </td>
            <td>{{ principal.disabled ? "Disabled" : "Enabled" }}</td>
            <td>
              <button
                class="quiet"
                :disabled="busy"
                :aria-label="`Manage account ${principal.id}`"
                @click="select(principal)"
              >
                Manage
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
      Load more accounts
    </button>
  </section>
  <template v-if="selected">
    <form class="panel stack" @submit.prevent="save">
      <div class="row">
        <h2>Manage {{ selected.id }}</h2>
        <button
          type="button"
          class="quiet"
          :disabled="busy"
          @click="refreshSelected"
        >
          Reload account
        </button>
      </div>
      <label class="check"
        ><input v-model="administrator" type="checkbox" />Administrator
        access</label
      ><label class="check"
        ><input v-model="disabled" type="checkbox" />Disable account</label
      >
      <p>
        Administrator access allows service-wide management. Disabling an
        account prevents its credentials from authenticating.
      </p>
      <label class="check"
        ><input v-model="acknowledged" type="checkbox" />I acknowledge this
        account access change</label
      >
      <div>
        <button :disabled="busy || !acknowledged">Save account access</button>
      </div>
    </form>
    <ArchiveLimits
      :key="`archive-${selected.id}`"
      :principal-id="selected.id"
    />
    <CredentialList :key="selected.id" :principal-id="selected.id" />
  </template>
</template>
