# Using altium-designer-mcp with Google Antigravity

This guide explains how to set up and use the Altium Designer MCP server with
[Google Antigravity](https://antigravity.google) (the Gemini-based agentic IDE and CLI)
for AI-assisted component library creation.

It only covers the Antigravity-specific setup. The build steps, server configuration,
tool reference, and example workflows are client-agnostic and shared with the
[Claude Code guide](CLAUDE_CODE_GUIDE.md); this guide links to them rather than repeating them.

---

## Overview

Antigravity can use this MCP server to read existing Altium libraries, create IPC-7351B
land patterns and schematic symbols, match an existing library's style, and generate whole
component libraries from specifications.

**Note:** While Altium Designer only runs on Windows, this MCP server runs on any platform
to generate library files that you then open in Altium Designer on Windows.

---

## Installation

### Prerequisites

- [Google Antigravity](https://antigravity.google) installed
- [Rust 1.75+](https://rustup.rs/) only if you build from source

### Step 1: Get the server binary

**Download** the archive for your platform from the
[Releases page](https://github.com/embedded-society/altium-designer-mcp/releases), unpack it,
and move the binary somewhere permanent — the bundled `README.md` walks through it. Or
**build from source** per [CONTRIBUTING.md § Development Setup](../CONTRIBUTING.md#development-setup);
the binary then lands at `target/release/altium-designer-mcp` (`.exe` on Windows).

Note the binary's absolute path — Step 3 needs it.

### Step 2: Create the server config file

This is the MCP server's own `config.json` (it defines `allowed_paths`, logging, etc.) — it is
**not** the Antigravity config. See [README.md § Configuration](../README.md#configuration) for the
options and the file location (`~/.altium-designer-mcp/config.json`, Windows
`%USERPROFILE%\.altium-designer-mcp\config.json`).

---

## Step 3: Register the server in Antigravity

Antigravity reads MCP servers from:

```text
~/.gemini/config/mcp_config.json
```

(Windows: `%USERPROFILE%\.gemini\config\mcp_config.json`)

It uses the standard `mcpServers` schema (`command` / `args` / `env`, plus an optional
`disabled` flag), so the server entry mirrors any other MCP client. The Antigravity IDE and
CLI share this same file.

### Option A — Edit the raw config (recommended)

In the IDE, open **Manage MCP Servers** (top of the MCP store) → **View raw config**, or edit
`mcp_config.json` directly, and add an `altium` entry. Use **absolute paths** for both the
binary and the server `config.json` (replace the example paths with yours):

**Windows** (note the escaped backslashes):

```json
{
    "mcpServers": {
        "altium": {
            "command": "C:\\Users\\you\\AppData\\Local\\Programs\\altium-designer-mcp\\altium-designer-mcp.exe",
            "args": ["C:\\Users\\you\\.altium-designer-mcp\\config.json"]
        }
    }
}
```

**Linux:**

```json
{
    "mcpServers": {
        "altium": {
            "command": "/usr/local/bin/altium-designer-mcp",
            "args": ["/home/you/.altium-designer-mcp/config.json"]
        }
    }
}
```

**macOS:**

```json
{
    "mcpServers": {
        "altium": {
            "command": "/usr/local/bin/altium-designer-mcp",
            "args": ["/Users/you/.altium-designer-mcp/config.json"]
        }
    }
}
```

Notes:

- `mcp_config.json` is strict JSON — **inline comments are not supported**.
- To temporarily turn the server off without deleting it, set `"disabled": true` on the entry.
- Reload/restart Antigravity after editing the file so it picks up the change.

### Option B — Add via the UI

**Settings → Customizations → Add MCP**, then fill in the server name (`altium`), command
(the binary path), and arguments (the `config.json` path). This writes the same entry into
`mcp_config.json`.

> The exact menu labels and any additional fields can vary by Antigravity version — see the
> official [Antigravity MCP docs](https://antigravity.google/docs/mcp) for the current UI.

---

## Verify and use

1. In the MCP store / server list, confirm `altium` is listed and enabled (toggle on).
2. Ask the agent: *"What MCP tools do you have available?"* — you should see `read_pcblib`,
   `write_pcblib`, `read_schlib`, `write_schlib`, and the rest.

For the full tool reference, see **[docs/TOOLS.md](TOOLS.md)**.

The example prompts and workflows are identical regardless of client — see
[CLAUDE_CODE_GUIDE.md § Example Workflows](CLAUDE_CODE_GUIDE.md#example-workflows) for the full
set. A couple to start with:

```text
Create an IPC-7351B compliant 0603 chip resistor footprint and save it to
./MyLibrary.PcbLib
```

```text
Read ./ExistingLibrary.PcbLib and describe the footprints it contains.
What silkscreen style does it use?
```

---

## Troubleshooting

### Server doesn't appear / won't start

- Confirm the entry is under the `mcpServers` key in `~/.gemini/config/mcp_config.json` and the
  file is valid JSON (no trailing commas, no comments).
- Use absolute paths; on Windows the `.exe` extension is required and backslashes must be
  escaped (`\\`).
- Run the binary by hand once (`altium-designer-mcp --version`): on the very first run
  Windows SmartScreen or macOS Gatekeeper may block an unsigned binary — **More info → Run
  anyway** on Windows, right-click → **Open** on macOS.
- Reload Antigravity after editing the file.
- [CLIENT_SETUP.md § Troubleshooting](CLIENT_SETUP.md#troubleshooting) has the full checklist.

### "Access denied" error

The target path is outside the server's `allowed_paths`. Add the directory to your
`~/.altium-designer-mcp/config.json` (see [README.md § Configuration](../README.md#configuration)).

### Library won't open in Altium

The files are verified against Altium Designer 24 (the project's golden fixtures are
AD24-authored); older versions that read the same library format should work but are
untested. The format is binary-compatible across platforms, so libraries generated on
Linux/macOS open on Windows. Ask the agent to run `validate_library` on a file that misbehaves.

For anything not covered here, the behaviour is identical to other MCP clients — see the
[Claude Code guide](CLAUDE_CODE_GUIDE.md#troubleshooting).

---

## Next Steps

- [docs/TOOLS.md](TOOLS.md) — full tool reference
- [AGENT_GUIDE.md](AGENT_GUIDE.md) — the unit and pin-geometry conventions; worth pasting into a project brief
- [AI_WORKFLOW.md](AI_WORKFLOW.md) — IPC-7351B workflow and symbol conventions
- [CLIENT_SETUP.md](CLIENT_SETUP.md) — every other MCP client
- [ARCHITECTURE.md](ARCHITECTURE.md) — technical details
- [Antigravity MCP docs](https://antigravity.google/docs/mcp) — authoritative, version-current setup
