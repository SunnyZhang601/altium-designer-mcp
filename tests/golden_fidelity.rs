//! Round-trip fidelity against the Altium-authored goldens.
//!
//! Every other test here checks that a *value* survives. This one checks that
//! the *file* does: each committed golden is read with our reader, written back
//! with our writer, and the result diffed against the original at two levels —
//! which OLE streams exist, and what every parameter block inside them says.
//!
//! It exists because the alternative kept failing. `CAVITYHEIGHT` sat on the
//! reader's modelled-keys list, excluded from the unknown-key passthrough, with
//! no field parsing it and the writer emitting a hard-coded `0mil`; the whole
//! `PrimitiveGuids` stream was never read or written at all. Neither is
//! reachable by a self-round-trip, because our reader and writer agreed with
//! each other perfectly in both cases. Both were found by hand, late.
//!
//! Differences correct by design sit in [`BY_DESIGN`]; real losses this test
//! found sit in [`KNOWN_DEFECTS`], which is debt meant to shrink to nothing.
//! Anything in neither list fails, so a newly dropped field surfaces here
//! rather than in a corrupted library weeks later.

use altium_designer_mcp::altium::pcblib::PcbLib;
use altium_designer_mcp::altium::schlib::SchLib;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn sample(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("samples")
        .join(name)
}

/// Keys whose value is expected to differ on every write and carries no
/// fidelity meaning: identities Altium regenerates, and the absolute path of
/// the file being written.
const VOLATILE_KEYS: &[&str] = &[
    "UNIQUEID",
    "ITEMGUID",
    "REVISIONGUID",
    "FILENAME",
    "MODELID",
    "MODEL.CHECKSUM",
    "VP.HX",
];

/// Key *prefix/suffix* patterns whose values are regenerated identities: the
/// per-layer GUID caches Altium rewrites on every save.
fn is_volatile_key(key: &str) -> bool {
    if VOLATILE_KEYS.contains(&key) {
        return true;
    }
    (key.starts_with("V9_CACHE_LAYER")
        || key.starts_with("V9_STACK_LAYER")
        || key.starts_with("LAYER_V8_"))
        && key.ends_with("ID")
}

/// Differences that are correct by design and will not change.
const BY_DESIGN: &[(&str, &str)] = &[
    (
        "library/data",
        "holds the absolute source path and library-wide identities, both \
         regenerated on write",
    ),
    (
        "library/padvialibrary/data",
        "an empty pad/via template cache with a fresh library id; no template is \
         modelled, so there is nothing to carry",
    ),
    (
        "library/componentparamstoc/data",
        "regenerated from the footprints, so ordering and spacing follow our \
         writer rather than the original",
    ),
];

/// Known defects: real fidelity losses this test found, each still open.
///
/// This is debt, not permission. Every entry is a field or stream Altium wrote
/// and we do not reproduce, and the list is meant to shrink to nothing — delete
/// an entry as its fix lands. It is spelled out here rather than left implicit
/// so the cost is visible in code review instead of being discovered in a
/// corrupted library.
const KNOWN_DEFECTS: &[(&str, &str)] = &[
    (
        "primitiveguids",
        "the per-primitive GUID stream is neither read nor written, so a \
         read-modify-write discards Altium's identity for every primitive",
    ),
    (
        "uniqueidprimitiveinformation",
        "written only when primitives carry unique ids, and the reader does not \
         populate them from this stream, so authored ids are lost",
    ),
    (
        "NAME golden=\" \"",
        "regions and component bodies carry NAME=<space> in an Altium file; our \
         writer emits an empty value",
    ),
    (
        "V7_LAYER",
        "Altium writes the short layer name (TOP); we write the long form \
         (TOPLAYER), and for a board cutout we write the resolved keep-out layer \
         rather than the stored one",
    ),
    (
        "TEXTURESIZE",
        "the component-body texture size is 0mil in an Altium file; our writer \
         hard-codes 0.0001mil",
    ),
    (
        "Library:",
        "our writer rebuilds the library layer stack rather than preserving it:          mechanical layers are renamed to their alias names (Mechanical 2 -> Top          Assembly), disabled layers are enabled, USEDBYPRIMS is recomputed and          LAYERSET1LAYERS gains every layer we know about. A library with custom          mechanical layer names loses them on a read-modify-write",
    ),
    (
        "sectionkeys",
        "Altium emits a SectionKeys stream mapping LibRef -> storage name for          every component whose name does not fit the CFB 31-character cap once          encoded; we neither read nor write it, so those components lose their          real name and keep only the truncated storage name",
    ),
    (
        "INDEXINSHEET",
        "our SchLib writer emits primitives grouped by type, renumbering them; \
         Altium preserves the original interleaved order",
    ),
];

/// KNOWN DEFECT: a component whose name leaves Windows-1252 is written under a
/// differently-encoded storage name, so its streams read as missing.
///
/// Altium maps the name's UTF-8 bytes through the authoring machine's ANSI
/// codepage (CP1250 on the box that made this golden); we map them through
/// Latin-1. Reproducing Altium exactly would make our output depend on the
/// local codepage, so the fix is to preserve the original storage name on read
/// rather than re-derive it on write.
const fn is_non_ascii_name_defect(what: &str) -> bool {
    !what.is_ascii()
}

fn is_known(what: &str) -> bool {
    let lower = what.to_lowercase();
    is_non_ascii_name_defect(what)
        || BY_DESIGN
            .iter()
            .chain(KNOWN_DEFECTS)
            .any(|(key, _)| lower.contains(&key.to_lowercase()))
}

/// Every stream path in an OLE file, lower-cased for case-insensitive compare.
fn stream_paths(path: &Path) -> BTreeSet<String> {
    let file = std::fs::File::open(path).expect("open library");
    let cfb = cfb::CompoundFile::open(file).expect("parse OLE");
    cfb.walk()
        .filter(cfb::Entry::is_stream)
        .map(|e| {
            e.path()
                .to_string_lossy()
                .trim_start_matches('/')
                .to_lowercase()
        })
        .collect()
}

/// Reads one stream's bytes, or `None` when it is absent.
fn stream_bytes(path: &Path, stream: &str) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).expect("open library");
    let mut cfb = cfb::CompoundFile::open(file).expect("parse OLE");
    let mut s = cfb.open_stream(stream).ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Splits a Data stream into its `|KEY=VALUE|…` parameter blocks and parses
/// each into a key/value map. Blocks are NUL-terminated inside the stream.
fn parameter_blocks(bytes: &[u8]) -> Vec<BTreeMap<String, String>> {
    let text = bytes
        .iter()
        .map(|&b| b as char) // Windows-1252 low range; adequate for key matching
        .collect::<String>();
    text.split('\0')
        .filter(|seg| seg.contains('|') && seg.contains('='))
        .map(|seg| {
            seg.split('|')
                .filter_map(|p| p.split_once('='))
                .map(|(k, v)| (k.to_ascii_uppercase(), v.to_string()))
                .collect()
        })
        .filter(|m: &BTreeMap<String, String>| m.len() >= 3)
        .collect()
}

/// Scores how well two blocks correspond: shared keys, plus a bonus for
/// agreeing on the values that identify a record rather than describe it.
fn similarity(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> usize {
    let shared: Vec<_> = a.keys().filter(|k| b.contains_key(*k)).collect();
    let agreeing = shared.iter().filter(|k| a.get(**k) == b.get(**k)).count();
    // Value agreement dominates: a region and a component body share almost
    // every key, so key overlap alone pairs them with each other.
    shared.len() + agreeing * 4
}

/// Pairs golden blocks with rewritten ones greedily, best match first and each
/// used only once, then reports every key whose value changed or vanished.
///
/// One-to-one matching matters: regions and component bodies share nearly their
/// whole key set, so a naive best-overlap pass happily compares a region
/// against a body and invents divergences for every field that legitimately
/// differs between them.
fn block_divergences(golden: &[u8], ours: &[u8], label: &str) -> Vec<String> {
    let (a, b) = (parameter_blocks(golden), parameter_blocks(ours));

    let mut pairs: Vec<(usize, usize, usize)> = Vec::new();
    for (i, ga) in a.iter().enumerate() {
        for (j, ob) in b.iter().enumerate() {
            pairs.push((similarity(ga, ob), i, j));
        }
    }
    pairs.sort_unstable_by_key(|&(score, _, _)| std::cmp::Reverse(score));

    let mut taken_g = vec![false; a.len()];
    let mut taken_o = vec![false; b.len()];
    let mut matched: Vec<(usize, usize)> = Vec::new();
    for (score, i, j) in pairs {
        if score < 3 || taken_g[i] || taken_o[j] {
            continue;
        }
        taken_g[i] = true;
        taken_o[j] = true;
        matched.push((i, j));
    }

    let mut out = Vec::new();
    for (i, j) in matched {
        for (key, want) in &a[i] {
            if is_volatile_key(key) {
                continue;
            }
            let got = b[j].get(key);
            if got != Some(want) {
                out.push(format!(
                    "{label}: {key} golden={want:?} ours={:?}",
                    got.map(String::as_str)
                ));
            }
        }
    }
    for (i, used) in taken_g.iter().enumerate() {
        if !used {
            let kind = a[i]
                .get("RECORD")
                .or_else(|| a[i].get("V7_LAYER"))
                .map_or("?", String::as_str);
            out.push(format!(
                "{label}: a golden block ({} keys, kind {kind}) has no counterpart in our output",
                a[i].len()
            ));
        }
    }
    out
}

#[test]
fn pcblib_golden_survives_a_round_trip() {
    let src = sample("footprints.PcbLib");
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("rewritten.PcbLib");
    let mut lib = PcbLib::open(&src).expect("read golden PcbLib");
    lib.save(&out).expect("write it back");

    let mut failures = Vec::new();

    // 1. Streams the golden has that we did not write back.
    let (before, after) = (stream_paths(&src), stream_paths(&out));
    for missing in before.difference(&after) {
        if !is_known(missing) {
            failures.push(format!("stream dropped entirely: {missing}"));
        }
    }

    // 2. The library-level layer stack and metadata.
    if let (Some(g), Some(o)) = (
        stream_bytes(&src, "/Library/Data"),
        stream_bytes(&out, "/Library/Data"),
    ) {
        failures.extend(
            block_divergences(&g, &o, "Library")
                .into_iter()
                .filter(|d| !is_known(d)),
        );
    }

    // 3. Parameter values inside each footprint's Data stream.
    for name in lib.names() {
        let stream = format!("/{name}/Data");
        let (Some(g), Some(o)) = (stream_bytes(&src, &stream), stream_bytes(&out, &stream)) else {
            continue;
        };
        failures.extend(
            block_divergences(&g, &o, &name)
                .into_iter()
                .filter(|d| !is_known(d)),
        );
    }

    assert!(
        failures.is_empty(),
        "the golden does not survive a read/write cycle intact ({} divergence(s)).\n\
         Each line is a field Altium stored that we changed or lost. Fix the \
         reader/writer, or add a reasoned entry to KNOWN_DEFECTS.\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn schlib_golden_survives_a_round_trip() {
    let src = sample("symbols.SchLib");
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("rewritten.SchLib");
    let lib = SchLib::open(&src).expect("read golden SchLib");
    lib.save(&out).expect("write it back");

    let mut failures = Vec::new();

    let (before, after) = (stream_paths(&src), stream_paths(&out));
    for missing in before.difference(&after) {
        if !is_known(missing) {
            failures.push(format!("stream dropped entirely: {missing}"));
        }
    }

    for name in lib.names() {
        let stream = format!("/{name}/Data");
        let (Some(g), Some(o)) = (stream_bytes(&src, &stream), stream_bytes(&out, &stream)) else {
            continue;
        };
        failures.extend(
            block_divergences(&g, &o, &name)
                .into_iter()
                .filter(|d| !is_known(d)),
        );
    }

    assert!(
        failures.is_empty(),
        "the golden does not survive a read/write cycle intact ({} divergence(s)).\n\
         Each line is a field Altium stored that we changed or lost. Fix the \
         reader/writer, or add a reasoned entry to KNOWN_DEFECTS.\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}
