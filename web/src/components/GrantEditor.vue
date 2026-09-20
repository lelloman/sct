<script setup lang="ts">
import { onMounted, ref } from "vue";
import type {
  RootInspection,
  GrantInspection,
  GrantPage,
  CursorToken,
  Capability,
} from "../generated/api";
import { api, errorMessage, segment } from "../api";
const props = defineProps<{ root: RootInspection }>();
const grants = ref<GrantInspection[]>([]);
const cursor = ref<CursorToken | null>(null);
const selected = ref<GrantInspection | null>(null);
const principal = ref("");
const capabilities = ref<Capability[]>([]);
const removal = ref<GrantInspection | null>(null);
const busy = ref(false);
const error = ref("");
const message = ref("");
const labels: Record<Capability, string> = {
  list_read: "Browse and download",
  write: "Program uploads and file changes",
  manage_policy: "Manage retention policy",
  delete: "Program file deletion",
  purge: "Program permanent purge",
  manage_grants: "Delegate permissions",
  manage_root: "Manage root settings",
};
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
  const page = await api<GrantPage>(
    `/api/v1/admin/grants?root_id=${segment(props.root.id)}&limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
  );
  grants.value = more ? [...grants.value, ...page.items] : page.items;
  cursor.value = page.next_cursor;
}
function edit(grant: GrantInspection | null) {
  selected.value = grant;
  principal.value = grant?.principal_id ?? "";
  capabilities.value = [...(grant?.capabilities ?? [])];
}
async function save() {
  await run(async () => {
    if (selected.value)
      await api<GrantInspection>(
        `/api/v1/admin/grants/${segment(selected.value.id)}`,
        "PATCH",
        {
          expected_revision: selected.value.revision,
          capabilities: capabilities.value,
        },
      );
    else
      await api<GrantInspection>("/api/v1/admin/grants", "POST", {
        root_id: props.root.id,
        principal_id: principal.value,
        capabilities: capabilities.value,
      });
    edit(null);
    await load();
    message.value = "Grant saved.";
  });
}
async function remove() {
  if (!removal.value) return;
  await run(async () => {
    await api<void>(
      `/api/v1/admin/grants/${segment(removal.value!.id)}?expected_revision=${segment(removal.value!.revision)}`,
      "DELETE",
    );
    removal.value = null;
    edit(null);
    await load();
    message.value = "Grant revoked.";
  });
}
onMounted(() => void run(() => load()));
</script>
<template>
  <section class="panel" :aria-busy="busy">
    <div class="row">
      <h2>Root grants</h2>
      <button class="quiet" :disabled="busy" @click="run(() => load())">
        Reload grants
      </button>
    </div>
    <p>
      Delegate only permissions available to your account. File changes remain
      program API operations.
    </p>
    <p v-if="error" class="notice error" role="alert">{{ error }}</p>
    <p v-if="message" class="notice success" role="status">{{ message }}</p>
    <div v-if="grants.length" class="table-scroll">
      <table>
        <thead>
          <tr>
            <th>Account</th>
            <th>Permissions</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="grant in grants" :key="grant.id">
            <td class="mono">{{ grant.principal_id }}</td>
            <td>
              {{
                grant.capabilities
                  .map((capability) => labels[capability])
                  .join(", ")
              }}
            </td>
            <td>
              <div class="actions">
                <button
                  class="quiet"
                  :disabled="busy"
                  :aria-label="`Edit grant for ${grant.principal_id}`"
                  @click="edit(grant)"
                >
                  Edit</button
                ><button
                  class="quiet"
                  :disabled="busy"
                  :aria-label="`Revoke grant for ${grant.principal_id}`"
                  @click="removal = grant"
                >
                  Revoke
                </button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <p v-else-if="!busy">No delegated grants.</p>
    <button
      v-if="cursor"
      class="quiet"
      :disabled="busy"
      @click="run(() => load(true))"
    >
      Load more grants
    </button>
    <div v-if="removal" class="notice" role="alert">
      <p>
        Revoke the grant for {{ removal.principal_id }}? Their delegated access
        will end immediately.
      </p>
      <div class="actions">
        <button class="danger" :disabled="busy" @click="remove">
          Confirm grant revocation</button
        ><button class="quiet" :disabled="busy" @click="removal = null">
          Cancel revocation
        </button>
      </div>
    </div>
    <form class="stack" @submit.prevent="save">
      <h3>{{ selected ? "Edit grant" : "Create grant" }}</h3>
      <label
        >Grant account ID<input
          v-model="principal"
          required
          :disabled="busy || !!selected"
      /></label>
      <fieldset :disabled="busy">
        <legend>Permissions</legend>
        <label
          v-for="capability in root.capabilities"
          :key="capability"
          class="check"
          ><input
            v-model="capabilities"
            :value="capability"
            type="checkbox"
          />{{ labels[capability] }}</label
        >
      </fieldset>
      <div class="actions">
        <button :disabled="busy || !capabilities.length">Save grant</button
        ><button
          v-if="selected"
          type="button"
          class="quiet"
          :disabled="busy"
          @click="edit(null)"
        >
          Cancel edit
        </button>
      </div>
    </form>
  </section>
</template>
