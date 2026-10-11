// The commands and events of the Rust side (src-tauri/src/lib.rs), typed.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Phase = "idle" | "recording" | "saving" | "transcribing" | "done";

export interface Status {
  phase: Phase;
  paused: boolean;
  title: string;
  format: string;
  language: string;
  elapsed: number;
  dir: string | null;
  stage: string;
  progress: number;
  lines: string[];
  problem: string | null;
  model_missing: [string, number] | null;
  model_download: number | null;
  mic_device: string | null;
  computer_device: string | null;
  mic_problem: string | null;
  computer_problem: string | null;
  computer_silent: number;
  chapters_busy: boolean;
  action_running: string | null;
  note: number | null;
  note_busy: boolean;
  note_permissions: { keys: boolean; paste: boolean };
}

export interface Choice {
  key: string;
  label: string;
}

export interface Options {
  formats: Choice[];
  languages: Choice[];
  agent: string | null;
  agent_problem: string | null;
  actions: string[];
  actions_docs: string;
  config_file: string;
  note_shortcut: string;
  note_shortcuts: Choice[];
  meetings_dir: string;
  platform: string;
}

export interface Levels {
  mic: number[];
  computer: number[];
  elapsed: number;
}

export interface SpeakerRow {
  role: string;
  name: string;
}

export interface ParagraphRow {
  sources: number[];
  time: string;
  start_ms: number;
  speaker: string;
  colour: number;
  text: string;
}

export interface ChapterRow {
  start_ms: number;
  clock: string;
  title: string;
}

export interface MeetingData {
  dir: string;
  title: string;
  started_at: number;
  duration_secs: number;
  meta: string[];
  notes: string[];
  language: string;
  imported: boolean;
  speakers: SpeakerRow[];
  paragraphs: ParagraphRow[];
  chapters: ChapterRow[];
  chapters_note: string | null;
  can_have_chapters: boolean;
  markdown: string;
  audio: string[];
}

export interface Unfinished {
  path: string;
  title: string;
  started_at: number;
  seconds: number;
}

export interface MeetingEntry {
  path: string;
  title: string;
  started_at: number;
  duration_secs: number;
  current: boolean;
  kind: "note" | "file" | "meeting";
  text: string;
  words: number;
  transcribed: boolean;
  target_app: string | null;
}

export interface Peaks {
  mic: number[];
  computer: number[];
}

export const api = {
  options: () => invoke<Options>("options"),
  status: () => invoke<Status>("status"),
  setFormat: (key: string) => invoke("set_format", { key }),
  setLanguage: (code: string) => invoke("set_language", { code }),
  setTitle: (title: string) => invoke("set_title", { title }),
  start: (title?: string) => invoke("start", { title }),
  togglePause: () => invoke("toggle_pause"),
  stop: () => invoke("stop"),
  cancel: () => invoke("cancel"),
  newRecording: () => invoke("new_recording"),
  importFile: (path: string, language: string, speakers: number | null) =>
    invoke("import_file", { path, language, speakers }),
  unfinished: () => invoke<Unfinished[]>("unfinished"),
  recover: (path: string) => invoke("recover", { path }),
  discard: (path: string) => invoke("discard", { path }),
  openMeeting: (path: string) => invoke("open_meeting", { path }),
  meetings: () => invoke<MeetingEntry[]>("meetings"),
  meetingText: (path: string) => invoke<string>("meeting_text", { path }),
  trashMeeting: (path: string) => invoke("trash_meeting", { path }),
  openFolder: (path: string) => invoke("open_folder", { path }),
  copyText: (text: string) => invoke("copy_text", { text }),
  copyMeeting: (path: string) => invoke("copy_meeting", { path }),
  toggleNote: () => invoke("toggle_note"),
  setNoteShortcut: (shortcut: string) => invoke("set_note_shortcut", { shortcut }),
  requestPermission: (which: "keys" | "paste") => invoke("request_permission", { which }),
  checkUpdate: () => invoke<{ version: string; notes: string | null } | null>("check_update"),
  installUpdate: () => invoke("install_update"),
  connectClaude: (target: "code" | "desktop") => invoke<string>("connect_claude", { target }),
  meeting: () => invoke<MeetingData | null>("meeting_data"),
  peaks: (bins: number) => invoke<Peaks>("peaks", { bins }),
  editParagraph: (sources: number[], time: string, speaker: string, text: string) =>
    invoke<string>("edit_paragraph", { sources, time, speaker, text }),
  deleteParagraph: (sources: number[]) => invoke<string>("delete_paragraph", { sources }),
  swapSpeaker: (sources: number[], speaker: string) =>
    invoke<string>("swap_speaker", { sources, speaker }),
  restoreTranscript: (markdown: string) => invoke("restore_transcript", { markdown }),
  renameSpeakers: (names: string[]) => invoke<boolean>("rename_speakers", { names }),
  transcribeAgain: (language: string) => invoke("transcribe_again", { language }),
  makeChapters: () => invoke("make_chapters"),
  runAction: (index: number) => invoke("run_action", { index }),
  downloadModel: () => invoke("download_model"),
};

export function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => handler(e.payload));
}

export function clock(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s % 60)}` : `${pad(m)}:${pad(s % 60)}`;
}

/** Colours for the speakers, in the order of the meeting. */
export const SPEAKER_COLOURS = [
  "var(--mic)",
  "var(--computer)",
  "#3498db",
  "#2ecc71",
  "#fb7585",
  "#a78bfa",
  "#eab308",
  "#14b8a6",
];

/** "Alt+Shift+Space" as the keys are printed on this keyboard. */
export function keys(shortcut: string, mac: boolean): string {
  if (!mac) return shortcut.replace("CmdOrCtrl", "Ctrl");
  const map: Record<string, string> = {
    alt: "⌥", option: "⌥", shift: "⇧", cmd: "⌘", command: "⌘", super: "⌘",
    cmdorctrl: "⌘", ctrl: "⌃", control: "⌃", space: "Space",
  };
  return shortcut
    .split("+")
    .map((k) => map[k.trim().toLowerCase()] ?? k.trim())
    .join("");
}
