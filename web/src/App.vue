<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import RootView from "./components/RootView.vue";
import type {
  AccountInspection,
  BrowserLoginOptions,
  RootInspection,
  RootPage,
  CursorToken,
} from "./generated/api";
import {
  api,
  ApiFailure,
  bytes,
  errorMessage,
  segment,
  tokenSession,
} from "./api";

const account = ref<AccountInspection | null>(null);
const loginOptions = ref<BrowserLoginOptions>({ oidc_enabled: false });
const loading = ref(true);
const busy = ref(false);
const error = ref("");
const token = ref("");
const roots = ref<RootInspection[]>([]);
const cursor = ref<CursorToken | null>(null);
const route = ref(location.hash.slice(1) || "/roots");
const rootId = computed(
  () => /^\/roots\/([A-Za-z0-9_-]+)$/.exec(route.value)?.[1] ?? null,
);
let generation = 0;
function focusMain() {
  document.getElementById("main")?.focus();
}
function navigate() {
  route.value = location.hash.slice(1) || "/roots";
  error.value = "";
}
function signedOut() {
  generation++;
  account.value = null;
  roots.value = [];
  cursor.value = null;
  token.value = "";
}
async function loadRoots(more = false) {
  const current = generation;
  busy.value = true;
  error.value = "";
  try {
    const page = await api<RootPage>(
      `/api/v1/roots?limit=100${more && cursor.value ? `&cursor=${segment(cursor.value)}` : ""}`,
    );
    if (current !== generation) return;
    roots.value = more ? [...roots.value, ...page.items] : page.items;
    cursor.value = page.next_cursor;
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
async function loadSession() {
  loading.value = true;
  error.value = "";
  try {
    loginOptions.value = await api<BrowserLoginOptions>("/auth/options");
    account.value = await api<AccountInspection>("/api/v1/account");
    await loadRoots();
  } catch (failure) {
    if (!(failure instanceof ApiFailure && failure.status === 401))
      error.value = errorMessage(failure);
  } finally {
    loading.value = false;
  }
}
async function signIn() {
  const credential = token.value;
  token.value = "";
  busy.value = true;
  error.value = "";
  try {
    await tokenSession(credential);
    await loadSession();
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
async function signOut() {
  busy.value = true;
  try {
    await api<void>("/auth/logout", "POST");
    signedOut();
    location.hash = "/roots";
  } catch (failure) {
    error.value = errorMessage(failure);
  } finally {
    busy.value = false;
  }
}
onMounted(() => {
  window.addEventListener("hashchange", navigate);
  window.addEventListener("sct:unauthorized", signedOut);
  void loadSession();
});
onUnmounted(() => {
  window.removeEventListener("hashchange", navigate);
  window.removeEventListener("sct:unauthorized", signedOut);
});
</script>

<template>
  <a class="skip-link" href="#main" @click.prevent="focusMain"
    >Skip to content</a
  >
  <header class="topbar">
    <a href="#/roots" class="wordmark"
      ><img src="/brand.svg" alt="" width="36" height="36" />ScT</a
    >
    <div v-if="account" class="identity">
      <span>{{ account.principal_id }}</span
      ><span class="badge">{{
        account.administrator ? "Administrator" : "Account"
      }}</span
      ><button class="quiet" :disabled="busy" @click="signOut">Sign out</button>
    </div>
    <span v-else class="muted">Storage for your programs</span>
  </header>
  <div class="layout" :class="{ anonymous: !account }">
    <nav v-if="account" aria-label="Main navigation" class="sidebar">
      <a
        href="#/roots"
        :aria-current="route.startsWith('/roots') ? 'page' : undefined"
        >Storage roots</a
      >
      <a
        href="#/account"
        :aria-current="route === '/account' ? 'page' : undefined"
        >My account</a
      >
      <template v-if="account.administrator">
        <p class="nav-label">Administration</p>
        <a
          href="#/storage"
          :aria-current="route === '/storage' ? 'page' : undefined"
          >Storage administration</a
        >
        <a
          href="#/operations"
          :aria-current="route === '/operations' ? 'page' : undefined"
          >Operations</a
        >
      </template>
      <p class="nav-note">
        Programs create backups.<br />Browse and manage them here.
      </p>
    </nav>
    <main id="main" tabindex="-1" :aria-busy="loading">
      <p v-if="error" role="alert" class="notice error">{{ error }}</p>
      <p v-if="loading" role="status">Loading your account…</p>
      <section v-else-if="!account" class="sign-in panel">
        <p class="eyebrow">Your backup control panel</p>
        <h1>Stipa ca Truova</h1>
        <p class="intro">
          Browse saved files, download snapshots, and manage your storage.
        </p>
        <a v-if="loginOptions.oidc_enabled" class="button" href="/auth/login"
          >Sign in with your identity provider</a
        >
        <details>
          <summary>Sign in with a program or recovery credential</summary>
          <form class="stack" @submit.prevent="signIn">
            <label
              >Credential<input
                v-model="token"
                type="password"
                autocomplete="off"
                required
                spellcheck="false"
            /></label>
            <p class="muted">
              The credential is exchanged for a browser session and is not saved
              in browser storage.
            </p>
            <button :disabled="busy">
              {{ busy ? "Signing in…" : "Sign in" }}
            </button>
          </form>
        </details>
      </section>
      <template v-else-if="route === '/roots'">
        <div class="page-heading">
          <div>
            <p class="eyebrow">Your storage</p>
            <h1>Storage roots</h1>
            <p class="muted">The roots available to your account.</p>
          </div>
          <button class="quiet" :disabled="busy" @click="loadRoots()">
            Refresh
          </button>
        </div>
        <p v-if="!roots.length && !busy" class="empty">
          No storage roots are assigned to this account yet.
        </p>
        <div class="root-grid">
          <article v-for="root in roots" :key="root.id" class="panel root-card">
            <div class="row">
              <h2>
                <a :href="`#/roots/${segment(root.id)}`">{{
                  root.display_name
                }}</a>
              </h2>
              <span class="badge">{{ root.state }}</span>
            </div>
            <p class="mono">{{ root.mount_path }}</p>
            <p class="muted">
              Quota {{ bytes(root.quota_bytes) }} · {{ root.placement.kind }}
            </p>
            <p class="muted">
              {{
                root.capabilities.includes("manage_root")
                  ? "Root management available"
                  : "Access follows your assigned permissions"
              }}
            </p>
          </article>
        </div>
        <button
          v-if="cursor"
          class="quiet"
          :disabled="busy"
          @click="loadRoots(true)"
        >
          Load more roots
        </button>
      </template>
      <RootView v-else-if="rootId" :key="rootId" :root-id="rootId" />
      <section v-else-if="route === '/account'" class="panel">
        <h1>My account</h1>
        <dl>
          <dt>Account ID</dt>
          <dd class="mono">{{ account.principal_id }}</dd>
          <dt>Role</dt>
          <dd>
            {{ account.administrator ? "Administrator" : "Regular account" }}
          </dd>
        </dl>
      </section>
      <section
        v-else-if="
          account.administrator &&
          (route === '/storage' || route === '/operations')
        "
        class="panel"
      >
        <h1>
          {{ route === "/storage" ? "Storage administration" : "Operations" }}
        </h1>
        <p class="muted">This area is restricted to administrators.</p>
      </section>
      <section v-else class="panel">
        <h1>Page unavailable</h1>
        <p>You may not have access to this page.</p>
        <a href="#/roots">Return to storage roots</a>
      </section>
    </main>
  </div>
</template>
