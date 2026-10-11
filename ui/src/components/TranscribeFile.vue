<script setup lang="ts">
// Transcribe a file: drop one here or choose it. The import dialog then asks
// for the language and the number of speakers.
defineProps<{ dropping: boolean }>();
const emit = defineEmits<{ choose: [] }>();
</script>

<template>
  <div class="h-full px-6 pt-2 pb-6">
    <div
      class="flex h-72 flex-col items-center justify-center gap-3 rounded-2xl border-2 border-dashed"
      :class="dropping ? 'border-accent bg-row-active' : 'border-line'"
    >
      <svg width="44" height="44" viewBox="0 0 44 44" class="text-muted">
        <path d="M22 6v18m0 0l-7-7m7 7l7-7" stroke="currentColor" stroke-width="2.4" fill="none" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M6 26l4-8h5M38 26l-4-8h-5M6 26v9a3 3 0 0 0 3 3h26a3 3 0 0 0 3-3v-9H29a7 7 0 0 1-14 0H6z" stroke="currentColor" stroke-width="2.4" fill="none" stroke-linejoin="round" />
      </svg>
      <p class="text-[17px] font-semibold">Drop files here</p>
      <p class="font-mono text-xs text-muted">MP3, M4A, WAV, AAC, OGG, OPUS, FLAC, MP4, MOV</p>
      <button class="mt-1 flex items-center gap-2 rounded-xl border border-line bg-card px-4 py-2 text-sm font-semibold hover:bg-card-2" @click="emit('choose')">
        <svg width="15" height="15" viewBox="0 0 15 15"><path d="M1.5 4A1.5 1.5 0 0 1 3 2.5h3l1.5 1.5H12A1.5 1.5 0 0 1 13.5 5.5v5.5A1.5 1.5 0 0 1 12 12.5H3A1.5 1.5 0 0 1 1.5 11z" stroke="currentColor" stroke-width="1.3" fill="none" stroke-linejoin="round" /></svg>
        Choose file…
      </button>
    </div>
    <p class="mt-4 text-center text-sm text-muted">
      A phone memo, an interview, a call recorded elsewhere. The voices are told apart and it is transcribed on this computer.
    </p>
  </div>
</template>
