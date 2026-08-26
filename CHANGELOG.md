# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Fixed

- **Saving a library is about 40× faster, opening one about 6×.** Both
  writers serialised straight into an unbuffered file, and a compound-file
  writer rewrites its sector and directory tables constantly — a disk round
  trip each time; the readers seeked through an unbuffered file the same way.
  A library is now built in memory and written once, and read into memory
  before it is parsed; the bytes are identical.
- **Reading and writing a library now scale linearly with its size.** The
  compound-file crate rebuilt the whole mini-stream chain on every access to
  a small stream and walked an unbalanced directory tree on every path
  lookup, so a library of `n` components cost a term in `n²`: 500 footprints
  opened in 135 ms where 50 took 4 ms. Both are fixed in a patched `cfb`
  the build pins until the fix is released upstream; 500 footprints now open
  in 20 ms, and the bytes written are identical.
- The performance tests assert what they can prove: that saving and opening
  scale linearly with library size (the accidental-quadratic guard, valid in
  any build), and absolute bounds only in an optimised build, which CI now
  runs. A wall-clock bound on a debug build measured the machine, not the
  code, and failed on a slow one.
- The old `docs/CLAUDE_CODE_GUIDE.md` and `docs/ANTIGRAVITY_GUIDE.md` addresses,
  still linked from search results and MCP directories, point at the merged
  `docs/CLIENT_SETUP.md` instead of a missing page.

## [0.2.0] - 2026-08-24

Everything in this release is the result of one campaign: hold the library
files we write to exactly what Altium itself writes, and refuse — rather than
quietly reinterpret — anything a caller gets wrong.

### Added

- **IEEE symbol support** (`RECORD=3`): the 35 schematic decorations Altium
  places from its IEEE toolbar, read, written and rendered.
- **`validate_library` checks 3D-model integrity**: a component body pointing at
  a model the library does not hold is an error, an embedded model no body
  references is a warning, in both formats.
- **A mutation-fidelity suite**: every mutating tool is held to leaving the
  components it did not touch byte-identical, and export→import and merge are
  held to reproducing the library byte-for-byte.
- **Corpus verification**: the round-trip suite can be pointed at a directory of
  real, Altium-authored libraries (`ALTIUM_CORPUS_DIR`) and holds every one of
  them byte-identical through a read-write cycle.
- **Layer names in any spelling**: every tool that takes a layer accepts Altium's
  own name (`Top Overlay`), the camel-case form (`TopOverlay`), any case, and
  either separator.
- **Documentation**: `docs/CLIENT_SETUP.md` (verified wiring for 17 MCP clients)
  and `docs/USAGE.md` (client-neutral workflows), a documentation index in the
  README, and link checking in CI.

### Changed

- **Breaking — a symbol's `text` array is now `ieee_symbols`.** What the format
  calls `RECORD=3` is an IEEE symbol, not a text annotation; free text on a
  symbol has always been the `label` record. Symbols that carried `text` should
  use `labels`.
- **Breaking — `read_pcblib` and `read_schlib` return the component's own JSON
  shape**, the same shape `write_*` accepts and `export_library` emits, with
  empty lists omitted. A read now replays through a write byte-for-byte.
- **Breaking — `export_library`'s CSV carries one count column per primitive
  kind**, replacing the previous partial column set.
- **An unrecognised value is refused, not defaulted.** Unknown tool arguments,
  unknown JSON keys, malformed records and unrecognised enum values (pad shape,
  hole shape, mask-expansion mode, stack mode, pin orientation and electrical
  type, text justification, region kind, layer names) are reported, naming the
  field and the accepted values. Previously a typo silently produced the default
  and the caller was told nothing.
- **Component names resolve the way the file does — regardless of case.** Two
  names differing only in case are one component to Altium and to the OLE
  directory, so every tool that creates one refuses the collision, naming the
  spelling on file; naming an existing component in another case never re-spells
  it.
- Tool descriptions and `docs/TOOLS.md` state what each tool reports and which
  fidelity-carrying fields it round-trips.

### Fixed

- **Round-trip fidelity.** Non-ASCII text grew a byte on every save (`WideStrings`
  are UTF-16 code units, not UTF-8 bytes); schematic records lost keys the reader
  had no field for; saves invented identities and reordered symbols, so two saves
  of one library differed. Every record is now replayed as read, and a save is
  deterministic.
- **Data that lived outside the component record was dropped**: embedded 3D
  models on merge and on export→import, external STEP references on write, and
  a footprint's models on a compact-mode read.
- **Reports that omitted primitive kinds.** `diff_libraries`, `compare_components`,
  `validate_library`, `extract_style`, `export_library`'s CSV, `list_components`
  and both ASCII renderers each covered only some kinds; all now walk the kind
  enums, so a kind cannot be missing from a report.
- **In-place edits overridden by sibling data**: a stacked pad's size or shape
  edit and a via's diameter edit did not reach the per-layer tables Altium
  actually draws from, and a primitive moved to another layer kept the byte or
  token naming the old one.
- **List edits that moved unrelated records**: deleting a parameter or a 3D body
  left the component's recorded primitive order one slot long, moving every
  later record.
- **Crashes and unsafe recovery**: a component name containing `:` panicked the
  process; a panicking tool now answers with an error instead of killing the
  server; `restore_backup` snapshots the current file and writes atomically.
- **Identity duplication**: a copied component carried its source's GUIDs and
  unique IDs.
- **Altium-specific storage details**: Mechanical 17–32 layers, region and body
  layer tokens, pad flag bits, the two forms of a non-embedded STEP reference,
  parameter key order, and non-ASCII pin names, all now stored the way Altium
  stores them.
- **Coordinates outside the safe range** are refused for every primitive kind
  before writing, rather than saturating silently in the file.

### Removed

- `docs/COVERAGE_AUDIT.md` and the two per-client setup guides, folded into the
  coverage map beside the samples and into `docs/CLIENT_SETUP.md`.

## [0.1.0] - 2026-08-18

An MCP server that gives AI assistants file I/O and primitive-placement tools
for Altium Designer `.PcbLib` (footprint) and `.SchLib` (symbol) libraries.

### Added

- **34 MCP tools** covering read/write, inspect/visualise (ASCII previews, style
  extraction), compare/diff, edit-in-place (component/pad/primitive updates, batch
  operations), component management (copy/rename/merge/reorder, cross-library),
  library operations (validate/repair, JSON/CSV export + import, `.LibPkg` project
  generation, embedded STEP extraction) and automatic timestamped backups with
  restore. See `docs/TOOLS.md` for the full generated reference.
- **PcbLib**: all eight footprint primitives (Pad, Via, Track, Arc, Region, Text,
  Fill, ComponentBody) modelled byte-identically to Altium's own output, including
  pad stacks and slot holes, thermal-relief/power-plane connection, solder/paste
  mask control, TrueType/barcode/inverted text, region kinds, embedded STEP models
  and 3D body handling.
- **SchLib**: every record type that occurs in a real symbol library — pins (with
  swap groups, symbol decorations and auxiliary streams), all graphic shapes
  (rectangles, rounded rectangles, lines, polylines, polygons, arcs, elliptical
  arcs, ellipses, pies, Béziers), images (including embedded image bytes in the
  `/Storage` stream), text frames, labels, text, parameters and footprint links —
  with fractional (off-grid) coordinate support and multi-part/display-mode symbols.
- **Safety**: path confinement to configured `allowed_paths`, path-sanitised error
  messages, automatic pre-mutation backups (5 retained), dry-run previews, token-
  bucket rate limiting on mutating tools and an optional append-only audit log.
- **Verification**: a strict independent Altium-readability oracle (pyaltiumlib) in
  CI, Altium-authored golden fixtures with exact assertions, byte-identity tests
  against captured Altium templates, and no-panic property tests over hostile input.
