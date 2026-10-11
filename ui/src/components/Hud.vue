<script setup lang="ts">
// The pill at the bottom of the screen while a note is spoken: a red dot,
// your voice as a small wave and the time; then "Writing…" until the text
// is pasted.
import { onBeforeUnmount, onMounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { api, clock, on, type Levels, type Status } from "../api";

const status = ref<Status | null>(null);
const levels = ref<number[]>([]);
const canvas = ref<HTMLCanvasElement | null>(null);
const unlisten: UnlistenFn[] = [];
let frame = 0;

function draw() {
  frame = requestAnimationFrame(draw);
  const el = canvas.value;
  if (!el) return;
  const dpr = window.devicePixelRatio || 1;
  const w = el.clientWidth;
  const h = el.clientHeight;
  if (el.width !== Math.round(w * dpr)) {
    el.width = Math.round(w * dpr);
    el.height = Math.round(h * dpr);
  }
  const ctx = el.getContext("2d")!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const bar = 3;
  const gap = 2;
  const n = Math.floor(w / (bar + gap));
  const values = levels.value.slice(-n);
  ctx.fillStyle = "#fe5c45";
  values.forEach((v, i) => {
    const half = Math.max(1, v * (h / 2 - 1));
    ctx.beginPath();
    ctx.roundRect(i * (bar + gap), h / 2 - half, bar, half * 2, 1.5);
    ctx.fill();
  });
}

onMounted(async () => {
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
  status.value = await api.status();
  unlisten.push(await on<Status>("status", (s) => (status.value = s)));
  unlisten.push(await on<Levels>("levels", (l) => (levels.value = l.mic)));
  frame = requestAnimationFrame(draw);
});
onBeforeUnmount(() => {
  unlisten.forEach((u) => u());
  cancelAnimationFrame(frame);
});
</script>

<template>
  <div class="flex h-full items-center justify-center bg-transparent">
    <div class="flex h-11 w-[232px] items-center gap-2.5 rounded-full bg-[#1c1917]/95 px-4 text-[#f7f5f4] ring-1 ring-white/10">
      <template v-if="status?.note !== null && status?.note !== undefined">
        <span class="h-2.5 w-2.5 shrink-0 animate-pulse rounded-full bg-[#fe5c45]" />
        <canvas ref="canvas" class="h-6 min-w-0 flex-1" />
        <span class="text-xs font-semibold tabular">{{ clock(status.note) }}</span>
      </template>
      <template v-else>
        <svg class="h-4 w-4 shrink-0 animate-spin" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6" stroke="#fd9046" stroke-width="2" fill="none" stroke-dasharray="26 12" /></svg>
        <span class="text-sm font-medium">Writing…</span>
      </template>
    </div>
  </div>
</template>
