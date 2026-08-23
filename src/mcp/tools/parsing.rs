//! JSON -> Altium primitive parsing helpers, split from `server.rs`.
//!
//! These extend `McpServer` with an additional `impl` block; the handlers in
//! other modules call them via `Self::parse_*` unchanged.

use serde_json::Value;

use crate::altium::pcblib::Footprint;
use crate::altium::schlib::Symbol;
use crate::mcp::server::{ErrorContext, McpServer, ToolCallResult};
use crate::mcp::tools::allowed_keys;

/// Maps a free-text component type to its reference-designator class letter,
/// following the conventions of IEEE 315 / ASME Y14.44 (commercial usage).
///
/// Used as the fallback when a symbol is written without an explicit
/// `designator_prefix`. Unknown or unspecified types resolve to `"U"`
/// (integrated circuit / inseparable assembly), the most common case.
// The explicit IC/regulator arm shares the `"U"` body with the wildcard
// fallback; it is kept to document the recognised IC synonyms rather than
// silently folding them into `_`.
#[allow(clippy::match_same_arms)]
pub fn ieee_designator_prefix(component_type: &str) -> &'static str {
    match component_type.trim().to_ascii_lowercase().as_str() {
        "resistor" | "res" | "potentiometer" | "pot" | "trimmer" | "rheostat" => "R",
        "resistor_network" | "resistor_array" | "network" => "RN",
        "thermistor" | "ntc" | "ptc" => "RT",
        "varistor" | "mov" => "RV",
        "capacitor" | "cap" => "C",
        "inductor" | "coil" | "choke" | "ferrite" | "ferrite_bead" | "bead" => "L",
        "diode" | "rectifier" | "schottky" | "zener" | "tvs" | "led" => "D",
        "display" | "lamp" | "indicator" | "lightbulb" => "DS",
        "transistor" | "mosfet" | "fet" | "bjt" | "igbt" | "jfet" => "Q",
        "ic" | "integrated_circuit" | "microcircuit" | "opamp" | "mcu" | "regulator"
        | "voltage_regulator" => "U",
        "connector" | "header" | "jack" | "receptacle" => "J",
        "plug" => "P",
        "socket" => "X",
        "crystal" | "oscillator" | "resonator" | "xtal" => "Y",
        "switch" | "button" | "pushbutton" | "dip_switch" | "dipswitch" => "S",
        "relay" | "contactor" => "K",
        "transformer" => "T",
        "fuse" => "F",
        "filter" => "FL",
        "battery" | "cell" => "BT",
        "test_point" | "testpoint" => "TP",
        "terminal_block" | "terminal" => "TB",
        "speaker" | "loudspeaker" | "buzzer" => "LS",
        "microphone" => "MK",
        "motor" | "fan" | "blower" => "B",
        "module" | "assembly" | "subassembly" => "A",
        "mechanical" | "standoff" | "screw" | "mounting" => "MP",
        "jumper" | "wire" | "cable" => "W",
        _ => "U",
    }
}

/// Reads a JSON integer field as `i32`, returning `None` if it is missing, not
/// an integer, or outside `i32` range — so an out-of-range value is rejected
/// rather than silently wrapped (`as i32`), which would let an absurd input
/// land as a small in-range coordinate that bypasses range validation.
fn json_i32(json: &Value, field: &str) -> Option<i32> {
    json.get(field)
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())
}

/// Reads a JSON number field as `f64`, accepting both integer and fractional
/// JSON values and rejecting non-finite (NaN/∞) inputs. Schematic graphic
/// coordinates use this so off-grid (fractional) positions survive instead of
/// being dropped by the integer-only [`json_i32`].
fn json_f64(json: &Value, field: &str) -> Option<f64> {
    json.get(field)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
}

/// Reads the four universal display/lock flags shared by every `SchLib` graphic
/// shape (`graphically_locked` / `disabled` / `dimmed` /
/// `owner_part_display_mode`) from a shape's JSON. Absent keys default to
/// `false` / `0`, matching Altium's omit-when-default records.
fn parse_schlib_display_flags(json: &Value) -> crate::altium::schlib::ShapeDisplayFlags {
    #[allow(clippy::cast_possible_truncation)]
    crate::altium::schlib::ShapeDisplayFlags {
        graphically_locked: json
            .get("graphically_locked")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        disabled: json
            .get("disabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        dimmed: json.get("dimmed").and_then(Value::as_bool).unwrap_or(false),
        owner_part_display_mode: json_i32(json, "owner_part_display_mode").unwrap_or(0),
    }
}

/// Reads the optional `flags` field of a `PcbLib` 2D primitive.
///
/// `read_pcblib` serialises [`crate::altium::pcblib::PcbFlags`] (a `bitflags`
/// set) via its serde impl, which in JSON is a string of `|`-separated flag
/// names, e.g. `"LOCKED"` or `"LOCKED | KEEPOUT"`. The write side deserialises
/// that exact form with serde so a value read from disk round-trips unchanged.
/// For caller convenience a raw `u16` bitmask integer is also accepted
/// (`1` = `LOCKED`, `4` = `KEEPOUT`, …). An absent or unparseable value yields
/// the empty flag set rather than erroring, matching the lenient handling of the
/// other optional tail fields.
fn json_flags(json: &Value) -> crate::altium::pcblib::PcbFlags {
    use crate::altium::pcblib::PcbFlags;
    match json.get("flags") {
        // Canonical round-trip shape: the bitflags serde string ("LOCKED | …").
        Some(v @ Value::String(_)) => {
            serde_json::from_value(v.clone()).unwrap_or_else(|_| PcbFlags::empty())
        }
        // Convenience shape: a raw bitmask integer.
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .map_or_else(PcbFlags::empty, PcbFlags::from_bits_truncate),
        _ => PcbFlags::empty(),
    }
}

/// Reads the optional `keepout_restrictions` bitmask (`u8`) of a `PcbLib` 2D
/// primitive, mirroring how `read_pcblib` serialises the `Option<u8>` field.
fn json_keepout(json: &Value) -> Option<u8> {
    json.get("keepout_restrictions")
        .and_then(Value::as_u64)
        .and_then(|v| u8::try_from(v).ok())
}

/// Reads the optional common-header net index (u16) of a `PcbLib` primitive,
/// defaulting to `0xFFFF` ("no net") — the from-scratch value the writer's
/// header fill emits. Mirrors how `read_pcblib` serialises the `net_index` field.
fn json_net_index(json: &Value) -> u16 {
    json.get("net_index")
        .and_then(Value::as_u64)
        .and_then(|v| u16::try_from(v).ok())
        .unwrap_or(0xFFFF)
}

/// Reads the optional common-header polygon index (u16) of a `PcbLib` primitive,
/// defaulting to `0xFFFF` (none) — the from-scratch value.
fn json_polygon_index(json: &Value) -> u16 {
    json.get("polygon_index")
        .and_then(Value::as_u64)
        .and_then(|v| u16::try_from(v).ok())
        .unwrap_or(0xFFFF)
}

/// Reads the optional common-header component index (i32) of a `PcbLib`
/// primitive, defaulting to `-1` (free primitive; stored as the `0xFFFF`
/// sentinel) — the from-scratch value.
fn json_component_index(json: &Value) -> i32 {
    json.get("component_index")
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())
        .unwrap_or(-1)
}

/// Reads the optional `unique_id` (identity GUID) of any primitive.
///
/// `read_pcblib` / `read_schlib` surface each primitive's 8-char Altium unique
/// ID via serde, so an AI doing a read-modify-write can pass it straight back
/// here to preserve stable primitive identity across saves (Altium tracks
/// primitives by this GUID for ECO). An absent value yields `None`, letting the
/// writer auto-generate a fresh GUID exactly as it does for from-scratch output.
fn json_unique_id(json: &Value) -> Option<String> {
    json.get("unique_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Reads a `SchLib` record's raw segments (`raw_params`) from its JSON, so a
/// read-modify-write replays the record as Altium wrote it.
fn json_raw_params(json: &Value) -> Vec<(String, String)> {
    json.get("raw_params")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Reads a primitive's Altium identity GUID from its JSON, so a
/// read-modify-write that passes through the tool layer keeps it.
fn json_guid(json: &Value) -> Option<String> {
    json.get("guid").and_then(Value::as_str).map(str::to_string)
}

/// The streams a symbol carries verbatim (`extra_streams`), in the
/// `[[name, base64], …]` form `read_schlib` emits; anything else is no
/// streams, like every other carrier this parser reads leniently.
fn json_extra_streams(json: &Value) -> Vec<(String, Vec<u8>)> {
    #[derive(serde::Deserialize)]
    struct Streams(#[serde(with = "crate::altium::base64_opt::named")] Vec<(String, Vec<u8>)>);
    json.get("extra_streams")
        .cloned()
        .and_then(|v| serde_json::from_value::<Streams>(v).ok())
        .map_or_else(Vec::new, |streams| streams.0)
}

/// A read primitive's header layer byte (`raw_layer_id`), carried so the
/// rewrite keeps an unmapped byte while the primitive still sits on the
/// Multi-Layer catch-all it decoded to.
fn json_raw_layer_id(json: &Value) -> Option<u8> {
    json.get("raw_layer_id")
        .and_then(Value::as_u64)
        .and_then(|v| u8::try_from(v).ok())
}

/// Reads a base64-encoded byte field: the raw replay bases `read_pcblib`
/// emits (a pad's `raw_tail`, a via's `raw_block`, a text's `raw_geometry`)
/// and embedded image bytes. Passing one back through the tool layer keeps
/// the write byte-identical to the source block. Invalid base64 is treated
/// as absent (this parser is lenient Option-style throughout) with a debug
/// log for diagnosis; the writer then falls back to its captured template,
/// so the write still succeeds semantically.
fn json_base64(json: &Value, key: &str) -> Option<Vec<u8>> {
    json.get(key).and_then(Value::as_str).and_then(|s| {
        use base64::Engine as _;
        match base64::engine::general_purpose::STANDARD.decode(s.as_bytes()) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                tracing::debug!(error = %e, key, "invalid base64; ignoring");
                None
            }
        }
    })
}

/// Reads an optional verbatim string field from a primitive's JSON.
fn json_guidless_opt(json: &Value, key: &str) -> Option<String> {
    json.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Accepted pad-shape spellings, quoted in every shape error so the two tools
/// give identical guidance.
pub const PAD_SHAPE_HELP: &str = "Valid shapes are: rectangle (or rectangular), round (or \
     circle), oval, octagonal, rounded_rectangle. Matching is case-insensitive and ignores \
     '_'/'-' separators.";

impl McpServer {
    /// Parses a pad shape name, shared by `write_pcblib` and `update_pad` so the
    /// same spelling is accepted everywhere.
    ///
    /// Both tools must accept the same spellings, or a pad written with one
    /// cannot be updated with the other. This takes the union of the
    /// vocabularies they document, case-insensitively, ignoring `_`/`-`.
    pub(crate) fn parse_pad_shape(s: &str) -> Option<crate::altium::pcblib::PadShape> {
        use crate::altium::pcblib::PadShape;
        match s.to_lowercase().replace(['_', '-'], "").as_str() {
            "rectangle" | "rectangular" | "rect" => Some(PadShape::Rectangle),
            "round" | "circle" | "circular" => Some(PadShape::Round),
            "oval" | "oblong" => Some(PadShape::Oval),
            "octagonal" | "octagon" => Some(PadShape::Octagonal),
            "roundedrectangle" | "rounded" => Some(PadShape::RoundedRectangle),
            _ => None,
        }
    }

    // ==================== Primitive Parsing Helpers ====================

    /// Parses one symbol object — the `symbols[]` element of `write_schlib`
    /// and the `symbol` of `update_component` — into a [`Symbol`], refusing
    /// unknown keys on every object and validating the geometry. Both tools
    /// go through here so neither can fall behind the other on a record kind
    /// or a replay field. The designator is always assigned: explicit
    /// `designator`, else `designator_prefix`, else `component_type` via the
    /// IEEE 315 / ASME Y14.44 table, else `U`.
    ///
    /// `operation` names the calling tool; `default_name` is the symbol name
    /// when the object carries none.
    #[allow(clippy::too_many_lines)] // one straight-line pass over every symbol field
    #[allow(clippy::unused_self)] // kept as a method beside parse_footprint_json
    pub(crate) fn parse_symbol_json(
        &self,
        sym_json: &Value,
        keys: &allowed_keys::SchLibKeys,
        operation: &str,
        filepath: &str,
        default_name: &str,
    ) -> Result<Symbol, ToolCallResult> {
        use crate::altium::schlib::FootprintModel;

        Self::refuse_unknown(sym_json, &keys.symbol)?;
        let name = sym_json
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(default_name);
        let mut symbol = Symbol::new(name);

        if let Some(desc) = sym_json.get("description").and_then(Value::as_str) {
            symbol.description = desc.to_string();
        }

        // Always assign a reference designator. Precedence:
        //   1. explicit `designator`
        //   2. explicit `designator_prefix`
        //   3. `component_type` mapped via IEEE 315 / ASME Y14.44 table
        //   4. fallback "U" (integrated circuit)
        // so every symbol carries a `<prefix>?` designator in the SchLib.
        let designator = sym_json
            .get("designator")
            .and_then(Value::as_str)
            .map_or_else(
                || {
                    let prefix = sym_json
                        .get("designator_prefix")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            sym_json
                                .get("component_type")
                                .and_then(Value::as_str)
                                .map(|t| ieee_designator_prefix(t).to_string())
                        })
                        .unwrap_or_else(|| "U".to_string());
                    format!("{prefix}?")
                },
                str::to_string,
            );
        symbol.designator = designator;

        // Designator text position (RECORD=34 Location.X/Y) and identity.
        // Defaults -5/5 per the AD24 golden; the unique id is reused when
        // supplied (e.g. a read-modify-write) so the record is deterministic.
        if let Some(x) = sym_json.get("designator_x").and_then(Value::as_f64) {
            symbol.designator_x = x;
        }
        if let Some(y) = sym_json.get("designator_y").and_then(Value::as_f64) {
            symbol.designator_y = y;
        }
        if let Some(uid) = sym_json.get("designator_unique_id").and_then(Value::as_str) {
            symbol.designator_unique_id = Some(uid.to_string());
        }

        // Parse part_count for multi-part symbols (e.g., dual op-amp)
        if let Some(part_count) = sym_json.get("part_count").and_then(Value::as_u64) {
            #[allow(clippy::cast_possible_truncation)]
            {
                symbol.part_count = part_count.clamp(1, 255) as u32;
            }
        }

        // Parse the remaining symbol header fields (mirrors
        // update_schlib_component): export_schlib emits them, so an
        // export -> write_schlib round-trip must not reset them to
        // defaults (e.g. collapsing a two-display-mode symbol to one).
        if let Some(v) = sym_json.get("display_mode_count").and_then(Value::as_u64) {
            symbol.display_mode_count = u32::try_from(v).unwrap_or(symbol.display_mode_count);
        }
        if let Some(v) = sym_json.get("current_part_id").and_then(Value::as_u64) {
            symbol.current_part_id = u32::try_from(v).unwrap_or(symbol.current_part_id);
        }
        if let Some(v) = sym_json.get("part_id_locked").and_then(Value::as_bool) {
            symbol.part_id_locked = v;
        }
        if let Some(v) = sym_json.get("source_library_name").and_then(Value::as_str) {
            symbol.source_library_name = v.to_string();
        }
        if let Some(v) = sym_json.get("target_file_name").and_then(Value::as_str) {
            symbol.target_file_name = v.to_string();
        }

        // Parse pins
        if let Some(pins) = sym_json.get("pins").and_then(Value::as_array) {
            for (i, pin_json) in pins.iter().enumerate() {
                Self::refuse_unknown(pin_json, &keys.pin)?;
                let pin = Self::parse_schlib_pin(pin_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "pin", i))?;
                symbol.add_pin(pin);
            }
        }

        // Parse rectangles
        if let Some(rects) = sym_json.get("rectangles").and_then(Value::as_array) {
            for (i, rect_json) in rects.iter().enumerate() {
                Self::refuse_unknown(rect_json, allowed_keys::RECTANGLE)?;
                let rect = Self::parse_schlib_rectangle(rect_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "rectangle", i))?;
                symbol.add_rectangle(rect);
            }
        }

        // Parse rounded rectangles
        if let Some(round_rects) = sym_json.get("round_rects").and_then(Value::as_array) {
            for (i, round_rect_json) in round_rects.iter().enumerate() {
                Self::refuse_unknown(round_rect_json, allowed_keys::ROUND_RECT)?;
                let round_rect = Self::parse_schlib_round_rect(round_rect_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "round_rect", i))?;
                symbol.add_round_rect(round_rect);
            }
        }

        // Parse lines
        if let Some(lines) = sym_json.get("lines").and_then(Value::as_array) {
            for (i, line_json) in lines.iter().enumerate() {
                Self::refuse_unknown(line_json, allowed_keys::LINE)?;
                let line = Self::parse_schlib_line(line_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "line", i))?;
                symbol.add_line(line);
            }
        }

        // Parse polylines
        if let Some(polylines) = sym_json.get("polylines").and_then(Value::as_array) {
            for (i, polyline_json) in polylines.iter().enumerate() {
                Self::refuse_unknown(polyline_json, allowed_keys::POLYLINE)?;
                let polyline = Self::parse_schlib_polyline(polyline_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "polyline", i))?;
                symbol.add_polyline(polyline);
            }
        }

        // Parse polygons
        if let Some(polygons) = sym_json.get("polygons").and_then(Value::as_array) {
            for (i, polygon_json) in polygons.iter().enumerate() {
                Self::refuse_unknown(polygon_json, allowed_keys::POLYGON)?;
                let polygon = Self::parse_schlib_polygon(polygon_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "polygon", i))?;
                symbol.add_polygon(polygon);
            }
        }

        // Parse arcs
        if let Some(arcs) = sym_json.get("arcs").and_then(Value::as_array) {
            for (i, arc_json) in arcs.iter().enumerate() {
                // SchLib arcs are centre/radius/angle based, NOT layer-based like PcbLib arcs; the
                // allow-list must match the documented fields in tool_definitions or every arc is
                // rejected as an "unknown field" (was erroneously copied from the PcbLib arc as ["layer"]).
                Self::refuse_unknown(arc_json, allowed_keys::ARC)?;
                let arc = Self::parse_schlib_arc(arc_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "arc", i))?;
                symbol.add_arc(arc);
            }
        }

        if let Some(pies) = sym_json.get("pies").and_then(Value::as_array) {
            for (i, pie_json) in pies.iter().enumerate() {
                Self::refuse_unknown(pie_json, allowed_keys::PIE)?;
                let pie = Self::parse_schlib_pie(pie_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "pie", i))?;
                symbol.add_pie(pie);
            }
        }

        if let Some(images) = sym_json.get("images").and_then(Value::as_array) {
            for (i, image_json) in images.iter().enumerate() {
                Self::refuse_unknown(image_json, allowed_keys::IMAGE)?;
                let image = Self::parse_schlib_image(image_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "image", i))?;
                symbol.add_image(image);
            }
        }

        if let Some(text_frames) = sym_json.get("text_frames").and_then(Value::as_array) {
            for (i, frame_json) in text_frames.iter().enumerate() {
                Self::refuse_unknown(frame_json, allowed_keys::TEXT_FRAME)?;
                let text_frame = Self::parse_schlib_text_frame(frame_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "text_frame", i))?;
                symbol.add_text_frame(text_frame);
            }
        }

        if let Some(beziers) = sym_json.get("beziers").and_then(Value::as_array) {
            for (i, bezier_json) in beziers.iter().enumerate() {
                Self::refuse_unknown(bezier_json, allowed_keys::BEZIER)?;
                let bezier = Self::parse_schlib_bezier(bezier_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "bezier", i))?;
                symbol.add_bezier(bezier);
            }
        }

        if let Some(ell_arcs) = sym_json.get("elliptical_arcs").and_then(Value::as_array) {
            for (i, ell_arc_json) in ell_arcs.iter().enumerate() {
                Self::refuse_unknown(ell_arc_json, allowed_keys::ELLIPTICAL_ARC)?;
                let ell_arc = Self::parse_schlib_elliptical_arc(ell_arc_json).ok_or_else(|| {
                    Self::malformed(operation, filepath, name, "elliptical_arc", i)
                })?;
                symbol.add_elliptical_arc(ell_arc);
            }
        }

        // Parse ellipses
        if let Some(ellipses) = sym_json.get("ellipses").and_then(Value::as_array) {
            for (i, ellipse_json) in ellipses.iter().enumerate() {
                Self::refuse_unknown(ellipse_json, allowed_keys::ELLIPSE)?;
                let ellipse = Self::parse_schlib_ellipse(ellipse_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "ellipse", i))?;
                symbol.add_ellipse(ellipse);
            }
        }

        // Parse labels
        if let Some(labels) = sym_json.get("labels").and_then(Value::as_array) {
            for (i, label_json) in labels.iter().enumerate() {
                Self::refuse_unknown(label_json, allowed_keys::LABEL)?;
                let label = Self::parse_schlib_label(label_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "label", i))?;
                symbol.add_label(label);
            }
        }

        // Parse IEEE symbols
        if let Some(symbols) = sym_json.get("ieee_symbols").and_then(Value::as_array) {
            for (i, symbol_json) in symbols.iter().enumerate() {
                Self::refuse_unknown(symbol_json, allowed_keys::IEEE_SYMBOL)?;
                let ieee_symbol = Self::parse_schlib_ieee_symbol(symbol_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "IEEE symbol", i))?;
                symbol.add_ieee_symbol(ieee_symbol);
            }
        }

        // Parse parameters
        if let Some(params) = sym_json.get("parameters").and_then(Value::as_array) {
            for (i, param_json) in params.iter().enumerate() {
                Self::refuse_unknown(param_json, allowed_keys::PARAMETER)?;
                let param = Self::parse_schlib_parameter(param_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "parameter", i))?;
                symbol.add_parameter(param);
            }
        }

        // Parse footprint references
        if let Some(footprints) = sym_json.get("footprints").and_then(Value::as_array) {
            for (i, fp_json) in footprints.iter().enumerate() {
                // A footprint reference is a model link, not an embedded
                // footprint: name, description, library_path, and the
                // read-preserved identity a replay carries back.
                Self::refuse_unknown(fp_json, &keys.footprint)?;
                let fp_name = fp_json.get("name").and_then(Value::as_str).ok_or_else(|| {
                    Self::malformed(operation, filepath, name, "footprint link", i)
                })?;
                {
                    let mut fp = FootprintModel::new(fp_name);
                    if let Some(desc) = fp_json.get("description").and_then(Value::as_str) {
                        fp.description = desc.to_string();
                    }
                    // Optional PcbLib path -> ModelDatafile0, so Altium can
                    // resolve the footprint instead of reporting "not found".
                    if let Some(lib_path) = fp_json.get("library_path").and_then(Value::as_str) {
                        fp.library_path = Some(lib_path.to_string());
                    }
                    fp.is_current = fp_json
                        .get("is_current")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    fp.unique_id = fp_json
                        .get("unique_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    fp.raw_params = json_raw_params(fp_json);
                    symbol.add_footprint(fp);
                }
            }
        }

        // The streams this crate does not read, carried verbatim.
        symbol.extra_streams = json_extra_streams(sym_json);
        // The header exactly as read — key order, unmodelled keys and the
        // stale AllPinCount — so a read-modify-write reproduces it.
        if let Some(params) = sym_json.get("header_params") {
            symbol.header_params = serde_json::from_value(params.clone()).unwrap_or_default();
        }
        if let Some(count) = sym_json.get("all_pin_count").and_then(Value::as_u64) {
            symbol.all_pin_count = u32::try_from(count).ok();
        }

        // The interleaved record order `read_schlib` reported; replaying it
        // replaces the grouped order the add_* calls accumulated, so a
        // read-modify-write keeps the source's record order.
        if let Some(order) = sym_json.get("primitive_order") {
            match serde_json::from_value(order.clone()) {
                Ok(kinds) => symbol.primitive_order = kinds,
                Err(e) => {
                    tracing::debug!(error = %e, "invalid primitive_order; using default order");
                }
            }
        }

        // Out-of-range or non-finite geometry is refused here so both tools
        // report it the same way.
        if let Err(e) = Self::validate_symbol_coordinates(&symbol) {
            return Err(ToolCallResult::error(e));
        }

        Ok(symbol)
    }

    /// Refuses a JSON object carrying a key outside `keys` (see
    /// [`check_unknown_fields`](Self::check_unknown_fields)).
    fn refuse_unknown(json: &Value, keys: &[&str]) -> Result<(), ToolCallResult> {
        Self::check_unknown_fields(json, keys).map_err(ToolCallResult::error)
    }

    /// The error for an object its parser could not build: a required field
    /// missing or of the wrong type. Named and indexed like a malformed pad,
    /// so a bad record is refused rather than silently left out of the file.
    fn malformed(
        operation: &str,
        filepath: &str,
        component: &str,
        kind: &str,
        index: usize,
    ) -> ToolCallResult {
        ToolCallResult::error_with_context(
            ErrorContext::new(
                operation,
                format!("Malformed {kind}: a required field is missing or invalid"),
            )
            .with_filepath(filepath)
            .with_component(component)
            .with_details(format!("Failed to parse {kind} at index {index}")),
        )
    }

    /// Parses one footprint object — the `footprints[]` element of
    /// `write_pcblib` and the `footprint` of `update_component` — into a
    /// [`Footprint`], refusing unknown keys on every object and validating the
    /// geometry. Both tools go through here so neither can fall behind the
    /// other on a primitive kind, a 3D-model spelling or a replay field.
    ///
    /// `operation` names the calling tool in error context; `default_name` is
    /// the footprint name when the object carries none.
    #[allow(clippy::too_many_lines)] // one straight-line pass over every footprint field
    pub(crate) fn parse_footprint_json(
        &self,
        fp_json: &Value,
        keys: &allowed_keys::PcbLibKeys,
        operation: &str,
        filepath: &str,
        default_name: &str,
    ) -> Result<Footprint, ToolCallResult> {
        use crate::altium::pcblib::Model3D;

        Self::refuse_unknown(fp_json, &keys.footprint)?;
        let name = fp_json
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(default_name);
        let mut footprint = Footprint::new(name);

        if let Some(desc) = fp_json.get("description").and_then(Value::as_str) {
            footprint.description = desc.to_string();
        }

        // Parse pads
        if let Some(pads) = fp_json.get("pads").and_then(Value::as_array) {
            for (i, pad_json) in pads.iter().enumerate() {
                Self::refuse_unknown(pad_json, &keys.pad)?;
                match Self::parse_pad(pad_json) {
                    Ok(pad) => footprint.add_pad(pad),
                    Err(e) => {
                        return Err(ToolCallResult::error_with_context(
                            ErrorContext::new(operation, e)
                                .with_filepath(filepath)
                                .with_component(name)
                                .with_details(format!("Failed to parse pad at index {i}")),
                        ))
                    }
                }
            }
        }

        // Parse tracks
        if let Some(tracks) = fp_json.get("tracks").and_then(Value::as_array) {
            for (i, track_json) in tracks.iter().enumerate() {
                Self::refuse_unknown(track_json, &keys.track)?;
                match Self::parse_track(track_json) {
                    Ok(track) => footprint.add_track(track),
                    Err(e) => {
                        return Err(ToolCallResult::error_with_context(
                            ErrorContext::new(operation, e)
                                .with_filepath(filepath)
                                .with_component(name)
                                .with_details(format!("Failed to parse track at index {i}")),
                        ))
                    }
                }
            }
        }

        // Parse vias
        if let Some(vias) = fp_json.get("vias").and_then(Value::as_array) {
            for (i, via_json) in vias.iter().enumerate() {
                Self::refuse_unknown(via_json, &keys.via)?;
                match Self::parse_via(via_json) {
                    Ok(via) => footprint.add_via(via),
                    Err(e) => {
                        return Err(ToolCallResult::error_with_context(
                            ErrorContext::new(operation, e)
                                .with_filepath(filepath)
                                .with_component(name)
                                .with_details(format!("Failed to parse via at index {i}")),
                        ))
                    }
                }
            }
        }

        // Parse fills
        if let Some(fills) = fp_json.get("fills").and_then(Value::as_array) {
            for (i, fill_json) in fills.iter().enumerate() {
                Self::refuse_unknown(fill_json, &keys.fill)?;
                match Self::parse_fill(fill_json) {
                    Ok(fill) => footprint.add_fill(fill),
                    Err(e) => {
                        return Err(ToolCallResult::error_with_context(
                            ErrorContext::new(operation, e)
                                .with_filepath(filepath)
                                .with_component(name)
                                .with_details(format!("Failed to parse fill at index {i}")),
                        ))
                    }
                }
            }
        }

        // Parse arcs
        if let Some(arcs) = fp_json.get("arcs").and_then(Value::as_array) {
            for (i, arc_json) in arcs.iter().enumerate() {
                Self::refuse_unknown(arc_json, &keys.arc)?;
                match Self::parse_arc(arc_json) {
                    Ok(arc) => footprint.add_arc(arc),
                    Err(e) => {
                        return Err(ToolCallResult::error_with_context(
                            ErrorContext::new(operation, e)
                                .with_filepath(filepath)
                                .with_component(name)
                                .with_details(format!("Failed to parse arc at index {i}")),
                        ))
                    }
                }
            }
        }

        // Parse regions
        if let Some(regions) = fp_json.get("regions").and_then(Value::as_array) {
            for (i, region_json) in regions.iter().enumerate() {
                Self::refuse_unknown(region_json, &keys.region)?;
                let region = Self::parse_region(region_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "region", i))?;
                footprint.add_region(region);
            }
        }

        // Parse text
        if let Some(texts) = fp_json.get("text").and_then(Value::as_array) {
            for (i, text_json) in texts.iter().enumerate() {
                Self::refuse_unknown(text_json, &keys.text)?;
                let text = Self::parse_text(text_json)
                    .ok_or_else(|| Self::malformed(operation, filepath, name, "text", i))?;
                footprint.add_text(text);
            }
        }

        // Parse 3D model
        if let Some(model_json) = fp_json.get("step_model") {
            Self::refuse_unknown(model_json, &keys.model)?;
            if let Some(model_path) = model_json.get("filepath").and_then(Value::as_str) {
                let embed = model_json
                    .get("embed")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);

                if embed {
                    // The embed source is read from disk at save time
                    // (prepare_3d_models_for_writing -> std::fs::read), far from
                    // this handler. Validate it against the allow-list now so a
                    // caller cannot embed an arbitrary file (e.g. "../../etc/passwd")
                    // into the library. External references (embed=false) are only
                    // stored as a string and never read, so they are not gated here.
                    if let Err(e) = self.validate_path(model_path) {
                        return Err(ToolCallResult::error(e));
                    }

                    // Embedded model - use Model3D which will read the file on save
                    footprint.model_3d = Some(Model3D {
                        filepath: model_path.to_string(),
                        x_offset: model_json
                            .get("x_offset")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0),
                        y_offset: model_json
                            .get("y_offset")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0),
                        z_offset: model_json
                            .get("z_offset")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0),
                        rotation: model_json
                            .get("rotation")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0),
                    });
                } else {
                    // External reference only: a model-backed body whose file
                    // stays outside the library. Altium stores such a body with
                    // an EMPTY MODELID and the MODEL.* group carrying
                    // MODEL.EMBED=FALSE and MODEL.NAME (a UI-authored
                    // `test_0805.step` reference), and the writer emits the
                    // group for any body that names a file. The full path is
                    // kept so organised subfolders resolve.
                    use crate::altium::pcblib::{ComponentBody, Layer};
                    footprint.add_component_body(ComponentBody {
                        model_id: String::new(),
                        identifier: String::new(),
                        texture_center_x: None,
                        texture_center_y: None,
                        texture_size_x: None,
                        texture_size_y: None,
                        texture_rotation: None,
                        raw_layer_id: None,
                        v7_layer: None,
                        model_name: model_path.to_string(), // Preserve full path
                        embedded: false,
                        rotation_x: 0.0,
                        rotation_y: 0.0,
                        rotation_z: model_json
                            .get("rotation")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0),
                        z_offset: model_json
                            .get("z_offset")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0),
                        overall_height: 0.0,
                        standoff_height: 0.0,
                        cavity_height: 0.0,
                        layer: Layer::Top3DBody,
                        outline: Vec::new(),
                        unique_id: None,
                        guid: None,
                        model_checksum: 0, // External reference: no embedded model.
                        name: " ".to_string(),
                        kind: 0,
                        sub_poly_index: -1,
                        union_index: 0,
                        is_shape_based: false,
                        body_projection: 0,
                        body_color_3d: 8_421_504,
                        body_opacity_3d: 1.0,
                        model_2d_rotation: 0.0,
                        model_2d_x: 0.0,
                        model_2d_y: 0.0,
                        // External reference: no board association (free primitive).
                        net_index: 0xFFFF,
                        polygon_index: 0xFFFF,
                        component_index: -1,
                        additional_parameters: Vec::new(),
                        param_key_order: Vec::new(),
                    });
                }
            }
        }

        // Parse "model_3d" — read_pcblib's spelling of the same model
        // reference (it emits the key for every footprint, null when there
        // is no model), accepted so a read result replays into
        // write_pcblib unchanged. `step_model` wins when both are given
        // (it is the authoring-time spelling, incl. the embed switch);
        // null is ignored. The fields mirror the Model3D serde shape
        // (filepath + offsets/rotation).
        if fp_json.get("step_model").is_none() {
            if let Some(model_json) = fp_json.get("model_3d").filter(|v| !v.is_null()) {
                Self::refuse_unknown(model_json, &keys.model)?;
                let model_path = model_json
                    .get("filepath")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                // The save path embeds the file (std::fs::read) when the
                // path resolves to an existing file, so gate exactly that
                // case against the allow-list — the same arbitrary-file-
                // read defence as step_model. Bare model names replayed
                // from read_pcblib output don't exist on disk and are kept
                // as inert references, so they are not gated.
                if std::path::Path::new(model_path).is_file() {
                    if let Err(e) = self.validate_path(model_path) {
                        return Err(ToolCallResult::error(e));
                    }
                }
                footprint.model_3d = Some(Model3D {
                    filepath: model_path.to_string(),
                    x_offset: model_json
                        .get("x_offset")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                    y_offset: model_json
                        .get("y_offset")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                    z_offset: model_json
                        .get("z_offset")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                    rotation: model_json
                        .get("rotation")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                });
            }
        }

        // Parse generic extruded 3D bodies (no STEP model). Each body is
        // defined by an optional 2D outline (auto-bounding-box from pads when
        // omitted) plus standoff/overall heights, on the Top/Bottom 3D Body
        // layer. model_id/model_name stay empty so the writer marks them as
        // shape-based extruded bodies.
        if let Some(bodies) = fp_json.get("component_bodies").and_then(Value::as_array) {
            for body_json in bodies {
                Self::refuse_unknown(body_json, &keys.component_body)?;
                footprint.add_component_body(Self::parse_component_body_json(body_json));
            }
        }

        // Footprint-level replay fields `read_pcblib` emits. The guid is
        // the kind-85 identity record; the interleaved Data-stream order
        // replaces the grouped order the add_* calls accumulated, so a
        // read-modify-write keeps the source's stream order
        // (`write_sequence` is advisory-safe against a stale list).
        if let Some(g) = fp_json.get("guid").and_then(Value::as_str) {
            footprint.guid = Some(g.to_string());
        }
        if let Some(order) = fp_json.get("primitive_order") {
            match serde_json::from_value(order.clone()) {
                Ok(kinds) => footprint.primitive_order = kinds,
                Err(e) => {
                    tracing::debug!(error = %e, "invalid primitive_order; using default order");
                }
            }
        }

        // Out-of-range or non-finite geometry would saturate in from_mm() on
        // save; refused here so both tools report it the same way.
        if let Err(e) = Self::validate_footprint_coordinates(&footprint) {
            return Err(ToolCallResult::error(e));
        }

        Ok(footprint)
    }

    pub(crate) fn check_unknown_fields(
        json: &serde_json::Value,
        allowed_keys: &[&str],
    ) -> Result<(), String> {
        if let Some(obj) = json.as_object() {
            for key in obj.keys() {
                if !allowed_keys.contains(&key.as_str()) {
                    return Err(format!(
                        "Unknown field '{key}'. Allowed fields are: {allowed_keys:?}"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Parses a pad from JSON.
    #[allow(clippy::too_many_lines)] // Pad has many fields requiring individual parsing
    pub(crate) fn parse_pad(json: &Value) -> Result<crate::altium::pcblib::Pad, String> {
        use crate::altium::pcblib::{
            Layer, MaskExpansionMode, Pad, PadStackMode, PowerPlaneConnectStyle,
        };

        let designator = json
            .get("designator")
            .and_then(Value::as_str)
            .ok_or("Pad missing required field 'designator'")?;

        // Validate designator is not empty
        if designator.trim().is_empty() {
            return Err("Pad designator cannot be empty".to_string());
        }

        let x = json
            .get("x")
            .and_then(Value::as_f64)
            .ok_or("Pad missing required field 'x'")?;
        let y = json
            .get("y")
            .and_then(Value::as_f64)
            .ok_or("Pad missing required field 'y'")?;
        let width = json
            .get("width")
            .and_then(Value::as_f64)
            .ok_or("Pad missing required field 'width'")?;
        let height = json
            .get("height")
            .and_then(Value::as_f64)
            .ok_or("Pad missing required field 'height'")?;

        // Validate pad dimensions are positive
        if width <= 0.0 {
            return Err(format!(
                "Pad '{designator}' has invalid width {width}. Width must be greater than 0."
            ));
        }
        if height <= 0.0 {
            return Err(format!(
                "Pad '{designator}' has invalid height {height}. Height must be greater than 0."
            ));
        }

        // Omitting `shape` yields RoundedRectangle. That suits chip/QFN lands but is
        // wrong for BGA/CSP, whose circular NSMD lands need an explicit "round" —
        // see the `shape` description in tool_definitions.rs.
        let shape_str = json
            .get("shape")
            .and_then(Value::as_str)
            .unwrap_or("rounded_rectangle");
        let shape = Self::parse_pad_shape(shape_str).ok_or_else(|| {
            format!("Pad '{designator}' has invalid shape '{shape_str}'. {PAD_SHAPE_HELP}")
        })?;

        // Parse hole_size first to determine default layer
        let hole_size = json.get("hole_size").and_then(Value::as_f64);
        let is_smd = hole_size.map_or(true, |h| h <= 0.0); // SMD if no hole or hole size <= 0

        // Plated hole (main-block byte @60). Altium defaults this to true for
        // every pad, SMD included (matches the golden fixture and AltiumSharp).
        let is_plated = json
            .get("is_plated")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let layer_str = json.get("layer").and_then(Value::as_str);
        let layer = match layer_str {
            Some(s) => Layer::parse(s).ok_or_else(|| {
                format!(
                    "Pad '{designator}' has invalid layer '{s}'. \
                     Valid layers include: Top Layer, Bottom Layer, Multi-Layer, Top Overlay, etc."
                )
            })?,
            // SMD pads default to Top Layer, through-hole pads default to Multi-Layer
            None => {
                if is_smd {
                    Layer::TopLayer
                } else {
                    Layer::MultiLayer
                }
            }
        };
        let rotation = json.get("rotation").and_then(Value::as_f64).unwrap_or(0.0);

        // Parse optional hole shape
        let hole_shape = json
            .get("hole_shape")
            .and_then(Value::as_str)
            .map(|s| match s.to_lowercase().as_str() {
                "square" => crate::altium::pcblib::HoleShape::Square,
                "slot" => crate::altium::pcblib::HoleShape::Slot,
                _ => crate::altium::pcblib::HoleShape::Round,
            })
            .unwrap_or_default();

        // Parse optional mask expansion values
        let paste_mask_expansion = json.get("paste_mask_expansion").and_then(Value::as_f64);
        let solder_mask_expansion = json.get("solder_mask_expansion").and_then(Value::as_f64);
        let paste_mask_expansion_mode = json
            .get("paste_mask_expansion_mode")
            .and_then(Value::as_str)
            .map(|s| match s.to_lowercase().as_str() {
                "manual" => MaskExpansionMode::Manual,
                "from_rule" => MaskExpansionMode::FromRule,
                // Anything else, "none" included, maps to the state a fresh
                // Altium pad carries, so an unrecognised value produces the
                // same bytes as saying nothing at all.
                _ => MaskExpansionMode::None,
            })
            .unwrap_or_default();
        let solder_mask_expansion_mode = json
            .get("solder_mask_expansion_mode")
            .and_then(Value::as_str)
            .map(|s| match s.to_lowercase().as_str() {
                "manual" => MaskExpansionMode::Manual,
                "from_rule" => MaskExpansionMode::FromRule,
                // Anything else, "none" included, maps to the state a fresh
                // Altium pad carries, so an unrecognised value produces the
                // same bytes as saying nothing at all.
                _ => MaskExpansionMode::None,
            })
            .unwrap_or_default();

        // Parse optional corner radius
        let corner_radius_percent = json
            .get("corner_radius_percent")
            .and_then(Value::as_u64)
            .and_then(|v| u8::try_from(v).ok())
            .filter(|&v| v <= 100);

        // Thermal-relief / power-plane connection fields. Absent keys keep the
        // from-scratch defaults (= Altium's pad template), so an unspecified pad
        // round-trips byte-identically.
        let power_plane_connect_style = json
            .get("power_plane_connect_style")
            .and_then(Value::as_str)
            .map(|s| match s.to_lowercase().as_str() {
                "direct" => PowerPlaneConnectStyle::Direct,
                "no_connect" | "noconnect" => PowerPlaneConnectStyle::NoConnect,
                _ => PowerPlaneConnectStyle::Relief,
            })
            .unwrap_or_default();
        let relief_conductor_width = json
            .get("relief_conductor_width")
            .and_then(Value::as_f64)
            .unwrap_or(0.254);
        let relief_entries = json
            .get("relief_entries")
            .and_then(Value::as_i64)
            .and_then(|v| i16::try_from(v).ok())
            .unwrap_or(4);
        let relief_air_gap = json
            .get("relief_air_gap")
            .and_then(Value::as_f64)
            .unwrap_or(0.254);
        let power_plane_relief_expansion = json
            .get("power_plane_relief_expansion")
            .and_then(Value::as_f64)
            .unwrap_or(0.508);
        let power_plane_clearance = json
            .get("power_plane_clearance")
            .and_then(Value::as_f64)
            .unwrap_or(0.508);

        // Slot geometry + drill tolerances. Absent keys keep the struct defaults
        // (slot 0, rotation 0, tolerances unset), so an unspecified pad round-trips
        // byte-identically.
        let hole_slot_length = json
            .get("hole_slot_length")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let hole_rotation = json
            .get("hole_rotation")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let hole_positive_tolerance = json.get("hole_positive_tolerance").and_then(Value::as_f64);
        let hole_negative_tolerance = json.get("hole_negative_tolerance").and_then(Value::as_f64);

        // Identity GUIDs (extended tail @126/@142). Absent -> None, so the
        // writer generates fresh per-pad GUIDs; a read-modify-write passes the
        // read value back and preserves the on-disk bytes verbatim.
        let identity_guid = json
            .get("identity_guid")
            .and_then(Value::as_str)
            .map(str::to_string);
        let identity_guid_b = json
            .get("identity_guid_b")
            .and_then(Value::as_str)
            .map(str::to_string);

        let solder_mask_expansion_from_hole_edge = json
            .get("solder_mask_expansion_from_hole_edge")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Per-layer pad stack. The model has carried these all along; without them
        // in the schema a caller could never author anything but a Simple pad.
        let stack_mode = match json
            .get("stack_mode")
            .and_then(Value::as_str)
            .map(str::to_lowercase)
            .as_deref()
        {
            Some("top_middle_bottom") => PadStackMode::TopMiddleBottom,
            Some("full_stack") => PadStackMode::FullStack,
            _ => PadStackMode::Simple,
        };
        // Each entry in any spelling the tools accept (`{width, height}`,
        // `{x, y}` or `[a, b]`); a malformed entry is a zero pair rather than
        // a silently shortened stack.
        let pairs = |field: &str| {
            json.get(field).and_then(Value::as_array).map(|a| {
                a.iter()
                    .map(|v| crate::altium::serde_round::pair_from_json(v).unwrap_or((0.0, 0.0)))
                    .collect::<Vec<_>>()
            })
        };
        let per_layer_sizes = pairs("per_layer_sizes");
        let per_layer_offsets = pairs("per_layer_offsets");
        let per_layer_shapes = json
            .get("per_layer_shapes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|v| {
                        v.as_str()
                            .and_then(Self::parse_pad_shape)
                            .unwrap_or(crate::altium::pcblib::PadShape::Round)
                    })
                    .collect::<Vec<_>>()
            });
        let per_layer_corner_radii = json
            .get("per_layer_corner_radii")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|v| u8::try_from(v.as_u64().unwrap_or(0)).unwrap_or(0))
                    .collect::<Vec<_>>()
            });

        Ok(Pad {
            raw_layer_id: json_raw_layer_id(json),
            designator: designator.to_string(),
            x,
            y,
            width,
            height,
            shape,
            layer,
            hole_size,
            is_plated,
            jumper_id: i16::try_from(json.get("jumper_id").and_then(Value::as_i64).unwrap_or(0))
                .unwrap_or(0),
            solder_mask_expansion_from_hole_edge,
            hole_shape,
            hole_slot_length,
            hole_rotation,
            hole_positive_tolerance,
            hole_negative_tolerance,
            rotation,
            paste_mask_expansion,
            solder_mask_expansion,
            paste_mask_expansion_mode,
            solder_mask_expansion_mode,
            power_plane_connect_style,
            relief_conductor_width,
            relief_entries,
            relief_air_gap,
            power_plane_relief_expansion,
            power_plane_clearance,
            corner_radius_percent,
            stack_mode,
            per_layer_sizes,
            per_layer_shapes,
            per_layer_corner_radii,
            per_layer_offsets,
            net_index: json_net_index(json),
            polygon_index: json_polygon_index(json),
            component_index: json_component_index(json),
            flags: json_flags(json),
            unique_id: json_unique_id(json),
            guid: json_guid(json),
            raw_tail: json_base64(json, "raw_tail"),
            identity_guid,
            identity_guid_b,
        })
    }

    /// Parses a track from JSON.
    pub(crate) fn parse_track(json: &Value) -> Result<crate::altium::pcblib::Track, String> {
        use crate::altium::pcblib::{Layer, Track};

        let x1 = json
            .get("x1")
            .and_then(Value::as_f64)
            .ok_or("Track missing required field 'x1'")?;
        let y1 = json
            .get("y1")
            .and_then(Value::as_f64)
            .ok_or("Track missing required field 'y1'")?;
        let x2 = json
            .get("x2")
            .and_then(Value::as_f64)
            .ok_or("Track missing required field 'x2'")?;
        let y2 = json
            .get("y2")
            .and_then(Value::as_f64)
            .ok_or("Track missing required field 'y2'")?;
        let width = json
            .get("width")
            .and_then(Value::as_f64)
            .ok_or("Track missing required field 'width'")?;

        let layer_str = json.get("layer").and_then(Value::as_str);
        let layer = match layer_str {
            Some(s) => Layer::parse(s).ok_or_else(|| {
                format!(
                    "Track has invalid layer '{s}'. \
                     Valid layers include: Top Layer, Bottom Layer, Top Overlay, Top Assembly, etc."
                )
            })?,
            None => Layer::TopOverlay, // Default for tracks is Top Overlay
        };

        let mut track = Track::new(x1, y1, x2, y2, width, layer);
        // Optional EE tail (mirrors the modelled optionals; absent keys keep the
        // `Track::new` defaults so a from-scratch track is byte-identical).
        track.flags = json_flags(json);
        track.net_index = json_net_index(json);
        track.polygon_index = json_polygon_index(json);
        track.component_index = json_component_index(json);
        track.solder_mask_expansion = json_f64(json, "solder_mask_expansion");
        track.keepout_restrictions = json_keepout(json);
        track.unique_id = json_unique_id(json);
        track.guid = json_guid(json);
        track.raw_layer_id = json_raw_layer_id(json);
        Ok(track)
    }

    /// Parses an arc from JSON.
    pub(crate) fn parse_arc(json: &Value) -> Result<crate::altium::pcblib::Arc, String> {
        use crate::altium::pcblib::{Arc, Layer};

        let x = json
            .get("x")
            .and_then(Value::as_f64)
            .ok_or("Arc missing required field 'x'")?;
        let y = json
            .get("y")
            .and_then(Value::as_f64)
            .ok_or("Arc missing required field 'y'")?;
        let radius = json
            .get("radius")
            .and_then(Value::as_f64)
            .ok_or("Arc missing required field 'radius'")?;
        let start_angle = json
            .get("start_angle")
            .and_then(Value::as_f64)
            .ok_or("Arc missing required field 'start_angle'")?;
        let end_angle = json
            .get("end_angle")
            .and_then(Value::as_f64)
            .ok_or("Arc missing required field 'end_angle'")?;
        let width = json
            .get("width")
            .and_then(Value::as_f64)
            .ok_or("Arc missing required field 'width'")?;

        let layer_str = json.get("layer").and_then(Value::as_str);
        let layer = match layer_str {
            Some(s) => Layer::parse(s).ok_or_else(|| {
                format!(
                    "Arc has invalid layer '{s}'. \
                     Valid layers include: Top Layer, Bottom Layer, Top Overlay, Top Assembly, etc."
                )
            })?,
            None => Layer::TopOverlay, // Default for arcs is Top Overlay
        };

        Ok(Arc {
            raw_layer_id: json_raw_layer_id(json),
            x,
            y,
            radius,
            start_angle,
            end_angle,
            width,
            layer,
            flags: json_flags(json),
            net_index: json_net_index(json),
            polygon_index: json_polygon_index(json),
            component_index: json_component_index(json),
            unique_id: json_unique_id(json),
            guid: json_guid(json),
            // Optional EE tail (mirrors the modelled optionals; absent keys keep
            // the default `None` so a from-scratch arc is byte-identical).
            solder_mask_expansion: json_f64(json, "solder_mask_expansion"),
            keepout_restrictions: json_keepout(json),
        })
    }

    /// Parses a region from JSON.
    pub(crate) fn parse_region(json: &Value) -> Option<crate::altium::pcblib::Region> {
        use crate::altium::pcblib::{Layer, Region, RegionKind, Vertex};

        let vertices_json = json.get("vertices").and_then(Value::as_array)?;
        let layer = json
            .get("layer")
            .and_then(Value::as_str)
            .and_then(Layer::parse)
            .unwrap_or(Layer::Mechanical15);

        let vertices: Vec<Vertex> = vertices_json
            .iter()
            .filter_map(|v| {
                let x = v.get("x").and_then(Value::as_f64)?;
                let y = v.get("y").and_then(Value::as_f64)?;
                Some(Vertex { x, y })
            })
            .collect();

        if vertices.len() < 3 {
            return None; // Need at least 3 vertices for a polygon
        }

        // `kind` accepts a name (matching the serde representation) or a raw
        // KIND integer. Board cutouts are not a kind of their own — AD24 stores
        // one as copper on the keep-out layer with `ISBOARDCUTOUT=TRUE`.
        let parse_kind_str = |s: &str| match s.to_ascii_lowercase().as_str() {
            "cutout" => RegionKind::Cutout,
            "copper" => RegionKind::Copper,
            "named_region" => RegionKind::NamedRegion,
            "cavity" => RegionKind::Cavity,
            other => other
                .parse::<i32>()
                .map_or(RegionKind::Copper, RegionKind::from_id),
        };
        let kind = match json.get("kind") {
            Some(v) if v.is_string() => parse_kind_str(v.as_str().unwrap_or("copper")),
            Some(v) => v
                .as_i64()
                .and_then(|i| i32::try_from(i).ok())
                .map_or(RegionKind::Copper, RegionKind::from_id),
            None => RegionKind::Copper,
        };
        let name = json
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let net_index = json_net_index(json);
        let polygon_index = json_polygon_index(json);
        let component_index = json_component_index(json);
        let cavity_height = json
            .get("cavity_height")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        // These four are always serialised by read_pcblib (no skip_serializing_if),
        // so a read-modify-write must accept AND preserve them, not reset to default.
        // Their defaults mirror Region::default() so a from-scratch region is unchanged.
        let arc_resolution = json
            .get("arc_resolution")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let sub_poly_index = json_i32(json, "sub_poly_index").unwrap_or(-1);
        let union_index = json_i32(json, "union_index").unwrap_or(0);
        let is_shape_based = json
            .get("is_shape_based")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Optional interior hole contours: an array of vertex arrays (each >= 3 pts).
        let holes: Vec<Vec<Vertex>> = json
            .get("holes")
            .and_then(Value::as_array)
            .map(|contours| {
                contours
                    .iter()
                    .filter_map(Value::as_array)
                    .map(|contour| {
                        contour
                            .iter()
                            .filter_map(|v| {
                                let x = v.get("x").and_then(Value::as_f64)?;
                                let y = v.get("y").and_then(Value::as_f64)?;
                                Some(Vertex { x, y })
                            })
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default();

        let text_field = |key: &str| json.get(key).and_then(Value::as_str).map(str::to_string);

        // Round-trip unmodelled board-region keys captured on read (a `read_pcblib`
        // emits `additional_parameters` as an array of `[key, value]` pairs; accept
        // that verbatim so a modify-write preserves them). Absent -> empty -> the
        // writer appends nothing (byte-identical to a from-scratch region).
        let additional_parameters = Self::parse_additional_parameters(json);

        Some(Region {
            vertices,
            holes,
            layer,
            // Derived from `layer` unless a read supplied a divergent token
            // (a board cutout); the tool schema does not expose it.
            v7_layer: text_field("v7_layer"),
            flags: json_flags(json),
            kind,
            name,
            net_index,
            polygon_index,
            component_index,
            arc_resolution,
            cavity_height,
            sub_poly_index,
            union_index,
            is_shape_based,
            unique_id: text_field("unique_id"),
            guid: text_field("guid"),
            additional_parameters,
            param_key_order: Self::parse_key_order(json),
        })
    }

    /// Parses the `param_key_order` list a `read_pcblib` region emits, so a
    /// tool-level read-modify-write keeps the block's original key order.
    /// Absent -> empty -> the writer's canonical order.
    pub(crate) fn parse_key_order(json: &Value) -> Vec<String> {
        json.get("param_key_order")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Parses an `additional_parameters` catch-all from a primitive's JSON: an
    /// array of `[key, value]` string pairs (the shape `read_pcblib` emits for the
    /// `Vec<(String, String)>` field). Missing/malformed entries yield an empty
    /// vector, so a from-scratch primitive re-emits nothing.
    pub(crate) fn parse_additional_parameters(json: &Value) -> Vec<(String, String)> {
        json.get("additional_parameters")
            .and_then(Value::as_array)
            .map(|pairs| {
                pairs
                    .iter()
                    .filter_map(|pair| {
                        let arr = pair.as_array()?;
                        let key = arr.first()?.as_str()?;
                        let value = arr.get(1)?.as_str()?;
                        Some((key.to_string(), value.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Parses text from JSON.
    // A flat JSON-to-field mapping: long because Text carries a lot of optional
    // properties, not because it branches.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_text(json: &Value) -> Option<crate::altium::pcblib::Text> {
        use crate::altium::pcblib::{Layer, StrokeFont, Text, TextJustification, TextKind};

        let x = json.get("x").and_then(Value::as_f64)?;
        let y = json.get("y").and_then(Value::as_f64)?;
        let text = json.get("text").and_then(Value::as_str)?;
        let height = json.get("height").and_then(Value::as_f64)?;
        let layer = json
            .get("layer")
            .and_then(Value::as_str)
            .and_then(Layer::parse)
            .unwrap_or(Layer::TopOverlay);
        let rotation = json.get("rotation").and_then(Value::as_f64).unwrap_or(0.0);
        // Optional stroke line width in mm; `None` keeps Altium's template default.
        let stroke_width = json
            .get("stroke_width")
            .and_then(Value::as_f64)
            .filter(|&w| w > 0.0);

        // Style/font fields are now authored from JSON instead of being hard-coded.
        // The string enums (`kind`, `stroke_font`, `justification`) deserialise via
        // serde so the accepted tokens match exactly what `read_pcblib` emits; an
        // absent or unparseable value falls back to the from-scratch default (which
        // keeps a default text byte-identical to the template).
        let kind = json
            .get("kind")
            .and_then(|v| serde_json::from_value::<TextKind>(v.clone()).ok())
            .unwrap_or_default();
        let stroke_font = json
            .get("stroke_font")
            .and_then(|v| serde_json::from_value::<StrokeFont>(v.clone()).ok());
        let italic = json.get("italic").and_then(Value::as_bool).unwrap_or(false);
        let bold = json.get("bold").and_then(Value::as_bool).unwrap_or(false);
        let mirror = json.get("mirror").and_then(Value::as_bool).unwrap_or(false);
        // Comment/Designator field markers (geometry @40/@41). Absent -> false,
        // the template bytes, so an unspecified text stays byte-identical.
        let is_comment = json
            .get("is_comment")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let is_designator = json
            .get("is_designator")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let font_name = json
            .get("font_name")
            .and_then(Value::as_str)
            .map_or_else(|| "Arial".to_string(), ToString::to_string);
        // The from-scratch default is `BottomLeft` (encodes to the template's
        // 0x03 byte, keeping a default text byte-identical).
        let justification = json
            .get("justification")
            .and_then(|v| serde_json::from_value::<TextJustification>(v.clone()).ok())
            .unwrap_or(TextJustification::BottomLeft);

        // Inverted (knockout) text-box descriptor. Absent booleans default to
        // false and absent dimensions to `None`, keeping a default text
        // byte-identical to the template.
        let is_inverted = json
            .get("is_inverted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let inverted_border = json.get("inverted_border").and_then(Value::as_f64);
        let use_inverted_rectangle = json
            .get("use_inverted_rectangle")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let inverted_rect_width = json.get("inverted_rect_width").and_then(Value::as_f64);
        let inverted_rect_height = json.get("inverted_rect_height").and_then(Value::as_f64);
        let inverted_rect_text_offset = json
            .get("inverted_rect_text_offset")
            .and_then(Value::as_f64);

        Some(Text {
            raw_layer_id: json_raw_layer_id(json),
            x,
            y,
            text: text.to_string(),
            height,
            layer,
            rotation,
            kind,
            stroke_font,
            stroke_width,
            italic,
            bold,
            mirror,
            is_comment,
            is_designator,
            font_name,
            justification,
            is_inverted,
            inverted_border,
            use_inverted_rectangle,
            inverted_rect_width,
            inverted_rect_height,
            inverted_rect_text_offset,
            flags: json_flags(json),
            net_index: json_net_index(json),
            polygon_index: json_polygon_index(json),
            component_index: json_component_index(json),
            unique_id: json_unique_id(json),
            guid: json_guid(json),
            raw_geometry: json_base64(json, "raw_geometry"),
            barcode_full_width: json_f64(json, "barcode_full_width"),
            barcode_full_height: json_f64(json, "barcode_full_height"),
            barcode_x_margin: json_f64(json, "barcode_x_margin"),
            barcode_y_margin: json_f64(json, "barcode_y_margin"),
            barcode_kind: u8::try_from(
                json.get("barcode_kind")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            )
            .unwrap_or(0),
            barcode_font_name: json
                .get("barcode_font_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            barcode_inverted: json
                .get("barcode_inverted")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            barcode_show_text: json
                .get("barcode_show_text")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// Parses a via from JSON.
    ///
    /// Mirrors [`Self::parse_pad`]'s layer-name parsing for the `from_layer` /
    /// Parses a `ComponentBody` (3D body) from JSON. Shared by the write-tool
    /// create path (`call_write_pcblib`) and the in-place update path
    /// (`update_pcblib_component`) so neither can silently drop bodies or drift.
    /// Every field defaults to the create handler's own value, so a
    /// from-scratch body stays byte-identical (oracle 0).
    #[allow(clippy::too_many_lines)] // ComponentBody has many optional fields
    pub(crate) fn parse_component_body_json(
        body_json: &Value,
    ) -> crate::altium::pcblib::ComponentBody {
        use crate::altium::pcblib::{ComponentBody, Layer};

        let layer = body_json
            .get("layer")
            .and_then(Value::as_str)
            .and_then(Layer::parse)
            .unwrap_or(Layer::Top3DBody);
        // Vertices in any spelling the tools accept (`{x, y}` or `[x, y]`).
        let outline = body_json
            .get("outline")
            .and_then(Value::as_array)
            .map(|verts| {
                verts
                    .iter()
                    .filter_map(crate::altium::serde_round::pair_from_json)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let f = |k: &str| body_json.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let str_or = |k: &str, d: &str| {
            body_json
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or(d)
                .to_string()
        };
        ComponentBody {
            identifier: str_or("identifier", ""),
            texture_center_x: json_guidless_opt(body_json, "texture_center_x"),
            texture_center_y: json_guidless_opt(body_json, "texture_center_y"),
            texture_size_x: json_guidless_opt(body_json, "texture_size_x"),
            texture_size_y: json_guidless_opt(body_json, "texture_size_y"),
            texture_rotation: json_guidless_opt(body_json, "texture_rotation"),
            raw_layer_id: json_raw_layer_id(body_json),
            v7_layer: json_guidless_opt(body_json, "v7_layer"),
            model_id: str_or("model_id", ""),
            model_name: str_or("model_name", ""),
            embedded: body_json
                .get("embedded")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            rotation_x: f("rotation_x"),
            rotation_y: f("rotation_y"),
            rotation_z: f("rotation_z"),
            z_offset: f("z_offset"),
            overall_height: f("overall_height"),
            standoff_height: f("standoff_height"),
            cavity_height: f("cavity_height"),
            layer,
            outline,
            unique_id: body_json
                .get("unique_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            guid: json_guid(body_json),
            model_checksum: body_json
                .get("model_checksum")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            name: str_or("name", " "),
            kind: body_json
                .get("kind")
                .and_then(Value::as_u64)
                .and_then(|v| u8::try_from(v).ok())
                .unwrap_or(0),
            sub_poly_index: body_json
                .get("sub_poly_index")
                .and_then(Value::as_i64)
                .and_then(|v| i32::try_from(v).ok())
                .unwrap_or(-1),
            union_index: body_json
                .get("union_index")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0),
            is_shape_based: body_json
                .get("is_shape_based")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            body_projection: body_json
                .get("body_projection")
                .and_then(Value::as_u64)
                .and_then(|v| u8::try_from(v).ok())
                .unwrap_or(0),
            body_color_3d: body_json
                .get("body_color_3d")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(8_421_504),
            body_opacity_3d: body_json
                .get("body_opacity_3d")
                .and_then(Value::as_f64)
                .unwrap_or(1.0),
            model_2d_rotation: body_json
                .get("model_2d_rotation")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            model_2d_x: body_json
                .get("model_2d_x")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            model_2d_y: body_json
                .get("model_2d_y")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            net_index: body_json
                .get("net_index")
                .and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok())
                .unwrap_or(0xFFFF),
            polygon_index: body_json
                .get("polygon_index")
                .and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok())
                .unwrap_or(0xFFFF),
            component_index: body_json
                .get("component_index")
                .and_then(Value::as_i64)
                .and_then(|v| i32::try_from(v).ok())
                .unwrap_or(-1),
            additional_parameters: Self::parse_additional_parameters(body_json),
            param_key_order: Self::parse_key_order(body_json),
        }
    }

    /// `to_layer` fields and reuses [`crate::altium::pcblib::MaskExpansionMode`]
    /// string parsing for the mask mode. Optionals default exactly as
    /// [`crate::altium::pcblib::Via::new`] does when absent.
    #[allow(clippy::too_many_lines)] // Via has many optional fields requiring individual parsing
    pub(crate) fn parse_via(json: &Value) -> Result<crate::altium::pcblib::Via, String> {
        use crate::altium::pcblib::{
            DrillLayerPairType, Layer, MaskExpansionMode, PowerPlaneConnectStyle, Via, ViaStackMode,
        };

        let x = json
            .get("x")
            .and_then(Value::as_f64)
            .ok_or("Via missing required field 'x'")?;
        let y = json
            .get("y")
            .and_then(Value::as_f64)
            .ok_or("Via missing required field 'y'")?;
        let diameter = json
            .get("diameter")
            .and_then(Value::as_f64)
            .ok_or("Via missing required field 'diameter'")?;
        let hole_size = json
            .get("hole_size")
            .and_then(Value::as_f64)
            .ok_or("Via missing required field 'hole_size'")?;

        // Validate via dimensions are sensible: the hole must fit inside the
        // annular ring, both positive.
        if diameter <= 0.0 {
            return Err(format!(
                "Via has invalid diameter {diameter}. Diameter must be greater than 0."
            ));
        }
        if hole_size <= 0.0 {
            return Err(format!(
                "Via has invalid hole_size {hole_size}. Hole size must be greater than 0."
            ));
        }
        if hole_size >= diameter {
            return Err(format!(
                "Via hole_size {hole_size} must be smaller than diameter {diameter}."
            ));
        }

        // Start from the struct's defaults (top->bottom layers, standard thermal
        // relief), then override with any supplied fields.
        let mut via = Via::new(x, y, diameter, hole_size);

        if let Some(s) = json.get("from_layer").and_then(Value::as_str) {
            via.from_layer = Layer::parse(s).ok_or_else(|| {
                format!(
                    "Via has invalid from_layer '{s}'. \
                     Valid layers include: Top Layer, Bottom Layer, Mid-Layer 1, etc."
                )
            })?;
        }
        if let Some(s) = json.get("to_layer").and_then(Value::as_str) {
            via.to_layer = Layer::parse(s).ok_or_else(|| {
                format!(
                    "Via has invalid to_layer '{s}'. \
                     Valid layers include: Top Layer, Bottom Layer, Mid-Layer 1, etc."
                )
            })?;
        }

        if let Some(v) = json.get("solder_mask_expansion").and_then(Value::as_f64) {
            via.solder_mask_expansion = v;
        }
        if let Some(b) = json
            .get("solder_mask_expansion_from_hole_edge")
            .and_then(Value::as_bool)
        {
            via.solder_mask_expansion_from_hole_edge = b;
        }
        if let Some(s) = json.get("drill_layer_pair_type").and_then(Value::as_str) {
            via.drill_layer_pair_type = match s.to_lowercase().as_str() {
                "blind_buried_start" => DrillLayerPairType::BlindBuriedStart,
                "mid" => DrillLayerPairType::Mid,
                "end" => DrillLayerPairType::End,
                _ => DrillLayerPairType::Through,
            };
        }
        if let Some(s) = json
            .get("solder_mask_expansion_mode")
            .and_then(Value::as_str)
        {
            via.solder_mask_expansion_mode = match s.to_lowercase().as_str() {
                "manual" => MaskExpansionMode::Manual,
                "from_rule" => MaskExpansionMode::FromRule,
                // Anything else, "none" included, maps to the state a fresh
                // Altium pad carries, so an unrecognised value produces the
                // same bytes as saying nothing at all.
                _ => MaskExpansionMode::None,
            };
        }
        if let Some(v) = json.get("thermal_relief_gap").and_then(Value::as_f64) {
            via.thermal_relief_gap = v;
        }
        if let Some(v) = json
            .get("thermal_relief_conductors")
            .and_then(Value::as_u64)
            .and_then(|v| u8::try_from(v).ok())
        {
            via.thermal_relief_conductors = v;
        }
        if let Some(v) = json.get("thermal_relief_width").and_then(Value::as_f64) {
            via.thermal_relief_width = v;
        }

        // Power-plane connection (SubRecord-1 @31/@42/@46) + paste-mask @50 +
        // net index @3. Absent keys keep the from-scratch defaults (= Altium's
        // via template), so an unspecified via round-trips byte-identically.
        if let Some(s) = json
            .get("power_plane_connect_style")
            .and_then(Value::as_str)
        {
            via.power_plane_connect_style = match s.to_lowercase().as_str() {
                "direct" => PowerPlaneConnectStyle::Direct,
                "no_connect" | "noconnect" => PowerPlaneConnectStyle::NoConnect,
                _ => PowerPlaneConnectStyle::Relief,
            };
        }
        if let Some(v) = json
            .get("power_plane_relief_expansion")
            .and_then(Value::as_f64)
        {
            via.power_plane_relief_expansion = v;
        }
        if let Some(v) = json.get("power_plane_clearance").and_then(Value::as_f64) {
            via.power_plane_clearance = v;
        }
        if let Some(v) = json.get("paste_mask_expansion").and_then(Value::as_f64) {
            via.paste_mask_expansion = v;
        }
        if let Some(v) = json
            .get("net_index")
            .and_then(Value::as_u64)
            .and_then(|v| u16::try_from(v).ok())
        {
            via.net_index = v;
        }
        // Polygon @5 / component @7 connectivity indices. Absent keys keep the
        // from-scratch defaults (none / free primitive), byte-identical.
        via.polygon_index = json_polygon_index(json);
        via.component_index = json_component_index(json);

        // Drill tolerances (SubRecord-1 @291/@295). Absent keys keep the
        // from-scratch defaults (tolerances unset), so an unspecified via
        // round-trips byte-identically.
        if let Some(v) = json.get("hole_positive_tolerance").and_then(Value::as_f64) {
            via.hole_positive_tolerance = Some(v);
        }
        if let Some(v) = json.get("hole_negative_tolerance").and_then(Value::as_f64) {
            via.hole_negative_tolerance = Some(v);
        }

        if let Some(v) = json
            .get("solder_mask_expansion_back")
            .and_then(Value::as_f64)
        {
            via.solder_mask_expansion_back = Some(v);
        }
        via.diameter_stack_mode = match json
            .get("diameter_stack_mode")
            .and_then(Value::as_str)
            .map(str::to_lowercase)
            .as_deref()
        {
            Some("top_middle_bottom") => ViaStackMode::TopMiddleBottom,
            Some("full_stack") => ViaStackMode::FullStack,
            _ => ViaStackMode::Simple,
        };
        if let Some(diameters) = json.get("per_layer_diameters").and_then(Value::as_array) {
            via.per_layer_diameters = Some(diameters.iter().filter_map(Value::as_f64).collect());
        }

        via.flags = json_flags(json);
        via.unique_id = json_unique_id(json);
        via.guid = json_guid(json);
        via.raw_block = json_base64(json, "raw_block");

        Ok(via)
    }

    /// Parses a fill from JSON.
    ///
    /// Reuses [`Self::parse_pad`]'s layer-name parsing for `layer`. Geometry
    /// (`x1`/`y1`/`x2`/`y2`) is required; `rotation` and the mask/keepout
    /// optionals default as [`crate::altium::pcblib::Fill::new`] does when absent.
    pub(crate) fn parse_fill(json: &Value) -> Result<crate::altium::pcblib::Fill, String> {
        use crate::altium::pcblib::{Fill, Layer};

        let x1 = json
            .get("x1")
            .and_then(Value::as_f64)
            .ok_or("Fill missing required field 'x1'")?;
        let y1 = json
            .get("y1")
            .and_then(Value::as_f64)
            .ok_or("Fill missing required field 'y1'")?;
        let x2 = json
            .get("x2")
            .and_then(Value::as_f64)
            .ok_or("Fill missing required field 'x2'")?;
        let y2 = json
            .get("y2")
            .and_then(Value::as_f64)
            .ok_or("Fill missing required field 'y2'")?;

        let layer_str = json.get("layer").and_then(Value::as_str);
        let layer = match layer_str {
            Some(s) => Layer::parse(s).ok_or_else(|| {
                format!(
                    "Fill has invalid layer '{s}'. \
                     Valid layers include: Top Layer, Bottom Layer, Top Overlay, Mechanical 1, etc."
                )
            })?,
            None => Layer::TopLayer, // Default for fills is Top Layer
        };

        let mut fill = Fill::new(x1, y1, x2, y2, layer);

        if let Some(r) = json.get("rotation").and_then(Value::as_f64) {
            fill.rotation = r;
        }
        // Optional flags + mask/keepout tail (mirrors the modelled optionals).
        fill.flags = json_flags(json);
        fill.net_index = json_net_index(json);
        fill.polygon_index = json_polygon_index(json);
        fill.component_index = json_component_index(json);
        fill.solder_mask_expansion = json.get("solder_mask_expansion").and_then(Value::as_f64);
        fill.keepout_restrictions = json_keepout(json);
        fill.unique_id = json_unique_id(json);
        fill.guid = json_guid(json);
        fill.raw_layer_id = json_raw_layer_id(json);

        Ok(fill)
    }

    // ==================== SchLib Primitive Parsing Helpers ====================

    /// Parses a schematic pin from JSON.
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::too_many_lines)] // Pin parsing with symbol attributes requires many lines
    pub(crate) fn parse_schlib_pin(json: &Value) -> Option<crate::altium::schlib::Pin> {
        use crate::altium::schlib::{Pin, PinElectricalType, PinOrientation, PinSymbol};

        let designator = json.get("designator").and_then(Value::as_str)?;
        let name = json
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(designator);
        let x = json_i32(json, "x")?;
        let y = json_i32(json, "y")?;
        let length = json_i32(json, "length").unwrap_or(10);

        let orientation =
            json.get("orientation")
                .and_then(Value::as_str)
                .map_or(PinOrientation::Right, |s| match s.to_lowercase().as_str() {
                    "left" => PinOrientation::Left,
                    "up" => PinOrientation::Up,
                    "down" => PinOrientation::Down,
                    _ => PinOrientation::Right,
                });

        let electrical_type = json.get("electrical_type").and_then(Value::as_str).map_or(
            PinElectricalType::Passive,
            |s| match s.to_lowercase().as_str() {
                "input" => PinElectricalType::Input,
                "output" => PinElectricalType::Output,
                "bidirectional" | "io" | "input_output" => PinElectricalType::Bidirectional,
                "power" => PinElectricalType::Power,
                "open_collector" => PinElectricalType::OpenCollector,
                "open_emitter" => PinElectricalType::OpenEmitter,
                "hiz" | "hi_z" | "tristate" => PinElectricalType::HiZ,
                _ => PinElectricalType::Passive,
            },
        );

        let hidden = json.get("hidden").and_then(Value::as_bool).unwrap_or(false);
        let show_name = json
            .get("show_name")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let show_designator = json
            .get("show_designator")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        // Helper to parse PinSymbol from string
        let parse_pin_symbol = |s: &str| -> PinSymbol {
            match s.to_lowercase().replace(['-', '_'], "").as_str() {
                "dot" | "invert" | "inversion" => PinSymbol::Dot,
                "clock" | "clk" => PinSymbol::Clock,
                "activelowinput" | "lowinput" => PinSymbol::ActiveLowInput,
                "activelowoutput" | "lowoutput" => PinSymbol::ActiveLowOutput,
                "rightleftsignalflow" | "rightleft" => PinSymbol::RightLeftSignalFlow,
                "leftrightsignalflow" | "leftright" => PinSymbol::LeftRightSignalFlow,
                "bidirectionalsignalflow" | "bidirectional" => PinSymbol::BidirectionalSignalFlow,
                "analogsignalin" | "analog" => PinSymbol::AnalogSignalIn,
                "digitalsignalin" | "digital" => PinSymbol::DigitalSignalIn,
                "notlogicconnection" | "notlogic" => PinSymbol::NotLogicConnection,
                "postponedoutput" | "postponed" => PinSymbol::PostponedOutput,
                "opencollector" => PinSymbol::OpenCollector,
                "opencollectorpullup" => PinSymbol::OpenCollectorPullUp,
                "openemitter" => PinSymbol::OpenEmitter,
                "openemitterpullup" => PinSymbol::OpenEmitterPullUp,
                "openoutput" => PinSymbol::OpenOutput,
                "hiz" | "highimpedance" => PinSymbol::HiZ,
                "highcurrent" => PinSymbol::HighCurrent,
                "pulse" => PinSymbol::Pulse,
                "schmitt" | "schmitttrigger" => PinSymbol::Schmitt,
                "shiftleft" => PinSymbol::ShiftLeft,
                _ => PinSymbol::None, // "none" and unknown values
            }
        };

        let symbol_inner_edge = json
            .get("symbol_inner_edge")
            .and_then(Value::as_str)
            .map_or(PinSymbol::None, parse_pin_symbol);
        let symbol_outer_edge = json
            .get("symbol_outer_edge")
            .and_then(Value::as_str)
            .map_or(PinSymbol::None, parse_pin_symbol);
        let symbol_inside = json
            .get("symbol_inside")
            .and_then(Value::as_str)
            .map_or(PinSymbol::None, parse_pin_symbol);
        let symbol_outside = json
            .get("symbol_outside")
            .and_then(Value::as_str)
            .map_or(PinSymbol::None, parse_pin_symbol);

        // Authoring fields read from JSON so an
        // AI can set them, matching the names `read_schlib` exposes (serialised
        // straight from the `Pin` struct). `colour` is a BGR integer; absent keys
        // keep the from-scratch defaults (`part_and_sequence` defaults to "|&|").
        let description = json
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let colour = json.get("colour").and_then(Value::as_u64).unwrap_or(0) as u32;
        let graphically_locked = json
            .get("graphically_locked")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let swap_id_group = json
            .get("swap_id_group")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let part_and_sequence = json
            .get("part_and_sequence")
            .and_then(Value::as_str)
            .unwrap_or("|&|")
            .to_string();
        let default_value = json
            .get("default_value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        // Pin binary-record display mode (own byte, distinct from the shape flag).
        let owner_part_display_mode = json_i32(json, "owner_part_display_mode").unwrap_or(0);
        // Symbol line width; 0 (default) writes no PinSymbolLineWidth aux stream.
        let symbol_line_width = json_i32(json, "symbol_line_width").unwrap_or(0);
        // Fractional pin coordinates ({x,y,length} in 1/100000 DXP units); an
        // all-zero or absent object writes no PinFrac aux stream.
        let frac = json.get("frac").and_then(|f| {
            let pf = crate::altium::schlib::PinFrac {
                x: json_i32(f, "x").unwrap_or(0),
                y: json_i32(f, "y").unwrap_or(0),
                length: json_i32(f, "length").unwrap_or(0),
            };
            (!pf.is_zero()).then_some(pf)
        });
        // Both fields are always serialised by read_schlib (no skip_serializing_if),
        // so a read-modify-write round-trip must accept and preserve them rather than
        // reset them to a hard-coded default. `is_not_accessible` defaults false (the
        // pin conglomerate `0x20` bit); `formal_type` defaults 1 (Altium's normal pin).
        let is_not_accessible = json
            .get("is_not_accessible")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let formal_type = json
            .get("formal_type")
            .and_then(Value::as_u64)
            .and_then(|v| u8::try_from(v).ok())
            .unwrap_or(1);

        Some(Pin {
            name: name.to_string(),
            designator: designator.to_string(),
            x,
            y,
            length,
            orientation,
            electrical_type,
            hidden,
            show_name,
            show_designator,
            description,
            owner_part_id,
            owner_part_display_mode,
            colour,
            graphically_locked,
            symbol_inner_edge,
            symbol_outer_edge,
            symbol_inside,
            symbol_outside,
            is_not_accessible,
            formal_type,
            swap_id_group,
            part_and_sequence,
            default_value,
            symbol_line_width,
            frac,
        })
    }

    /// Parses a schematic rectangle from JSON.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_rectangle(json: &Value) -> Option<crate::altium::schlib::Rectangle> {
        use crate::altium::schlib::Rectangle;

        let x1 = json_f64(json, "x1")?;
        let y1 = json_f64(json, "y1")?;
        let x2 = json_f64(json, "x2")?;
        let y2 = json_f64(json, "y2")?;

        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let line_color = json
            .get("line_color")
            .and_then(Value::as_u64)
            .unwrap_or(0x00_00_80) as u32;
        let fill_color = json
            .get("fill_color")
            .and_then(Value::as_u64)
            .unwrap_or(0xB0_FF_FF) as u32;
        let filled = json.get("filled").and_then(Value::as_bool).unwrap_or(true);
        // Style fields read from JSON (matches the
        // names `read_schlib` exposes). `line_style`: 0=Solid, 1=Dashed, 2=Dotted.
        let line_style = json.get("line_style").and_then(Value::as_u64).unwrap_or(0) as u8;
        let transparent = json
            .get("transparent")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(Rectangle {
            x1,
            y1,
            x2,
            y2,
            line_width,
            line_color,
            fill_color,
            line_style,
            filled,
            transparent,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic rounded rectangle from JSON.
    ///
    /// Mirrors [`Self::parse_schlib_rectangle`] (geometry + fill/border colours +
    /// `filled`), adding the `corner_x_radius` / `corner_y_radius` rounding fields.
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::similar_names)] // corner_x_radius / corner_y_radius mirror the struct fields
    pub(crate) fn parse_schlib_round_rect(
        json: &Value,
    ) -> Option<crate::altium::schlib::RoundRect> {
        use crate::altium::schlib::RoundRect;

        let x1 = json_f64(json, "x1")?;
        let y1 = json_f64(json, "y1")?;
        let x2 = json_f64(json, "x2")?;
        let y2 = json_f64(json, "y2")?;
        let corner_x_radius = json_f64(json, "corner_x_radius").unwrap_or(0.0);
        let corner_y_radius = json_f64(json, "corner_y_radius").unwrap_or(0.0);

        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let line_color = json
            .get("line_color")
            .and_then(Value::as_u64)
            .unwrap_or(0x00_00_80) as u32;
        let fill_color = json
            .get("fill_color")
            .and_then(Value::as_u64)
            .unwrap_or(0xB0_FF_FF) as u32;
        let filled = json.get("filled").and_then(Value::as_bool).unwrap_or(true);
        // Style fields read from JSON (matches the
        // names `read_schlib` exposes). `line_style`: 0=Solid, 1=Dashed, 2=Dotted.
        let line_style = json.get("line_style").and_then(Value::as_u64).unwrap_or(0) as u8;
        let transparent = json
            .get("transparent")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(RoundRect {
            x1,
            y1,
            x2,
            y2,
            corner_x_radius,
            corner_y_radius,
            line_width,
            line_color,
            fill_color,
            line_style,
            filled,
            transparent,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic line from JSON.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_line(json: &Value) -> Option<crate::altium::schlib::Line> {
        use crate::altium::schlib::Line;

        // Coordinates accept fractional values; integer-only `json_i32` would drop
        // an off-grid endpoint like 3.75.
        let x1 = json_f64(json, "x1")?;
        let y1 = json_f64(json, "y1")?;
        let x2 = json_f64(json, "x2")?;
        let y2 = json_f64(json, "y2")?;

        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let color = json
            .get("color")
            .and_then(Value::as_u64)
            .unwrap_or(0x00_00_80) as u32;
        // `line_style` read from JSON (matches the name
        // `read_schlib` exposes). 0=Solid, 1=Dashed, 2=Dotted.
        let line_style = json.get("line_style").and_then(Value::as_u64).unwrap_or(0) as u8;
        // `is_not_accessible` read from JSON, defaulting true (matches
        // the name `read_schlib` exposes). Altium tags every line, so default true.
        let is_not_accessible = json
            .get("is_not_accessible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(Line {
            x1,
            y1,
            x2,
            y2,
            line_width,
            color,
            line_style,
            is_not_accessible,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic parameter from JSON.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_parameter(json: &Value) -> Option<crate::altium::schlib::Parameter> {
        use crate::altium::schlib::Parameter;

        let name = json.get("name").and_then(Value::as_str)?;
        let value = json
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("*")
            .to_string();

        let x = json_f64(json, "x").unwrap_or(0.0);
        let y = json_f64(json, "y").unwrap_or(0.0);
        let font_id = json.get("font_id").and_then(Value::as_u64).unwrap_or(1) as u8;
        let color = json
            .get("color")
            .and_then(Value::as_u64)
            .unwrap_or(0x80_00_00) as u32;
        let hidden = json.get("hidden").and_then(Value::as_bool).unwrap_or(false);
        // De-hardcoded: the core already models these, so read them from JSON.
        // Defaults equal the previous hard-coded values, keeping a default
        // parameter byte-identical.
        let read_only_state = json
            .get("read_only_state")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u8;
        let param_type = json.get("param_type").and_then(Value::as_u64).unwrap_or(0) as u8;
        let unique_id = json
            .get("unique_id")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        // EE-meaningful display fields (omit-when-default).
        let orientation = json_i32(json, "orientation").unwrap_or(0);
        // Altium anchor id 0-8 (0 = bottom-left ... 8 = top-right); default 0.
        let justification = json
            .get("justification")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u8;
        let show_name = json
            .get("show_name")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let hide_name = json
            .get("hide_name")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let description = json
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let is_configurable = json
            .get("is_configurable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let auto_position = json
            .get("auto_position")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let is_rule = json
            .get("is_rule")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let is_system_parameter = json
            .get("is_system_parameter")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text_horz_anchor = json
            .get("text_horz_anchor")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u8;
        let text_vert_anchor = json
            .get("text_vert_anchor")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u8;
        let is_mirrored = json
            .get("is_mirrored")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(Parameter {
            raw_params: json_raw_params(json),
            name: name.to_string(),
            value,
            x,
            y,
            font_id,
            color,
            hidden,
            read_only_state,
            param_type,
            orientation,
            justification,
            is_mirrored,
            show_name,
            hide_name,
            description,
            is_configurable,
            auto_position,
            is_rule,
            is_system_parameter,
            text_horz_anchor,
            text_vert_anchor,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id,
        })
    }

    /// Parses a schematic polyline from JSON.
    /// Accepts both "points" and "vertices" field names for the coordinate array.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_polyline(json: &Value) -> Option<crate::altium::schlib::Polyline> {
        use crate::altium::schlib::Polyline;

        // Accept both "points" and "vertices" for flexibility
        let points_json = json
            .get("points")
            .or_else(|| json.get("vertices"))
            .and_then(Value::as_array)?;
        // Each point in any spelling the tools accept (`{x, y}` or `[x, y]`).
        let points: Vec<(f64, f64)> = points_json
            .iter()
            .filter_map(crate::altium::serde_round::pair_from_json)
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .collect();

        if points.len() < 2 {
            return None; // Need at least 2 points for a polyline
        }

        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let color = json
            .get("color")
            .and_then(Value::as_u64)
            .unwrap_or(0x00_00_80) as u32;
        // Style + arrowhead fields read from JSON
        // (matches the names `read_schlib` exposes). `line_style`: 0=Solid,
        // 1=Dashed, 2=Dotted. `start_line_shape`/`end_line_shape` are endpoint
        // (arrowhead) shapes and `line_shape_size` their size.
        let line_style = json.get("line_style").and_then(Value::as_u64).unwrap_or(0) as u8;
        let start_line_shape = json
            .get("start_line_shape")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u8;
        let end_line_shape = json
            .get("end_line_shape")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u8;
        let line_shape_size = json
            .get("line_shape_size")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u8;
        let transparent = json
            .get("transparent")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // `is_not_accessible` read from JSON (matches
        // the name `read_schlib` exposes). Altium tags every polyline, so
        // default true.
        let is_not_accessible = json
            .get("is_not_accessible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(Polyline {
            points,
            line_width,
            color,
            line_style,
            start_line_shape,
            end_line_shape,
            line_shape_size,
            transparent,
            is_not_accessible,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic filled polygon from JSON.
    ///
    /// Mirrors [`Self::parse_schlib_polyline`] (reads the `points`/`vertices`
    /// array of `[x, y]` pairs), adding the polygon's `filled` / `fill_color`
    /// fields.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_polygon(json: &Value) -> Option<crate::altium::schlib::Polygon> {
        use crate::altium::schlib::Polygon;

        // Accept both "points" and "vertices" for flexibility (matches polyline).
        let points_json = json
            .get("points")
            .or_else(|| json.get("vertices"))
            .and_then(Value::as_array)?;
        // Each point in any spelling the tools accept (`{x, y}` or `[x, y]`).
        let points: Vec<(f64, f64)> = points_json
            .iter()
            .filter_map(crate::altium::serde_round::pair_from_json)
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .collect();

        if points.len() < 3 {
            return None; // Need at least 3 vertices for a polygon
        }

        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let line_color = json
            .get("line_color")
            .and_then(Value::as_u64)
            .unwrap_or(0x00_00_80) as u32;
        let fill_color = json
            .get("fill_color")
            .and_then(Value::as_u64)
            .unwrap_or(0xB0_FF_FF) as u32;
        let line_style = json.get("line_style").and_then(Value::as_u64).unwrap_or(0) as u8;
        let filled = json.get("filled").and_then(Value::as_bool).unwrap_or(true);
        let transparent = json
            .get("transparent")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let is_not_accessible = json
            .get("is_not_accessible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(Polygon {
            points,
            line_width,
            line_color,
            fill_color,
            line_style,
            filled,
            transparent,
            is_not_accessible,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic arc from JSON.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_arc(json: &Value) -> Option<crate::altium::schlib::Arc> {
        use crate::altium::schlib::Arc;

        let x = json_f64(json, "x")?;
        let y = json_f64(json, "y")?;
        let radius = json_f64(json, "radius")?;
        let start_angle = json
            .get("start_angle")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let end_angle = json
            .get("end_angle")
            .and_then(Value::as_f64)
            .unwrap_or(360.0);
        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let color = json
            .get("color")
            .and_then(Value::as_u64)
            .unwrap_or(0x00_00_80) as u32;
        // `fill_color` read from JSON, defaulting 0 (matches the name
        // `read_schlib` exposes). Maps to the `AreaColor` param; 0 = no fill.
        let fill_color = json.get("fill_color").and_then(Value::as_u64).unwrap_or(0) as u32;
        // `is_not_accessible` read from JSON, defaulting true (matches
        // the name `read_schlib` exposes). Altium tags every arc, so default true.
        let is_not_accessible = json
            .get("is_not_accessible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(Arc {
            x,
            y,
            radius,
            is_not_accessible,
            start_angle,
            end_angle,
            line_width,
            color,
            fill_color,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic pie (filled circular sector, `RECORD=9`) from JSON.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_pie(json: &Value) -> Option<crate::altium::schlib::Pie> {
        use crate::altium::schlib::Pie;

        let x = json_f64(json, "x")?;
        let y = json_f64(json, "y")?;
        let radius = json_f64(json, "radius")?;
        let start_angle = json
            .get("start_angle")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let end_angle = json
            .get("end_angle")
            .and_then(Value::as_f64)
            .unwrap_or(360.0);
        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let line_color = json.get("line_color").and_then(Value::as_u64).unwrap_or(0) as u32;
        let fill_color = json.get("fill_color").and_then(Value::as_u64).unwrap_or(0) as u32;
        let filled = json.get("filled").and_then(Value::as_bool).unwrap_or(true);
        let transparent = json
            .get("transparent")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let is_not_accessible = json
            .get("is_not_accessible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(Pie {
            x,
            y,
            radius,
            is_not_accessible,
            start_angle,
            end_angle,
            line_width,
            line_color,
            fill_color,
            filled,
            transparent,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic image (embedded/linked picture, `RECORD=30`) from JSON.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_image(json: &Value) -> Option<crate::altium::schlib::Image> {
        use crate::altium::schlib::Image;

        let x1 = json_f64(json, "x1")?;
        let y1 = json_f64(json, "y1")?;
        let x2 = json_f64(json, "x2")?;
        let y2 = json_f64(json, "y2")?;
        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let line_color = json.get("line_color").and_then(Value::as_u64).unwrap_or(0) as u32;
        let line_style = json.get("line_style").and_then(Value::as_u64).unwrap_or(0) as u8;
        let fill_color = json.get("fill_color").and_then(Value::as_u64).unwrap_or(0) as u32;
        let b = |k: &str| json.get(k).and_then(Value::as_bool).unwrap_or(false);
        let is_not_accessible = json
            .get("is_not_accessible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let file_name = json
            .get("file_name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        // Base64-encoded raw image bytes destined for the library /Storage
        // stream.
        let image_data = json_base64(json, "image_data");
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(Image {
            x1,
            y1,
            x2,
            y2,
            is_not_accessible,
            line_width,
            line_color,
            line_style,
            fill_color,
            filled: b("filled"),
            transparent: b("transparent"),
            show_border: b("show_border"),
            keep_aspect: b("keep_aspect"),
            embed_image: b("embed_image"),
            file_name,
            image_data,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic text frame (bordered multi-line text box,
    /// `RECORD=28`) from JSON. Requires the frame box (`x1`..`y2`) and `text`;
    /// optionals default as [`crate::altium::schlib::TextFrame::new`] does when
    /// absent (white fill, centre alignment, border/word-wrap/clip-to-rect on).
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_text_frame(
        json: &Value,
    ) -> Option<crate::altium::schlib::TextFrame> {
        use crate::altium::schlib::TextFrame;

        let x1 = json_f64(json, "x1")?;
        let y1 = json_f64(json, "y1")?;
        let x2 = json_f64(json, "x2")?;
        let y2 = json_f64(json, "y2")?;
        let text = json.get("text").and_then(Value::as_str)?.to_string();
        let color = json.get("color").and_then(Value::as_u64).unwrap_or(0) as u32;
        let area_color = json
            .get("area_color")
            .and_then(Value::as_u64)
            .unwrap_or(16_777_215) as u32;
        let text_color = json.get("text_color").and_then(Value::as_u64).unwrap_or(0) as u32;
        let text_margin = json
            .get("text_margin")
            .and_then(Value::as_f64)
            .unwrap_or(0.000_05);
        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(0) as u8;
        let line_style = json.get("line_style").and_then(Value::as_u64).unwrap_or(0) as u8;
        let font_id = json.get("font_id").and_then(Value::as_u64).unwrap_or(1) as u8;
        let orientation = json.get("orientation").and_then(Value::as_u64).unwrap_or(0) as u8;
        let alignment = json.get("alignment").and_then(Value::as_u64).unwrap_or(1) as u8;
        let b_false = |k: &str| json.get(k).and_then(Value::as_bool).unwrap_or(false);
        let b_true = |k: &str| json.get(k).and_then(Value::as_bool).unwrap_or(true);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(TextFrame {
            x1,
            y1,
            x2,
            y2,
            text,
            color,
            area_color,
            text_color,
            text_margin,
            line_width,
            line_style,
            transparent: b_false("transparent"),
            font_id,
            orientation,
            alignment,
            is_solid: b_false("is_solid"),
            show_border: b_true("show_border"),
            word_wrap: b_true("word_wrap"),
            clip_to_rect: b_true("clip_to_rect"),
            is_not_accessible: b_true("is_not_accessible"),
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a `SchLib` Bezier from JSON. Requires the four control points
    /// (`x1`..`y4`); optionals default as [`crate::altium::schlib::Bezier::new`]
    /// does when absent.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_bezier(json: &Value) -> Option<crate::altium::schlib::Bezier> {
        use crate::altium::schlib::Bezier;

        let x1 = json_f64(json, "x1")?;
        let y1 = json_f64(json, "y1")?;
        let x2 = json_f64(json, "x2")?;
        let y2 = json_f64(json, "y2")?;
        let x3 = json_f64(json, "x3")?;
        let y3 = json_f64(json, "y3")?;
        let x4 = json_f64(json, "x4")?;
        let y4 = json_f64(json, "y4")?;
        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let color = json
            .get("color")
            .and_then(Value::as_u64)
            .unwrap_or(0x00_00_80) as u32;
        let is_not_accessible = json
            .get("is_not_accessible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(Bezier {
            x1,
            y1,
            x2,
            y2,
            x3,
            y3,
            x4,
            y4,
            line_width,
            color,
            is_not_accessible,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a `SchLib` elliptical arc from JSON. Requires centre and both
    /// radii; optionals default as
    /// [`crate::altium::schlib::EllipticalArc::new`] does when absent.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_elliptical_arc(
        json: &Value,
    ) -> Option<crate::altium::schlib::EllipticalArc> {
        use crate::altium::schlib::EllipticalArc;

        let x = json_f64(json, "x")?;
        let y = json_f64(json, "y")?;
        let radius = json_f64(json, "radius")?;
        let secondary_radius = json_f64(json, "secondary_radius")?;
        let start_angle = json
            .get("start_angle")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let end_angle = json
            .get("end_angle")
            .and_then(Value::as_f64)
            .unwrap_or(360.0);
        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let color = json
            .get("color")
            .and_then(Value::as_u64)
            .unwrap_or(0x00_00_80) as u32;
        let fill_color = json.get("fill_color").and_then(Value::as_u64).unwrap_or(0) as u32;
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(EllipticalArc {
            x,
            y,
            radius,
            secondary_radius,
            start_angle,
            end_angle,
            line_width,
            color,
            fill_color,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic ellipse from JSON.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_ellipse(json: &Value) -> Option<crate::altium::schlib::Ellipse> {
        use crate::altium::schlib::Ellipse;

        let x = json_f64(json, "x")?;
        let y = json_f64(json, "y")?;
        let radius_x = json_f64(json, "radius_x")?;
        let radius_y = json_f64(json, "radius_y")?;

        let line_width = json.get("line_width").and_then(Value::as_u64).unwrap_or(1) as u8;
        let line_color = json
            .get("line_color")
            .and_then(Value::as_u64)
            .unwrap_or(0x00_00_80) as u32;
        let fill_color = json
            .get("fill_color")
            .and_then(Value::as_u64)
            .unwrap_or(0xB0_FF_FF) as u32;
        let filled = json.get("filled").and_then(Value::as_bool).unwrap_or(true);
        // `transparent` read from JSON (matches the name
        // `read_schlib` exposes). The ellipse struct carries no `line_style`.
        let transparent = json
            .get("transparent")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // `is_not_accessible` read from JSON (matches
        // the name `read_schlib` exposes). Altium tags every ellipse, so
        // default true.
        let is_not_accessible = json
            .get("is_not_accessible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(Ellipse {
            x,
            y,
            radius_x,
            radius_y,
            line_width,
            line_color,
            fill_color,
            filled,
            transparent,
            is_not_accessible,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic label from JSON.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn parse_schlib_label(json: &Value) -> Option<crate::altium::schlib::Label> {
        use crate::altium::schlib::{Label, TextJustification};

        let x = json_f64(json, "x")?;
        let y = json_f64(json, "y")?;
        let text = json.get("text").and_then(Value::as_str)?.to_string();

        let font_id = json.get("font_id").and_then(Value::as_u64).unwrap_or(1) as u8;
        let color = json
            .get("color")
            .and_then(Value::as_u64)
            .unwrap_or(0x00_00_80) as u32;
        let rotation = json.get("rotation").and_then(Value::as_f64).unwrap_or(0.0);
        let is_mirrored = json
            .get("is_mirrored")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let is_hidden = json
            .get("is_hidden")
            .or_else(|| json.get("hidden"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        let justification = json.get("justification").and_then(Value::as_str).map_or(
            TextJustification::BottomLeft,
            |s| {
                match s.to_lowercase().replace(['-', '_'], "").as_str() {
                    "bottomcenter" | "bottomcentre" => TextJustification::BottomCenter,
                    "bottomright" => TextJustification::BottomRight,
                    "middleleft" | "centerleft" | "centreleft" => TextJustification::MiddleLeft,
                    "middlecenter" | "middlecentre" | "center" | "centre" => {
                        TextJustification::MiddleCenter
                    }
                    "middleright" | "centerright" | "centreright" => TextJustification::MiddleRight,
                    "topleft" => TextJustification::TopLeft,
                    "topcenter" | "topcentre" => TextJustification::TopCenter,
                    "topright" => TextJustification::TopRight,
                    _ => TextJustification::BottomLeft, // "bottomleft" and unknown values
                }
            },
        );

        Some(Label {
            x,
            y,
            text,
            font_id,
            color,
            justification,
            rotation,
            is_mirrored,
            is_hidden,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            unique_id: json_unique_id(json),
            raw_params: json_raw_params(json),
        })
    }

    /// Parses a schematic IEEE symbol (RECORD=3) from JSON. `symbol` is
    /// Altium's `TIeeeSymbol` id (1 Dot, 3 Clock, …); see
    /// `docs/SCHLIB_FORMAT.md` for the table.
    pub(crate) fn parse_schlib_ieee_symbol(
        json: &Value,
    ) -> Option<crate::altium::schlib::IeeeSymbol> {
        use crate::altium::schlib::IeeeSymbol;

        let x = json_f64(json, "x")?;
        let y = json_f64(json, "y")?;
        let symbol = json
            .get("symbol")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())?;
        let scale_factor = json_f64(json, "scale_factor").unwrap_or(10.0);
        let rotation = json_f64(json, "rotation").unwrap_or(0.0);
        let is_mirrored = json
            .get("is_mirrored")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let line_width = json
            .get("line_width")
            .and_then(Value::as_u64)
            .and_then(|v| u8::try_from(v).ok())
            .unwrap_or(1);
        let color = json
            .get("color")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let owner_part_id = json_i32(json, "owner_part_id").unwrap_or(1);

        Some(IeeeSymbol {
            x,
            y,
            symbol,
            scale_factor,
            rotation,
            is_mirrored,
            line_width,
            color,
            owner_part_id,
            display_flags: parse_schlib_display_flags(json),
            raw_params: json_raw_params(json),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{json_f64, json_i32};
    use crate::mcp::server::McpServer;
    use serde_json::json;

    #[test]
    fn key_order_is_taken_from_the_region_that_read_it() {
        // A read-modify-write hands the original key order straight back so the
        // block keeps its byte layout. Absent or malformed, the writer's own
        // canonical order applies instead — an empty list, not a guess.
        assert_eq!(
            McpServer::parse_key_order(&json!({
                "param_key_order": ["VERSION", "UNITS", "DATE"]
            })),
            vec!["VERSION", "UNITS", "DATE"]
        );

        // Non-string entries are dropped rather than stringified.
        assert_eq!(
            McpServer::parse_key_order(&json!({ "param_key_order": ["A", 7, null, "B"] })),
            vec!["A", "B"]
        );

        for absent in [json!({}), json!({ "param_key_order": null })] {
            assert!(McpServer::parse_key_order(&absent).is_empty(), "{absent}");
        }
    }

    #[test]
    fn json_i32_drops_fractional_coordinate() {
        // Demonstrates the original defect: the integer reader rejects an
        // off-grid value, so a fractional coordinate was silently dropped while
        // an integer one passed through.
        assert_eq!(json_i32(&json!({ "x": 3.75 }), "x"), None);
        assert_eq!(json_i32(&json!({ "x": 3 }), "x"), Some(3));
    }

    #[test]
    fn json_f64_accepts_numbers_and_rejects_non_numeric() {
        // The fix: accept fractional and integer JSON numbers; reject non-numeric.
        assert_eq!(json_f64(&json!({ "x": 3.75 }), "x"), Some(3.75));
        assert_eq!(json_f64(&json!({ "x": -28 }), "x"), Some(-28.0));
        assert_eq!(json_f64(&json!({ "x": "nope" }), "x"), None);
        assert_eq!(json_f64(&json!({}), "x"), None);
    }

    #[test]
    fn parse_schlib_line_preserves_fractional_coords() {
        // End-to-end: a fractional line now parses (instead of being dropped)
        // and keeps its exact coordinates, including a negative fractional X.
        let line = McpServer::parse_schlib_line(&json!({
            "x1": -28.995, "y1": 7.5, "x2": 10.0, "y2": 0.0
        }))
        .expect("fractional line should parse");
        assert!((line.x1 - (-28.995)).abs() < 1e-9, "x1 kept: {}", line.x1);
        assert!((line.y1 - 7.5).abs() < 1e-9, "y1 kept: {}", line.y1);
        assert!((line.x2 - 10.0).abs() < 1e-9, "x2 kept: {}", line.x2);
    }

    // --- PR-4: flags / solder_mask_expansion / keepout_restrictions on the
    // write (JSON -> primitive) path. The `flags` JSON shape is the raw u16
    // bitmask that `read_pcblib` serialises (PcbFlags is #[serde(transparent)]),
    // so the values these tests feed in are the same ones a read would emit.

    #[test]
    fn json_flags_reads_read_dto_string_form() {
        use crate::altium::pcblib::PcbFlags;
        // Canonical round-trip shape: the bitflags serde string read_pcblib emits.
        let flags = super::json_flags(&json!({ "flags": "LOCKED | KEEPOUT" }));
        assert!(flags.contains(PcbFlags::LOCKED));
        assert!(flags.contains(PcbFlags::KEEPOUT));
        let single = super::json_flags(&json!({ "flags": "LOCKED" }));
        assert!(single.contains(PcbFlags::LOCKED));
        assert!(!single.contains(PcbFlags::KEEPOUT));
        // Convenience shape: a raw bitmask integer (LOCKED 1 | KEEPOUT 4 = 5).
        let int_flags = super::json_flags(&json!({ "flags": 5 }));
        assert!(int_flags.contains(PcbFlags::LOCKED));
        assert!(int_flags.contains(PcbFlags::KEEPOUT));
        // Absent key -> empty (default), matching the read-side skip_serializing_if.
        assert!(super::json_flags(&json!({})).is_empty());
    }

    #[test]
    fn pcbflags_write_then_read_dto_round_trip() {
        use crate::altium::pcblib::PcbFlags;
        // The string the read DTO serialises must parse back to the same flags on
        // the write path — guards the read/write shape reconciliation.
        let original = PcbFlags::LOCKED | PcbFlags::KEEPOUT;
        let dto = serde_json::to_value(original).expect("serialise flags");
        let parsed = super::json_flags(&json!({ "flags": dto }));
        assert_eq!(parsed, original);
    }

    #[test]
    fn parse_pad_reads_flags_and_solder_mask() {
        use crate::altium::pcblib::{MaskExpansionMode, PcbFlags};
        let pad = McpServer::parse_pad(&json!({
            "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
            "flags": "LOCKED",
            "solder_mask_expansion": 0.05,
            "solder_mask_expansion_mode": "manual",
        }))
        .expect("pad should parse");
        assert!(pad.flags.contains(PcbFlags::LOCKED));
        assert_eq!(pad.solder_mask_expansion, Some(0.05));
        assert_eq!(pad.solder_mask_expansion_mode, MaskExpansionMode::Manual);
    }

    #[test]
    fn parse_pad_reads_plating_and_identity_guids() {
        // is_plated @60 and the two identity GUIDs @126/@142 flow from JSON so
        // a read-modify-write preserves them.
        let pad = McpServer::parse_pad(&json!({
            "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
            "is_plated": false,
            "identity_guid": "{A5172B29-10E4-C726-929A-64E441352E67}",
            "identity_guid_b": "{00000000-0000-0000-0000-000000000000}",
        }))
        .expect("pad should parse");
        assert!(!pad.is_plated);
        assert_eq!(
            pad.identity_guid.as_deref(),
            Some("{A5172B29-10E4-C726-929A-64E441352E67}")
        );
        assert_eq!(
            pad.identity_guid_b.as_deref(),
            Some("{00000000-0000-0000-0000-000000000000}")
        );

        // Absent keys keep the from-scratch defaults: plated (Altium's default
        // for every pad) and fresh writer-generated GUIDs (None).
        let bare = McpServer::parse_pad(&json!({
            "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
        }))
        .expect("bare pad should parse");
        assert!(bare.is_plated);
        assert_eq!(bare.identity_guid, None);
        assert_eq!(bare.identity_guid_b, None);
    }

    #[test]
    fn parse_pad_reads_thermal_relief_fields() {
        use crate::altium::pcblib::PowerPlaneConnectStyle;
        // Non-default thermal-relief / power-plane keys parse into the model.
        let pad = McpServer::parse_pad(&json!({
            "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
            "power_plane_connect_style": "direct",
            "relief_conductor_width": 0.3,
            "relief_entries": 2,
            "relief_air_gap": 0.2,
            "power_plane_relief_expansion": 0.6,
            "power_plane_clearance": 0.7,
        }))
        .expect("pad should parse");
        assert_eq!(
            pad.power_plane_connect_style,
            PowerPlaneConnectStyle::Direct
        );
        assert!((pad.relief_conductor_width - 0.3).abs() < 1e-9);
        assert_eq!(pad.relief_entries, 2);
        assert!((pad.relief_air_gap - 0.2).abs() < 1e-9);
        assert!((pad.power_plane_relief_expansion - 0.6).abs() < 1e-9);
        assert!((pad.power_plane_clearance - 0.7).abs() < 1e-9);
    }

    #[test]
    fn parse_pad_thermal_relief_defaults() {
        use crate::altium::pcblib::PowerPlaneConnectStyle;
        // Absent keys keep the from-scratch defaults (= Altium's pad template).
        let pad = McpServer::parse_pad(&json!({
            "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
        }))
        .expect("pad should parse");
        assert_eq!(
            pad.power_plane_connect_style,
            PowerPlaneConnectStyle::Relief
        );
        assert!((pad.relief_conductor_width - 0.254).abs() < 1e-9);
        assert_eq!(pad.relief_entries, 4);
        assert!((pad.relief_air_gap - 0.254).abs() < 1e-9);
        assert!((pad.power_plane_relief_expansion - 0.508).abs() < 1e-9);
        assert!((pad.power_plane_clearance - 0.508).abs() < 1e-9);
    }

    #[test]
    fn parse_via_reads_power_plane_and_flags() {
        use crate::altium::pcblib::{PcbFlags, PowerPlaneConnectStyle};
        // PR-7: power-plane connection, paste-mask, net index and flags parse in.
        let via = McpServer::parse_via(&json!({
            "x": 0.0, "y": 0.0, "diameter": 0.8, "hole_size": 0.4,
            "power_plane_connect_style": "direct",
            "power_plane_relief_expansion": 0.6,
            "power_plane_clearance": 0.7,
            "paste_mask_expansion": 0.05,
            "net_index": 42,
            "flags": "TENTING_TOP | LOCKED",
        }))
        .expect("via should parse");
        assert_eq!(
            via.power_plane_connect_style,
            PowerPlaneConnectStyle::Direct
        );
        assert!((via.power_plane_relief_expansion - 0.6).abs() < 1e-9);
        assert!((via.power_plane_clearance - 0.7).abs() < 1e-9);
        assert!((via.paste_mask_expansion - 0.05).abs() < 1e-9);
        assert_eq!(via.net_index, 42);
        assert!(via.flags.contains(PcbFlags::TENTING_TOP));
        assert!(via.flags.contains(PcbFlags::LOCKED));
    }

    #[test]
    fn parse_via_defaults_match_template() {
        use crate::altium::pcblib::{PcbFlags, PowerPlaneConnectStyle};
        // Absent keys keep the from-scratch defaults (= Altium's via template).
        let via = McpServer::parse_via(&json!({
            "x": 0.0, "y": 0.0, "diameter": 0.8, "hole_size": 0.4,
        }))
        .expect("via should parse");
        assert_eq!(
            via.power_plane_connect_style,
            PowerPlaneConnectStyle::Relief
        );
        assert!((via.power_plane_relief_expansion - 0.508).abs() < 1e-9);
        assert!((via.power_plane_clearance - 0.508).abs() < 1e-9);
        assert!((via.paste_mask_expansion - 0.0).abs() < 1e-9);
        assert_eq!(via.net_index, 0xFFFF);
        assert_eq!(via.flags, PcbFlags::empty());
    }

    #[test]
    fn parse_track_reads_flags_solder_mask_keepout() {
        use crate::altium::pcblib::PcbFlags;
        let track = McpServer::parse_track(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 0.0, "width": 0.15,
            "layer": "Top Overlay",
            "flags": "KEEPOUT",
            "solder_mask_expansion": 0.1,
            "keepout_restrictions": 3,
        }))
        .expect("track should parse");
        assert!(track.flags.contains(PcbFlags::KEEPOUT));
        assert_eq!(track.solder_mask_expansion, Some(0.1));
        assert_eq!(track.keepout_restrictions, Some(3));
        // Absent keys leave the Track::new defaults untouched.
        let bare = McpServer::parse_track(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 0.0, "width": 0.15, "layer": "Top Overlay"
        }))
        .expect("bare track should parse");
        assert!(bare.flags.is_empty());
        assert_eq!(bare.solder_mask_expansion, None);
        assert_eq!(bare.keepout_restrictions, None);
    }

    #[test]
    fn parse_arc_reads_flags_solder_mask_keepout() {
        use crate::altium::pcblib::PcbFlags;
        let arc = McpServer::parse_arc(&json!({
            "x": 0.0, "y": 0.0, "radius": 1.0,
            "start_angle": 0.0, "end_angle": 90.0, "width": 0.15,
            "layer": "Top Overlay",
            "flags": "LOCKED",
            "solder_mask_expansion": 0.2,
            "keepout_restrictions": 5,
        }))
        .expect("arc should parse");
        assert!(arc.flags.contains(PcbFlags::LOCKED));
        assert_eq!(arc.solder_mask_expansion, Some(0.2));
        assert_eq!(arc.keepout_restrictions, Some(5));
    }

    #[test]
    fn parse_region_reads_flags() {
        use crate::altium::pcblib::PcbFlags;
        let region = McpServer::parse_region(&json!({
            "layer": "Top Courtyard",
            "vertices": [{"x": 0.0, "y": 0.0}, {"x": 1.0, "y": 0.0}, {"x": 0.0, "y": 1.0}],
            "flags": "KEEPOUT",
        }))
        .expect("region should parse");
        assert!(region.flags.contains(PcbFlags::KEEPOUT));
    }

    #[test]
    fn parse_region_reads_additional_parameters() {
        // PR-R5: the read DTO's `additional_parameters` (an array of [key, value]
        // pairs) must land on the struct so a read-modify-write preserves them.
        let region = McpServer::parse_region(&json!({
            "layer": "Top Courtyard",
            "vertices": [{"x": 0.0, "y": 0.0}, {"x": 1.0, "y": 0.0}, {"x": 0.0, "y": 1.0}],
            "additional_parameters": [["LAYER", "TOP"], ["LAYERSTACKID", "7"]],
        }))
        .expect("region should parse");
        assert_eq!(
            region.additional_parameters,
            vec![
                ("LAYER".to_string(), "TOP".to_string()),
                ("LAYERSTACKID".to_string(), "7".to_string()),
            ],
        );
    }

    #[test]
    fn parse_region_additional_parameters_default_empty() {
        // Absent -> empty, so a from-scratch region re-emits nothing (byte-identical).
        let region = McpServer::parse_region(&json!({
            "layer": "Top Courtyard",
            "vertices": [{"x": 0.0, "y": 0.0}, {"x": 1.0, "y": 0.0}, {"x": 0.0, "y": 1.0}],
        }))
        .expect("region should parse");
        assert!(region.additional_parameters.is_empty());
    }

    #[test]
    fn parse_fill_reads_flags() {
        use crate::altium::pcblib::PcbFlags;
        let fill = McpServer::parse_fill(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0, "layer": "Top Layer",
            "flags": "LOCKED",
            "solder_mask_expansion": 0.05,
            "keepout_restrictions": 2,
        }))
        .expect("fill should parse");
        assert!(fill.flags.contains(PcbFlags::LOCKED));
        assert_eq!(fill.solder_mask_expansion, Some(0.05));
        assert_eq!(fill.keepout_restrictions, Some(2));
    }

    #[test]
    fn parse_text_reads_flags() {
        use crate::altium::pcblib::PcbFlags;
        let text = McpServer::parse_text(&json!({
            "x": 0.0, "y": 0.0, "text": "REF", "height": 0.5, "layer": "Top Overlay",
            "flags": "LOCKED",
        }))
        .expect("text should parse");
        assert!(text.flags.contains(PcbFlags::LOCKED));
    }

    #[test]
    fn parse_text_reads_authoring_fields() {
        // kind/stroke_font/italic/bold/mirror/font_name/justification must each
        // flow from JSON onto the struct.
        use crate::altium::pcblib::{StrokeFont, TextJustification, TextKind};
        let text = McpServer::parse_text(&json!({
            "x": 0.0, "y": 0.0, "text": "REF", "height": 0.5, "layer": "Top Overlay",
            "kind": "true_type",
            "stroke_font": "serif",
            "italic": true,
            "bold": true,
            "mirror": true,
            "is_comment": true,
            "is_designator": true,
            "font_name": "Times New Roman",
            "justification": "top_right",
        }))
        .expect("text should parse");
        assert_eq!(text.kind, TextKind::TrueType);
        assert_eq!(text.stroke_font, Some(StrokeFont::Serif));
        assert!(text.italic);
        assert!(text.bold);
        assert!(text.mirror);
        assert!(text.is_comment);
        assert!(text.is_designator);
        assert_eq!(text.font_name, "Times New Roman");
        assert_eq!(text.justification, TextJustification::TopRight);
    }

    #[test]
    fn parse_text_defaults_are_template_identical() {
        // A minimal text must keep the from-scratch defaults (stroke, no font
        // override, Arial, middle-center) so it stays byte-identical on write.
        use crate::altium::pcblib::{TextJustification, TextKind};
        let text = McpServer::parse_text(&json!({
            "x": 0.0, "y": 0.0, "text": "REF", "height": 0.5, "layer": "Top Overlay",
        }))
        .expect("text should parse");
        assert_eq!(text.kind, TextKind::Stroke);
        assert_eq!(text.stroke_font, None);
        assert!(!text.italic);
        assert!(!text.bold);
        assert!(!text.mirror);
        assert!(!text.is_comment, "absent is_comment stays template false");
        assert!(
            !text.is_designator,
            "absent is_designator stays template false"
        );
        assert_eq!(text.font_name, "Arial");
        assert_eq!(text.justification, TextJustification::BottomLeft);
    }

    // --- SchLib write-path authoring fields. Each must reach the struct from
    // JSON on write, not just round-trip on read. Each test sets a non-default

    // asserts it lands on the struct (the field names match the read DTO).

    #[test]
    fn parse_schlib_pin_reads_authoring_fields() {
        let pin = McpServer::parse_schlib_pin(&json!({
            "designator": "1", "name": "P1", "x": 0, "y": 0, "length": 10,
            "orientation": "left",
            "description": "clock input",
            "colour": 0x00_FF_00,
            "graphically_locked": true,
            "swap_id_group": "grpA",
            "part_and_sequence": "|1&2|",
            "default_value": "0",
            "owner_part_display_mode": 2,
            "symbol_line_width": 3,
            "frac": { "x": 50000, "y": -25000, "length": 0 },
        }))
        .expect("pin should parse");
        assert_eq!(pin.description, "clock input");
        assert_eq!(pin.colour, 0x00_FF_00);
        assert!(pin.graphically_locked);
        assert_eq!(pin.swap_id_group, "grpA");
        assert_eq!(pin.part_and_sequence, "|1&2|");
        assert_eq!(pin.default_value, "0");
        assert_eq!(pin.owner_part_display_mode, 2);
        assert_eq!(pin.symbol_line_width, 3);
        assert_eq!(
            pin.frac,
            Some(crate::altium::schlib::PinFrac {
                x: 50000,
                y: -25000,
                length: 0
            })
        );
    }

    #[test]
    fn parse_schlib_pin_defaults_match_struct() {
        // Absent authoring keys keep the from-scratch defaults (notably the
        // `|&|` part_and_sequence Altium uses for a fresh pin).
        let pin = McpServer::parse_schlib_pin(&json!({
            "designator": "1", "name": "P1", "x": 0, "y": 0, "length": 10,
            "orientation": "left",
        }))
        .expect("pin should parse");
        assert_eq!(pin.description, "");
        assert_eq!(pin.colour, 0);
        assert!(!pin.graphically_locked);
        assert_eq!(pin.swap_id_group, "");
        assert_eq!(pin.part_and_sequence, "|&|");
        assert_eq!(pin.default_value, "");
        // PR-R3 aux fields default so no aux stream is written for a plain pin.
        assert_eq!(pin.owner_part_display_mode, 0);
        assert_eq!(pin.symbol_line_width, 0);
        assert_eq!(pin.frac, None);
    }

    #[test]
    fn parse_schlib_pin_reads_open_collector_electrical_type() {
        use crate::altium::schlib::PinElectricalType;
        let oc = McpServer::parse_schlib_pin(&json!({
            "designator": "1", "name": "P1", "x": 0, "y": 0, "length": 10,
            "orientation": "left", "electrical_type": "open_collector",
        }))
        .expect("pin should parse");
        assert_eq!(oc.electrical_type, PinElectricalType::OpenCollector);
        // `tristate` is the advertised alias for HiZ.
        let tri = McpServer::parse_schlib_pin(&json!({
            "designator": "2", "name": "P2", "x": 0, "y": 0, "length": 10,
            "orientation": "left", "electrical_type": "tristate",
        }))
        .expect("pin should parse");
        assert_eq!(tri.electrical_type, PinElectricalType::HiZ);
    }

    #[test]
    fn parse_schlib_rectangle_reads_line_style_and_transparent() {
        let rect = McpServer::parse_schlib_rectangle(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 10.0,
            "line_style": 2, "transparent": true,
        }))
        .expect("rectangle should parse");
        assert_eq!(rect.line_style, 2);
        assert!(rect.transparent);
    }

    #[test]
    fn parse_schlib_round_rect_reads_line_style_and_transparent() {
        let rr = McpServer::parse_schlib_round_rect(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 10.0,
            "corner_x_radius": 2.0, "corner_y_radius": 2.0,
            "line_style": 1, "transparent": true,
        }))
        .expect("round_rect should parse");
        assert_eq!(rr.line_style, 1);
        assert!(rr.transparent);
    }

    #[test]
    fn parse_schlib_line_reads_line_style() {
        let line = McpServer::parse_schlib_line(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 0.0, "line_style": 2,
        }))
        .expect("line should parse");
        assert_eq!(line.line_style, 2);
    }

    #[test]
    fn parse_schlib_polyline_reads_style_and_arrowheads() {
        let pl = McpServer::parse_schlib_polyline(&json!({
            "points": [{"x": 0.0, "y": 0.0}, {"x": 10.0, "y": 0.0}],
            "line_style": 1,
            "start_line_shape": 2,
            "end_line_shape": 3,
            "line_shape_size": 4,
            "transparent": true,
        }))
        .expect("polyline should parse");
        assert_eq!(pl.line_style, 1);
        assert_eq!(pl.start_line_shape, 2);
        assert_eq!(pl.end_line_shape, 3);
        assert_eq!(pl.line_shape_size, 4);
        assert!(pl.transparent);
    }

    #[test]
    fn parse_schlib_arc_reads_fill_color() {
        let arc = McpServer::parse_schlib_arc(&json!({
            "x": 0.0, "y": 0.0, "radius": 5.0, "fill_color": 0x11_22_33,
        }))
        .expect("arc should parse");
        assert_eq!(arc.fill_color, 0x11_22_33);
    }

    #[test]
    fn parse_schlib_ellipse_reads_transparent() {
        let el = McpServer::parse_schlib_ellipse(&json!({
            "x": 0.0, "y": 0.0, "radius_x": 5.0, "radius_y": 3.0, "transparent": true,
        }))
        .expect("ellipse should parse");
        assert!(el.transparent);
    }

    #[test]
    fn parse_schlib_ellipse_and_polyline_read_is_not_accessible() {
        // Absent defaults true (Altium tags every ellipse/polyline); an explicit
        // false must round-trip through the JSON write path.
        let el = McpServer::parse_schlib_ellipse(&json!({
            "x": 0.0, "y": 0.0, "radius_x": 5.0, "radius_y": 3.0,
        }))
        .expect("ellipse should parse");
        assert!(el.is_not_accessible, "ellipse defaults true");
        let el = McpServer::parse_schlib_ellipse(&json!({
            "x": 0.0, "y": 0.0, "radius_x": 5.0, "radius_y": 3.0,
            "is_not_accessible": false,
        }))
        .expect("ellipse should parse");
        assert!(!el.is_not_accessible, "explicit false is honoured");

        let points = json!([{ "x": 0.0, "y": 0.0 }, { "x": 5.0, "y": 5.0 }]);
        let pl = McpServer::parse_schlib_polyline(&json!({ "points": points }))
            .expect("polyline should parse");
        assert!(pl.is_not_accessible, "polyline defaults true");
        let pl = McpServer::parse_schlib_polyline(
            &json!({ "points": points, "is_not_accessible": false }),
        )
        .expect("polyline should parse");
        assert!(!pl.is_not_accessible, "explicit false is honoured");
    }

    #[test]
    fn parse_schlib_parameter_reads_justification() {
        // Altium anchor id 0-8 (golden JUSTIFY: 8 = top-right); absent = 0.
        let p = McpServer::parse_schlib_parameter(&json!({
            "name": "Value", "value": "10k", "justification": 8,
        }))
        .expect("parameter should parse");
        assert_eq!(p.justification, 8);
        let p = McpServer::parse_schlib_parameter(&json!({ "name": "Value" }))
            .expect("parameter should parse");
        assert_eq!(p.justification, 0, "absent justification defaults to 0");
    }

    // --- PR-R1: round-trip preservation of a primitive's `unique_id` (identity
    // GUID). An absent `unique_id` MUST stay `None` (the writer then

    // `unique_id` MUST stay `None` (the writer then auto-generates, keeping
    // from-scratch output byte-identical).

    #[test]
    fn json_unique_id_reads_and_defaults() {
        assert_eq!(
            super::json_unique_id(&json!({ "unique_id": "QHHMRSCB" })).as_deref(),
            Some("QHHMRSCB")
        );
        // Absent -> None, so the writer auto-generates exactly as before.
        assert_eq!(super::json_unique_id(&json!({})), None);
    }

    #[test]
    fn parse_via_preserves_provided_unique_id() {
        let via = McpServer::parse_via(&json!({
            "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3,
            "unique_id": "VIAUID01",
        }))
        .expect("via should parse");
        assert_eq!(via.unique_id.as_deref(), Some("VIAUID01"));
    }

    #[test]
    fn parse_via_without_unique_id_defaults_none() {
        // From-scratch: no unique_id -> None (writer auto-generates; byte-identical).
        let via = McpServer::parse_via(&json!({
            "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3,
        }))
        .expect("via should parse");
        assert_eq!(via.unique_id, None);
    }

    #[test]
    fn parse_component_body_reads_raw_layer_pair() {
        // #391: the unmapped-byte pair read_pcblib echoes must survive the
        // tool-layer JSON boundary like every other replay field.
        let body = McpServer::parse_component_body_json(&json!({
            "model_id": "{G-1}",
            "layer": "Multi-Layer",
            "raw_layer_id": 150,
            "v7_layer": "MECHANICAL22",
        }));
        assert_eq!(body.raw_layer_id, Some(150));
        assert_eq!(body.v7_layer.as_deref(), Some("MECHANICAL22"));

        // Absent -> None, out-of-range -> None (lenient Option-style).
        let plain = McpServer::parse_component_body_json(&json!({
            "model_id": "{G-1}",
            "raw_layer_id": 300,
        }));
        assert_eq!(plain.raw_layer_id, None);
        assert_eq!(plain.v7_layer, None);
    }

    #[test]
    fn json_base64_decodes_and_ignores_invalid() {
        // "AAEC" is [0, 1, 2]; invalid base64 and absent keys both read None,
        // so the writer falls back to its template instead of erroring.
        assert_eq!(
            super::json_base64(&json!({ "raw": "AAEC" }), "raw"),
            Some(vec![0u8, 1, 2])
        );
        assert_eq!(super::json_base64(&json!({ "raw": "!!!" }), "raw"), None);
        assert_eq!(super::json_base64(&json!({}), "raw"), None);
    }

    #[test]
    fn parse_pad_preserves_raw_tail() {
        let pad = McpServer::parse_pad(&json!({
            "designator": "1", "x": 0.0, "y": 0.0, "width": 0.6, "height": 0.5,
            "raw_tail": "AAECAwQ=",
        }))
        .expect("pad should parse");
        assert_eq!(pad.raw_tail.as_deref(), Some(&[0u8, 1, 2, 3, 4][..]));
    }

    #[test]
    fn parse_via_reads_diameter_stack_and_back_mask() {
        use crate::altium::pcblib::ViaStackMode;
        let via = McpServer::parse_via(&json!({
            "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3,
            "diameter_stack_mode": "full_stack",
            "per_layer_diameters": [0.6, 0.7, 0.8],
            "solder_mask_expansion_back": 0.05,
        }))
        .expect("via should parse");
        assert_eq!(via.diameter_stack_mode, ViaStackMode::FullStack);
        assert_eq!(
            via.per_layer_diameters.as_deref(),
            Some(&[0.6, 0.7, 0.8][..])
        );
        assert_eq!(via.solder_mask_expansion_back, Some(0.05));

        // Absent -> struct defaults, so a from-scratch via is unchanged.
        let plain = McpServer::parse_via(&json!({
            "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3,
        }))
        .expect("via should parse");
        assert_eq!(plain.diameter_stack_mode, ViaStackMode::Simple);
        assert_eq!(plain.per_layer_diameters, None);
        assert_eq!(plain.solder_mask_expansion_back, None);
    }

    #[test]
    fn parse_via_preserves_guid_and_raw_block() {
        let via = McpServer::parse_via(&json!({
            "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3,
            "guid": "{01234567-89AB-CDEF-0123-456789ABCDEF}",
            "raw_block": "BQYH",
        }))
        .expect("via should parse");
        assert_eq!(
            via.guid.as_deref(),
            Some("{01234567-89AB-CDEF-0123-456789ABCDEF}")
        );
        assert_eq!(via.raw_block.as_deref(), Some(&[5u8, 6, 7][..]));
    }

    #[test]
    fn parse_text_preserves_raw_geometry() {
        let text = McpServer::parse_text(&json!({
            "text": "REF", "x": 0.0, "y": 0.0, "height": 1.0, "layer": "Top Overlay",
            "raw_geometry": "CAkK",
        }))
        .expect("text should parse");
        assert_eq!(text.raw_geometry.as_deref(), Some(&[8u8, 9, 10][..]));
    }

    #[test]
    fn parse_track_and_fill_preserve_guid() {
        let guid = "{FEDCBA98-7654-3210-FEDC-BA9876543210}";
        let track = McpServer::parse_track(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 0.0, "width": 0.2,
            "layer": "Top Overlay", "guid": guid,
        }))
        .expect("track should parse");
        assert_eq!(track.guid.as_deref(), Some(guid));

        let fill = McpServer::parse_fill(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0,
            "layer": "Top Layer", "guid": guid,
        }))
        .expect("fill should parse");
        assert_eq!(fill.guid.as_deref(), Some(guid));
    }

    #[test]
    fn every_layered_primitive_carries_its_read_layer_byte() {
        // An unmapped header byte read onto the Multi-Layer catch-all
        // survives the JSON boundary on each of the five kinds that carry one.
        let with = |fields: serde_json::Value| {
            let mut json = fields;
            json["layer"] = json!("Multi-Layer");
            json["raw_layer_id"] = json!(100);
            json
        };
        let pad = McpServer::parse_pad(&with(json!({
            "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
        })))
        .expect("pad");
        let track = McpServer::parse_track(&with(json!({
            "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 0.0, "width": 0.2,
        })))
        .expect("track");
        let arc = McpServer::parse_arc(&with(json!({
            "x": 0.0, "y": 0.0, "radius": 1.0, "start_angle": 0.0, "end_angle": 90.0, "width": 0.2,
        })))
        .expect("arc");
        let text = McpServer::parse_text(&with(json!({
            "x": 0.0, "y": 0.0, "text": "T", "height": 1.0,
        })))
        .expect("text");
        let fill = McpServer::parse_fill(&with(json!({
            "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0,
        })))
        .expect("fill");
        assert_eq!(
            [
                pad.raw_layer_id,
                track.raw_layer_id,
                arc.raw_layer_id,
                text.raw_layer_id,
                fill.raw_layer_id,
            ],
            [Some(100); 5]
        );
        assert_eq!(track.layer, crate::altium::pcblib::Layer::MultiLayer);
        // An out-of-range value is not a byte.
        let plain = McpServer::parse_fill(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0, "layer": "Top Layer", "raw_layer_id": 300,
        }))
        .expect("fill");
        assert_eq!(plain.raw_layer_id, None);
    }

    #[test]
    fn parse_schlib_rectangle_preserves_provided_unique_id() {
        let rect = McpServer::parse_schlib_rectangle(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 10.0,
            "unique_id": "RECTUID1",
        }))
        .expect("rectangle should parse");
        assert_eq!(rect.unique_id.as_deref(), Some("RECTUID1"));
        // From-scratch -> None (writer auto-generates; byte-identical).
        let plain = McpServer::parse_schlib_rectangle(&json!({
            "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 10.0,
        }))
        .expect("rectangle should parse");
        assert_eq!(plain.unique_id, None);
    }

    #[test]
    fn pad_shape_vocabulary_is_shared_and_lenient() {
        use crate::altium::pcblib::PadShape;
        // Both tools resolve the same spellings. The pairs below span the two
        // documented vocabularies: `rounded_rectangle`/`circle` from
        // write_pcblib, `roundedrectangle`/`circular`/`rect` from update_pad.
        for (input, want) in [
            ("rectangle", PadShape::Rectangle),
            ("rect", PadShape::Rectangle),
            ("round", PadShape::Round),
            ("circle", PadShape::Round),
            ("circular", PadShape::Round),
            ("oval", PadShape::Oval),
            ("oblong", PadShape::Oval),
            ("octagonal", PadShape::Octagonal),
            ("octagon", PadShape::Octagonal),
            ("rounded_rectangle", PadShape::RoundedRectangle),
            ("roundedrectangle", PadShape::RoundedRectangle),
            ("rounded-rectangle", PadShape::RoundedRectangle),
            ("rounded", PadShape::RoundedRectangle),
            // case-insensitive: update_pad accepted these, write_pcblib did not
            ("Round", PadShape::Round),
            ("ROUNDED_RECTANGLE", PadShape::RoundedRectangle),
        ] {
            assert_eq!(
                McpServer::parse_pad_shape(input),
                Some(want),
                "shape {input:?} must resolve"
            );
        }
        assert_eq!(McpServer::parse_pad_shape("hexagon"), None);
    }

    #[test]
    fn parse_pad_shape_defaults_to_rounded_rectangle() {
        // Documents the default that BGA callers must override. A change here is a
        // silent geometry change for every caller that omits `shape`, so it should
        // be deliberate.
        use crate::altium::pcblib::PadShape;
        let pad = McpServer::parse_pad(&json!({
            "designator": "A1", "x": 0.0, "y": 0.0, "width": 0.3, "height": 0.3
        }))
        .expect("pad without shape should parse");
        assert_eq!(pad.shape, PadShape::RoundedRectangle);

        // A BGA land opting in explicitly stays circular.
        let bga = McpServer::parse_pad(&json!({
            "designator": "A1", "x": 0.0, "y": 0.0, "width": 0.3, "height": 0.3,
            "shape": "round"
        }))
        .expect("round pad should parse");
        assert_eq!(bga.shape, PadShape::Round);
    }

    // ==================== rejection paths and optional-field arms ============
    //
    // Every `parse_*` helper answers a malformed primitive with a message
    // naming the field, and silently defaults the optional ones. Both halves
    // are contract; the tests below pin each rejection and each default.

    mod rejections {
        use crate::mcp::server::McpServer;
        use serde_json::json;

        /// Asserts the parse failed and its message mentions `needle`.
        fn rejects<T: std::fmt::Debug>(result: Result<T, String>, needle: &str) {
            let err = result.expect_err("expected a rejection");
            assert!(
                err.contains(needle),
                "expected the message to mention {needle:?}, got: {err}"
            );
        }

        /// A pad payload that parses, so a test can drop or corrupt one field.
        fn pad_json() -> serde_json::Value {
            json!({ "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 })
        }

        // ---- unknown-field guard ---------------------------------------------

        #[test]
        fn unknown_fields_are_named_along_with_what_was_allowed() {
            // A typo in an optional key would otherwise be silently ignored and
            // the caller would never learn the value did not take effect.
            let err = McpServer::check_unknown_fields(&json!({ "widht": 1.0 }), &["width"])
                .expect_err("an unknown field must be refused");
            assert!(err.contains("widht"), "{err}");
            assert!(err.contains("width"), "{err}");

            // A non-object carries no keys to check, and is left alone.
            assert!(McpServer::check_unknown_fields(&json!(42), &["width"]).is_ok());
            assert!(McpServer::check_unknown_fields(&json!({ "width": 1.0 }), &["width"]).is_ok());
        }

        // ---- parse_pad --------------------------------------------------------

        #[test]
        fn parse_pad_names_the_missing_required_field() {
            for field in ["designator", "x", "y", "width", "height"] {
                let mut json = pad_json();
                json.as_object_mut().unwrap().remove(field);
                rejects(McpServer::parse_pad(&json), field);
            }
        }

        #[test]
        fn parse_pad_rejects_an_empty_designator() {
            // Whitespace is not a designator: the pad would land in the library
            // unnameable and unmatched by any schematic pin.
            let mut json = pad_json();
            json["designator"] = json!("   ");
            rejects(McpServer::parse_pad(&json), "cannot be empty");
        }

        #[test]
        fn parse_pad_rejects_non_positive_dimensions() {
            for field in ["width", "height"] {
                let mut zero = pad_json();
                zero[field] = json!(0.0);
                rejects(McpServer::parse_pad(&zero), "greater than 0");

                let mut negative = pad_json();
                negative[field] = json!(-1.0);
                rejects(McpServer::parse_pad(&negative), "greater than 0");
            }
        }

        #[test]
        fn parse_pad_rejects_unknown_shape_and_layer_names() {
            let mut shape = pad_json();
            shape["shape"] = json!("trapezoid");
            rejects(McpServer::parse_pad(&shape), "invalid shape");

            let mut layer = pad_json();
            layer["layer"] = json!("Layer 47");
            rejects(McpServer::parse_pad(&layer), "invalid layer");
        }

        #[test]
        fn parse_pad_defaults_the_layer_from_whether_it_is_drilled() {
            use crate::altium::pcblib::Layer;

            // No hole: an SMD land, which lives on the top layer.
            let smd = McpServer::parse_pad(&pad_json()).expect("smd pad should parse");
            assert_eq!(smd.layer, Layer::TopLayer);

            // A drilled pad spans the stack, so it defaults to multi-layer.
            let mut drilled = pad_json();
            drilled["hole_size"] = json!(0.8);
            let through = McpServer::parse_pad(&drilled).expect("through pad should parse");
            assert_eq!(through.layer, Layer::MultiLayer);

            // A zero hole is not a hole, so it stays an SMD land.
            let mut zero_hole = pad_json();
            zero_hole["hole_size"] = json!(0.0);
            let still_smd = McpServer::parse_pad(&zero_hole).expect("pad should parse");
            assert_eq!(still_smd.layer, Layer::TopLayer);
        }

        #[test]
        fn parse_pad_reads_hole_shape_and_falls_back_to_round() {
            use crate::altium::pcblib::HoleShape;

            let with_shape = |s: &str| {
                let mut json = pad_json();
                json["hole_size"] = json!(0.8);
                json["hole_shape"] = json!(s);
                McpServer::parse_pad(&json)
                    .expect("pad should parse")
                    .hole_shape
            };
            assert_eq!(with_shape("square"), HoleShape::Square);
            assert_eq!(with_shape("slot"), HoleShape::Slot);
            assert_eq!(with_shape("round"), HoleShape::Round);
            // An unrecognised name is not an error — a round hole is the safe
            // reading, and the pad still writes.
            assert_eq!(with_shape("hexagon"), HoleShape::Round);
        }

        #[test]
        fn parse_pad_reads_the_per_layer_stack_arrays() {
            use crate::altium::pcblib::PadShape;

            let mut json = pad_json();
            json["stack_mode"] = json!("full_stack");
            json["per_layer_shapes"] = json!(["round", "rectangle", "not-a-shape"]);
            json["per_layer_corner_radii"] = json!([25, 50, 300]);
            let pad = McpServer::parse_pad(&json).expect("stacked pad should parse");

            let shapes = pad.per_layer_shapes.expect("shapes should be read");
            assert_eq!(shapes[0], PadShape::Round);
            assert_eq!(shapes[1], PadShape::Rectangle);
            // An unreadable entry falls back rather than failing the whole pad.
            assert_eq!(shapes[2], PadShape::Round);

            let radii = pad.per_layer_corner_radii.expect("radii should be read");
            assert_eq!(radii[0], 25);
            assert_eq!(radii[1], 50);
            // Out of a byte's range, so it reads as 0 rather than wrapping.
            assert_eq!(radii[2], 0);
        }

        // ---- parse_track / parse_arc / parse_fill -----------------------------

        #[test]
        fn parse_track_names_the_missing_field_and_bad_layer() {
            let base = json!({ "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0, "width": 0.2 });
            for field in ["x1", "y1", "x2", "y2", "width"] {
                let mut json = base.clone();
                json.as_object_mut().unwrap().remove(field);
                rejects(McpServer::parse_track(&json), field);
            }

            let mut bad_layer = base;
            bad_layer["layer"] = json!("Nowhere");
            rejects(McpServer::parse_track(&bad_layer), "invalid layer");
        }

        #[test]
        fn parse_arc_names_the_missing_field_and_bad_layer() {
            let base = json!({
                "x": 0.0, "y": 0.0, "radius": 1.0,
                "start_angle": 0.0, "end_angle": 90.0, "width": 0.2,
            });
            for field in ["x", "y", "radius", "start_angle", "end_angle", "width"] {
                let mut json = base.clone();
                json.as_object_mut().unwrap().remove(field);
                rejects(McpServer::parse_arc(&json), field);
            }

            let mut bad_layer = base;
            bad_layer["layer"] = json!("Nowhere");
            rejects(McpServer::parse_arc(&bad_layer), "invalid layer");
        }

        #[test]
        fn parse_fill_names_the_missing_field_bad_layer_and_reads_rotation() {
            let base = json!({ "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 });
            for field in ["x1", "y1", "x2", "y2"] {
                let mut json = base.clone();
                json.as_object_mut().unwrap().remove(field);
                rejects(McpServer::parse_fill(&json), field);
            }

            let mut bad_layer = base.clone();
            bad_layer["layer"] = json!("Nowhere");
            rejects(McpServer::parse_fill(&bad_layer), "invalid layer");

            let mut rotated = base;
            rotated["rotation"] = json!(45.0);
            let fill = McpServer::parse_fill(&rotated).expect("fill should parse");
            assert!((fill.rotation - 45.0).abs() < f64::EPSILON);
        }

        // ---- parse_via --------------------------------------------------------

        #[test]
        fn parse_via_names_the_missing_field() {
            let base = json!({ "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3 });
            for field in ["x", "y", "diameter", "hole_size"] {
                let mut json = base.clone();
                json.as_object_mut().unwrap().remove(field);
                rejects(McpServer::parse_via(&json), field);
            }
        }

        #[test]
        fn parse_via_rejects_a_hole_that_cannot_fit_inside_the_ring() {
            let base = json!({ "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3 });

            let mut no_hole = base.clone();
            no_hole["hole_size"] = json!(0.0);
            rejects(McpServer::parse_via(&no_hole), "greater than 0");

            // A hole at or past the outer diameter leaves no annular ring, so
            // the via would be a bare drill with no copper to connect to.
            let mut swallowed = base;
            swallowed["hole_size"] = json!(0.6);
            rejects(McpServer::parse_via(&swallowed), "smaller than diameter");
        }

        #[test]
        fn parse_via_rejects_unknown_layer_names_on_either_end() {
            let base = json!({ "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3 });
            for field in ["from_layer", "to_layer"] {
                let mut json = base.clone();
                json[field] = json!("Nowhere");
                rejects(McpServer::parse_via(&json), field);
            }
        }

        #[test]
        fn parse_via_reads_its_optional_tail() {
            use crate::altium::pcblib::{
                DrillLayerPairType, Layer, MaskExpansionMode, PowerPlaneConnectStyle,
            };

            let via = McpServer::parse_via(&json!({
                "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3,
                "from_layer": "Top Layer", "to_layer": "Mid-Layer 1",
                "solder_mask_expansion": 0.05,
                "solder_mask_expansion_from_hole_edge": true,
                "solder_mask_expansion_mode": "manual",
                "drill_layer_pair_type": "mid",
                "thermal_relief_gap": 0.25,
                "thermal_relief_conductors": 4,
                "thermal_relief_width": 0.3,
                "power_plane_connect_style": "direct",
                "power_plane_relief_expansion": 0.4,
                "power_plane_clearance": 0.5,
                "paste_mask_expansion": 0.06,
                "net_index": 7,
                "hole_positive_tolerance": 0.02,
                "hole_negative_tolerance": 0.01,
            }))
            .expect("via should parse");

            assert_eq!(via.from_layer, Layer::TopLayer);
            assert_eq!(via.to_layer, Layer::MidLayer1);
            assert!(via.solder_mask_expansion_from_hole_edge);
            assert_eq!(via.solder_mask_expansion_mode, MaskExpansionMode::Manual);
            assert_eq!(via.drill_layer_pair_type, DrillLayerPairType::Mid);
            assert_eq!(via.thermal_relief_conductors, 4);
            assert_eq!(
                via.power_plane_connect_style,
                PowerPlaneConnectStyle::Direct
            );
            assert_eq!(via.net_index, 7);
            assert_eq!(via.hole_positive_tolerance, Some(0.02));
            assert_eq!(via.hole_negative_tolerance, Some(0.01));
        }

        #[test]
        fn parse_via_maps_unrecognised_enum_names_to_the_altium_default() {
            use crate::altium::pcblib::{
                DrillLayerPairType, MaskExpansionMode, PowerPlaneConnectStyle,
            };

            // An unrecognised name produces the state a fresh Altium via
            // carries, so a typo writes the same bytes as saying nothing.
            let via = McpServer::parse_via(&json!({
                "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3,
                "solder_mask_expansion_mode": "whatever",
                "drill_layer_pair_type": "whatever",
                "power_plane_connect_style": "whatever",
            }))
            .expect("via should parse");

            assert_eq!(via.solder_mask_expansion_mode, MaskExpansionMode::None);
            assert_eq!(via.drill_layer_pair_type, DrillLayerPairType::Through);
            assert_eq!(
                via.power_plane_connect_style,
                PowerPlaneConnectStyle::Relief
            );
        }

        // ---- parse_region -----------------------------------------------------

        #[test]
        fn parse_region_needs_three_vertices() {
            let vertex = |x: f64, y: f64| json!({ "x": x, "y": y });
            assert!(McpServer::parse_region(&json!({
                "layer": "Top Layer",
                "vertices": [vertex(0.0, 0.0), vertex(1.0, 0.0)],
            }))
            .is_none());

            // Entries missing a coordinate are dropped, which can take an
            // apparently-large outline below the minimum.
            assert!(McpServer::parse_region(&json!({
                "layer": "Top Layer",
                "vertices": [vertex(0.0, 0.0), vertex(1.0, 0.0), { "x": 1.0 }],
            }))
            .is_none());

            assert!(McpServer::parse_region(&json!({ "layer": "Top Layer" })).is_none());
        }

        #[test]
        fn parse_region_accepts_a_kind_by_name_or_by_id() {
            use crate::altium::pcblib::RegionKind;

            let with_kind = |kind: serde_json::Value| {
                let mut json = json!({
                    "layer": "Top Layer",
                    "vertices": [
                        { "x": 0.0, "y": 0.0 }, { "x": 1.0, "y": 0.0 }, { "x": 0.0, "y": 1.0 },
                    ],
                });
                json["kind"] = kind;
                McpServer::parse_region(&json)
                    .expect("region should parse")
                    .kind
            };

            assert_eq!(with_kind(json!("cutout")), RegionKind::Cutout);
            assert_eq!(with_kind(json!("copper")), RegionKind::Copper);
            assert_eq!(with_kind(json!("named_region")), RegionKind::NamedRegion);
            assert_eq!(with_kind(json!("cavity")), RegionKind::Cavity);
            // A numeric KIND, as a string and as a number, and an unreadable
            // value falling back to copper.
            assert_eq!(with_kind(json!("4")), RegionKind::Cavity);
            assert_eq!(with_kind(json!(4)), RegionKind::Cavity);
            assert_eq!(with_kind(json!("nonsense")), RegionKind::Copper);
        }

        #[test]
        fn parse_region_reads_the_name_cavity_and_hole_contours() {
            let region = McpServer::parse_region(&json!({
                "layer": "Top Layer",
                "vertices": [
                    { "x": -1.0, "y": -1.0 }, { "x": 1.0, "y": -1.0 },
                    { "x": 1.0, "y": 1.0 }, { "x": -1.0, "y": 1.0 },
                ],
                "name": "CAVITY_A",
                "cavity_height": 1.5,
                "sub_poly_index": 2,
                "union_index": 3,
                "is_shape_based": true,
                "holes": [[
                    { "x": -0.5, "y": -0.5 }, { "x": 0.5, "y": -0.5 }, { "x": 0.0, "y": 0.5 },
                ]],
            }))
            .expect("region should parse");

            assert_eq!(region.name, "CAVITY_A");
            assert!((region.cavity_height - 1.5).abs() < f64::EPSILON);
            assert_eq!(region.sub_poly_index, 2);
            assert_eq!(region.union_index, 3);
            assert!(region.is_shape_based);
            assert_eq!(region.holes.len(), 1);
            assert_eq!(region.holes[0].len(), 3);
        }

        // ---- SchLib shape helpers ---------------------------------------------

        #[test]
        fn polylines_and_polygons_need_enough_points_to_draw() {
            let point = |x: f64, y: f64| json!({ "x": x, "y": y });

            // A polyline is a run of segments, so one point draws nothing.
            assert!(
                McpServer::parse_schlib_polyline(&json!({ "points": [point(0.0, 0.0)] })).is_none()
            );
            assert!(McpServer::parse_schlib_polyline(
                &json!({ "points": [point(0.0, 0.0), point(1.0, 1.0)] })
            )
            .is_some());

            // A polygon is a closed area, so it needs a third point.
            assert!(McpServer::parse_schlib_polygon(
                &json!({ "points": [point(0.0, 0.0), point(1.0, 0.0)] })
            )
            .is_none());
            assert!(McpServer::parse_schlib_polygon(&json!({
                "points": [point(0.0, 0.0), point(1.0, 0.0), point(0.0, 1.0)],
            }))
            .is_some());
        }

        #[test]
        fn an_image_with_undecodable_data_still_parses_without_it() {
            // The bytes are optional decoration; a corrupt payload must not
            // take the whole symbol down with it.
            let image = McpServer::parse_schlib_image(&json!({
                "x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 10.0,
                "filename": "logo.bmp",
                "image_data": "!!! not base64 !!!",
            }))
            .expect("image should parse");
            assert!(image.image_data.is_none());
        }

        #[test]
        fn label_justification_accepts_the_spelling_variants() {
            // A schematic label takes its anchor as a free-text name, so both
            // spellings of centre and either separator have to land on the same
            // anchor. (PcbLib text is separate — it deserialises through serde
            // and accepts only the canonical serde spelling.)
            use crate::altium::schlib::TextJustification;

            let justified = |s: &str| {
                McpServer::parse_schlib_label(&json!({
                    "x": 0.0, "y": 0.0, "text": "REF", "justification": s,
                }))
                .expect("label should parse")
                .justification
            };

            // Both spellings of centre, with and without separators.
            assert_eq!(justified("middle-center"), TextJustification::MiddleCenter);
            assert_eq!(justified("middle_centre"), TextJustification::MiddleCenter);
            assert_eq!(justified("center"), TextJustification::MiddleCenter);
            assert_eq!(justified("bottom-centre"), TextJustification::BottomCenter);
            assert_eq!(justified("bottomright"), TextJustification::BottomRight);
            assert_eq!(justified("center-left"), TextJustification::MiddleLeft);
            assert_eq!(justified("centre-right"), TextJustification::MiddleRight);
            assert_eq!(justified("topleft"), TextJustification::TopLeft);
            assert_eq!(justified("top-centre"), TextJustification::TopCenter);
            // Unrecognised falls back to Altium's own default corner.
            assert_eq!(justified("sideways"), TextJustification::BottomLeft);
        }
    }
}
