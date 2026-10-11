# Oorapp

Every call written down, and the audio stays on your Mac.

Oorapp records your microphone and your computer's sound as two separate tracks, so it knows who said what in any call app. No bot joins the meeting. Transcription runs on your own machine. It is free and open source.

Website: https://oorapp.sitelane.nl

## What it does

- **Meetings**: records both sides of a call (Zoom, Meet, Teams, Slack, anything that plays sound), transcribes it locally with Whisper on the GPU, tells the voices apart, filters echo through speakers, and gives you a transcript you can listen to, fix and copy. Chapters by the coding agent you already have (Claude Code, Codex), with every tool switched off.
- **Notes**: hold the note key anywhere (right ⌥ by default), speak, let go: the text is pasted where your cursor is. Runs on NVIDIA Parakeet, a few sentences in well under a second.
- **Transcribe a file**: a phone memo or an interview, with the speakers told apart.
- **History**: every meeting, note and file, grouped by day and searchable.
- **MCP server**: Claude and other agents can search and read your meetings. In the app: *Use with Claude…*, or by hand:
  ```
  claude mcp add --scope user oorapp -- /Applications/Oorapp.app/Contents/MacOS/oorapp-app mcp
  ```
  See [docs/mcp.md](docs/mcp.md).

Every meeting is a plain folder in `~/Documents/Meetings`: the audio, `transcript.md` and an `.oorapp` file that opens it in the app.

## Install

Download the latest release from [GitHub Releases](https://github.com/mondayrunner/oorapp/releases/latest). macOS 14.4 or newer on Apple silicon. Oorapp updates itself.

The app is not notarized by Apple: the first time, right-click Oorapp and choose Open. macOS then asks for the microphone and system audio; for notes also Input Monitoring and Accessibility.

Windows builds are on the way.

## Build from source

Needs Rust, Node 22 with pnpm, and CMake.

```
pnpm install
pnpm tauri dev                 # the app, with hot reload
scripts/build-mac.sh           # Oorapp.app, signed with a fixed identity when there is one
cargo test -p oorapp-core      # the core
python3 bench/run.py --model small.en --check   # transcription and speaker scores
```

Layout:

| Path | What |
|---|---|
| `crates/oorapp-core` | Transcription, speakers, meeting files, chapters, actions, MCP. No GUI. |
| `crates/oorapp-capture` | Mic and system audio through cpal (Core Audio process tap, WASAPI loopback). |
| `crates/oorapp-cli` | `oorapp transcribe`, `transcribe-file`, `record`, `mcp` and friends. |
| `src-tauri`, `ui` | The app: Tauri 2 and Vue. |
| `bench` | Recorded test calls and the scoring script. |
| `upstream-linux` | The original GTK app, kept as reference. |

## Credits

Oorapp is built on [omarchy-meeting-recorder](https://github.com/jankeesvw/omarchy-meeting-recorder) by Jankees van Woezik. The two-track recording, the way speakers are told apart, the echo filter, the transcript page and the test suite come from his work. On Linux, use his: it is the original and it is excellent.

Speech recognition by [whisper.cpp](https://github.com/ggml-org/whisper.cpp) and NVIDIA Parakeet; speaker diarization by NVIDIA Nemotron 3 Diarization (OpenMDW license).

Made by Tim van den Bosch at [Sitelane](https://sitelane.nl). "Oor" is Dutch for ear.

## License

MIT
