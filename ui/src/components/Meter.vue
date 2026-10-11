<script setup lang="ts">
// One live meter. Before recording: a still line that thickens as sound comes
// in, so you can see the audio arrives. While recording: the last three
// seconds as bars around the line. Paused: the bars freeze and dim.
import { onBeforeUnmount, onMounted, ref, watch } from "vue";

const props = defineProps<{
  levels: number[];
  colour: string;
  recording: boolean;
  paused: boolean;
  label: string;
}>();

const canvas = ref<HTMLCanvasElement | null>(null);
let frame = 0;
let frozen: number[] | null = null;

watch(
  () => props.paused,
  (paused) => {
    frozen = paused ? props.levels.slice() : null;
  },
);

function resolve(colour: string): string {
  if (!colour.startsWith("var(")) return colour;
  const name = colour.slice(4, -1).trim();
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || "#fe5c45";
}

function draw() {
  frame = requestAnimationFrame(draw);
  const el = canvas.value;
  if (!el) return;
  const dpr = window.devicePixelRatio || 1;
  const w = el.clientWidth;
  const h = el.clientHeight;
  if (el.width !== Math.round(w * dpr) || el.height !== Math.round(h * dpr)) {
    el.width = Math.round(w * dpr);
    el.height = Math.round(h * dpr);
  }
  const ctx = el.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const colour = resolve(props.colour);
  const mid = h / 2;
  const levels = frozen ?? props.levels;

  if (!props.recording) {
    // A line that thickens with the last few peaks.
    const recent = levels.slice(-6);
    const peak = recent.length ? Math.max(...recent) : 0;
    const thickness = 1 + peak * (h * 0.7);
    ctx.fillStyle = colour;
    ctx.globalAlpha = 0.35 + peak * 0.65;
    ctx.beginPath();
    ctx.roundRect(8, mid - thickness / 2, w - 16, thickness, Math.min(thickness / 2, 8));
    ctx.fill();
    ctx.globalAlpha = 1;
    return;
  }

  const bar = 3;
  const gap = 2;
  const count = Math.floor((w - 8) / (bar + gap));
  const shown = levels.slice(-count);
  const offset = w - 4 - shown.length * (bar + gap);
  ctx.fillStyle = colour;
  ctx.globalAlpha = props.paused ? 0.35 : 1;
  shown.forEach((v, i) => {
    const half = Math.max(1, v * (h / 2 - 4));
    ctx.beginPath();
    ctx.roundRect(offset + i * (bar + gap), mid - half, bar, half * 2, 1.5);
    ctx.fill();
  });
  ctx.globalAlpha = 1;
}

onMounted(() => {
  frame = requestAnimationFrame(draw);
});
onBeforeUnmount(() => cancelAnimationFrame(frame));
</script>

<template>
  <div class="relative h-20 rounded-xl border border-line bg-card overflow-hidden">
    <canvas ref="canvas" class="absolute inset-0 h-full w-full" :aria-label="label" />
    <div
      v-if="!recording"
      class="absolute inset-x-0 top-1.5 text-center text-xs text-muted pointer-events-none"
    >
      Not recording
    </div>
    <div
      v-else-if="paused"
      class="absolute inset-0 flex items-center justify-center text-xs font-semibold tracking-wide text-muted pointer-events-none"
    >
      ❚❚ PAUSED
    </div>
  </div>
</template>
