#!/bin/sh
# Prints how to add Oorapp's MCP server to Claude Code. Nothing is changed:
# run the printed command yourself.
#
# The server is the `oorapp` command line with `mcp`: it reads the meetings
# and notes in ~/Documents/Meetings and can transcribe a file. See docs/mcp.md.
set -e
cd "$(dirname "$0")/.."
bin="$PWD/target/release/oorapp"
if [ ! -x "$bin" ]; then
  echo "No $bin yet. Build it first:" >&2
  echo "  cargo build --release -p oorapp-cli --features metal" >&2
  exit 1
fi
echo "Claude Code (all your projects):"
echo "  claude mcp add --scope user oorapp -- \"$bin\" mcp"
echo
echo "Claude Desktop: add this to ~/Library/Application Support/Claude/claude_desktop_config.json"
cat <<JSON
{
  "mcpServers": {
    "oorapp": { "command": "$bin", "args": ["mcp"] }
  }
}
JSON
