<script setup lang="ts">
// Everything Oorapp made, newest first, by day: quick notes with their whole
// text and the app they went to, meetings and imported files with the start
// of the transcript. Filter by kind, search, copy, open, or move to the trash.
import { computed, onMounted, ref, watch } from "vue";
import { ask } from "@tauri-apps/plugin-dialog";
import { api, clock, type MeetingEntry } from "../api";

const props = defineProps<{ refresh: number }>();
const emit = defineEmits<{ toast: [message: string]; open: [path: string] }>();

const entries = ref<MeetingEntry[]>([]);
const loading = ref(true);
const query = ref("");
const filter = ref<"all" | "note" | "meeting" | "file" | "untranscribed">("all");
const now = ref(Date.now());

async function load() {
  entries.value = await api.meetings().catch(() => []);
  loading.value = false;
  now.value = Date.now();
}
onMounted(load);
watch(() => props.refresh, load);

const filters = [
  { key: "all", label: "All" },
  { key: "note", label: "Notes" },
  { key: "meeting", label: "Meetings" },
  { key: "file", label: "Files" },
  { key: "untranscribed", label: "Untranscribed" },
] as const;

function dateText(e: MeetingEntry) {
  const d = new Date(e.started_at * 1000);
  return `${d.toISOString().slice(0, 10)} ${d.toLocaleDateString()} ${d.toLocaleDateString(undefined, { month: "long" })}`;
}

const shown = computed(() => {
  const q = query.value.trim().toLowerCase();
  return entries.value.filter((e) => {
    if (filter.value === "untranscribed" ? e.transcribed : filter.value !== "all" && e.kind !== filter.value) return false;
    if (!q) return true;
    return `${e.title} ${e.text} ${e.target_app ?? ""} ${dateText(e)}`.toLowerCase().includes(q);
  });
});

function dayOf(e: MeetingEntry) {
  const d = new Date(e.started_at * 1000);
  const start = new Date();
  start.setHours(0, 0, 0, 0);
  const days = Math.floor((start.getTime() - new Date(d).setHours(0, 0, 0, 0)) / 86_400_000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return d.toLocaleDateString(undefined, { weekday: "long" });
  return d.toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" });
}

const groups = computed(() => {
  const out: { day: string; items: MeetingEntry[] }[] = [];
  for (const e of shown.value) {
    const day = dayOf(e);
    if (out.at(-1)?.day !== day) out.push({ day, items: [] });
    out.at(-1)!.items.push(e);
  }
  return out;
});

function ago(e: MeetingEntry) {
  const secs = Math.max(0, Math.floor((now.value - e.started_at * 1000) / 1000));
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h ago`;
  return new Date(e.started_at * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

async function copy(e: MeetingEntry) {
  try {
    await api.copyMeeting(e.path);
    emit("toast", e.kind === "note" ? "Note copied" : "Transcript copied");
  } catch (err) {
    emit("toast", String(err));
  }
}

async function trash(e: MeetingEntry) {
  const what = e.kind === "note" ? "this note" : `“${e.title}”`;
  const sure = await ask(`Move ${what} to the trash? You can put it back from the trash.`, {
    title: "Move to the trash",
    kind: "warning",
    okLabel: "Move to trash",
    cancelLabel: "Cancel",
  });
  if (!sure) return;
  try {
    await api.trashMeeting(e.path);
    entries.value = entries.value.filter((x) => x.path !== e.path);
    emit("toast", "Moved to the trash");
  } catch (err) {
    emit("toast", String(err));
  }
}

function onSearchKey(ev: KeyboardEvent) {
  if (ev.key === "Enter" && shown.value[0]) emit("open", shown.value[0].path);
  if (ev.key === "Escape") query.value = "";
}
</script>

<template>
  <div class="flex h-full flex-col">
    <div class="flex items-center gap-2 px-6 pt-2 pb-4">
      <div class="flex gap-1">
        <button
          v-for="f in filters"
          :key="f.key"
          class="rounded-lg px-3 py-1.5 text-sm font-medium"
          :class="filter === f.key ? 'bg-card-2 text-ink' : 'text-muted hover:text-ink'"
          @click="filter = f.key"
        >
          {{ f.label }}
        </button>
      </div>
      <div class="ml-auto flex w-72 items-center gap-2 rounded-xl bg-card-2 px-3 py-1.5">
        <svg width="14" height="14" viewBox="0 0 14 14" class="shrink-0 text-muted"><circle cx="6" cy="6" r="4.2" stroke="currentColor" stroke-width="1.4" fill="none" /><path d="M9.2 9.2L12.5 12.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
        <input
          v-model="query"
          class="w-full bg-transparent text-sm outline-none placeholder:text-muted"
          placeholder="Search…"
          @keydown="onSearchKey"
        />
      </div>
    </div>

    <div class="min-h-0 flex-1 overflow-y-auto px-6 pb-4">
      <p v-if="!loading && !entries.length" class="py-16 text-center text-sm text-muted">
        Nothing here yet. Record a meeting, transcribe a file, or hold the note shortcut and speak.
      </p>
      <p v-else-if="!loading && !shown.length" class="py-16 text-center text-sm text-muted">Nothing matches.</p>
      <section v-for="g in groups" :key="g.day" class="mb-5">
        <div class="mb-2 flex items-center gap-3">
          <h2 class="text-xs font-semibold text-muted">{{ g.day }}</h2>
          <div class="h-px flex-1 bg-line" />
        </div>
        <div class="flex flex-col gap-2.5">
          <article
            v-for="e in g.items"
            :key="e.path"
            class="group relative cursor-default rounded-xl border bg-card px-4 py-3 hover:border-muted/40"
            :class="e.current ? 'border-accent' : 'border-line'"
            @click="emit('open', e.path)"
          >
            <h3 v-if="e.kind !== 'note'" class="mb-1 text-[15px] font-semibold">{{ e.title }}</h3>
            <p
              class="selectable text-[15px] leading-relaxed"
              :class="e.kind === 'note' ? '' : 'line-clamp-2 text-sm text-muted'"
            >
              {{ e.text || (e.transcribed ? "Nothing was said." : "Not transcribed yet.") }}
            </p>
            <div class="mt-2 flex items-center gap-2 font-mono text-xs text-muted">
              <span>{{ ago(e) }}</span>
              <span>·</span>
              <span>{{ clock(e.duration_secs) }}</span>
              <template v-if="e.kind === 'note' && e.target_app">
                <span>·</span>
                <span>→ {{ e.target_app }}</span>
              </template>
              <template v-else-if="e.kind !== 'note'">
                <span>·</span>
                <span>{{ e.kind === "file" ? "File" : "Meeting" }}</span>
              </template>
            </div>
            <div class="absolute right-3 bottom-2.5 flex gap-1 opacity-0 group-hover:opacity-100" @click.stop>
              <button class="rounded-md p-1.5 text-muted hover:bg-card-2 hover:text-ink" title="Copy" @click="copy(e)">
                <svg width="15" height="15" viewBox="0 0 15 15"><rect x="4.5" y="4.5" width="8" height="8.5" rx="1.5" stroke="currentColor" stroke-width="1.3" fill="none" /><path d="M10.5 4.5V3a1.5 1.5 0 0 0-1.5-1.5H4A1.5 1.5 0 0 0 2.5 3v6A1.5 1.5 0 0 0 4 10.5h.5" stroke="currentColor" stroke-width="1.3" fill="none" /></svg>
              </button>
              <button class="rounded-md p-1.5 text-muted hover:bg-card-2 hover:text-danger" title="Move to the trash" @click="trash(e)">
                <svg width="15" height="15" viewBox="0 0 15 15"><path d="M3 4.5h9M6 4.5V3h3v1.5M4.2 4.5l.6 8h5.4l.6-8" stroke="currentColor" stroke-width="1.3" fill="none" stroke-linecap="round" stroke-linejoin="round" /></svg>
              </button>
            </div>
          </article>
        </div>
      </section>
    </div>
    <div class="flex items-center border-t border-line px-6 py-2 text-xs text-muted">
      {{ shown.length }} of {{ entries.length }}
    </div>
  </div>
</template>
