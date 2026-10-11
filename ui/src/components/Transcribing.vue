<script setup lang="ts">
// The transcribing screen: a sun over a scrolling grid, the stage and the
// progress, and the lines as they are recognised, typed out. In the spirit
// of the upstream recorder's 90s animation, in Oorapp's colours.
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";

const props = defineProps<{ stage: string; progress: number; lines: string[]; cancellable: boolean }>();
const emit = defineEmits<{ cancel: [] }>();

const canvas = ref<HTMLCanvasElement | null>(null);
let frame = 0;
const started = performance.now();

// The newest line types itself out; older ones are shown whole.
const typed = ref(0);
let typing = 0;
watch(
  () => props.lines[props.lines.length - 1],
  () => {
    typed.value = 0;
    clearInterval(typing);
    typing = window.setInterval(() => {
      const last = props.lines[props.lines.length - 1] ?? "";
      if (typed.value >= last.length) clearInterval(typing);
      else typed.value += 2;
    }, 16);
  },
);

const shown = computed(() => {
  const lines = props.lines.slice(-6);
  return lines.map((line, i) => (i === lines.length - 1 ? line.slice(0, typed.value) : line));
});

const percent = computed(() => Math.round(Math.min(1, Math.max(0, props.progress)) * 100));
const blocks = 24;
const filled = computed(() => Math.round((percent.value / 100) * blocks));
const title = computed(() => (props.stage === "Done" ? "DONE" : "TRANSCRIBING"));

function cssVar(name: string) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function draw(now: number) {
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
  const t = (now - started) / 1000;
  const accent = cssVar("--accent") || "#fe5c45";
  const orange = cssVar("--computer") || "#fd9046";

  // Sky.
  const sky = ctx.createLinearGradient(0, 0, 0, h);
  sky.addColorStop(0, "#0f0d0c");
  sky.addColorStop(0.6, "#2a1714");
  sky.addColorStop(1, "#140e0c");
  ctx.fillStyle = sky;
  ctx.fillRect(0, 0, w, h);

  // Sun with bands cut out of its lower half.
  const horizon = h * 0.6;
  const r = Math.min(w * 0.3, h * 0.24);
  const cx = w / 2;
  const sun = ctx.createLinearGradient(0, horizon - r, 0, horizon);
  sun.addColorStop(0, orange);
  sun.addColorStop(1, accent);
  ctx.save();
  ctx.shadowColor = accent;
  ctx.shadowBlur = 40;
  ctx.fillStyle = sun;
  ctx.beginPath();
  ctx.arc(cx, horizon, r, Math.PI, 0);
  ctx.fill();
  ctx.restore();
  ctx.fillStyle = "#1d1210";
  for (let i = 0; i < 6; i++) {
    const y = horizon - r * 0.08 - i * r * 0.11 + ((t * 8) % (r * 0.11));
    const thick = 2 + (6 - i) * 0.9;
    if (y < horizon - r * 0.55) continue;
    ctx.fillRect(cx - r, y, r * 2, thick);
  }

  // Ground with a grid running towards you.
  ctx.fillStyle = "#120c0b";
  ctx.fillRect(0, horizon, w, h - horizon);
  ctx.strokeStyle = accent;
  ctx.globalAlpha = 0.55;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, horizon);
  ctx.lineTo(w, horizon);
  ctx.stroke();
  for (let i = -12; i <= 12; i++) {
    ctx.beginPath();
    ctx.moveTo(cx + i * 12, horizon);
    ctx.lineTo(cx + i * w * 0.18, h);
    ctx.stroke();
  }
  const speed = (t * 0.35) % 1;
  for (let i = 0; i < 12; i++) {
    const p = (i + speed) / 12;
    const y = horizon + (h - horizon) * p * p;
    ctx.globalAlpha = 0.15 + 0.5 * p;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();
  }
  ctx.globalAlpha = 1;

  // Scanlines.
  ctx.fillStyle = "rgba(0,0,0,0.18)";
  for (let y = 0; y < h; y += 3) ctx.fillRect(0, y, w, 1);
}

onMounted(() => {
  frame = requestAnimationFrame(draw);
});
onBeforeUnmount(() => {
  cancelAnimationFrame(frame);
  clearInterval(typing);
});
</script>

<template>
  <div class="relative h-full w-full overflow-hidden bg-black font-mono text-[#f7f5f4]">
    <canvas ref="canvas" class="absolute inset-0 h-full w-full" />
    <div class="absolute inset-x-0 top-[12%] text-center">
      <div
        class="text-4xl font-bold tracking-[0.25em] drop-shadow-[0_0_12px_rgba(254,92,69,0.8)]"
        aria-live="polite"
      >
        {{ title }}
      </div>
    </div>
    <div
      class="absolute inset-x-5 bottom-5 rounded-lg border border-white/15 bg-black/60 p-4 backdrop-blur-sm"
    >
      <div class="flex items-baseline justify-between text-sm font-semibold">
        <span>&gt; {{ stage.toUpperCase() }}</span>
        <span class="tabular">{{ percent }}%</span>
      </div>
      <div class="mt-2 flex gap-[3px]" role="progressbar" :aria-valuenow="percent">
        <div
          v-for="i in blocks"
          :key="i"
          class="h-3 flex-1 rounded-[2px]"
          :class="i <= filled ? 'bg-accent' : 'bg-white/10'"
        />
      </div>
      <div class="mt-3 space-y-1 text-[13px] leading-snug">
        <div
          v-for="(line, i) in shown"
          :key="i"
          class="truncate"
          :style="{ opacity: 0.35 + (0.65 * (i + 1)) / shown.length }"
        >
          “{{ line }}<span v-if="i === shown.length - 1" class="animate-pulse">▌</span>
        </div>
      </div>
      <button
        v-if="cancellable && stage !== 'Done'"
        class="mt-3 text-xs text-white/60 hover:text-white"
        @click="emit('cancel')"
      >
        Cancel transcription
      </button>
    </div>
  </div>
</template>
