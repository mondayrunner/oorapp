# Oorapp for your agent (MCP)

`oorapp mcp` makes your meetings and quick notes available to Claude Code, Claude Desktop and any other MCP client. It runs on your own computer over stdio: it reads the meeting folders in `~/Documents/Meetings`, and nothing is sent anywhere by Oorapp itself. Whatever your agent does with the text is up to the agent.

It does not record. Recording stays in the Oorapp app, which owns the microphone.

## Tools

| Tool | What it does |
|---|---|
| `list_meetings` | Meetings, notes and imported files, newest first. Filter with `kind` (`note`, `meeting`, `file`), `limit` (default 20) and `since` (`YYYY-MM-DD`). |
| `search` | Searches titles and transcripts, case-insensitive, and returns each match with a snippet around the hit. Takes `query`, and optionally `kind` and `limit`. |
| `get_transcript` | The transcript of one meeting (Markdown, a time and a speaker on every line) or the text of a note, with the speakers, chapters and language. `path` is a meeting folder or its `.oorapp` file, as the other tools return it. Only paths inside `~/Documents/Meetings` are read. |
| `get_latest` | The newest meeting, note or file with its transcript, optionally of one `kind`. |
| `transcribe_file` | Transcribes an audio or video file on this computer, telling the voices apart, and keeps it in Oorapp like an import from the app. Takes `path`, and optionally `language` (`auto`, `en`, `nl`, `de`, `fr`, `es`, `it`, `pt`) and `speakers`. This takes about a tenth of the file's length: minutes for a long recording. |

## Install

Build the command line once:

```bash
cargo build --release -p oorapp-cli --features metal   # macOS; use --features vulkan on Windows
scripts/install-mcp.sh                                  # prints the commands below with your path
```

### Claude Code

```bash
claude mcp add --scope user oorapp -- /path/to/oorapp/target/release/oorapp mcp
```

### Claude Desktop

In `~/Library/Application Support/Claude/claude_desktop_config.json` (Windows: `%APPDATA%\Claude\claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "oorapp": {
      "command": "/path/to/oorapp/target/release/oorapp",
      "args": ["mcp"]
    }
  }
}
```

Restart Claude Desktop afterwards.

## Things to ask

- "What did we agree in yesterday's call with Anna?"
- "Find my notes about the installer and turn them into a to-do list."
- "Transcribe ~/Downloads/interview.m4a, two speakers, and summarise it."

## For developers

JSON-RPC 2.0 over stdin and stdout, one message per line, MCP protocol `2025-06-18`: `initialize`, `ping`, `tools/list` and `tools/call`. Logs, including the progress of `transcribe_file`, go to stderr. The code is in `crates/oorapp-cli/src/mcp.rs`.
