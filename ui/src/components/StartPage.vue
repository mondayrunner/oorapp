<script setup lang="ts">
// Ready and recording: the name, the audio format and the language, both
// meters, the clock, and Start / Pause / Stop. The name, format and language
// can still change during the call.
import { computed, ref, watch } from "vue";
import { api, clock, type Levels, type Options, type Status } from "../api";
import Meter from "./Meter.vue";

const props = defineProps<{ status: Status; options: Options | null; levels: Levels }>();
const emit = defineEmits<{ import: []; history: [] }>();

const name = ref(props.status.title);
watch(
  () => props.status.title,
  (t) => {
    if (document.activeElement?.id !== "meeting-name") name.value = t;
  },
);

const recording = computed(() => props.status.phase === "recording");
const paused = computed(() => props.status.paused);

const hint = computed(() => {
  if (recording.value && paused.value) return "Paused. Nothing is recorded until you resume.";
  if (recording.value) return "Recording. Name, audio file and language can still be changed.";
  return "Ready. Press Start recording when the meeting begins.";
});

// On macOS a process tap without permission hands over exact silence.
const permissionHint = computed(
  () => props.options?.platform === "macos" && props.status.computer_silent >= 8,
);

async function saveName() {
  await api.setTitle(name.value);
}

async function start() {
  await api.start(name.value || undefined).catch((e) => alert(e));
}
</script>

<template>
  <div class="flex h-full flex-col gap-5 px-6 pb-6">
    <div class="overflow-hidden rounded-xl border border-line bg-card">
      <label class="block border-b border-line px-4 py-2.5">
        <span class="block text-xs text-muted">Meeting name</span>
        <input
          id="meeting-name"
          v-model="name"
          class="w-full bg-transparent text-[15px] outline-none placeholder:text-muted/60"
          placeholder="Meeting 14:30"
          @keydown.enter="($event.target as HTMLInputElement).blur()"
          @blur="saveName"
        />
      </label>
      <div class="flex items-center justify-between gap-3 border-b border-line px-4 py-2.5">
        <div>
          <div class="text-[15px]">Audio file</div>
          <div class="text-xs text-muted">Can be changed during the call</div>
        </div>
        <select
          class="max-w-[55%] truncate rounded-md bg-card-2 px-2 py-1 text-sm outline-none"
          :value="status.format"
          @change="api.setFormat(($event.target as HTMLSelectElement).value)"
        >
          <option v-for="f in options?.formats ?? []" :key="f.key" :value="f.key">{{ f.label }}</option>
        </select>
      </div>
      <div class="flex items-center justify-between gap-3 px-4 py-2.5">
        <div>
          <div class="text-[15px]">Language</div>
          <div class="text-xs text-muted">Used for the transcript after the call</div>
        </div>
        <select
          class="rounded-md bg-card-2 px-2 py-1 text-sm outline-none"
          :value="status.language"
          @change="api.setLanguage(($event.target as HTMLSelectElement).value)"
        >
          <option v-for="l in options?.languages ?? []" :key="l.key" :value="l.key">{{ l.label }}</option>
        </select>
      </div>
    </div>

    <section>
      <div class="mb-1.5 flex items-baseline justify-between">
        <h2 class="text-sm font-semibold">You (microphone)</h2>
        <span class="truncate pl-3 text-xs text-muted">{{ status.mic_problem ?? status.mic_device ?? "" }}</span>
      </div>
      <Meter :levels="levels.mic" colour="var(--mic)" :recording="recording" :paused="paused" label="Microphone level" />
    </section>
    <section>
      <div class="mb-1.5 flex items-baseline justify-between">
        <h2 class="text-sm font-semibold">Computer audio</h2>
        <span class="truncate pl-3 text-xs text-muted">{{
          status.computer_problem ?? status.computer_device ?? ""
        }}</span>
      </div>
      <Meter
        :levels="levels.computer"
        colour="var(--computer)"
        :recording="recording"
        :paused="paused"
        label="Computer audio level"
      />
      <p v-if="permissionHint" class="mt-2 text-xs text-muted">
        Nothing comes in from the computer. If something is playing, allow Oorapp under System Settings,
        Privacy &amp; Security, Screen &amp; System Audio Recording.
      </p>
    </section>

    <div class="mt-1 flex flex-col items-center gap-3">
      <div class="flex items-center gap-3 text-4xl font-bold tabular">
        <span
          v-if="recording"
          class="h-3.5 w-3.5 rounded-full bg-accent"
          :class="paused ? 'opacity-40' : 'animate-pulse'"
        />
        {{ clock(recording ? levels.elapsed : 0) }}
      </div>
      <p class="text-center text-sm text-muted">{{ hint }}</p>
      <div v-if="!recording" class="flex flex-col items-center gap-3">
        <button
          class="rounded-full bg-accent px-8 py-3 text-[15px] font-semibold text-on-accent hover:bg-accent-hover"
          @click="start"
        >
          Start recording
        </button>
        <button class="text-sm font-medium text-muted hover:text-ink" @click="emit('import')">
          Transcribe a file instead
        </button>
      </div>
      <div v-else class="flex gap-3">
        <button
          class="rounded-full bg-card-2 px-7 py-3 text-[15px] font-semibold hover:bg-line"
          @click="api.togglePause()"
        >
          {{ paused ? "Resume" : "Pause" }}
        </button>
        <button
          class="rounded-full bg-accent px-7 py-3 text-[15px] font-semibold text-on-accent hover:bg-accent-hover"
          @click="api.stop()"
        >
          Stop recording
        </button>
      </div>
    </div>
  </div>
</template>
