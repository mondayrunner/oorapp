<script setup lang="ts">
// The window: a sidebar with History, Record and Transcribe file, and the
// page on the right. The compact strip replaces it all while recording.
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  api,
  clock,
  keys,
  on,
  type Levels,
  type MeetingData,
  type MeetingEntry,
  type Options,
  type Status,
  type Unfinished,
} from "./api";
import StartPage from "./components/StartPage.vue";
import Transcribing from "./components/Transcribing.vue";
import DonePage from "./components/DonePage.vue";
import Meter from "./components/Meter.vue";
import HistoryPage from "./components/HistoryPage.vue";
import TranscribeFile from "./components/TranscribeFile.vue";

type View = "history" | "record" | "file";

const status = ref<Status | null>(null);
const options = ref<Options | null>(null);
const levels = ref<Levels>({ mic: [], computer: [], elapsed: 0 });
const meeting = ref<MeetingData | null>(null);
const unfinished = ref<Unfinished[]>([]);
const stats = ref({ words: 0, sessions: 0, saved: 0 });
const view = ref<View>("record");
const historyRefresh = ref(0);
const update = ref<{ version: string; notes: string | null } | null>(null);
const updating = ref(false);
const claudeOpen = ref(false);

async function connect(target: "code" | "desktop") {
  try {
    toast(await api.connectClaude(target));
  } catch (e) {
    toast(String(e));
  }
}
const compact = ref(false);
const dropping = ref(false);
const importing = ref<{ path: string; language: string; speakers: number | null } | null>(null);
const toasts = ref<{ id: number; message: string; undo?: () => void; url?: string }[]>([]);
const win = getCurrentWebviewWindow();
const isMac = navigator.userAgent.includes("Mac");
let toastId = 0;
const unlisten: UnlistenFn[] = [];

const phase = computed(() => status.value?.phase ?? "idle");
const busy = computed(() => ["recording", "saving", "transcribing"].includes(phase.value));

function toast(message: string, undo?: () => void, url?: string) {
  const id = ++toastId;
  toasts.value.push({ id, message, undo, url });
  setTimeout(() => (toasts.value = toasts.value.filter((t) => t.id !== id)), undo || url ? 8000 : 3500);
}

async function refreshMeeting() {
  meeting.value = await api.meeting();
  historyRefresh.value++;
}

async function refreshUnfinished() {
  unfinished.value = phase.value === "idle" ? await api.unfinished() : [];
}

async function refreshStats() {
  const all: MeetingEntry[] = await api.meetings().catch(() => []);
  const words = all.reduce((n, e) => n + e.words, 0);
  // Typing at 40 words a minute, against the time it took to say a note.
  const saved = all.reduce(
    (s, e) => s + Math.max(0, (e.words / 40) * 60 - (e.kind === "note" ? e.duration_secs : 0)),
    0,
  );
  stats.value = { words, sessions: all.length, saved };
}

function short(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}m`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}
function hours(secs: number) {
  return secs >= 3600 ? `${Math.round(secs / 3600)}h` : `${Math.round(secs / 60)}m`;
}

// Wide with the sidebar; a small strip in compact mode.
async function fit() {
  if (compact.value) {
    await win.setMinSize(new LogicalSize(300, 56));
    await win.setSize(new LogicalSize(380, 64));
    await win.setAlwaysOnTop(true);
  } else {
    await win.setAlwaysOnTop(false);
    await win.setMinSize(new LogicalSize(820, 600));
    const size = await win.innerSize();
    const scale = await win.scaleFactor();
    if (size.width / scale < 820 || size.height / scale < 600) await win.setSize(new LogicalSize(1180, 800));
  }
}

watch(phase, async (p, before) => {
  if (p !== "recording") compact.value = false;
  if (p !== "idle" && p !== before) view.value = "record";
  if (p === "done" || before === "done") await refreshMeeting();
  if (p === "idle") await refreshUnfinished();
  if (before && before !== p) {
    historyRefresh.value++;
    refreshStats();
  }
});
watch(
  () => status.value?.note_busy,
  (now, before) => {
    if (before && !now) {
      historyRefresh.value++;
      refreshStats();
    }
  },
);
watch(compact, fit);

// Leaving a meeting closes it: the next visit to Record starts fresh.
watch(view, (v, before) => {
  if (before === "record" && v !== "record" && phase.value === "done") api.newRecording();
});

async function chooseImport() {
  const path = await open({
    multiple: false,
    filters: [
      {
        name: "Audio and video",
        extensions: ["mp3", "m4a", "wav", "ogg", "opus", "flac", "aac", "mp4", "mov", "caf", "mkv", "webm"],
      },
    ],
  });
  if (typeof path === "string") askImport(path);
}

function askImport(path: string) {
  if (path.endsWith(".oorapp") || path.endsWith(".meeting-recorder")) {
    openMeeting(path);
    return;
  }
  if (busy.value) {
    toast("Finish the current recording first");
    return;
  }
  importing.value = { path, language: status.value?.language ?? "auto", speakers: null };
}

async function confirmImport() {
  const i = importing.value;
  importing.value = null;
  if (i) await api.importFile(i.path, i.language, i.speakers).catch((e) => toast(String(e)));
}

async function openMeeting(path: string) {
  if (busy.value) {
    toast("Finish the current recording first");
    return;
  }
  await api.openMeeting(path).catch((e) => toast(String(e)));
  view.value = "record";
}

async function recover(u: Unfinished, save: boolean) {
  if (save) await api.recover(u.path).catch((e) => toast(String(e)));
  else await api.discard(u.path).catch((e) => toast(String(e)));
  await refreshUnfinished();
}

const noteHint = computed(() => {
  const o = options.value;
  if (!o) return "";
  const label = o.note_shortcuts.find((c) => c.key === o.note_shortcut)?.label ?? keys(o.note_shortcut, o.platform === "macos");
  const hold = label.startsWith("Hold ");
  const key = hold ? label.slice(5) : keys(o.note_shortcut, o.platform === "macos");
  return hold
    ? `Hold ${key} anywhere, speak and let go: the text is pasted where your cursor is. A short tap keeps listening until the next tap.`
    : `Hold ${key} anywhere, speak and let go: the text is pasted where your cursor is.`;
});

async function installUpdate() {
  updating.value = true;
  try {
    await api.installUpdate();
  } catch (e) {
    toast(String(e));
  } finally {
    updating.value = false;
  }
}

async function setNoteKey(value: string) {
  try {
    await api.setNoteShortcut(value);
    if (options.value) options.value.note_shortcut = value;
    toast(value === "none" ? "Notes are off" : "Note key saved");
  } catch (e) {
    toast(String(e));
  }
}

function onKey(e: KeyboardEvent) {
  const mod = isMac ? e.metaKey : e.ctrlKey;
  if (!mod || compact.value) return;
  const key = e.key.toLowerCase();
  if (key === "1" || key === "o") view.value = "history";
  else if (key === "2") view.value = "record";
  else if (key === "3") view.value = "file";
  else if (key === "m" && phase.value === "recording") compact.value = true;
  else return;
  e.preventDefault();
}

onMounted(async () => {
  options.value = await api.options();
  status.value = await api.status();
  unlisten.push(await on<Status>("status", (s) => (status.value = s)));
  unlisten.push(await on<Levels>("levels", (l) => (levels.value = l)));
  unlisten.push(await on("meeting", refreshMeeting));
  unlisten.push(await on<string>("toast", (m) => toast(m)));
  unlisten.push(await on<{ version: string; notes: string | null }>("update", (u) => (update.value = u)));
  unlisten.push(
    await on<{ message: string; url: string | null }>("action", (a) => toast(a.message, undefined, a.url ?? undefined)),
  );
  unlisten.push(
    await win.onDragDropEvent((e) => {
      if (e.payload.type === "enter" || e.payload.type === "over") dropping.value = !busy.value;
      else if (e.payload.type === "leave") dropping.value = false;
      else if (e.payload.type === "drop") {
        dropping.value = false;
        const path = e.payload.paths[0];
        if (path) askImport(path);
      }
    }),
  );
  if (phase.value === "done") await refreshMeeting();
  await refreshUnfinished();
  await refreshStats();
  await fit();
  window.addEventListener("keydown", onKey);
});

onBeforeUnmount(() => {
  unlisten.forEach((u) => u());
  window.removeEventListener("keydown", onKey);
});

const modelSize = computed(() => {
  const m = status.value?.model_missing;
  if (!m) return "";
  return m[1] >= 1000 ? `${(m[1] / 1000).toFixed(1)} GB` : `${m[1]} MB`;
});

const title = computed(() => {
  if (view.value === "history") return "History";
  if (view.value === "file") return "Transcribe file";
  const titles = {
    idle: "Meeting",
    recording: "Recording",
    saving: "Saving",
    transcribing: "Transcribing",
    done: meeting.value?.title ?? "Meeting",
  };
  return titles[phase.value];
});

const nav: { key: View; label: string }[] = [
  { key: "history", label: "History" },
  { key: "record", label: "Meeting" },
  { key: "file", label: "Transcribe file" },
];
</script>

<template>
  <div v-if="status" class="h-full">
    <!-- Compact strip: the clock and both waves, draggable. -->
    <div v-if="compact" class="flex h-full items-center gap-3 px-3" data-tauri-drag-region>
      <span class="h-2.5 w-2.5 shrink-0 rounded-full bg-accent" :class="status.paused ? 'opacity-40' : 'animate-pulse'" />
      <span class="text-sm font-semibold tabular" data-tauri-drag-region>{{ clock(levels.elapsed) }}</span>
      <div class="grid min-w-0 flex-1 grid-rows-2 gap-1 [&_.h-20]:h-5" data-tauri-drag-region>
        <Meter :levels="levels.mic" colour="var(--mic)" recording :paused="status.paused" label="Microphone" />
        <Meter :levels="levels.computer" colour="var(--computer)" recording :paused="status.paused" label="Computer audio" />
      </div>
      <button class="rounded p-1 text-muted hover:text-ink" title="Full window (⌘M)" @click="compact = false">
        <svg width="14" height="14" viewBox="0 0 14 14"><path d="M8 2h4v4M6 12H2V8M12 2L8 6M2 12l4-4" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linecap="round" /></svg>
      </button>
    </div>

    <div v-else class="flex h-full">
      <aside class="flex w-60 shrink-0 flex-col border-r border-line bg-card px-3 pb-3" data-tauri-drag-region>
        <div class="h-11 shrink-0" data-tauri-drag-region />
        <div class="mb-4 flex items-center gap-2 px-2" data-tauri-drag-region>
          <svg width="22" height="22" viewBox="0 0 22 22" data-tauri-drag-region>
            <rect width="22" height="22" rx="6" fill="#1c1917" />
            <g fill="#fe5c45"><rect x="4" y="8" width="2" height="2.5" rx="1" /><rect x="7.5" y="5" width="2" height="5.5" rx="1" /><rect x="11" y="6.5" width="2" height="4" rx="1" /><rect x="14.5" y="4" width="2" height="6.5" rx="1" /></g>
            <g fill="#fd9046"><rect x="4" y="11.5" width="2" height="2" rx="1" /><rect x="7.5" y="11.5" width="2" height="4.5" rx="1" /><rect x="11" y="11.5" width="2" height="6" rx="1" /><rect x="14.5" y="11.5" width="2" height="3" rx="1" /></g>
          </svg>
          <span class="text-[17px] font-bold" data-tauri-drag-region>Oorapp</span>
        </div>
        <nav class="flex flex-col gap-0.5">
          <button
            v-for="item in nav"
            :key="item.key"
            class="flex items-center gap-3 rounded-lg px-3 py-2 text-left text-sm font-medium"
            :class="view === item.key ? 'bg-row-active text-ink' : 'text-muted hover:bg-card-2 hover:text-ink'"
            @click="view = item.key"
          >
            <svg v-if="item.key === 'history'" width="17" height="17" viewBox="0 0 17 17"><path d="M2.5 8.5a6 6 0 1 0 1.8-4.3M2.5 2.5v2.5H5" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linecap="round" stroke-linejoin="round" /><path d="M8.5 5.5v3l2 1.3" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linecap="round" /></svg>
            <svg v-else-if="item.key === 'record'" width="17" height="17" viewBox="0 0 17 17"><rect x="6" y="2" width="5" height="8.5" rx="2.5" stroke="currentColor" stroke-width="1.4" fill="none" /><path d="M3.5 8a5 5 0 0 0 10 0M8.5 13v2" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linecap="round" /></svg>
            <svg v-else width="17" height="17" viewBox="0 0 17 17"><path d="M4 2h6l3.5 3.5V15H4z" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linejoin="round" /><path d="M10 2v3.5h3.5" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linejoin="round" /></svg>
            <span class="flex-1">{{ item.label }}</span>
            <span v-if="item.key === 'record' && phase === 'recording'" class="flex items-center gap-1.5 text-xs tabular text-accent">
              <span class="h-1.5 w-1.5 rounded-full bg-accent" :class="status.paused ? 'opacity-40' : 'animate-pulse'" />{{ clock(levels.elapsed) }}
            </span>
            <span
              v-else-if="item.key === 'record' && (phase === 'saving' || phase === 'transcribing')"
              class="text-xs tabular text-muted"
            >
              {{ Math.round(status.progress * 100) }}%
            </span>
          </button>
        </nav>

        <div class="mt-auto flex flex-col gap-3">
          <div v-if="update" class="rounded-xl border border-accent/40 bg-row-active px-3 py-2.5 text-xs">
            <p class="font-semibold text-ink">Oorapp {{ update.version }} is ready</p>
            <button
              class="mt-1.5 w-full rounded-lg bg-accent py-1.5 font-semibold text-on-accent disabled:opacity-60"
              :disabled="updating || busy"
              @click="installUpdate"
            >
              {{ updating ? "Updating…" : "Install and restart" }}
            </button>
          </div>
          <button
            class="flex items-center justify-center gap-2 rounded-xl py-2.5 text-sm font-semibold"
            :class="status.note !== null ? 'bg-accent text-on-accent' : 'bg-card-2 hover:bg-line'"
            :disabled="status.note_busy || phase === 'recording'"
            @click="api.toggleNote()"
          >
            <span v-if="status.note !== null" class="h-2 w-2 animate-pulse rounded-full bg-on-accent" />
            {{ status.note !== null ? `Stop note ${clock(status.note)}` : status.note_busy ? "Writing…" : "New note" }}
          </button>
          <div v-if="options" class="px-1 text-xs leading-relaxed text-muted">
            <label class="mb-1.5 flex items-center justify-between gap-2">
              <span class="font-semibold text-ink">Note key</span>
              <select
                class="min-w-0 rounded-md bg-card-2 px-1.5 py-1 text-xs text-ink outline-none"
                :value="options.note_shortcut"
                @change="setNoteKey(($event.target as HTMLSelectElement).value)"
              >
                <option v-for="c in options.note_shortcuts" :key="c.key" :value="c.key">{{ c.label }}</option>
              </select>
            </label>
            <div
              v-if="options.note_shortcut.toLowerCase() !== 'none' && (!status.note_permissions.keys || !status.note_permissions.paste)"
              class="mb-2 rounded-lg border border-accent/40 bg-row-active px-2.5 py-2 text-ink"
            >
              <p v-if="!status.note_permissions.keys" class="mb-1.5">
                Oorapp only hears the note key inside its own window.
                <button class="font-semibold text-accent underline-offset-2 hover:underline" @click="api.requestPermission('keys')">
                  Allow Input Monitoring
                </button>
              </p>
              <p v-if="!status.note_permissions.paste">
                To paste into other apps Oorapp needs
                <button class="font-semibold text-accent underline-offset-2 hover:underline" @click="api.requestPermission('paste')">
                  Accessibility
                </button>
              </p>
            </div>
            <template v-if="status.note !== null">
              <span class="font-semibold text-accent">● Speaking a note</span>, let go to paste it.
            </template>
            <template v-else-if="status.note_busy">Turning the note into text…</template>
            <template v-else-if="options.note_shortcut.toLowerCase() !== 'none'">
              {{ noteHint }}
            </template>
          </div>
          <div class="relative">
            <button class="w-full rounded-lg px-3 py-1.5 text-left text-xs font-semibold text-muted hover:bg-card-2 hover:text-ink" @click="claudeOpen = !claudeOpen">
              Use with Claude…
            </button>
            <div v-if="claudeOpen" class="absolute bottom-full left-0 z-20 mb-2 w-64 rounded-xl border border-line bg-card p-3 text-xs shadow-xl">
              <p class="mb-2 leading-relaxed text-muted">Let Claude search and read your meetings and notes. It runs on this Mac; nothing is uploaded until you ask Claude something.</p>
              <button class="mb-1.5 w-full rounded-lg bg-card-2 py-1.5 font-semibold hover:bg-line" @click="connect('code'); claudeOpen = false">Connect Claude Code</button>
              <button class="w-full rounded-lg bg-card-2 py-1.5 font-semibold hover:bg-line" @click="connect('desktop'); claudeOpen = false">Connect Claude Desktop</button>
            </div>
          </div>
          <div class="grid grid-cols-3 rounded-xl border border-line px-2 py-2.5 text-center">
            <div>
              <div class="text-[17px] font-semibold tabular">{{ short(stats.words) }}</div>
              <div class="text-[11px] text-muted">words</div>
            </div>
            <div class="border-x border-line">
              <div class="text-[17px] font-semibold tabular">{{ short(stats.sessions) }}</div>
              <div class="text-[11px] text-muted">sessions</div>
            </div>
            <div>
              <div class="text-[17px] font-semibold tabular">{{ hours(stats.saved) }}</div>
              <div class="text-[11px] text-muted">saved</div>
            </div>
          </div>
        </div>
      </aside>

      <main class="relative flex min-w-0 flex-1 flex-col">
        <header
          class="flex h-12 shrink-0 items-center px-6"
          :class="view === 'record' && (phase === 'transcribing' || phase === 'saving') ? 'absolute inset-x-0 top-0 z-10 text-white' : ''"
          data-tauri-drag-region
        >
          <h1 class="truncate text-[15px] font-semibold" data-tauri-drag-region>{{ title }}</h1>
          <button
            v-if="view === 'record' && phase === 'recording'"
            class="ml-auto rounded p-1 text-muted hover:text-ink"
            :title="isMac ? 'Compact (⌘M)' : 'Compact (Ctrl+M)'"
            @click="compact = true"
          >
            <svg width="14" height="14" viewBox="0 0 14 14"><path d="M12 6H8V2M2 8h4v4M8 6l4-4M6 8l-4 4" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linecap="round" /></svg>
          </button>
        </header>

        <template v-if="view === 'record'">
          <div
            v-if="status.model_missing && phase === 'idle'"
            class="mx-auto mb-4 flex w-full max-w-md items-center justify-between gap-3 rounded-xl bg-card-2 px-4 py-3 text-sm"
          >
            <span v-if="status.model_download === null">
              The speech model ({{ status.model_missing[0] }}, {{ modelSize }}) is needed to transcribe
            </span>
            <span v-else>Downloading the speech model… {{ Math.round(status.model_download * 100) }}%</span>
            <button
              v-if="status.model_download === null"
              class="shrink-0 rounded-full bg-accent px-3 py-1 font-semibold text-on-accent"
              @click="api.downloadModel()"
            >
              Download
            </button>
          </div>
          <div
            v-for="u in unfinished"
            :key="u.path"
            class="mx-auto mb-4 w-full max-w-md rounded-xl border border-line bg-card px-4 py-3 text-sm"
          >
            <p class="font-semibold">Unfinished recording found</p>
            <p class="mt-0.5 text-muted">
              {{ u.title }}, {{ new Date(u.started_at * 1000).toLocaleString() }} ({{ clock(u.seconds) }}), was not
              stopped properly. Save it as a meeting?
            </p>
            <div class="mt-2 flex gap-2">
              <button class="rounded-full bg-accent px-3 py-1 font-semibold text-on-accent" @click="recover(u, true)">Save</button>
              <button class="rounded-full bg-card-2 px-3 py-1 font-semibold" @click="unfinished = unfinished.filter((x) => x !== u)">
                Later
              </button>
              <button class="rounded-full px-3 py-1 font-semibold text-danger" @click="recover(u, false)">Discard</button>
            </div>
          </div>
        </template>

        <div class="min-h-0 flex-1">
          <HistoryPage v-if="view === 'history'" :refresh="historyRefresh" @toast="(m) => toast(m)" @open="openMeeting" />
          <TranscribeFile v-else-if="view === 'file'" :dropping="dropping" @choose="chooseImport" />
          <template v-else>
            <div v-if="phase === 'idle' || phase === 'recording'" class="mx-auto h-full max-w-md">
              <StartPage :status="status" :options="options" :levels="levels" @import="view = 'file'" @history="view = 'history'" />
            </div>
            <Transcribing
              v-else-if="phase === 'saving' || phase === 'transcribing'"
              :stage="status.stage"
              :progress="status.progress"
              :lines="status.lines"
              :cancellable="phase === 'transcribing'"
              @cancel="api.cancel()"
            />
            <DonePage v-else-if="phase === 'done' && meeting" :status="status" :options="options" :meeting="meeting" @toast="toast" />
          </template>
        </div>
      </main>
    </div>

    <div
      v-if="dropping && view !== 'file'"
      class="pointer-events-none fixed inset-3 flex items-center justify-center rounded-2xl border-2 border-dashed border-accent bg-bg/80 text-lg font-semibold"
    >
      Drop to transcribe
    </div>

    <div v-if="importing" class="fixed inset-0 flex items-center justify-center bg-black/40 p-6">
      <div class="w-full max-w-sm rounded-2xl bg-card p-5 shadow-xl">
        <h2 class="text-lg font-semibold">Transcribe file</h2>
        <p class="truncate text-sm text-muted">{{ importing.path.split(/[\\/]/).pop() }}</p>
        <div class="mt-4 overflow-hidden rounded-xl border border-line">
          <label class="flex items-center justify-between border-b border-line px-4 py-2.5 text-sm">
            Language
            <select v-model="importing.language" class="rounded-md bg-card-2 px-2 py-1 outline-none">
              <option v-for="l in options?.languages ?? []" :key="l.key" :value="l.key">{{ l.label }}</option>
            </select>
          </label>
          <label class="flex items-center justify-between px-4 py-2.5 text-sm">
            <span>Speakers<span class="block text-xs text-muted">Recognized by their voices</span></span>
            <select v-model="importing.speakers" class="rounded-md bg-card-2 px-2 py-1 outline-none">
              <option :value="null">Automatic</option>
              <option v-for="n in 6" :key="n" :value="n">{{ n }}</option>
            </select>
          </label>
        </div>
        <div class="mt-5 flex justify-end gap-2">
          <button class="rounded-full bg-card-2 px-4 py-2 text-sm font-semibold" @click="importing = null">Cancel</button>
          <button class="rounded-full bg-accent px-4 py-2 text-sm font-semibold text-on-accent" @click="confirmImport">
            Transcribe
          </button>
        </div>
      </div>
    </div>

    <div class="pointer-events-none fixed inset-x-0 bottom-4 flex flex-col items-center gap-2">
      <div
        v-for="t in toasts"
        :key="t.id"
        class="pointer-events-auto flex items-center gap-3 rounded-full bg-ink px-4 py-2 text-sm text-bg shadow-lg"
      >
        {{ t.message }}
        <button v-if="t.undo" class="font-semibold text-accent" @click="t.undo(); toasts = toasts.filter((x) => x !== t)">
          Undo
        </button>
        <button v-if="t.url" class="font-semibold text-accent" @click="openUrl(t.url)">Open</button>
      </div>
    </div>
  </div>
</template>
