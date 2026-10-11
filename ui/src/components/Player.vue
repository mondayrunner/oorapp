<script setup lang="ts">
// Plays the meeting with a waveform of both sides: your side above the line,
// the other side below it. Click or drag to seek; chapters are markers.
// With separate files (mic.ogg and computer.ogg) both play in step.
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, clock, type ChapterRow, type Peaks } from "../api";

const props = defineProps<{ audio: string[]; chapters: ChapterRow[]; dir: string }>();
const emit = defineEmits<{ time: [ms: number] }>();

const players = ref<HTMLAudioElement[]>([]);
const playing = ref(false);
const position = ref(0);
const duration = ref(0);
const peaks = ref<Peaks>({ mic: [], computer: [] });
const canvas = ref<HTMLCanvasElement | null>(null);
const failed = ref(false);
const BINS = 600;

const sources = computed(() => {
  // One file plays alone; mic.ogg and computer.ogg together.
  const single = props.audio.find((a) => a.endsWith("audio.ogg"));
  return (single ? [single] : props.audio).map((p) => convertFileSrc(p));
});

async function loadPeaks() {
  peaks.value = await api.peaks(BINS).catch(() => ({ mic: [], computer: [] }));
  draw();
}

watch(() => props.dir, loadPeaks);
watch(sources, () => {
  playing.value = false;
  position.value = 0;
});

function main() {
  return players.value[0];
}

function toggle() {
  const all = players.value;
  if (!all.length) return;
  if (playing.value) all.forEach((a) => a.pause());
  else {
    const t = main().currentTime;
    all.forEach((a) => {
      a.currentTime = t;
      void a.play().catch(() => (failed.value = true));
    });
  }
}

function seek(ms: number) {
  players.value.forEach((a) => (a.currentTime = ms / 1000));
  position.value = ms / 1000;
  draw();
}

defineExpose({
  playFrom(ms: number) {
    seek(ms);
    if (!playing.value) toggle();
  },
  toggle,
});

let raf = 0;
function tick() {
  raf = requestAnimationFrame(tick);
  const a = main();
  if (!a) return;
  if (!a.paused) {
    position.value = a.currentTime;
    emit("time", a.currentTime * 1000);
    draw();
  }
}

function onLoaded(e: Event) {
  const a = e.target as HTMLAudioElement;
  if (Number.isFinite(a.duration)) duration.value = Math.max(duration.value, a.duration);
  draw();
}

function draw() {
  const el = canvas.value;
  if (!el) return;
  const dpr = window.devicePixelRatio || 1;
  const w = el.clientWidth;
  const h = el.clientHeight;
  el.width = Math.round(w * dpr);
  el.height = Math.round(h * dpr);
  const ctx = el.getContext("2d")!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const style = getComputedStyle(document.documentElement);
  const mic = style.getPropertyValue("--mic").trim();
  const computer = style.getPropertyValue("--computer").trim();
  const line = style.getPropertyValue("--line").trim();
  const text = style.getPropertyValue("--text").trim();
  const mid = h / 2;
  const played = duration.value ? position.value / duration.value : 0;
  const draws: [number[], string, 1 | -1][] = [
    [peaks.value.mic, mic, -1],
    [peaks.value.computer, computer, 1],
  ];
  // An import has one track: draw it on both sides.
  if (!peaks.value.computer.length) draws[1] = [peaks.value.mic, mic, 1];
  for (const [values, colour, dir] of draws) {
    const n = values.length;
    if (!n) continue;
    const step = w / n;
    for (let i = 0; i < n; i++) {
      const x = i * step;
      ctx.fillStyle = colour;
      ctx.globalAlpha = x / w <= played ? 1 : 0.45;
      const len = Math.max(0.5, values[i] * (mid - 3));
      ctx.fillRect(x, dir < 0 ? mid - len : mid, Math.max(1, step - 0.5), len);
    }
  }
  ctx.globalAlpha = 1;
  ctx.fillStyle = line;
  ctx.fillRect(0, mid - 0.5, w, 1);
  if (duration.value) {
    for (const c of props.chapters) {
      const x = (c.start_ms / 1000 / duration.value) * w;
      ctx.fillStyle = text;
      ctx.beginPath();
      ctx.moveTo(x - 4, 0);
      ctx.lineTo(x + 4, 0);
      ctx.lineTo(x, 5);
      ctx.fill();
    }
    ctx.fillStyle = text;
    ctx.fillRect(Math.round(played * w) - 0.5, 0, 1.5, h);
  }
}

let dragging = false;
function pointer(e: PointerEvent) {
  const el = canvas.value;
  if (!el || !duration.value) return;
  const r = el.getBoundingClientRect();
  const x = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
  seek(x * duration.value * 1000);
}
function down(e: PointerEvent) {
  dragging = true;
  (e.target as HTMLElement).setPointerCapture(e.pointerId);
  pointer(e);
}
function move(e: PointerEvent) {
  if (dragging) pointer(e);
}
function up() {
  dragging = false;
}

const hoverChapter = ref<string | null>(null);
function hover(e: MouseEvent) {
  const el = canvas.value;
  if (!el || !duration.value) return;
  const r = el.getBoundingClientRect();
  const x = e.clientX - r.left;
  const near = props.chapters.find((c) => Math.abs((c.start_ms / 1000 / duration.value) * r.width - x) < 6);
  hoverChapter.value = near ? `${near.clock} ${near.title}` : null;
}

onMounted(() => {
  loadPeaks();
  raf = requestAnimationFrame(tick);
  window.addEventListener("resize", draw);
});
onBeforeUnmount(() => {
  cancelAnimationFrame(raf);
  window.removeEventListener("resize", draw);
  players.value.forEach((a) => a.pause());
});
</script>

<template>
  <div class="flex items-center gap-3 rounded-xl border border-line bg-card px-3 py-2">
    <button
      class="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-card-2 hover:bg-line"
      :aria-label="playing ? 'Pause' : 'Play'"
      :disabled="!sources.length"
      @click="toggle"
    >
      <svg v-if="!playing" width="14" height="14" viewBox="0 0 14 14"><path d="M3 1.5v11l9-5.5z" fill="currentColor" /></svg>
      <svg v-else width="14" height="14" viewBox="0 0 14 14"><path d="M3 1.5h3v11H3zM8 1.5h3v11H8z" fill="currentColor" /></svg>
    </button>
    <div class="relative h-16 flex-1" :title="hoverChapter ?? ''">
      <canvas
        ref="canvas"
        class="absolute inset-0 h-full w-full"
        @pointerdown="down"
        @pointermove="move"
        @pointerup="up"
        @mousemove="hover"
      />
    </div>
    <div class="w-24 shrink-0 text-right text-xs text-muted tabular">
      {{ clock(position) }} / {{ clock(duration) }}
    </div>
    <audio
      v-for="(src, i) in sources"
      :key="src"
      :ref="(el) => { if (el) players[i] = el as HTMLAudioElement }"
      :src="src"
      preload="metadata"
      @loadedmetadata="onLoaded"
      @play="playing = true"
      @pause="playing = false"
      @ended="playing = false"
      @error="failed = true"
    />
  </div>
  <p v-if="failed" class="mt-1 text-xs text-muted">This audio cannot be played here; open the folder to play it elsewhere.</p>
</template>
