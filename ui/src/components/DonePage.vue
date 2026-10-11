<script setup lang="ts">
// The done page: on the left the name, the speakers, the chapters and what
// to do next; on the right the player and the transcript, one paragraph per
// turn, with edit, next speaker and delete on hover.
import { computed, nextTick, ref, watch } from "vue";
import { api, SPEAKER_COLOURS, type MeetingData, type Options, type ParagraphRow, type Status } from "../api";
import Player from "./Player.vue";

const props = defineProps<{ status: Status; options: Options | null; meeting: MeetingData }>();
const emit = defineEmits<{ toast: [message: string, undo?: () => void] }>();

const player = ref<InstanceType<typeof Player> | null>(null);
const nowMs = ref(-1);
const title = ref(props.meeting.title);
const names = ref(props.meeting.speakers.map((s) => s.name));
const language = ref(props.meeting.language);
const editing = ref<number | null>(null);
const draft = ref("");

watch(
  () => props.meeting,
  (m) => {
    if (document.activeElement?.id !== "done-name") title.value = m.title;
    if (!(document.activeElement as HTMLElement | null)?.dataset?.speaker) names.value = m.speakers.map((s) => s.name);
    language.value = m.language;
  },
);

const ok = computed(() => !props.status.problem);

// Paragraphs and chapter headings in order.
const rows = computed(() => {
  const out: ({ kind: "chapter"; title: string; start_ms: number } | { kind: "line"; p: ParagraphRow; i: number })[] = [];
  let next = 0;
  const chapters = props.meeting.chapters;
  props.meeting.paragraphs.forEach((p, i) => {
    while (next < chapters.length && chapters[next].start_ms <= p.start_ms) {
      out.push({ kind: "chapter", title: chapters[next].title, start_ms: chapters[next].start_ms });
      next++;
    }
    out.push({ kind: "line", p, i });
  });
  return out;
});

const current = computed(() => {
  let found = -1;
  props.meeting.paragraphs.forEach((p, i) => {
    if (p.start_ms <= nowMs.value) found = i;
  });
  return found;
});

const currentChapter = computed(() => {
  let found = -1;
  props.meeting.chapters.forEach((c, i) => {
    if (c.start_ms <= nowMs.value) found = i;
  });
  return found;
});

watch(current, async (i) => {
  if (i < 0) return;
  await nextTick();
  document.getElementById(`p-${i}`)?.scrollIntoView({ block: "nearest", behavior: "smooth" });
});

async function saveTitle() {
  if (title.value.trim() && title.value !== props.meeting.title) {
    await api.setTitle(title.value).catch((e) => emit("toast", String(e)));
  }
}

async function saveNames() {
  const merged = await api.renameSpeakers(names.value).catch((e) => {
    emit("toast", String(e));
    return null;
  });
  if (merged === true) emit("toast", "Merged into one speaker");
}

function startEdit(i: number) {
  editing.value = i;
  draft.value = props.meeting.paragraphs[i].text;
  nextTick(() => (document.getElementById(`edit-${i}`) as HTMLTextAreaElement | null)?.focus());
}

async function saveEdit(p: ParagraphRow) {
  editing.value = null;
  if (draft.value.trim() === p.text) return;
  await api.editParagraph(p.sources, p.time, p.speaker, draft.value).catch((e) => emit("toast", String(e)));
}

async function remove(p: ParagraphRow) {
  const before = await api.deleteParagraph(p.sources).catch((e) => {
    emit("toast", String(e));
    return null;
  });
  if (before !== null) emit("toast", "Line deleted", () => api.restoreTranscript(before));
}

async function swap(p: ParagraphRow) {
  await api.swapSpeaker(p.sources, p.speaker).catch((e) => emit("toast", String(e)));
}

async function copy() {
  await api.copyText(props.meeting.markdown);
  emit("toast", "Transcript copied");
}

async function again() {
  await api.transcribeAgain(language.value).catch((e) => emit("toast", String(e)));
}

function onKey(e: KeyboardEvent) {
  const target = e.target as HTMLElement;
  if (e.key === "Enter" && !["INPUT", "TEXTAREA", "SELECT", "BUTTON"].includes(target.tagName)) copy();
}
</script>

<template>
  <div class="flex h-full gap-5 px-6 pb-6" tabindex="-1" @keydown="onKey">
    <aside class="flex w-80 shrink-0 flex-col gap-4 overflow-y-auto">
      <div class="flex items-start gap-3">
        <svg v-if="ok" class="mt-1.5 shrink-0 text-accent" width="20" height="20" viewBox="0 0 20 20">
          <path d="M4 10.5l4 4 8-9" stroke="currentColor" stroke-width="2.2" fill="none" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        <svg v-else class="mt-1.5 shrink-0 text-danger" width="20" height="20" viewBox="0 0 20 20">
          <path d="M10 3l8 14H2zM10 8v4M10 14.5v.5" stroke="currentColor" stroke-width="1.8" fill="none" stroke-linecap="round" />
        </svg>
        <div>
          <h1 class="text-2xl font-semibold">{{ ok ? "Transcript ready" : "Meeting saved" }}</h1>
          <p class="text-sm text-muted">{{ meeting.meta.join("  ·  ") }}</p>
        </div>
      </div>

      <div class="overflow-hidden rounded-xl border border-line bg-card">
        <label class="block border-b border-line px-4 py-2.5">
          <span class="block text-xs text-muted">Meeting name</span>
          <input
            id="done-name"
            v-model="title"
            class="w-full bg-transparent text-[15px] outline-none"
            @keydown.enter="($event.target as HTMLInputElement).blur()"
            @blur="saveTitle"
          />
        </label>
        <label
          v-for="(s, i) in meeting.speakers"
          :key="i"
          class="flex items-center gap-3 border-b border-line px-4 py-2.5 last:border-b-0"
        >
          <span class="h-2.5 w-2.5 shrink-0 rounded-full" :style="{ background: SPEAKER_COLOURS[i % SPEAKER_COLOURS.length] }" />
          <span class="min-w-0 flex-1">
            <span class="block text-xs text-muted">{{ s.role }}</span>
            <input
              v-model="names[i]"
              :data-speaker="i"
              class="w-full bg-transparent text-[15px] outline-none"
              @keydown.enter="($event.target as HTMLInputElement).blur()"
              @blur="saveNames"
            />
          </span>
        </label>
      </div>

      <section v-if="meeting.chapters.length || options?.agent">
        <div class="mb-1.5 flex items-center justify-between">
          <h2 class="text-sm font-semibold">Chapters</h2>
          <button
            v-if="options?.agent"
            class="text-xs font-medium text-muted hover:text-ink disabled:opacity-40"
            :disabled="status.chapters_busy || !meeting.can_have_chapters"
            @click="api.makeChapters()"
          >
            {{ meeting.chapters.length ? "Redo" : "Generate" }}
          </button>
        </div>
        <p class="mb-1.5 text-xs text-muted">
          {{
            meeting.chapters_note ??
            (meeting.can_have_chapters
              ? `Let ${options?.agent} divide the meeting into chapters`
              : "For meetings of three minutes or more")
          }}
        </p>
        <div v-if="meeting.chapters.length" class="overflow-hidden rounded-xl border border-line bg-card">
          <button
            v-for="(c, i) in meeting.chapters"
            :key="c.start_ms"
            class="flex w-full items-baseline gap-3 border-b border-line px-4 py-2.5 text-left last:border-b-0 hover:bg-card-2"
            :class="i === currentChapter ? 'bg-row-active' : ''"
            @click="player?.playFrom(c.start_ms)"
          >
            <span class="text-sm text-muted tabular">{{ c.clock }}</span>
            <span class="text-sm">{{ c.title }}</span>
          </button>
        </div>
      </section>

      <div class="mt-auto flex flex-col gap-2.5">
        <button
          class="rounded-full bg-accent py-3 text-[15px] font-semibold text-on-accent hover:bg-accent-hover"
          @click="copy"
        >
          Copy transcript
        </button>
        <div class="grid grid-cols-2 gap-2.5">
          <button class="rounded-full bg-card-2 py-2.5 text-sm font-semibold hover:bg-line" @click="api.openFolder(meeting.dir).catch((e) => emit('toast', String(e)))">
            Open folder
          </button>
          <button class="rounded-full bg-card-2 py-2.5 text-sm font-semibold hover:bg-line" @click="api.newRecording()">
            New recording
          </button>
        </div>
        <div class="flex items-center justify-between gap-3 rounded-xl border border-line bg-card px-4 py-2.5">
          <div>
            <div class="text-sm">Language</div>
            <div class="text-xs text-muted">Transcribe again</div>
          </div>
          <div class="flex items-center gap-2">
            <select v-model="language" class="rounded-md bg-card-2 px-2 py-1 text-sm outline-none">
              <option v-for="l in options?.languages ?? []" :key="l.key" :value="l.key">{{ l.label }}</option>
            </select>
            <button class="rounded-full p-1.5 hover:bg-card-2" aria-label="Transcribe again" @click="again">
              <svg width="16" height="16" viewBox="0 0 16 16"><path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5v3h-3" stroke="currentColor" stroke-width="1.6" fill="none" stroke-linecap="round" stroke-linejoin="round" /></svg>
            </button>
          </div>
        </div>
      </div>
    </aside>

    <main class="flex min-w-0 flex-1 flex-col gap-3">
      <Player
        v-if="meeting.audio.length"
        ref="player"
        :audio="meeting.audio"
        :chapters="meeting.chapters"
        :dir="meeting.dir"
        @time="(ms) => (nowMs = ms)"
      />
      <div class="selectable min-h-0 flex-1 overflow-y-auto rounded-xl border border-line bg-card px-2 py-2">
        <p v-for="n in meeting.notes" :key="n" class="px-3 py-2 text-sm italic text-muted">{{ n }}</p>
        <p v-if="status.problem" class="px-3 py-2 text-sm text-danger">{{ status.problem }}</p>
        <template v-for="row in rows" :key="row.kind === 'line' ? `p${row.i}` : `c${row.start_ms}`">
          <h3 v-if="row.kind === 'chapter'" class="px-3 pt-4 pb-1 text-[15px] font-semibold">{{ row.title }}</h3>
          <div
            v-else
            :id="`p-${row.i}`"
            class="group grid grid-cols-[3.5rem_auto_1fr_auto] gap-x-3 rounded-lg px-3 py-2"
            :class="row.i === current ? 'bg-row-active' : 'hover:bg-card-2'"
            @click="editing === null && player?.playFrom(row.p.start_ms)"
          >
            <span class="text-sm text-muted tabular">{{ row.p.time }}</span>
            <span class="text-sm font-semibold" :style="{ color: SPEAKER_COLOURS[row.p.colour % SPEAKER_COLOURS.length] }">
              {{ row.p.speaker }}
            </span>
            <textarea
              v-if="editing === row.i"
              :id="`edit-${row.i}`"
              v-model="draft"
              rows="3"
              class="w-full resize-y rounded-md bg-bg p-1.5 text-sm outline-none"
              @click.stop
              @keydown.enter.exact.prevent="saveEdit(row.p)"
              @keydown.esc="editing = null"
              @blur="saveEdit(row.p)"
            />
            <span v-else class="text-sm leading-relaxed">{{ row.p.text }}</span>
            <span class="flex gap-0.5 opacity-0 group-hover:opacity-100" @click.stop>
              <button class="rounded p-1 text-muted hover:bg-line hover:text-ink" title="Edit" @click="startEdit(row.i)">
                <svg width="14" height="14" viewBox="0 0 14 14"><path d="M9.5 2.5l2 2L5 11H3V9z" stroke="currentColor" stroke-width="1.3" fill="none" stroke-linejoin="round" /></svg>
              </button>
              <button
                v-if="meeting.speakers.length > 1"
                class="rounded p-1 text-muted hover:bg-line hover:text-ink"
                title="Give to the next speaker"
                @click="swap(row.p)"
              >
                <svg width="14" height="14" viewBox="0 0 14 14"><path d="M2 5h9l-2.5-2.5M12 9H3l2.5 2.5" stroke="currentColor" stroke-width="1.3" fill="none" stroke-linecap="round" stroke-linejoin="round" /></svg>
              </button>
              <button class="rounded p-1 text-muted hover:bg-line hover:text-danger" title="Delete" @click="remove(row.p)">
                <svg width="14" height="14" viewBox="0 0 14 14"><path d="M3 4h8M5.5 4V2.5h3V4M4 4l.5 8h5L10 4" stroke="currentColor" stroke-width="1.3" fill="none" stroke-linecap="round" stroke-linejoin="round" /></svg>
              </button>
            </span>
          </div>
        </template>
      </div>
    </main>
  </div>
</template>
