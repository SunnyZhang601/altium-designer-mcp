# Using altium-designer-mcp with Claude Code

This guide explains how to set up and use the Altium Designer MCP server with Claude Code
for AI-assisted component library creation.

---

## Overview

Claude Code can use this MCP server to:

- Read existing Altium libraries and analyse their structure
- Create new footprints with IPC-7351B compliant land patterns
- Create schematic symbols with proper pin definitions
- Match the style of existing libraries
- Generate entire component libraries from specifications

**Note:** While Altium Designer only runs on Windows, you can use this MCP server on any
platform to generate library files that can then be opened in Altium Designer on Windows.

---

## Installation

### Prerequisites

- [Claude Code](https://code.claude.com/docs) CLI installed
- [Rust 1.75+](https://rustup.rs/) only if you build from source

### Step 1: Get the Binary

**Download** the archive for your platform from the
[Releases page](https://github.com/embedded-society/altium-designer-mcp/releases), unpack it,
and move the binary somewhere permanent — the bundled `README.md` walks through it. Or
**build from source** per [CONTRIBUTING.md § Development Setup](../CONTRIBUTING.md#development-setup);
the binary then lands at `target/release/altium-designer-mcp` (`.exe` on Windows).

Either way, note the binary's absolute path — every configuration step below needs it.

### Step 2: Create Configuration File

See [README.md § Configuration](../README.md#configuration) for configuration options.

**Config file location:**

- **Windows:** `%USERPROFILE%\.altium-designer-mcp\config.json`
- **Linux/macOS:** `~/.altium-designer-mcp/config.json`

Create the directory and config file for your platform:

**Windows (PowerShell):**

```powershell
mkdir $env:USERPROFILE\.altium-designer-mcp -ErrorAction SilentlyContinue
```

**Linux/macOS:**

```bash
mkdir -p ~/.altium-designer-mcp
```

### Step 3: Configure Claude Code

Claude Code uses a `.mcp.json` file in your project root to configure MCP servers.

#### Option A: Project-Level Configuration (Recommended)

Create a `.mcp.json` file in your Altium project's root directory. It can be committed, so a
team shares one setup; Claude Code asks each person to approve the server the first time it
loads the file.

#### Windows

```json
{
    "mcpServers": {
        "altium": {
            "command": "C:\\Users\\yourname\\AppData\\Local\\Programs\\altium-designer-mcp\\altium-designer-mcp.exe",
            "args": ["C:\\Users\\yourname\\.altium-designer-mcp\\config.json"]
        }
    }
}
```

> Use **absolute paths** with every backslash doubled. JSON `args` reach the server verbatim —
> nothing expands `%USERPROFILE%` or `~` for you.

#### Linux

```json
{
    "mcpServers": {
        "altium": {
            "command": "/usr/local/bin/altium-designer-mcp",
            "args": ["/home/yourname/.altium-designer-mcp/config.json"]
        }
    }
}
```

#### macOS

```json
{
    "mcpServers": {
        "altium": {
            "command": "/usr/local/bin/altium-designer-mcp",
            "args": ["/Users/yourname/.altium-designer-mcp/config.json"]
        }
    }
}
```

#### Option B: Configuration via CLI

`claude mcp add` writes the entry for you. Everything after `--` is the command that starts
the server. With `--scope user` it is available in every project on this machine; without it,
only in the current project (local scope).

**Windows (PowerShell):**

```powershell
claude mcp add --scope user altium -- "$env:LOCALAPPDATA\Programs\altium-designer-mcp\altium-designer-mcp.exe" "$env:USERPROFILE\.altium-designer-mcp\config.json"
```

**Linux / macOS:**

```bash
claude mcp add --scope user altium -- /usr/local/bin/altium-designer-mcp ~/.altium-designer-mcp/config.json
```

To verify it was added and connects:

```bash
claude mcp list
```

`altium` should show `✔ Connected`.

---

## Using with Claude Code CLI

### Starting Claude Code

Navigate to your Altium project directory and run:

```bash
claude
```

Claude Code will automatically detect and load the MCP server from:

1. The `.mcp.json` file in the current directory (if present)
2. Your global MCP configuration

### Verify MCP is Loaded

Ask Claude Code:

```text
What MCP tools do you have available?
```

Or use the CLI command:

```bash
claude mcp list
```

You should see the Altium tools listed — `read_pcblib`, `write_pcblib`, `read_schlib`,
`write_schlib`, and the rest.

For the categorised overview of every tool, see [README § MCP Tools](../README.md#mcp-tools);
for full parameters and examples, see **[docs/TOOLS.md](TOOLS.md)**.

---

## Example Workflows

### 1. Create a Single Footprint

```text
Create an IPC-7351B compliant 0603 chip resistor footprint and save it to
./MyLibrary.PcbLib
```

Claude Code will:

1. Calculate the land pattern using IPC-7351B
2. Generate pad coordinates, silkscreen, and courtyard
3. Call `write_pcblib` to create the file

### 2. Create a Matching Schematic Symbol

```text
Now create a matching schematic symbol for the 0603 resistor and save it to
./MyLibrary.SchLib. Use designator "R?" and link it to the RESC1608X55N footprint.
```

### 3. Analyse an Existing Library

```text
Read ./ExistingLibrary.PcbLib and describe the footprints it contains.
What silkscreen style does it use?
```

Claude Code will:

1. Call `read_pcblib` to read the library
2. Analyse the primitives
3. Describe the styling conventions

### 4. Match an Existing Style

```text
Extract the style from ./CompanyLibrary.PcbLib and create a new 0805 capacitor
footprint that matches the same style conventions.
```

Claude Code will:

1. Call `extract_style` to analyse the existing library
2. Apply the same track widths, pad shapes, and layer usage
3. Create a style-matched footprint

### 5. Create a Complete Component Library

```text
Create a chip resistor library with footprints and symbols for:
- 0201, 0402, 0603, 0805, 1206, 2010, 2512

Use IPC-7351B nominal density. Save to ./ChipResistors.PcbLib and
./ChipResistors.SchLib
```

Claude Code will batch-create all components using IPC-7351B calculations.

### 6. Create from Datasheet Specifications

```text
Create a footprint for a QFN-24 package with:
- Body: 4mm x 4mm
- 24 pins, 0.5mm pitch
- Thermal pad: 2.5mm x 2.5mm
- Use IPC-7351B nominal density

Save to ./ICs.PcbLib
```

---

## Example Prompts

### Basic Component Creation

```text
Create an 0805 chip capacitor footprint with IPC-7351B nominal land pattern.
```

```text
Create a 2-pin polarised capacitor schematic symbol.
```

### Working with Existing Libraries

```text
List all components in ./MyLibrary.PcbLib
```

```text
Read ./Passives.SchLib and show me the pin configuration for the RESISTOR symbol.
```

### Style Matching

```text
Analyse the silkscreen style in ./ExistingLib.PcbLib - what line width does it use?
```

```text
Create a new footprint matching the style of ./CompanyStandard.PcbLib
```

### Batch Creation

```text
Create a complete SMD inductor library with sizes: 0402, 0603, 0805, 1008, 1206
```

```text
Create schematic symbols for all footprints in ./Passives.PcbLib
```

---

## Tips for Best Results

### 1. Be Specific About Standards

```text
Use IPC-7351B nominal density (not maximum or minimum)
```

### 2. Specify Layer Preferences

```text
Put silkscreen on Top Overlay, courtyard on Top Courtyard layer
```

### 3. Request Style Analysis First

```text
First analyse ./ExistingLib.PcbLib, then create new components matching that style
```

### 4. Provide Datasheet Details

When creating custom packages, provide:

- Body dimensions (L x W x H)
- Pin pitch
- Pin count and arrangement
- Thermal pad dimensions (if applicable)

### 5. Use Append Mode for Incremental Building

```text
Add an 0402 resistor footprint to the existing ./Passives.PcbLib (append mode)
```

---

## Troubleshooting

### "Access denied" Error

The file path is outside `allowed_paths`. Update your config.json to include the
directory where you want to create libraries.

### MCP Server Not Found / Failed to Connect

- The configured path must point at the binary itself (`.exe` on Windows) and be absolute.
- Run it by hand first: `altium-designer-mcp --version`. On the very first run Windows
  SmartScreen or macOS Gatekeeper may block an unsigned binary — **More info → Run anyway**
  on Windows, right-click → **Open** on macOS.
- `claude mcp list` shows the connection state;
  [CLIENT_SETUP.md § Troubleshooting](CLIENT_SETUP.md#troubleshooting) has the full checklist.

### Library Won't Open in Altium

- The files are verified against Altium Designer 24 (the project's golden fixtures are
  AD24-authored). Older versions that read the same library format should work but are
  untested.
- Check that the file was created successfully (non-zero file size), and ask Claude Code to
  run `validate_library` on it.

### Style Extraction Shows Unexpected Values

The `extract_style` tool analyses all primitives in the library. If a library has
mixed styles, you may see multiple values for each property.

---

## Platform-Specific Notes

### Windows

This is the primary platform since Altium Designer runs on Windows. You can:

- Generate libraries directly in your Altium project folder
- Use Windows paths with backslashes in config.json (escape them: `\\`)
- Run Claude Code in PowerShell, CMD, or Windows Terminal

### Linux

Generate libraries on Linux and transfer to Windows for use in Altium:

- Use a shared folder, cloud sync, or version control
- File format is binary-compatible across platforms

### macOS

Same approach as Linux:

- Generate libraries and transfer to Windows
- Use Apple Silicon or Intel Mac - both work
- File format is binary-compatible

---

## Security Notes

The MCP server validates all file paths against the `allowed_paths` configuration.
This prevents the AI from accessing or modifying files outside designated directories.

Always configure `allowed_paths` to include only the directories where you want
to allow library operations.

---

## Next Steps

- Read [AI_WORKFLOW.md](AI_WORKFLOW.md) for the IPC-7351B workflow and symbol conventions
- Paste [AGENT_GUIDE.md](AGENT_GUIDE.md) into a project brief so Claude knows the unit and
  pin-geometry conventions
- Using a different assistant as well? [CLIENT_SETUP.md](CLIENT_SETUP.md) covers every other
  MCP client
- See [ARCHITECTURE.md](ARCHITECTURE.md) for technical details and `scripts/samples/` for
  Altium-authored example libraries
