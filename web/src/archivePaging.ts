import { onScopeDispose, ref, shallowRef, watch } from "vue";
import { api, errorMessage, segment } from "./api";
import type { CursorToken } from "./generated/api";

// Keep one bounded page in memory. A changed selection fences late responses.
export function archivePage<T>(url: () => string, field: string) {
  const items = shallowRef<T[]>([]);
  const next = ref<CursorToken | null>(null);
  const busy = ref(false);
  const error = ref("");
  const page = ref(1);
  let cursors: (CursorToken | null)[] = [null];
  let generation = 0;
  async function load(reset = false) {
    const current = ++generation;
    if (reset) {
      cursors = [null];
      page.value = 1;
      items.value = [];
      next.value = null;
    }
    const path = url();
    if (!path) {
      busy.value = false;
      return;
    }
    busy.value = true;
    error.value = "";
    try {
      const cursor = cursors[page.value - 1];
      const result = await api<Record<string, unknown>>(
        `${path}?limit=20${cursor ? `&cursor=${segment(cursor)}` : ""}`,
      );
      if (current !== generation) return;
      items.value = result[field] as T[];
      next.value = result.next_cursor as CursorToken | null;
    } catch (failure) {
      if (current === generation) {
        error.value = errorMessage(failure);
        items.value = [];
        next.value = null;
      }
    } finally {
      if (current === generation) busy.value = false;
    }
  }
  function forward() {
    if (!next.value || busy.value) return;
    cursors[page.value] = next.value;
    page.value++;
    void load();
  }
  function back() {
    if (page.value === 1 || busy.value) return;
    page.value--;
    void load();
  }
  watch(url, () => void load(true), { immediate: true });
  onScopeDispose(() => {
    generation++;
  });
  return {
    items,
    next,
    busy,
    error,
    page,
    forward,
    back,
    refresh: () => load(true),
  };
}
