//! Read/write/list/style tools. Split from `server.rs`.

use serde_json::{json, Value};

use crate::mcp::server::{ErrorContext, McpServer, ToolCallResult};

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
fn ieee_designator_prefix(component_type: &str) -> &'static str {
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

/// Computes a pin's connection-tip coordinate from its body-attach end `(x,y)`,
/// `length`, and `orientation`, mirroring how the pin is drawn: the tip is
/// `length` units from `(x,y)` in the `orientation` direction.
const fn pin_tip(pin: &crate::altium::schlib::Pin) -> (i32, i32) {
    use crate::altium::schlib::PinOrientation::{Down, Left, Right, Up};
    match pin.orientation {
        Right => (pin.x + pin.length, pin.y),
        Left => (pin.x - pin.length, pin.y),
        Up => (pin.x, pin.y + pin.length),
        Down => (pin.x, pin.y - pin.length),
    }
}

/// Builds a geometry summary for a written symbol so the caller can self-check
/// pin placement (catching flipped or misaligned pins without opening Altium).
/// For each pin it reports the body-attach end, the computed connection tip, and
/// the orientation; plus the symbol's bounding box. All values are in schematic
/// units (10 = 1 grid square).
#[allow(clippy::cast_possible_truncation)] // rectangle coords rounded onto the integer bbox grid
fn symbol_geometry(symbol: &crate::altium::schlib::Symbol) -> Value {
    let mut xs: Vec<i32> = Vec::new();
    let mut ys: Vec<i32> = Vec::new();
    let pins: Vec<Value> = symbol
        .pins
        .iter()
        .map(|p| {
            let (tx, ty) = pin_tip(p);
            xs.push(p.x);
            xs.push(tx);
            ys.push(p.y);
            ys.push(ty);
            json!({
                "designator": p.designator,
                "name": p.name,
                "orientation": p.orientation,
                "body_end": { "x": p.x, "y": p.y },
                "tip": { "x": tx, "y": ty },
            })
        })
        .collect();
    for r in &symbol.rectangles {
        xs.push(r.x1.round() as i32);
        xs.push(r.x2.round() as i32);
        ys.push(r.y1.round() as i32);
        ys.push(r.y2.round() as i32);
    }
    let bounding_box = if xs.is_empty() {
        Value::Null
    } else {
        json!({
            "min_x": xs.iter().min(),
            "max_x": xs.iter().max(),
            "min_y": ys.iter().min(),
            "max_y": ys.iter().max(),
        })
    };
    json!({ "name": symbol.name, "pins": pins, "bounding_box": bounding_box })
}

/// True if the segment `(x1,y1)-(x2,y2)` intersects the axis-aligned rectangle
/// `[xmin,xmax] x [ymin,ymax]` (Liang-Barsky clip; an endpoint inside counts).
#[allow(clippy::too_many_arguments)]
fn segment_intersects_rect(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    xmin: f64,
    ymin: f64,
    xmax: f64,
    ymax: f64,
) -> bool {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let p = [-dx, dx, -dy, dy];
    let q = [x1 - xmin, xmax - x1, y1 - ymin, ymax - y1];
    let mut u1 = 0.0_f64;
    let mut u2 = 1.0_f64;
    for (&pi, &qi) in p.iter().zip(q.iter()) {
        if pi.abs() <= f64::EPSILON {
            if qi < 0.0 {
                return false; // parallel to this edge and outside the slab
            }
        } else {
            let t = qi / pi;
            if pi < 0.0 {
                if t > u2 {
                    return false;
                }
                u1 = u1.max(t);
            } else {
                if t < u1 {
                    return false;
                }
                u2 = u2.min(t);
            }
        }
    }
    u1 <= u2
}

/// Warns about silkscreen (overlay) tracks that overlap a pad's copper. Silk on a
/// pad is almost always a defect — it prints on the land and trips silk-to-mask
/// DRC. Only overlay TRACKS are checked (the common offender); text and arcs are
/// not. The pad rectangle is inflated by the track half-width so a grazing track
/// is caught. This is topology-agnostic, so it is safe for any footprint.
fn silk_over_pad_warnings(fp: &crate::altium::pcblib::Footprint) -> Vec<Value> {
    use crate::altium::pcblib::Layer;
    let mut warnings = Vec::new();
    for track in &fp.tracks {
        let (top, bottom) = match track.layer {
            Layer::TopOverlay => (true, false),
            Layer::BottomOverlay => (false, true),
            _ => continue,
        };
        let half = track.width / 2.0;
        for pad in &fp.pads {
            let pad_top = matches!(pad.layer, Layer::TopLayer | Layer::MultiLayer);
            let pad_bottom = matches!(pad.layer, Layer::BottomLayer | Layer::MultiLayer);
            if !((top && pad_top) || (bottom && pad_bottom)) {
                continue;
            }
            let hw = pad.width / 2.0 + half;
            let hh = pad.height / 2.0 + half;
            if segment_intersects_rect(
                track.x1,
                track.y1,
                track.x2,
                track.y2,
                pad.x - hw,
                pad.y - hh,
                pad.x + hw,
                pad.y + hh,
            ) {
                warnings.push(json!({
                    "footprint": fp.name,
                    "type": "silk_over_pad",
                    "layer": track.layer.as_str(),
                    "pad": pad.designator,
                    "message": format!(
                        "{} track overlaps pad '{}' — move silkscreen clear of the pad",
                        track.layer.as_str(),
                        pad.designator
                    ),
                }));
            }
        }
    }
    warnings
}

/// Warns when two pads' copper overlaps on a shared layer. Overlapping copper
/// merges into one net, so a footprint can be structurally valid while every pin
/// is shorted together. Advisory only — same-designator pads are excluded because
/// stacking them is a legitimate way to build a compound land.
///
/// Reporting is capped so a systematic error on a large BGA cannot bury the
/// response; the cap message carries the true total.
fn pad_copper_overlap_warnings(fp: &crate::altium::pcblib::Footprint) -> Vec<Value> {
    use crate::altium::pcblib::MAX_REPORTED_PAD_OVERLAPS as MAX_REPORTED;

    let hits = fp.overlapping_pad_pairs();
    let mut warnings: Vec<Value> = hits
        .iter()
        .take(MAX_REPORTED)
        .map(|&(i, j, ox, oy)| {
            let (a, b) = (&fp.pads[i], &fp.pads[j]);
            json!({
                "footprint": fp.name,
                "type": "pad_copper_overlap",
                "layer": a.layer.as_str(),
                "pads": [a.designator, b.designator],
                "overlap_mm": [ox, oy],
                "message": format!(
                    "pads '{}' and '{}' overlap by {:.3} x {:.3} mm on {} — overlapping copper merges into one net",
                    a.designator, b.designator, ox, oy, a.layer.as_str()
                ),
            })
        })
        .collect();
    if hits.len() > MAX_REPORTED {
        warnings.push(json!({
            "footprint": fp.name,
            "type": "pad_copper_overlap",
            "message": format!(
                "{} overlapping pad pairs total; {} shown",
                hits.len(),
                MAX_REPORTED
            ),
        }));
    }
    warnings
}

/// Summarises a footprint's 3D body for the `write_pcblib` response so the caller
/// knows the body height that was written and whether one was auto-created (with
/// a default, `assumed` height it should confirm). All heights are in mm.
fn body_3d_summary(fp: &crate::altium::pcblib::Footprint, assumed_height: bool) -> Value {
    if fp.model_3d.is_some() {
        return json!({ "name": fp.name, "source": "step-embedded" });
    }
    if let Some(ext) = fp
        .component_bodies
        .iter()
        .find(|b| !b.model_name.is_empty())
    {
        return json!({ "name": fp.name, "source": "step-external", "model": ext.model_name });
    }
    if let Some(b) = fp.component_bodies.iter().find(|b| b.model_name.is_empty()) {
        let mut summary = json!({
            "name": fp.name,
            "source": if assumed_height { "auto-extruded" } else { "extruded" },
            "overall_height": b.overall_height,
            "standoff_height": b.standoff_height,
            "assumed_height": assumed_height,
        });
        if assumed_height {
            // Make the placeholder actionable: tell the caller to replace it rather
            // than leaving the guessed 1.0 mm height in the part.
            summary["action_required"] = json!(format!(
                "No 3D body height was given for '{}', so a {} mm placeholder was used. \
                 This is almost certainly wrong — look up the component's real height from \
                 its datasheet and call write_pcblib again with component_bodies[].overall_height \
                 set to the correct value.",
                fp.name, b.overall_height
            ));
        }
        return summary;
    }
    json!({
        "name": fp.name,
        "source": "none",
        "note": "No 3D body written. Set component_bodies[].overall_height to the real \
                 part height, or pass auto_3d_body:true for a flagged 1.0 mm placeholder.",
    })
}

macro_rules! check_keys {
    ($json:expr, $keys:expr) => {
        if let Err(e) = McpServer::check_unknown_fields($json, $keys) {
            return crate::mcp::server::ToolCallResult::error(e);
        }
    };
}

impl McpServer {
    // ==================== Tool Handlers ====================

    /// Reads a `PcbLib` file and returns its contents.
    /// Supports pagination via limit/offset and filtering by `component_name`.
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::too_many_lines)] // Complex formatting logic for compact mode
    pub(crate) fn call_read_pcblib(&self, arguments: &Value) -> ToolCallResult {
        use crate::altium::pcblib::primitives::PadStackMode;
        use crate::altium::PcbLib;

        let Some(filepath) = arguments.get("filepath").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: filepath");
        };

        // Validate path is within allowed directories
        if let Err(e) = self.validate_path(filepath) {
            return ToolCallResult::error(e);
        }

        // Parse optional pagination/filter parameters
        let component_name = arguments.get("component_name").and_then(Value::as_str);
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| v as usize);
        let offset = arguments
            .get("offset")
            .and_then(Value::as_u64)
            .map_or(0, |v| v as usize);

        // Parse compact parameter (default: true - omit redundant per-layer data)
        let compact = arguments
            .get("compact")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        match PcbLib::open(filepath) {
            Ok(library) => {
                let total_count = library.len();

                // Apply filtering and pagination
                let footprints: Vec<_> = library
                    .iter()
                    .filter(|fp| {
                        // If component_name specified, only include matching
                        component_name.map_or(true, |name| fp.name == name)
                    })
                    .skip(offset)
                    .take(limit.unwrap_or(usize::MAX))
                    .map(|fp| {
                        // If compact mode, strip per-layer data when it's redundant
                        let pads: Vec<Value> = if compact {
                            fp.pads
                                .iter()
                                .map(|pad| {
                                    let mut pad_json = serde_json::to_value(pad).unwrap();
                                    // Remove per-layer data if stack_mode is Simple OR all values are uniform
                                    let should_strip = pad.stack_mode == PadStackMode::Simple
                                        || Self::pad_has_uniform_per_layer_data(pad);
                                    if should_strip {
                                        if let Value::Object(ref mut obj) = pad_json {
                                            obj.remove("per_layer_sizes");
                                            obj.remove("per_layer_shapes");
                                            obj.remove("per_layer_corner_radii");
                                            obj.remove("per_layer_offsets");
                                            // Downgrade stack_mode to simple if we stripped uniform data
                                            if pad.stack_mode != PadStackMode::Simple {
                                                obj.insert(
                                                    "stack_mode".to_string(),
                                                    json!("simple"),
                                                );
                                            }
                                        }
                                    }
                                    pad_json
                                })
                                .collect()
                        } else {
                            fp.pads
                                .iter()
                                .map(|p| serde_json::to_value(p).unwrap())
                                .collect()
                        };

                        json!({
                            "name": fp.name,
                            "description": fp.description,
                            "pads": pads,
                            "vias": fp.vias,
                            "tracks": fp.tracks,
                            "arcs": fp.arcs,
                            "regions": fp.regions,
                            "fills": fp.fills,
                            "text": fp.text,
                            "model_3d": fp.model_3d,
                            "component_bodies": fp.component_bodies,
                        })
                    })
                    .collect();

                let returned_count = footprints.len();
                let has_more = if component_name.is_some() {
                    false // Single component fetch, no pagination
                } else {
                    offset + returned_count < total_count
                };

                let result = json!({
                    "status": "success",
                    "filepath": filepath,
                    "units": "mm",
                    "total_count": total_count,
                    "returned_count": returned_count,
                    "offset": offset,
                    "has_more": has_more,
                    "compact": compact,
                    "footprints": footprints,
                });

                ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
            }
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap())
            }
        }
    }

    /// Writes footprints to a `PcbLib` file.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn call_write_pcblib(&self, arguments: &Value) -> ToolCallResult {
        use crate::altium::pcblib::{Footprint, Model3D, PcbLib};

        let Some(filepath) = arguments.get("filepath").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: filepath");
        };

        // Validate path is within allowed directories
        if let Err(e) = self.validate_path(filepath) {
            return ToolCallResult::error(e);
        }

        let Some(footprints_json) = arguments.get("footprints").and_then(Value::as_array) else {
            return ToolCallResult::error("Missing required parameter: footprints");
        };

        // Collect and validate footprint names for duplicates
        let new_names: Vec<&str> = footprints_json
            .iter()
            .filter_map(|fp| fp.get("name").and_then(Value::as_str))
            .collect();

        // Check for duplicates within the new footprints
        {
            let mut seen = std::collections::HashSet::new();
            for name in &new_names {
                if !seen.insert(*name) {
                    return ToolCallResult::error_with_context(
                        ErrorContext::new(
                            "write_pcblib",
                            format!("Duplicate footprint name: '{name}'"),
                        )
                        .with_filepath(filepath)
                        .with_component(*name)
                        .with_details("Each footprint in the request must have a unique name"),
                    );
                }
            }
        }

        // Validate footprint names
        // Note: OLE storage names are limited to 31 characters, but the library layer
        // handles this by truncating storage names while preserving full names in PATTERN.
        #[allow(clippy::items_after_statements)]
        const INVALID_CHARS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
        for name in &new_names {
            if name.is_empty() {
                return ToolCallResult::error("Footprint name cannot be empty");
            }
            if let Some(c) = name.chars().find(|c| INVALID_CHARS.contains(c)) {
                return ToolCallResult::error(format!(
                    "Footprint name '{name}' contains invalid character '{c}'. \
                     Names cannot contain: / \\ : * ? \" < > |",
                ));
            }
        }

        let append = arguments
            .get("append")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Opt-in: synthesise a placeholder extruded 3D body for footprints that have
        // pads but no body/STEP. Off by default so the tool never adds geometry the
        // caller didn't request (a body is wrong for fiducials / test points / mounting
        // holes); the always-on `bodies` echo still reports `source: "none"` to nudge.
        let auto_3d_body = arguments
            .get("auto_3d_body")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // If append mode and file exists, read existing library; otherwise create new
        let mut library = if append && std::path::Path::new(filepath).exists() {
            match PcbLib::open(filepath) {
                Ok(lib) => lib,
                Err(e) => {
                    return ToolCallResult::error_with_context(
                        ErrorContext::new(
                            "write_pcblib",
                            format!("Failed to read existing library: {e}"),
                        )
                        .with_filepath(filepath)
                        .with_details(
                            "The library file exists but could not be opened for appending",
                        ),
                    );
                }
            }
        } else {
            PcbLib::new()
        };

        // Check for duplicates with existing footprints in append mode
        if append {
            let existing_names: std::collections::HashSet<_> =
                library.names().into_iter().collect();
            for name in &new_names {
                if existing_names.contains(*name) {
                    return ToolCallResult::error(format!(
                        "Footprint '{name}' already exists in the library"
                    ));
                }
            }
        }

        // Silkscreen-over-pad warnings, echoed back so the caller can fix silk that
        // prints on a pad (a DRC defect) without opening Altium.
        let mut silk_warnings: Vec<Value> = Vec::new();

        // Per-footprint 3D-body summary echoed back so the caller sees the body
        // height that was written and whether one was auto-created.
        let mut bodies_echo: Vec<Value> = Vec::new();

        for fp_json in footprints_json {
            check_keys!(
                fp_json,
                &[
                    "name",
                    "description",
                    "pads",
                    "tracks",
                    "arcs",
                    "regions",
                    "text",
                    "vias",
                    "fills",
                    "step_model",
                    "model_3d",
                    "component_bodies",
                    "guid",
                    "primitive_order"
                ]
            );
            let name = fp_json
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Unnamed");
            let mut footprint = Footprint::new(name);

            if let Some(desc) = fp_json.get("description").and_then(Value::as_str) {
                footprint.description = desc.to_string();
            }

            // Parse pads
            if let Some(pads) = fp_json.get("pads").and_then(Value::as_array) {
                for (i, pad_json) in pads.iter().enumerate() {
                    match Self::parse_pad(pad_json) {
                        Ok(pad) => footprint.add_pad(pad),
                        Err(e) => {
                            return ToolCallResult::error_with_context(
                                ErrorContext::new("write_pcblib", e)
                                    .with_filepath(filepath)
                                    .with_component(name)
                                    .with_details(format!("Failed to parse pad at index {i}")),
                            )
                        }
                    }
                }
            }

            // Parse tracks
            if let Some(tracks) = fp_json.get("tracks").and_then(Value::as_array) {
                for (i, track_json) in tracks.iter().enumerate() {
                    match Self::parse_track(track_json) {
                        Ok(track) => footprint.add_track(track),
                        Err(e) => {
                            return ToolCallResult::error_with_context(
                                ErrorContext::new("write_pcblib", e)
                                    .with_filepath(filepath)
                                    .with_component(name)
                                    .with_details(format!("Failed to parse track at index {i}")),
                            )
                        }
                    }
                }
            }

            // Parse vias
            if let Some(vias) = fp_json.get("vias").and_then(Value::as_array) {
                for (i, via_json) in vias.iter().enumerate() {
                    match Self::parse_via(via_json) {
                        Ok(via) => footprint.add_via(via),
                        Err(e) => {
                            return ToolCallResult::error_with_context(
                                ErrorContext::new("write_pcblib", e)
                                    .with_filepath(filepath)
                                    .with_component(name)
                                    .with_details(format!("Failed to parse via at index {i}")),
                            )
                        }
                    }
                }
            }

            // Parse fills
            if let Some(fills) = fp_json.get("fills").and_then(Value::as_array) {
                for (i, fill_json) in fills.iter().enumerate() {
                    match Self::parse_fill(fill_json) {
                        Ok(fill) => footprint.add_fill(fill),
                        Err(e) => {
                            return ToolCallResult::error_with_context(
                                ErrorContext::new("write_pcblib", e)
                                    .with_filepath(filepath)
                                    .with_component(name)
                                    .with_details(format!("Failed to parse fill at index {i}")),
                            )
                        }
                    }
                }
            }

            // Parse arcs
            if let Some(arcs) = fp_json.get("arcs").and_then(Value::as_array) {
                for (i, arc_json) in arcs.iter().enumerate() {
                    match Self::parse_arc(arc_json) {
                        Ok(arc) => footprint.add_arc(arc),
                        Err(e) => {
                            return ToolCallResult::error_with_context(
                                ErrorContext::new("write_pcblib", e)
                                    .with_filepath(filepath)
                                    .with_component(name)
                                    .with_details(format!("Failed to parse arc at index {i}")),
                            )
                        }
                    }
                }
            }

            // Parse regions
            if let Some(regions) = fp_json.get("regions").and_then(Value::as_array) {
                for region_json in regions {
                    check_keys!(
                        region_json,
                        &[
                            "layer",
                            "vertices",
                            "flags",
                            "kind",
                            "name",
                            "net_index",
                            "polygon_index",
                            "component_index",
                            "arc_resolution",
                            "cavity_height",
                            "sub_poly_index",
                            "union_index",
                            "is_shape_based",
                            "holes",
                            "unique_id",
                            "additional_parameters",
                            "guid",
                            "v7_layer",
                            "param_key_order"
                        ]
                    );
                    if let Some(region) = Self::parse_region(region_json) {
                        footprint.add_region(region);
                    }
                }
            }

            // Parse text
            if let Some(texts) = fp_json.get("text").and_then(Value::as_array) {
                for text_json in texts {
                    check_keys!(
                        text_json,
                        &[
                            "bold",
                            "component_index",
                            "flags",
                            "font_name",
                            "height",
                            "inverted_border",
                            "inverted_rect_height",
                            "inverted_rect_text_offset",
                            "inverted_rect_width",
                            "is_comment",
                            "is_designator",
                            "is_inverted",
                            "italic",
                            "justification",
                            "kind",
                            "layer",
                            "mirror",
                            "net_index",
                            "polygon_index",
                            "rotation",
                            "stroke_font",
                            "stroke_width",
                            "text",
                            "unique_id",
                            "use_inverted_rectangle",
                            "x",
                            "y",
                            "guid",
                            "raw_geometry"
                        ]
                    );
                    if let Some(text) = Self::parse_text(text_json) {
                        footprint.add_text(text);
                    }
                }
            }

            // Parse 3D model
            if let Some(model_json) = fp_json.get("step_model") {
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
                            return ToolCallResult::error(e);
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
                        // External reference only - create ComponentBody directly
                        // Preserve the full path for external references so organized subfolders work
                        use crate::altium::pcblib::{ComponentBody, Layer};
                        footprint.add_component_body(ComponentBody {
                            model_id: String::new(), // No GUID for external reference
                            identifier: String::new(),
                            texture_center_x: None,
                            texture_center_y: None,
                            texture_size_x: None,
                            texture_size_y: None,
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
                            return ToolCallResult::error(e);
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
                    footprint.add_component_body(Self::parse_component_body_json(body_json));
                }
            }

            // Auto-inject the `.Designator` special string on the Top Overlay if the
            // caller did not provide one, so every placed footprint renders its
            // reference designator. Placed just above the topmost pad (or at the
            // origin when there are no pads); the user can reposition in Altium.
            let has_designator = footprint
                .text
                .iter()
                .any(|t| t.text.trim().eq_ignore_ascii_case(".designator"));
            if !has_designator {
                use crate::altium::pcblib::{Layer, PcbFlags, Text, TextJustification, TextKind};
                let top = footprint
                    .pads
                    .iter()
                    .map(|p| p.y + p.height / 2.0)
                    .fold(f64::NEG_INFINITY, f64::max);
                let y = if top.is_finite() { top + 0.6 } else { 0.0 };
                footprint.add_text(Text {
                    barcode_full_width: None,
                    barcode_full_height: None,
                    barcode_x_margin: None,
                    barcode_y_margin: None,
                    barcode_kind: 0,
                    barcode_font_name: String::new(),
                    barcode_inverted: false,
                    barcode_show_text: false,
                    x: 0.0,
                    y,
                    text: ".Designator".to_string(),
                    height: 1.0,
                    layer: Layer::TopOverlay,
                    rotation: 0.0,
                    kind: TextKind::Stroke,
                    stroke_font: None,
                    stroke_width: None,
                    italic: false,
                    bold: false,
                    mirror: false,
                    // The `.Designator` special string works through its content;
                    // is_designator@41 stays at the template's 0x00 (byte-identity —
                    // no golden carries a `.Designator` text to settle Altium's own
                    // authoring value for this byte).
                    is_comment: false,
                    is_designator: false,
                    font_name: "Arial".to_string(),
                    // BottomLeft = the template's 0x03 anchor: the writer now honours
                    // @132, so keep the auto-designator on the template default to stay
                    // byte-identical (and oracle-safe).
                    justification: TextJustification::BottomLeft,
                    is_inverted: false,
                    inverted_border: None,
                    use_inverted_rectangle: false,
                    inverted_rect_width: None,
                    inverted_rect_height: None,
                    inverted_rect_text_offset: None,
                    flags: PcbFlags::empty(),
                    net_index: 0xFFFF,
                    polygon_index: 0xFFFF,
                    component_index: -1,
                    unique_id: None,
                    guid: None,
                    raw_geometry: None,
                });
            }

            // Opt-in (`auto_3d_body`): synthesise an extruded 3D body for a footprint
            // with pads but no STEP model and no component body, so it has a 3D presence
            // in Altium. Height can't be inferred from a 2D footprint, so it defaults to
            // 1.0 mm and is flagged `assumed_height` for the caller to confirm/override.
            // The empty outline makes the writer synthesise a bounding box from pads.
            let assumed_height = if auto_3d_body
                && footprint.model_3d.is_none()
                && footprint.component_bodies.is_empty()
                && !footprint.pads.is_empty()
            {
                use crate::altium::pcblib::{ComponentBody, Layer};
                footprint.add_component_body(ComponentBody {
                    model_id: String::new(),
                    identifier: String::new(),
                    texture_center_x: None,
                    texture_center_y: None,
                    texture_size_x: None,
                    texture_size_y: None,
                    model_name: String::new(),
                    embedded: false,
                    rotation_x: 0.0,
                    rotation_y: 0.0,
                    rotation_z: 0.0,
                    z_offset: 0.0,
                    overall_height: 1.0,
                    standoff_height: 0.0,
                    cavity_height: 0.0,
                    layer: Layer::Top3DBody,
                    outline: Vec::new(),
                    unique_id: None,
                    guid: None,
                    model_checksum: 0,
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
                    // Synthesised body: no board association (free primitive).
                    net_index: 0xFFFF,
                    polygon_index: 0xFFFF,
                    component_index: -1,
                    additional_parameters: Vec::new(),
                });
                true
            } else {
                false
            };
            bodies_echo.push(body_3d_summary(&footprint, assumed_height));

            // Validate coordinates before adding
            if let Err(e) = Self::validate_footprint_coordinates(&footprint) {
                return ToolCallResult::error(e);
            }

            silk_warnings.extend(silk_over_pad_warnings(&footprint));
            silk_warnings.extend(pad_copper_overlap_warnings(&footprint));

            library.add(footprint);
        }

        // Create backup before destructive operation (if file exists)
        if let Err(e) = Self::create_backup(filepath) {
            return ToolCallResult::error(e);
        }

        match library.save(filepath) {
            Ok(()) => {
                let mut result = json!({
                    "status": "success",
                    "filepath": filepath,
                    "footprint_count": library.len(),
                    "footprint_names": library.names(),
                });

                // Silkscreen-over-pad warnings (non-blocking): silk printed on a pad
                // is almost always a defect. Always present so the caller knows the
                // check ran; empty array when clean.
                result["warnings"] = Value::Array(silk_warnings);

                // Echo each footprint's 3D body (height + source), so the caller can
                // confirm an auto-created body's assumed height or correct it.
                result["bodies"] = Value::Array(bodies_echo);

                // Run post-write validation
                if let Some(validation) = Self::post_write_validation_pcblib(filepath) {
                    result["validation"] = validation;
                }

                ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
            }
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap())
            }
        }
    }

    /// Reads a `SchLib` file and returns its contents.
    /// Supports pagination via limit/offset and filtering by `component_name`.
    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn call_read_schlib(&self, arguments: &Value) -> ToolCallResult {
        use crate::altium::SchLib;

        let Some(filepath) = arguments.get("filepath").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: filepath");
        };

        // Validate path is within allowed directories
        if let Err(e) = self.validate_path(filepath) {
            return ToolCallResult::error(e);
        }

        // Parse optional pagination/filter parameters
        let component_name = arguments.get("component_name").and_then(Value::as_str);
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| v as usize);
        let offset = arguments
            .get("offset")
            .and_then(Value::as_u64)
            .map_or(0, |v| v as usize);

        match SchLib::open(filepath) {
            Ok(library) => {
                let total_count = library.len();

                // Apply filtering and pagination
                let symbols: Vec<_> = library
                    .iter()
                    .filter(|symbol| {
                        // If component_name specified, only include matching
                        component_name.map_or(true, |filter| symbol.name == filter)
                    })
                    .skip(offset)
                    .take(limit.unwrap_or(usize::MAX))
                    .map(|symbol| {
                        json!({
                            "name": symbol.name,
                            "description": symbol.description,
                            "designator": symbol.designator,
                            "designator_x": symbol.designator_x,
                            "designator_y": symbol.designator_y,
                            "designator_unique_id": symbol.designator_unique_id,
                            "part_count": symbol.part_count,
                            "pins": symbol.pins,
                            "rectangles": symbol.rectangles,
                            "round_rects": symbol.round_rects,
                            "lines": symbol.lines,
                            "polylines": symbol.polylines,
                            "polygons": symbol.polygons,
                            "arcs": symbol.arcs,
                            "pies": symbol.pies,
                            "images": symbol.images,
                            "text_frames": symbol.text_frames,
                            "beziers": symbol.beziers,
                            "ellipses": symbol.ellipses,
                            "elliptical_arcs": symbol.elliptical_arcs,
                            "labels": symbol.labels,
                            "text": symbol.text,
                            "parameters": symbol.parameters,
                            "footprints": symbol.footprints,
                        })
                    })
                    .collect();

                let returned_count = symbols.len();
                let has_more = if component_name.is_some() {
                    false // Single component fetch, no pagination
                } else {
                    offset + returned_count < total_count
                };

                let result = json!({
                    "status": "success",
                    "filepath": filepath,
                    "units": "schematic units (10 = 1 grid)",
                    "total_count": total_count,
                    "returned_count": returned_count,
                    "offset": offset,
                    "has_more": has_more,
                    "symbols": symbols,
                });

                ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
            }
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap())
            }
        }
    }

    /// Writes symbols to a `SchLib` file.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn call_write_schlib(&self, arguments: &Value) -> ToolCallResult {
        use crate::altium::schlib::{FootprintModel, SchLib, Symbol};

        let Some(filepath) = arguments.get("filepath").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: filepath");
        };

        // Validate path is within allowed directories
        if let Err(e) = self.validate_path(filepath) {
            return ToolCallResult::error(e);
        }

        let Some(symbols_json) = arguments.get("symbols").and_then(Value::as_array) else {
            return ToolCallResult::error("Missing required parameter: symbols");
        };

        // Collect and validate symbol names
        let new_names: Vec<&str> = symbols_json
            .iter()
            .filter_map(|sym| sym.get("name").and_then(Value::as_str))
            .collect();

        // Check for duplicates within the new symbols
        {
            let mut seen = std::collections::HashSet::new();
            for name in &new_names {
                if !seen.insert(*name) {
                    return ToolCallResult::error_with_context(
                        ErrorContext::new(
                            "write_schlib",
                            format!("Duplicate symbol name: '{name}'"),
                        )
                        .with_filepath(filepath)
                        .with_component(*name)
                        .with_details("Each symbol in the request must have a unique name"),
                    );
                }
            }
        }

        // Validate symbol names
        // Note: OLE storage names are limited to 31 characters, but the library layer
        // handles this by truncating storage names while preserving full names in LIBREFERENCE.
        #[allow(clippy::items_after_statements)]
        const INVALID_CHARS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
        for name in &new_names {
            if name.is_empty() {
                return ToolCallResult::error("Symbol name cannot be empty");
            }
            if let Some(c) = name.chars().find(|c| INVALID_CHARS.contains(c)) {
                return ToolCallResult::error(format!(
                    "Symbol name '{name}' contains invalid character '{c}'. \
                     Names cannot contain: / \\ : * ? \" < > |",
                ));
            }
        }

        let append = arguments
            .get("append")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // If append mode and file exists, read existing library; otherwise create new
        let mut library = if append && std::path::Path::new(filepath).exists() {
            match SchLib::open(filepath) {
                Ok(lib) => lib,
                Err(e) => {
                    return ToolCallResult::error_with_context(
                        ErrorContext::new(
                            "write_schlib",
                            format!("Failed to read existing library: {e}"),
                        )
                        .with_filepath(filepath)
                        .with_details(
                            "The library file exists but could not be opened for appending",
                        ),
                    );
                }
            }
        } else {
            SchLib::new()
        };

        // Check for duplicates with existing symbols in append mode
        if append {
            for name in &new_names {
                if library.get(name).is_some() {
                    return ToolCallResult::error(format!(
                        "Symbol '{name}' already exists in the library"
                    ));
                }
            }
        }

        // Names of the symbols written by *this* call. Recorded as they are added
        // (rather than reused from `new_names`) so a symbol that omitted "name" and
        // fell back to the default is still represented. Used to scope the geometry
        // echo below to what the caller actually wrote.
        let mut written_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        for sym_json in symbols_json {
            check_keys!(
                sym_json,
                &[
                    "name",
                    "description",
                    "designator",
                    "designator_prefix",
                    "designator_x",
                    "designator_y",
                    "designator_unique_id",
                    "component_type",
                    "part_count",
                    "display_mode_count",
                    "current_part_id",
                    "part_id_locked",
                    "source_library_name",
                    "target_file_name",
                    "pins",
                    "rectangles",
                    "round_rects",
                    "lines",
                    "polylines",
                    "polygons",
                    "arcs",
                    "pies",
                    "images",
                    "text_frames",
                    "beziers",
                    "ellipses",
                    "elliptical_arcs",
                    "labels",
                    "text",
                    "parameters",
                    "footprints",
                    "primitive_order"
                ]
            );
            let name = sym_json
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Unnamed");
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
                for pin_json in pins {
                    check_keys!(
                        pin_json,
                        &[
                            "name",
                            "designator",
                            "x",
                            "y",
                            "length",
                            "orientation",
                            "electrical_type",
                            "hidden",
                            "show_name",
                            "show_designator",
                            "owner_part_id",
                            "symbol_inner_edge",
                            "symbol_outer_edge",
                            "symbol_inside",
                            "symbol_outside",
                            "description",
                            "colour",
                            "graphically_locked",
                            "swap_id_group",
                            "part_and_sequence",
                            "default_value",
                            "owner_part_display_mode",
                            "symbol_line_width",
                            "frac",
                            "is_not_accessible",
                            "formal_type"
                        ]
                    );
                    if let Some(pin) = Self::parse_schlib_pin(pin_json) {
                        symbol.add_pin(pin);
                    }
                }
            }

            // Parse rectangles
            if let Some(rects) = sym_json.get("rectangles").and_then(Value::as_array) {
                for rect_json in rects {
                    check_keys!(
                        rect_json,
                        &[
                            "fill_color",
                            "filled",
                            "line_color",
                            "line_style",
                            "line_width",
                            "owner_part_id",
                            "transparent",
                            "x1",
                            "x2",
                            "y1",
                            "y2",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(rect) = Self::parse_schlib_rectangle(rect_json) {
                        symbol.add_rectangle(rect);
                    }
                }
            }

            // Parse rounded rectangles
            if let Some(round_rects) = sym_json.get("round_rects").and_then(Value::as_array) {
                for round_rect_json in round_rects {
                    check_keys!(
                        round_rect_json,
                        &[
                            "corner_x_radius",
                            "corner_y_radius",
                            "fill_color",
                            "filled",
                            "line_color",
                            "line_style",
                            "line_width",
                            "owner_part_id",
                            "transparent",
                            "x1",
                            "x2",
                            "y1",
                            "y2",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(round_rect) = Self::parse_schlib_round_rect(round_rect_json) {
                        symbol.add_round_rect(round_rect);
                    }
                }
            }

            // Parse lines
            if let Some(lines) = sym_json.get("lines").and_then(Value::as_array) {
                for line_json in lines {
                    check_keys!(
                        line_json,
                        &[
                            "color",
                            "line_style",
                            "line_width",
                            "is_not_accessible",
                            "owner_part_id",
                            "x1",
                            "x2",
                            "y1",
                            "y2",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(line) = Self::parse_schlib_line(line_json) {
                        symbol.add_line(line);
                    }
                }
            }

            // Parse polylines
            if let Some(polylines) = sym_json.get("polylines").and_then(Value::as_array) {
                for polyline_json in polylines {
                    check_keys!(
                        polyline_json,
                        &[
                            "color",
                            "end_line_shape",
                            "line_shape_size",
                            "line_style",
                            "line_width",
                            "is_not_accessible",
                            "owner_part_id",
                            "points",
                            "start_line_shape",
                            "transparent",
                            "vertices",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(polyline) = Self::parse_schlib_polyline(polyline_json) {
                        symbol.add_polyline(polyline);
                    }
                }
            }

            // Parse polygons
            if let Some(polygons) = sym_json.get("polygons").and_then(Value::as_array) {
                for polygon_json in polygons {
                    check_keys!(
                        polygon_json,
                        &[
                            "fill_color",
                            "filled",
                            "line_color",
                            "line_width",
                            "line_style",
                            "transparent",
                            "is_not_accessible",
                            "owner_part_id",
                            "points",
                            "vertices",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(polygon) = Self::parse_schlib_polygon(polygon_json) {
                        symbol.add_polygon(polygon);
                    }
                }
            }

            // Parse arcs
            if let Some(arcs) = sym_json.get("arcs").and_then(Value::as_array) {
                for arc_json in arcs {
                    // SchLib arcs are centre/radius/angle based, NOT layer-based like PcbLib arcs; the
                    // allow-list must match the documented fields in tool_definitions or every arc is
                    // rejected as an "unknown field" (was erroneously copied from the PcbLib arc as ["layer"]).
                    check_keys!(
                        arc_json,
                        &[
                            "color",
                            "end_angle",
                            "fill_color",
                            "line_width",
                            "is_not_accessible",
                            "owner_part_id",
                            "radius",
                            "start_angle",
                            "x",
                            "y",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(arc) = Self::parse_schlib_arc(arc_json) {
                        symbol.add_arc(arc);
                    }
                }
            }

            if let Some(pies) = sym_json.get("pies").and_then(Value::as_array) {
                for pie_json in pies {
                    check_keys!(
                        pie_json,
                        &[
                            "x",
                            "y",
                            "radius",
                            "start_angle",
                            "end_angle",
                            "line_width",
                            "line_color",
                            "fill_color",
                            "filled",
                            "transparent",
                            "is_not_accessible",
                            "owner_part_id",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(pie) = Self::parse_schlib_pie(pie_json) {
                        symbol.add_pie(pie);
                    }
                }
            }

            if let Some(images) = sym_json.get("images").and_then(Value::as_array) {
                for image_json in images {
                    check_keys!(
                        image_json,
                        &[
                            "x1",
                            "y1",
                            "x2",
                            "y2",
                            "line_width",
                            "line_color",
                            "line_style",
                            "fill_color",
                            "filled",
                            "transparent",
                            "show_border",
                            "keep_aspect",
                            "embed_image",
                            "file_name",
                            "image_data",
                            "is_not_accessible",
                            "owner_part_id",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(image) = Self::parse_schlib_image(image_json) {
                        symbol.add_image(image);
                    }
                }
            }

            if let Some(text_frames) = sym_json.get("text_frames").and_then(Value::as_array) {
                for frame_json in text_frames {
                    check_keys!(
                        frame_json,
                        &[
                            "x1",
                            "y1",
                            "x2",
                            "y2",
                            "text",
                            "color",
                            "area_color",
                            "text_color",
                            "text_margin",
                            "line_width",
                            "line_style",
                            "transparent",
                            "font_id",
                            "orientation",
                            "alignment",
                            "is_solid",
                            "show_border",
                            "word_wrap",
                            "clip_to_rect",
                            "is_not_accessible",
                            "owner_part_id",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(text_frame) = Self::parse_schlib_text_frame(frame_json) {
                        symbol.add_text_frame(text_frame);
                    }
                }
            }

            if let Some(beziers) = sym_json.get("beziers").and_then(Value::as_array) {
                for bezier_json in beziers {
                    check_keys!(
                        bezier_json,
                        &[
                            "x1",
                            "y1",
                            "x2",
                            "y2",
                            "x3",
                            "y3",
                            "x4",
                            "y4",
                            "line_width",
                            "color",
                            "is_not_accessible",
                            "owner_part_id",
                            "unique_id"
                        ]
                    );
                    if let Some(bezier) = Self::parse_schlib_bezier(bezier_json) {
                        symbol.add_bezier(bezier);
                    }
                }
            }

            if let Some(ell_arcs) = sym_json.get("elliptical_arcs").and_then(Value::as_array) {
                for ell_arc_json in ell_arcs {
                    check_keys!(
                        ell_arc_json,
                        &[
                            "x",
                            "y",
                            "radius",
                            "secondary_radius",
                            "start_angle",
                            "end_angle",
                            "line_width",
                            "color",
                            "fill_color",
                            "owner_part_id",
                            "unique_id"
                        ]
                    );
                    if let Some(ell_arc) = Self::parse_schlib_elliptical_arc(ell_arc_json) {
                        symbol.add_elliptical_arc(ell_arc);
                    }
                }
            }

            // Parse ellipses
            if let Some(ellipses) = sym_json.get("ellipses").and_then(Value::as_array) {
                for ellipse_json in ellipses {
                    check_keys!(
                        ellipse_json,
                        &[
                            "fill_color",
                            "filled",
                            "line_color",
                            "line_width",
                            "is_not_accessible",
                            "owner_part_id",
                            "radius_x",
                            "radius_y",
                            "transparent",
                            "x",
                            "y",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(ellipse) = Self::parse_schlib_ellipse(ellipse_json) {
                        symbol.add_ellipse(ellipse);
                    }
                }
            }

            // Parse labels
            if let Some(labels) = sym_json.get("labels").and_then(Value::as_array) {
                for label_json in labels {
                    check_keys!(
                        label_json,
                        &[
                            "color",
                            "font_id",
                            "hidden",
                            "is_hidden",
                            "is_mirrored",
                            "justification",
                            "owner_part_id",
                            "rotation",
                            "text",
                            "x",
                            "y",
                            "unique_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(label) = Self::parse_schlib_label(label_json) {
                        symbol.add_label(label);
                    }
                }
            }

            // Parse text annotations
            if let Some(texts) = sym_json.get("text").and_then(Value::as_array) {
                for text_json in texts {
                    check_keys!(
                        text_json,
                        &[
                            "color",
                            "font_id",
                            "hidden",
                            "is_hidden",
                            "is_mirrored",
                            "justification",
                            "owner_part_id",
                            "rotation",
                            "text",
                            "x",
                            "y",
                            "unique_id"
                        ]
                    );
                    if let Some(text) = Self::parse_schlib_text(text_json) {
                        symbol.add_text(text);
                    }
                }
            }

            // Parse parameters
            if let Some(params) = sym_json.get("parameters").and_then(Value::as_array) {
                for param_json in params {
                    check_keys!(
                        param_json,
                        &[
                            "name",
                            "value",
                            "x",
                            "y",
                            "hidden",
                            "font_id",
                            "color",
                            "read_only_state",
                            "param_type",
                            "unique_id",
                            "orientation",
                            "justification",
                            "show_name",
                            "hide_name",
                            "description",
                            "is_configurable",
                            "owner_part_id",
                            "graphically_locked",
                            "disabled",
                            "dimmed",
                            "owner_part_display_mode"
                        ]
                    );
                    if let Some(param) = Self::parse_schlib_parameter(param_json) {
                        symbol.add_parameter(param);
                    }
                }
            }

            // Parse footprint references
            if let Some(footprints) = sym_json.get("footprints").and_then(Value::as_array) {
                for fp_json in footprints {
                    // A footprint reference is a model link (name + optional
                    // description + library_path), not an embedded footprint, so
                    // only those fields are read here.
                    check_keys!(fp_json, &["name", "description", "library_path"]);
                    if let Some(fp_name) = fp_json.get("name").and_then(Value::as_str) {
                        let mut fp = FootprintModel::new(fp_name);
                        if let Some(desc) = fp_json.get("description").and_then(Value::as_str) {
                            fp.description = desc.to_string();
                        }
                        // Optional PcbLib path -> ModelDatafile0, so Altium can
                        // resolve the footprint instead of reporting "not found".
                        if let Some(lib_path) = fp_json.get("library_path").and_then(Value::as_str)
                        {
                            fp.library_path = Some(lib_path.to_string());
                        }
                        symbol.add_footprint(fp);
                    }
                }
            }

            // Validate coordinates before adding
            if let Err(e) = Self::validate_symbol_coordinates(&symbol) {
                return ToolCallResult::error(e);
            }

            written_names.insert(symbol.name.clone());
            library.add(symbol);
        }

        // Create backup before destructive operation (if file exists)
        if let Err(e) = Self::create_backup(filepath) {
            return ToolCallResult::error(e);
        }

        match library.save(filepath) {
            Ok(()) => {
                let symbol_names: Vec<_> = library.iter().map(|s| s.name.clone()).collect();
                let mut result = json!({
                    "status": "success",
                    "filepath": filepath,
                    "symbol_count": library.len(),
                    "symbol_names": symbol_names,
                });

                // Echo computed pin geometry (body-attach end, connection tip,
                // orientation, bounding box) so the caller can verify pin placement
                // and catch flipped/misaligned pins without opening Altium.
                //
                // Scoped to the symbols written by this call. Echoing the whole
                // library made an `append: true` sequence grow the response
                // quadratically — a 27-symbol library built over 11 appends echoed
                // 196 symbol-geometry blocks instead of 26, large enough to stop the
                // response being usable — and pre-existing symbols tell the caller
                // nothing about the write it just performed.
                result["geometry"] = Value::Array(
                    library
                        .iter()
                        .filter(|s| written_names.contains(&s.name))
                        .map(symbol_geometry)
                        .collect(),
                );

                // Run post-write validation
                if let Some(validation) = Self::post_write_validation_schlib(filepath) {
                    result["validation"] = validation;
                }

                ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
            }
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap())
            }
        }
    }

    /// Writes an Altium Library Package (`.LibPkg`) project file that groups
    /// the given source documents so Altium can compile them into an
    /// Integrated Library. Only generates the project source; compiling to
    /// `.IntLib` is done inside Altium.
    pub(crate) fn call_write_libpkg(&self, arguments: &Value) -> ToolCallResult {
        use crate::altium::libpkg;

        let Some(filepath) = arguments.get("filepath").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: filepath");
        };

        // Validate path is within allowed directories
        if let Err(e) = self.validate_path(filepath) {
            return ToolCallResult::error(e);
        }

        // Validate file extension
        let ext = std::path::Path::new(filepath)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase);
        if ext.as_deref() != Some("libpkg") {
            return ToolCallResult::error("write_libpkg only supports .LibPkg files");
        }

        let Some(documents) = arguments.get("documents").and_then(Value::as_array) else {
            return ToolCallResult::error("Missing required parameter: documents");
        };
        let docs: Vec<String> = documents
            .iter()
            .filter_map(|d| d.as_str().map(String::from))
            .collect();
        if docs.is_empty() {
            return ToolCallResult::error(
                "documents must contain at least one .SchLib/.PcbLib path",
            );
        }

        let path = std::path::Path::new(filepath);
        let content = libpkg::build_libpkg(path, &docs);
        if let Err(e) = std::fs::write(path, content) {
            return ToolCallResult::error(format!("Failed to write LibPkg: {e}"));
        }

        let relative: Vec<String> = docs
            .iter()
            .map(|d| libpkg::relative_to_libpkg(path, d))
            .collect();
        let result = json!({
            "status": "success",
            "filepath": filepath,
            "documents": relative,
            "count": relative.len(),
            "note": "Open in Altium and run Project > Compile Integrated Library to produce the .IntLib.",
        });
        ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
    }

    /// Lists component names in a library file.
    #[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
    pub(crate) fn call_list_components(&self, arguments: &Value) -> ToolCallResult {
        use crate::altium::{PcbLib, SchLib};

        let Some(filepath) = arguments.get("filepath").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: filepath");
        };

        // Validate path is within allowed directories
        if let Err(e) = self.validate_path(filepath) {
            return ToolCallResult::error(e);
        }

        // Parse optional pagination parameters
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| v as usize);
        let offset = arguments
            .get("offset")
            .and_then(Value::as_u64)
            .map_or(0, |v| v as usize);

        // Parse include_metadata parameter (default: false)
        let include_metadata = arguments
            .get("include_metadata")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Try to determine file type from extension
        let path = std::path::Path::new(filepath);
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase);

        match extension.as_deref() {
            Some("pcblib") => match PcbLib::open(filepath) {
                Ok(library) => {
                    let total_count = library.len();

                    // Apply pagination and optionally include metadata
                    let components: Vec<Value> = if include_metadata {
                        library
                            .iter()
                            .skip(offset)
                            .take(limit.unwrap_or(usize::MAX))
                            .map(|fp| {
                                json!({
                                    "name": fp.name,
                                    "description": fp.description,
                                    "pad_count": fp.pads.len(),
                                    "track_count": fp.tracks.len(),
                                    "arc_count": fp.arcs.len(),
                                    "region_count": fp.regions.len(),
                                    "text_count": fp.text.len(),
                                    "has_3d_model": fp.model_3d.is_some() || !fp.component_bodies.is_empty(),
                                })
                            })
                            .collect()
                    } else {
                        library
                            .names()
                            .into_iter()
                            .skip(offset)
                            .take(limit.unwrap_or(usize::MAX))
                            .map(|n| json!(n))
                            .collect()
                    };

                    let returned_count = components.len();
                    let has_more = offset + returned_count < total_count;

                    let result = json!({
                        "status": "success",
                        "filepath": filepath,
                        "file_type": "PcbLib",
                        "total_count": total_count,
                        "returned_count": returned_count,
                        "offset": offset,
                        "has_more": has_more,
                        "include_metadata": include_metadata,
                        "components": components,
                    });
                    ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
                }
                Err(e) => {
                    let result = json!({
                        "status": "error",
                        "filepath": filepath,
                        "error": e.to_string(),
                    });
                    ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap())
                }
            },
            Some("schlib") => match SchLib::open(filepath) {
                Ok(library) => {
                    let total_count = library.len();

                    // Apply pagination and optionally include metadata
                    let components: Vec<Value> = if include_metadata {
                        library
                            .iter()
                            .skip(offset)
                            .take(limit.unwrap_or(usize::MAX))
                            .map(|s| {
                                json!({
                                    "name": s.name,
                                    "description": s.description,
                                    "designator": s.designator,
                                    "part_count": s.part_count,
                                    "pin_count": s.pins.len(),
                                    "footprint_count": s.footprints.len(),
                                })
                            })
                            .collect()
                    } else {
                        library
                            .iter()
                            .map(|s| json!(s.name.clone()))
                            .skip(offset)
                            .take(limit.unwrap_or(usize::MAX))
                            .collect()
                    };

                    let returned_count = components.len();
                    let has_more = offset + returned_count < total_count;

                    let result = json!({
                        "status": "success",
                        "filepath": filepath,
                        "file_type": "SchLib",
                        "total_count": total_count,
                        "returned_count": returned_count,
                        "offset": offset,
                        "has_more": has_more,
                        "include_metadata": include_metadata,
                        "components": components,
                    });
                    ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
                }
                Err(e) => {
                    let result = json!({
                        "status": "error",
                        "filepath": filepath,
                        "error": e.to_string(),
                    });
                    ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap())
                }
            },
            _ => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": "Unknown file type. Expected .PcbLib or .SchLib extension.",
                });
                ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap())
            }
        }
    }

    /// Extracts style information from a library file.
    pub(crate) fn call_extract_style(&self, arguments: &Value) -> ToolCallResult {
        let Some(filepath) = arguments.get("filepath").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: filepath");
        };

        // Validate path is within allowed directories
        if let Err(e) = self.validate_path(filepath) {
            return ToolCallResult::error(e);
        }

        let path = std::path::Path::new(filepath);
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase);

        match extension.as_deref() {
            Some("pcblib") => Self::extract_pcblib_style(filepath),
            Some("schlib") => Self::extract_schlib_style(filepath),
            _ => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": "Unknown file type. Expected .PcbLib or .SchLib extension.",
                });
                ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap())
            }
        }
    }

    /// Extracts style from a `PcbLib` file.
    pub(crate) fn extract_pcblib_style(filepath: &str) -> ToolCallResult {
        use crate::altium::PcbLib;
        use std::collections::HashMap;

        match PcbLib::open(filepath) {
            Ok(library) => {
                // Track widths by layer
                let mut track_widths: HashMap<String, Vec<f64>> = HashMap::new();
                // Pad shapes count
                let mut pad_shapes: HashMap<String, usize> = HashMap::new();
                // Text heights
                let mut text_heights: Vec<f64> = Vec::new();
                // Layers used
                let mut layers_used: HashMap<String, usize> = HashMap::new();

                for fp in library.iter() {
                    // Analyse tracks
                    for track in &fp.tracks {
                        let layer_name = track.layer.as_str().to_string();
                        track_widths
                            .entry(layer_name.clone())
                            .or_default()
                            .push(track.width);
                        *layers_used.entry(layer_name).or_insert(0) += 1;
                    }

                    // Analyse pads
                    for pad in &fp.pads {
                        let shape_name = format!("{:?}", pad.shape);
                        *pad_shapes.entry(shape_name).or_insert(0) += 1;
                        let layer_name = pad.layer.as_str().to_string();
                        *layers_used.entry(layer_name).or_insert(0) += 1;
                    }

                    // Analyse text
                    for text in &fp.text {
                        text_heights.push(text.height);
                        let layer_name = text.layer.as_str().to_string();
                        *layers_used.entry(layer_name).or_insert(0) += 1;
                    }

                    // Analyse regions
                    for region in &fp.regions {
                        let layer_name = region.layer.as_str().to_string();
                        *layers_used.entry(layer_name).or_insert(0) += 1;
                    }
                }

                // Calculate statistics for track widths
                #[allow(clippy::cast_precision_loss)]
                let track_width_stats: HashMap<String, Value> = track_widths
                    .into_iter()
                    .map(|(layer, widths)| {
                        let min = widths.iter().copied().fold(f64::INFINITY, f64::min);
                        let max = widths.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                        let avg = widths.iter().sum::<f64>() / widths.len() as f64;
                        let most_common = Self::most_common_f64(&widths);
                        (
                            layer,
                            json!({
                                "min_mm": min,
                                "max_mm": max,
                                "avg_mm": avg,
                                "most_common_mm": most_common,
                                "count": widths.len()
                            }),
                        )
                    })
                    .collect();

                // Calculate text height stats
                let text_height_stats = if text_heights.is_empty() {
                    json!(null)
                } else {
                    let min = text_heights.iter().copied().fold(f64::INFINITY, f64::min);
                    let max = text_heights
                        .iter()
                        .copied()
                        .fold(f64::NEG_INFINITY, f64::max);
                    let most_common = Self::most_common_f64(&text_heights);
                    json!({
                        "min_mm": min,
                        "max_mm": max,
                        "most_common_mm": most_common,
                        "count": text_heights.len()
                    })
                };

                let result = json!({
                    "status": "success",
                    "filepath": filepath,
                    "file_type": "PcbLib",
                    "footprint_count": library.len(),
                    "style": {
                        "track_widths_by_layer": track_width_stats,
                        "pad_shapes": pad_shapes,
                        "text_heights": text_height_stats,
                        "layers_used": layers_used
                    }
                });

                ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
            }
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap())
            }
        }
    }

    /// Extracts style from a `SchLib` file.
    pub(crate) fn extract_schlib_style(filepath: &str) -> ToolCallResult {
        use crate::altium::SchLib;
        use std::collections::HashMap;

        match SchLib::open(filepath) {
            Ok(library) => {
                // Line widths
                let mut line_widths: Vec<u8> = Vec::new();
                // Pin lengths
                let mut pin_lengths: Vec<i32> = Vec::new();
                // Colours used
                let mut line_colors: HashMap<String, usize> = HashMap::new();
                let mut fill_colors: HashMap<String, usize> = HashMap::new();
                // Rectangle stats
                let mut rect_filled_count = 0usize;
                let mut rect_unfilled_count = 0usize;

                for symbol in library.iter() {
                    // Analyse pins
                    for pin in &symbol.pins {
                        pin_lengths.push(pin.length);
                    }

                    // Analyse rectangles
                    for rect in &symbol.rectangles {
                        line_widths.push(rect.line_width);
                        let line_color = format!("#{:06X}", rect.line_color);
                        let fill_color = format!("#{:06X}", rect.fill_color);
                        *line_colors.entry(line_color).or_insert(0) += 1;
                        *fill_colors.entry(fill_color).or_insert(0) += 1;
                        if rect.filled {
                            rect_filled_count += 1;
                        } else {
                            rect_unfilled_count += 1;
                        }
                    }

                    // Analyse lines
                    for line in &symbol.lines {
                        line_widths.push(line.line_width);
                        let color = format!("#{:06X}", line.color);
                        *line_colors.entry(color).or_insert(0) += 1;
                    }
                }

                // Calculate stats
                let pin_length_stats = if pin_lengths.is_empty() {
                    json!(null)
                } else {
                    let min = *pin_lengths.iter().min().unwrap();
                    let max = *pin_lengths.iter().max().unwrap();
                    let most_common = Self::most_common(&pin_lengths);
                    json!({
                        "min_units": min,
                        "max_units": max,
                        "most_common_units": most_common,
                        "count": pin_lengths.len()
                    })
                };

                let line_width_stats = if line_widths.is_empty() {
                    json!(null)
                } else {
                    let min = *line_widths.iter().min().unwrap();
                    let max = *line_widths.iter().max().unwrap();
                    let most_common = Self::most_common(&line_widths);
                    json!({
                        "min": min,
                        "max": max,
                        "most_common": most_common,
                        "count": line_widths.len()
                    })
                };

                let result = json!({
                    "status": "success",
                    "filepath": filepath,
                    "file_type": "SchLib",
                    "symbol_count": library.len(),
                    "style": {
                        "pin_lengths": pin_length_stats,
                        "line_widths": line_width_stats,
                        "line_colors": line_colors,
                        "fill_colors": fill_colors,
                        "rectangles": {
                            "filled_count": rect_filled_count,
                            "unfilled_count": rect_unfilled_count
                        }
                    }
                });

                ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
            }
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap())
            }
        }
    }

    /// Finds the most common value in a slice of hashable, copyable values.
    ///
    /// Returns the default value if the slice is empty.
    pub(crate) fn most_common<T>(values: &[T]) -> T
    where
        T: std::hash::Hash + Eq + Copy + Default,
    {
        use std::collections::HashMap;
        let mut counts: HashMap<T, usize> = HashMap::new();
        for &v in values {
            *counts.entry(v).or_insert(0) += 1;
        }
        counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map_or_else(T::default, |(key, _)| key)
    }

    /// Finds the most common value in a slice of f64, rounded to 2 decimal places.
    ///
    /// Since f64 doesn't implement Hash/Eq, values are quantized to centesimal
    /// precision (0.01) for grouping purposes.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub(crate) fn most_common_f64(values: &[f64]) -> f64 {
        use std::collections::HashMap;
        let mut counts: HashMap<i64, usize> = HashMap::new();
        for &v in values {
            // Round to 2 decimal places for grouping
            let key = (v * 100.0).round() as i64;
            *counts.entry(key).or_insert(0) += 1;
        }
        counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map_or(0.0, |(key, _)| key as f64 / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::ieee_designator_prefix;
    use super::{body_3d_summary, pin_tip, symbol_geometry};
    use crate::altium::schlib::{Pin, PinOrientation, Rectangle, Symbol};

    #[test]
    fn segment_rect_intersection_detects_silk_over_pad_geometry() {
        use super::segment_intersects_rect;
        // Horizontal segment straight through the rect.
        assert!(segment_intersects_rect(
            -5.0, 0.0, 5.0, 0.0, -1.0, -1.0, 1.0, 1.0
        ));
        // Vertical stripe through the rect (the reported silk-on-pad case).
        assert!(segment_intersects_rect(
            0.0, -5.0, 0.0, 5.0, -1.0, -1.0, 1.0, 1.0
        ));
        // Endpoint inside the rect.
        assert!(segment_intersects_rect(
            0.0, 0.0, 5.0, 5.0, -1.0, -1.0, 1.0, 1.0
        ));
        // Clear of the rect (no overlap).
        assert!(!segment_intersects_rect(
            2.0, 2.0, 3.0, 3.0, -1.0, -1.0, 1.0, 1.0
        ));
        // Parallel and outside the slab.
        assert!(!segment_intersects_rect(
            -5.0, 2.0, 5.0, 2.0, -1.0, -1.0, 1.0, 1.0
        ));
    }

    #[test]
    fn body_3d_summary_reports_source_and_height() {
        use crate::altium::pcblib::{ComponentBody, Footprint, Layer};
        let body = |h: f64, name: &str| ComponentBody {
            model_id: String::new(),
            identifier: String::new(),
            texture_center_x: None,
            texture_center_y: None,
            texture_size_x: None,
            texture_size_y: None,
            model_name: name.to_string(),
            embedded: false,
            rotation_x: 0.0,
            rotation_y: 0.0,
            rotation_z: 0.0,
            z_offset: 0.0,
            overall_height: h,
            standoff_height: 0.0,
            cavity_height: 0.0,
            layer: Layer::Top3DBody,
            outline: Vec::new(),
            unique_id: None,
            guid: None,
            model_checksum: 0,
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
            net_index: 0xFFFF,
            polygon_index: 0xFFFF,
            component_index: -1,
            additional_parameters: Vec::new(),
        };

        // Explicit extruded body: reports its height, not assumed.
        let mut ext = Footprint::new("EXT");
        ext.add_component_body(body(2.5, ""));
        assert_eq!(body_3d_summary(&ext, false)["source"], "extruded");
        assert_eq!(body_3d_summary(&ext, false)["overall_height"], 2.5);
        assert_eq!(body_3d_summary(&ext, false)["assumed_height"], false);

        // Same body, auto-created path: flagged assumed.
        assert_eq!(body_3d_summary(&ext, true)["source"], "auto-extruded");
        assert_eq!(body_3d_summary(&ext, true)["assumed_height"], true);
        // The assumed case carries an actionable message prompting a real height.
        assert!(body_3d_summary(&ext, true)["action_required"].is_string());
        // The explicit case does not.
        assert!(body_3d_summary(&ext, false)["action_required"].is_null());

        // No body at all: source none.
        let none = Footprint::new("NONE");
        assert_eq!(body_3d_summary(&none, false)["source"], "none");
    }

    #[test]
    fn pin_tip_points_outward_per_orientation() {
        assert_eq!(
            pin_tip(&Pin::new("N", "1", -40, 20, 30, PinOrientation::Left)),
            (-70, 20)
        );
        assert_eq!(
            pin_tip(&Pin::new("N", "1", 40, 20, 30, PinOrientation::Right)),
            (70, 20)
        );
        assert_eq!(
            pin_tip(&Pin::new("N", "1", 0, 0, 30, PinOrientation::Up)),
            (0, 30)
        );
        assert_eq!(
            pin_tip(&Pin::new("N", "1", 0, 0, 30, PinOrientation::Down)),
            (0, -30)
        );
    }

    #[test]
    fn symbol_geometry_reports_tip_orientation_and_bbox() {
        let mut s = Symbol::new("U1");
        s.add_pin(Pin::new("VIN", "1", -50, 20, 30, PinOrientation::Left));
        s.add_pin(Pin::new("OUT", "2", 50, 20, 30, PinOrientation::Right));
        s.add_rectangle(Rectangle::new(-50, 40, 50, -40));
        let g = symbol_geometry(&s);
        assert_eq!(g["pins"][0]["orientation"], "left");
        assert_eq!(g["pins"][0]["body_end"]["x"], -50);
        assert_eq!(g["pins"][0]["tip"]["x"], -80);
        assert_eq!(g["pins"][1]["tip"]["x"], 80);
        assert_eq!(g["bounding_box"]["min_x"], -80);
        assert_eq!(g["bounding_box"]["max_x"], 80);
    }

    #[test]
    fn ieee_map_known_types() {
        assert_eq!(ieee_designator_prefix("resistor"), "R");
        assert_eq!(ieee_designator_prefix("capacitor"), "C");
        assert_eq!(ieee_designator_prefix("inductor"), "L");
        assert_eq!(ieee_designator_prefix("diode"), "D");
        assert_eq!(ieee_designator_prefix("led"), "D");
        assert_eq!(ieee_designator_prefix("transistor"), "Q");
        assert_eq!(ieee_designator_prefix("mosfet"), "Q");
        assert_eq!(ieee_designator_prefix("connector"), "J");
        assert_eq!(ieee_designator_prefix("crystal"), "Y");
        assert_eq!(ieee_designator_prefix("ic"), "U");
        assert_eq!(ieee_designator_prefix("regulator"), "U");
    }

    #[test]
    fn ieee_map_is_case_and_whitespace_insensitive() {
        assert_eq!(ieee_designator_prefix("  Resistor "), "R");
        assert_eq!(ieee_designator_prefix("CAPACITOR"), "C");
    }

    #[test]
    fn ieee_map_unknown_falls_back_to_u() {
        assert_eq!(ieee_designator_prefix("flux_capacitor"), "U");
        assert_eq!(ieee_designator_prefix(""), "U");
    }

    // ==================== extract_style ====================

    mod extract_style {
        use crate::altium::pcblib::{Footprint, Layer, Pad, PcbLib, Track};
        use crate::mcp::tools::test_support::{
            create_test_schlib, create_test_server, get_result_text, parse_result_json,
            test_temp_dir,
        };
        use serde_json::json;

        #[test]
        fn extract_style_pcblib_reports_track_and_pad_statistics() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            // Two footprints: three 0.2 mm overlay tracks and one 0.4 mm, plus
            // four rectangular pads.
            let mut lib = PcbLib::new();
            let mut fp1 = Footprint::new("A");
            fp1.add_pad(Pad::smd("1", -0.5, 0.0, 0.6, 0.5));
            fp1.add_pad(Pad::smd("2", 0.5, 0.0, 0.6, 0.5));
            fp1.add_track(Track::new(-1.0, -1.0, 1.0, -1.0, 0.2, Layer::TopOverlay));
            fp1.add_track(Track::new(-1.0, 1.0, 1.0, 1.0, 0.2, Layer::TopOverlay));
            lib.add(fp1);
            let mut fp2 = Footprint::new("B");
            fp2.add_pad(Pad::smd("1", -0.8, 0.0, 0.8, 0.8));
            fp2.add_pad(Pad::smd("2", 0.8, 0.0, 0.8, 0.8));
            fp2.add_track(Track::new(-2.0, -2.0, 2.0, -2.0, 0.2, Layer::TopOverlay));
            fp2.add_track(Track::new(-2.0, 2.0, 2.0, 2.0, 0.4, Layer::TopOverlay));
            lib.add(fp2);
            let path = dir.path().join("Style.PcbLib");
            lib.save(&path).unwrap();

            let result = server.call_extract_style(&json!({
                "filepath": path.to_string_lossy(),
            }));
            assert!(!result.is_error, "{}", get_result_text(&result));
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "success");
            assert_eq!(parsed["file_type"], "PcbLib");
            assert_eq!(parsed["footprint_count"], 2);

            let overlay = &parsed["style"]["track_widths_by_layer"]["Top Overlay"];
            assert_eq!(overlay["count"], 4);
            // Widths quantise to 0.01 mm for the most-common statistic.
            assert!((overlay["most_common_mm"].as_f64().unwrap() - 0.2).abs() < 1e-9);
            assert!((overlay["min_mm"].as_f64().unwrap() - 0.2).abs() < 1e-3);
            assert!((overlay["max_mm"].as_f64().unwrap() - 0.4).abs() < 1e-3);

            // `Pad::smd` creates rounded-rectangle pads.
            assert_eq!(parsed["style"]["pad_shapes"]["RoundedRectangle"], 4);
            assert_eq!(parsed["style"]["layers_used"]["Top Overlay"], 4);
            assert_eq!(parsed["style"]["text_heights"], serde_json::Value::Null);
        }

        #[test]
        fn extract_style_schlib_reports_pin_and_line_statistics() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Style.SchLib");
            create_test_schlib(&path);

            let result = server.call_extract_style(&json!({
                "filepath": path.to_string_lossy(),
            }));
            assert!(!result.is_error, "{}", get_result_text(&result));
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "success");
            assert_eq!(parsed["file_type"], "SchLib");
            assert_eq!(parsed["symbol_count"], 2);

            // Four fixture pins, all 10 units long.
            let pins = &parsed["style"]["pin_lengths"];
            assert_eq!(pins["count"], 4);
            assert_eq!(pins["min_units"], 10);
            assert_eq!(pins["max_units"], 10);
            assert_eq!(pins["most_common_units"], 10);

            // One fixture rectangle contributes the only line width.
            assert_eq!(parsed["style"]["line_widths"]["count"], 1);
            assert_eq!(parsed["style"]["rectangles"]["filled_count"], 1);
            assert_eq!(parsed["style"]["rectangles"]["unfilled_count"], 0);
        }

        #[test]
        fn extract_style_error_paths() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let result = server.call_extract_style(&json!({}));
            assert!(result.is_error);
            assert_eq!(
                get_result_text(&result),
                "Missing required parameter: filepath"
            );

            // Unknown extension.
            let txt = dir.path().join("x.txt");
            let result = server.call_extract_style(&json!({
                "filepath": txt.to_string_lossy(),
            }));
            assert!(result.is_error);
            assert!(get_result_text(&result).contains("Unknown file type"));

            // Unreadable library.
            let missing = dir.path().join("Missing.PcbLib");
            let result = server.call_extract_style(&json!({
                "filepath": missing.to_string_lossy(),
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
        }
    }

    // ==================== read/write handler error paths ====================

    mod handler_error_paths {
        use crate::mcp::tools::test_support::{
            create_test_pcblib, create_test_server, get_result_text, test_temp_dir,
        };
        use serde_json::json;

        #[test]
        fn read_pcblib_missing_filepath() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let r = server.call_read_pcblib(&json!({}));
            assert!(r.is_error);
            assert_eq!(get_result_text(&r), "Missing required parameter: filepath");
        }

        #[test]
        fn read_pcblib_denied_path_outside_allowed() {
            let allowed = test_temp_dir();
            let outside = test_temp_dir();
            let server = create_test_server(allowed.path());
            // Create a real library so its parent canonicalises — the denial is
            // about the path being outside the allow-list, not a missing file.
            let path = outside.path().join("X.PcbLib");
            create_test_pcblib(&path);
            let r = server.call_read_pcblib(&json!({ "filepath": path.to_string_lossy() }));
            assert!(r.is_error);
            assert!(
                get_result_text(&r).contains("Access denied"),
                "{}",
                get_result_text(&r)
            );
        }

        #[test]
        fn read_pcblib_nonexistent_file_is_error() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Nope.PcbLib");
            let r = server.call_read_pcblib(&json!({ "filepath": path.to_string_lossy() }));
            assert!(r.is_error);
        }

        #[test]
        fn write_pcblib_missing_filepath_then_footprints() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let r = server.call_write_pcblib(&json!({}));
            assert!(r.is_error);
            assert_eq!(get_result_text(&r), "Missing required parameter: filepath");

            let path = dir.path().join("W.PcbLib");
            let r = server.call_write_pcblib(&json!({ "filepath": path.to_string_lossy() }));
            assert!(r.is_error);
            assert_eq!(
                get_result_text(&r),
                "Missing required parameter: footprints"
            );
        }

        #[test]
        fn write_pcblib_denied_path_outside_allowed() {
            let allowed = test_temp_dir();
            let outside = test_temp_dir();
            let server = create_test_server(allowed.path());
            let path = outside.path().join("W.PcbLib");
            let r = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [{ "name": "X", "pads": [] }],
                "append": false,
            }));
            assert!(r.is_error);
            assert!(
                get_result_text(&r).contains("Access denied"),
                "{}",
                get_result_text(&r)
            );
        }

        #[test]
        fn read_schlib_missing_filepath() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let r = server.call_read_schlib(&json!({}));
            assert!(r.is_error);
            assert_eq!(get_result_text(&r), "Missing required parameter: filepath");
        }

        #[test]
        fn write_schlib_missing_filepath_then_symbols() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let r = server.call_write_schlib(&json!({}));
            assert!(r.is_error);
            assert_eq!(get_result_text(&r), "Missing required parameter: filepath");

            let path = dir.path().join("W.SchLib");
            let r = server.call_write_schlib(&json!({ "filepath": path.to_string_lossy() }));
            assert!(r.is_error);
            assert_eq!(get_result_text(&r), "Missing required parameter: symbols");
        }

        #[test]
        fn list_components_missing_filepath_and_nonexistent() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let r = server.call_list_components(&json!({}));
            assert!(r.is_error);
            assert_eq!(get_result_text(&r), "Missing required parameter: filepath");

            let path = dir.path().join("Nope.PcbLib");
            let r = server.call_list_components(&json!({ "filepath": path.to_string_lossy() }));
            assert!(r.is_error);
        }
    }

    // ==================== read/write handler success paths ====================

    mod handler_success_paths {
        use crate::altium::pcblib::{Footprint, Pad, PcbLib};
        use crate::mcp::tools::test_support::{
            create_test_pcblib, create_test_schlib, create_test_server, get_result_text,
            parse_result_json, test_temp_dir,
        };
        use serde_json::json;

        // ---- write_pcblib 3D-body summary sources ----

        #[test]
        fn write_pcblib_component_body_reports_extruded() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Body.PcbLib");
            let r = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [{
                    "name": "BODYFP",
                    "pads": [{ "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 }],
                    "component_bodies": [{ "overall_height": 2.5, "standoff_height": 0.1 }],
                }],
            }));
            assert!(!r.is_error, "{}", get_result_text(&r));
            let p = parse_result_json(&r);
            assert_eq!(p["status"], "success");
            assert_eq!(p["footprint_count"], 1);
            assert_eq!(p["bodies"][0]["source"], "extruded");
            assert_eq!(p["bodies"][0]["assumed_height"], false);
        }

        #[test]
        fn write_pcblib_step_model_external_reports_step_external() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Ext.PcbLib");
            let r = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [{
                    "name": "EXTMODEL",
                    "pads": [{ "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 }],
                    "step_model": { "filepath": "models/CHIP.step", "embed": false, "rotation": 90.0, "z_offset": 0.5 },
                }],
            }));
            assert!(!r.is_error, "{}", get_result_text(&r));
            let p = parse_result_json(&r);
            assert_eq!(p["bodies"][0]["source"], "step-external");
        }

        #[test]
        fn write_pcblib_auto_3d_body_reports_auto_extruded() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Auto.PcbLib");
            let r = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "auto_3d_body": true,
                "footprints": [{
                    "name": "AUTO",
                    "pads": [{ "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 }],
                }],
            }));
            assert!(!r.is_error, "{}", get_result_text(&r));
            let p = parse_result_json(&r);
            assert_eq!(p["bodies"][0]["source"], "auto-extruded");
            assert_eq!(p["bodies"][0]["assumed_height"], true);
            assert!(p["bodies"][0]["action_required"].is_string());
        }

        #[test]
        fn write_pcblib_silk_over_pad_warns() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Silk.PcbLib");
            let r = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [{
                    "name": "SILK",
                    "pads": [{ "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 }],
                    "tracks": [{ "x1": -2.0, "y1": 0.0, "x2": 2.0, "y2": 0.0, "width": 0.2, "layer": "Top Overlay" }],
                }],
            }));
            assert!(!r.is_error, "{}", get_result_text(&r));
            let p = parse_result_json(&r);
            let warnings = p["warnings"].as_array().unwrap();
            assert!(!warnings.is_empty());
            assert_eq!(warnings[0]["type"], "silk_over_pad");
            assert_eq!(warnings[0]["pad"], "1");
        }

        #[test]
        fn write_pcblib_append_adds_to_existing() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Append.PcbLib");
            let fp = |name: &str| json!({ "name": name, "pads": [{ "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 }] });
            server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(), "footprints": [fp("A")],
            }));
            let r = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(), "append": true, "footprints": [fp("B")],
            }));
            assert!(!r.is_error, "{}", get_result_text(&r));
            assert_eq!(parse_result_json(&r)["footprint_count"], 2);
        }

        // ---- read_pcblib emission + compact + pagination ----

        #[test]
        fn read_pcblib_emits_vias_fills_bodies_and_is_compact() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Rich.PcbLib");
            server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [{
                    "name": "RICH",
                    "pads": [{ "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 }],
                    "vias": [{ "x": 0.0, "y": 0.0, "diameter": 0.6, "hole_size": 0.3 }],
                    "fills": [{ "x1": -1.0, "y1": -1.0, "x2": 1.0, "y2": 1.0, "layer": "Top Layer" }],
                    "component_bodies": [{ "overall_height": 2.0 }],
                }],
            }));

            let r = server.call_read_pcblib(&json!({ "filepath": path.to_string_lossy() }));
            assert!(!r.is_error, "{}", get_result_text(&r));
            let p = parse_result_json(&r);
            assert_eq!(p["status"], "success");
            assert_eq!(p["compact"], true);
            assert_eq!(p["units"], "mm");
            let fp0 = &p["footprints"][0];
            assert_eq!(fp0["vias"].as_array().unwrap().len(), 1);
            assert_eq!(fp0["fills"].as_array().unwrap().len(), 1);
            assert_eq!(fp0["component_bodies"].as_array().unwrap().len(), 1);
        }

        #[test]
        fn read_pcblib_non_compact_and_pagination() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Page.PcbLib");
            create_test_pcblib(&path); // 2 footprints

            let non_compact = server
                .call_read_pcblib(&json!({ "filepath": path.to_string_lossy(), "compact": false }));
            assert_eq!(parse_result_json(&non_compact)["compact"], false);

            let paged = server.call_read_pcblib(&json!({
                "filepath": path.to_string_lossy(), "limit": 1, "offset": 0,
            }));
            let p = parse_result_json(&paged);
            assert_eq!(p["returned_count"], 1);
            assert_eq!(p["has_more"], true);

            let named = server.call_read_pcblib(&json!({
                "filepath": path.to_string_lossy(), "component_name": "CHIP_0402",
            }));
            let p = parse_result_json(&named);
            assert_eq!(p["returned_count"], 1);
            assert_eq!(p["has_more"], false);
        }

        // ---- read_schlib emission + write_schlib deep ----

        #[test]
        fn write_then_read_schlib_emits_parameters_and_footprints() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Sym.SchLib");
            let w = server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(),
                "symbols": [{
                    "name": "R1",
                    "component_type": "resistor",
                    "pins": [
                        { "name": "1", "designator": "1", "x": -20, "y": 0, "length": 10, "orientation": "left" },
                        { "name": "2", "designator": "2", "x": 20, "y": 0, "length": 10, "orientation": "right" },
                    ],
                    "parameters": [{ "name": "Value", "value": "10k" }],
                    "footprints": [{ "name": "CHIP_0402", "library_path": "parts.PcbLib" }],
                }],
            }));
            assert!(!w.is_error, "{}", get_result_text(&w));
            let wp = parse_result_json(&w);
            assert_eq!(wp["symbol_count"], 1);
            // geometry echo: left pin tip = x - length.
            assert_eq!(wp["geometry"][0]["pins"][0]["tip"]["x"], -30);
            assert_eq!(wp["geometry"][0]["bounding_box"]["min_x"], -30);

            let r = server.call_read_schlib(&json!({ "filepath": path.to_string_lossy() }));
            let p = parse_result_json(&r);
            assert_eq!(p["units"], "schematic units (10 = 1 grid)");
            let sym = &p["symbols"][0];
            assert_eq!(sym["parameters"].as_array().unwrap().len(), 1);
            assert_eq!(sym["footprints"].as_array().unwrap().len(), 1);
        }

        #[test]
        fn write_schlib_component_type_sets_designator_prefix() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Cap.SchLib");
            server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(),
                "symbols": [{
                    "name": "C1",
                    "component_type": "capacitor",
                    "part_count": 2,
                    "pins": [{ "name": "1", "designator": "1", "x": -20, "y": 0, "length": 10, "orientation": "left" }],
                }],
            }));
            let r = server.call_read_schlib(&json!({ "filepath": path.to_string_lossy() }));
            let sym = &parse_result_json(&r)["symbols"][0];
            assert_eq!(sym["designator"], "C?");
            assert_eq!(sym["part_count"], 2);
        }

        // ---- write_libpkg (fully uncovered) ----

        #[test]
        fn write_libpkg_success_and_errors() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Proj.LibPkg");
            let r = server.call_write_libpkg(&json!({
                "filepath": path.to_string_lossy(),
                "documents": ["Symbols.SchLib", "Footprints.PcbLib"],
            }));
            assert!(!r.is_error, "{}", get_result_text(&r));
            let p = parse_result_json(&r);
            assert_eq!(p["status"], "success");
            assert_eq!(p["count"], 2);
            assert!(p["note"].as_str().unwrap().contains("Compile"));

            // Wrong extension and empty documents are errors.
            let bad_ext = dir.path().join("x.txt");
            assert!(
                server
                    .call_write_libpkg(
                        &json!({ "filepath": bad_ext.to_string_lossy(), "documents": ["a.SchLib"] })
                    )
                    .is_error
            );
            assert!(
                server
                    .call_write_libpkg(
                        &json!({ "filepath": path.to_string_lossy(), "documents": [] })
                    )
                    .is_error
            );
        }

        // ---- list_components metadata + pagination ----

        #[test]
        fn list_components_pcblib_metadata_and_pagination() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("L.PcbLib");
            create_test_pcblib(&path);

            let meta = server.call_list_components(&json!({
                "filepath": path.to_string_lossy(), "include_metadata": true,
            }));
            let p = parse_result_json(&meta);
            assert_eq!(p["file_type"], "PcbLib");
            assert_eq!(p["include_metadata"], true);
            assert_eq!(p["components"][0]["pad_count"], 2);
            assert_eq!(p["components"][0]["has_3d_model"], false);

            let paged = server.call_list_components(&json!({
                "filepath": path.to_string_lossy(), "limit": 1, "offset": 0,
            }));
            let p = parse_result_json(&paged);
            assert_eq!(p["returned_count"], 1);
            assert_eq!(p["has_more"], true);
        }

        #[test]
        fn list_components_schlib_metadata() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("L.SchLib");
            create_test_schlib(&path);
            let r = server.call_list_components(&json!({
                "filepath": path.to_string_lossy(), "include_metadata": true,
            }));
            let p = parse_result_json(&r);
            assert_eq!(p["file_type"], "SchLib");
            assert_eq!(p["components"][0]["pin_count"], 2);
        }

        // ---- extract_style statistic branches ----

        #[test]
        fn extract_style_pcblib_text_heights_non_null() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("T.PcbLib");
            // write_pcblib auto-injects a .Designator text (height 1.0) per footprint.
            server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [{ "name": "F", "pads": [{ "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 }] }],
            }));
            let r = server.call_extract_style(&json!({ "filepath": path.to_string_lossy() }));
            let p = parse_result_json(&r);
            assert!(p["style"]["text_heights"]["count"].as_u64().unwrap() >= 1);
        }

        #[test]
        fn extract_style_pcblib_pad_shape_distribution() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Shapes.PcbLib");
            let mut lib = PcbLib::new();
            let mut fp = Footprint::new("MIX");
            fp.add_pad(Pad::smd("1", -1.0, 0.0, 0.6, 0.5)); // RoundedRectangle
            fp.add_pad(Pad::through_hole("2", 1.0, 0.0, 0.8, 0.8, 0.4)); // Round, Multi-Layer
            lib.add(fp);
            lib.save(&path).unwrap();

            let r = server.call_extract_style(&json!({ "filepath": path.to_string_lossy() }));
            let p = parse_result_json(&r);
            let shapes = &p["style"]["pad_shapes"];
            assert_eq!(shapes["RoundedRectangle"], 1);
            assert_eq!(shapes["Round"], 1);
            assert!(p["style"]["layers_used"].get("Multi-Layer").is_some());
        }

        #[test]
        fn extract_style_schlib_unfilled_rect_and_lines() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("S.SchLib");
            server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(),
                "symbols": [{
                    "name": "S",
                    "pins": [{ "name": "1", "designator": "1", "x": -20, "y": 0, "length": 10, "orientation": "left" }],
                    "rectangles": [{ "x1": -10, "y1": -5, "x2": 10, "y2": 5, "filled": false }],
                    "lines": [{ "x1": -5, "y1": 0, "x2": 5, "y2": 0, "line_width": 1 }],
                }],
            }));
            let r = server.call_extract_style(&json!({ "filepath": path.to_string_lossy() }));
            let p = parse_result_json(&r);
            assert_eq!(p["style"]["rectangles"]["unfilled_count"], 1);
            assert_eq!(p["style"]["rectangles"]["filled_count"], 0);
            assert!(p["style"]["line_widths"]["count"].as_u64().unwrap() >= 1);
        }
    }

    // ==================== rejection and failure paths ====================
    //
    // Every handler in this file answers a bad request by returning a
    // `ToolCallResult::error` rather than by panicking, so the rejection is the
    // contract and needs a test each. Grouped by handler, in call order.

    mod failure_paths {
        use crate::mcp::tools::test_support::{
            create_test_server, get_result_text, parse_result_json, test_temp_dir,
        };
        use serde_json::{json, Value};

        /// The minimal pad payload a footprint needs to be writable, so each
        /// test can vary exactly one field away from a known-good request.
        fn pad(designator: &str) -> Value {
            json!({ "designator": designator, "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 })
        }

        /// A footprint carrying one valid pad.
        fn footprint(name: &str) -> Value {
            json!({ "name": name, "pads": [pad("1")] })
        }

        /// A symbol carrying one valid pin.
        fn symbol(name: &str) -> Value {
            json!({
                "name": name,
                "pins": [{
                    "name": "1", "designator": "1",
                    "x": -20, "y": 0, "length": 10, "orientation": "left",
                }],
            })
        }

        /// Writes bytes that are not an OLE compound file, so `open` fails.
        fn write_garbage(path: &std::path::Path) {
            std::fs::write(path, b"not an OLE compound document").unwrap();
        }

        /// Flips a file's read-only bit, used to make a save fail without
        /// depending on the caller running unprivileged.
        /// Fails the library's next save — and ONLY the save — by occupying
        /// the deterministic temp path `save_atomic` must create beside the
        /// target (`<name>.pcblib.tmp` / `<name>.schlib.tmp`) with a
        /// directory: `File::create` over a directory fails on every platform,
        /// while the `.bak` backup (a plain copy) is untouched. Same mechanism
        /// as `BlockedSave` in `library_ops.rs`. Permissions cannot do this
        /// portably: a read-only FILE only blocks the rename-over on Windows
        /// (on Unix that permission belongs to the parent directory), and a
        /// read-only DIRECTORY fails the backup before the save is reached.
        fn block_save(path: &std::path::Path, blocked: bool) {
            let tmp_ext = if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("schlib"))
            {
                "schlib.tmp"
            } else {
                "pcblib.tmp"
            };
            let tmp = path.with_extension(tmp_ext);
            if blocked {
                std::fs::create_dir(&tmp).expect("occupy the save temp path");
            } else {
                let _ = std::fs::remove_dir(&tmp);
            }
        }

        /// Asserts the call failed and its message mentions `needle`.
        fn assert_error_mentions(result: &crate::mcp::server::ToolCallResult, needle: &str) {
            let text = get_result_text(result);
            assert!(result.is_error, "expected an error, got: {text}");
            assert!(
                text.contains(needle),
                "expected the error to mention {needle:?}, got: {text}"
            );
        }

        // ---- geometry helpers -------------------------------------------------

        #[test]
        fn segment_rect_misses_when_the_whole_segment_is_outside_the_slab() {
            use super::super::segment_intersects_rect;
            // Points away from the rect along +x while lying entirely to its
            // left: the entering parameter overshoots the exit, which is the
            // `t > u2` rejection rather than the `t < u1` one the other
            // direction takes.
            assert!(!segment_intersects_rect(
                -5.0, 0.0, -3.0, 0.0, -1.0, -1.0, 1.0, 1.0
            ));
            // Mirror case in -y, so the vertical slab takes the same branch.
            assert!(!segment_intersects_rect(
                0.0, 5.0, 0.0, 3.0, -1.0, -1.0, 1.0, 1.0
            ));
        }

        #[test]
        fn silk_warning_follows_the_side_the_track_is_on() {
            use super::super::silk_over_pad_warnings;
            use crate::altium::pcblib::{Footprint, Layer, Pad, Track};

            // Bottom overlay silk over a bottom-layer pad: reported.
            let mut hit = Footprint::new("BOT");
            let mut bottom_pad = Pad::smd("1", 0.0, 0.0, 1.0, 1.0);
            bottom_pad.layer = Layer::BottomLayer;
            hit.add_pad(bottom_pad);
            hit.add_track(Track::new(-2.0, 0.0, 2.0, 0.0, 0.2, Layer::BottomOverlay));
            let warnings = silk_over_pad_warnings(&hit);
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert_eq!(warnings[0]["layer"], "Bottom Overlay");

            // Same silk, but the pad is top-only: opposite sides never clash,
            // so the pad is skipped even though the geometry overlaps.
            let mut miss = Footprint::new("TOP");
            let mut top_pad = Pad::smd("1", 0.0, 0.0, 1.0, 1.0);
            top_pad.layer = Layer::TopLayer;
            miss.add_pad(top_pad);
            miss.add_track(Track::new(-2.0, 0.0, 2.0, 0.0, 0.2, Layer::BottomOverlay));
            assert!(silk_over_pad_warnings(&miss).is_empty());
        }

        #[test]
        fn pad_overlap_warnings_report_pairs_and_cap_the_list() {
            use super::super::pad_copper_overlap_warnings;
            use crate::altium::pcblib::{Footprint, Pad, MAX_REPORTED_PAD_OVERLAPS};

            // Two overlapping pads: one warning naming both designators.
            let mut two = Footprint::new("TWO");
            two.add_pad(Pad::smd("1", 0.0, 0.0, 1.0, 1.0));
            two.add_pad(Pad::smd("2", 0.2, 0.0, 1.0, 1.0));
            let warnings = pad_copper_overlap_warnings(&two);
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert_eq!(warnings[0]["type"], "pad_copper_overlap");
            assert_eq!(warnings[0]["pads"], json!(["1", "2"]));

            // Overlapping pairs are quadratic in pad count: 8 stacked pads make
            // 28 pairs, which must truncate to the cap plus one summary line
            // carrying the true total.
            let mut many = Footprint::new("MANY");
            for i in 0..8 {
                many.add_pad(Pad::smd(format!("{i}"), 0.0, 0.0, 1.0, 1.0));
            }
            let warnings = pad_copper_overlap_warnings(&many);
            assert_eq!(warnings.len(), MAX_REPORTED_PAD_OVERLAPS + 1);
            let summary = warnings.last().unwrap()["message"].as_str().unwrap();
            assert!(
                summary.starts_with("28 overlapping pad pairs total"),
                "{summary}"
            );
        }

        // ---- write_pcblib -----------------------------------------------------

        #[test]
        fn write_pcblib_rejects_a_duplicate_name_within_one_request() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let r = server.call_write_pcblib(&json!({
                "filepath": dir.path().join("Dup.PcbLib").to_string_lossy(),
                "footprints": [footprint("SAME"), footprint("SAME")],
            }));
            assert_error_mentions(&r, "Duplicate footprint name");
        }

        #[test]
        fn write_pcblib_rejects_empty_and_invalid_names() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Names.PcbLib");

            let empty = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [footprint("")],
            }));
            assert_error_mentions(&empty, "cannot be empty");

            let invalid = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [footprint("BAD/NAME")],
            }));
            assert_error_mentions(&invalid, "invalid character");
        }

        #[test]
        fn write_pcblib_append_reports_an_unreadable_existing_library() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Corrupt.PcbLib");
            write_garbage(&path);
            let r = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "append": true,
                "footprints": [footprint("A")],
            }));
            assert_error_mentions(&r, "Failed to read existing library");
        }

        #[test]
        fn write_pcblib_append_rejects_a_name_already_in_the_library() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Existing.PcbLib");
            server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [footprint("A")],
            }));
            let r = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "append": true,
                "footprints": [footprint("A")],
            }));
            assert_error_mentions(&r, "already exists in the library");
        }

        #[test]
        fn write_pcblib_reports_which_primitive_failed_to_parse() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Parse.PcbLib");

            // One case per primitive family, each malformed in its own way, so
            // the index and family named in `details` are both exercised.
            let cases: [(&str, Value, &str); 5] = [
                (
                    "pads",
                    json!([{ "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0 }]),
                    "pad at index 0",
                ),
                (
                    "tracks",
                    json!([{ "y1": 0.0, "x2": 1.0, "y2": 0.0, "width": 0.2 }]),
                    "track at index 0",
                ),
                (
                    "vias",
                    json!([{ "x": 0.0, "y": 0.0, "diameter": 0.0, "hole_size": 0.3 }]),
                    "via at index 0",
                ),
                (
                    "fills",
                    json!([{ "y1": 0.0, "x2": 1.0, "y2": 1.0 }]),
                    "fill at index 0",
                ),
                (
                    "arcs",
                    json!([{ "x": 0.0, "y": 0.0, "radius": 1.0, "start_angle": 0.0, "end_angle": 90.0 }]),
                    "arc at index 0",
                ),
            ];

            for (key, payload, expected) in cases {
                let mut fp = json!({ "name": "FP", "pads": [pad("1")] });
                fp[key] = payload;
                let r = server.call_write_pcblib(&json!({
                    "filepath": path.to_string_lossy(),
                    "footprints": [fp],
                }));
                assert_error_mentions(&r, expected);
            }
        }

        #[test]
        fn write_pcblib_gates_embedded_step_models_against_the_allowlist() {
            // The embed source is read from disk at save time, so a path
            // outside the allow-list would be an arbitrary-file read.
            let allowed = test_temp_dir();
            let outside = test_temp_dir();
            let server = create_test_server(allowed.path());
            let model = outside.path().join("secret.step");
            std::fs::write(&model, b"ISO-10303-21;\n").unwrap();

            let r = server.call_write_pcblib(&json!({
                "filepath": allowed.path().join("Gated.PcbLib").to_string_lossy(),
                "footprints": [{
                    "name": "FP",
                    "pads": [pad("1")],
                    "step_model": { "filepath": model.to_string_lossy(), "embed": true },
                }],
            }));
            assert!(r.is_error, "{}", get_result_text(&r));
        }

        #[test]
        fn write_pcblib_embeds_a_permitted_step_model_and_keeps_external_refs() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let model = dir.path().join("body.step");
            std::fs::write(
                &model,
                b"ISO-10303-21;\nHEADER;\nENDSEC;\nEND-ISO-10303-21;\n",
            )
            .unwrap();

            // embed = true takes the Model3D path and reports step-embedded.
            let embedded = server.call_write_pcblib(&json!({
                "filepath": dir.path().join("Embed.PcbLib").to_string_lossy(),
                "footprints": [{
                    "name": "FP",
                    "pads": [pad("1")],
                    "step_model": {
                        "filepath": model.to_string_lossy(), "embed": true,
                        "x_offset": 1.0, "y_offset": 2.0, "z_offset": 3.0, "rotation": 90.0,
                    },
                }],
            }));
            assert!(!embedded.is_error, "{}", get_result_text(&embedded));
            assert_eq!(
                parse_result_json(&embedded)["bodies"][0]["source"],
                "step-embedded"
            );

            // embed = false stores a bare reference and never reads the file,
            // so it is not gated and reports step-external.
            let external = server.call_write_pcblib(&json!({
                "filepath": dir.path().join("External.PcbLib").to_string_lossy(),
                "footprints": [{
                    "name": "FP",
                    "pads": [pad("1")],
                    "step_model": {
                        "filepath": "3D_Models/elsewhere.step", "embed": false,
                        "rotation": 45.0, "z_offset": 1.5,
                    },
                }],
            }));
            assert!(!external.is_error, "{}", get_result_text(&external));
            let body = &parse_result_json(&external)["bodies"][0];
            assert_eq!(body["source"], "step-external");
            assert_eq!(body["model"], "3D_Models/elsewhere.step");
        }

        #[test]
        fn write_pcblib_gates_model_3d_only_when_it_names_a_real_file() {
            let allowed = test_temp_dir();
            let outside = test_temp_dir();
            let server = create_test_server(allowed.path());
            let model = outside.path().join("outside.step");
            std::fs::write(&model, b"ISO-10303-21;\n").unwrap();

            // An existing file outside the allow-list is refused...
            let gated = server.call_write_pcblib(&json!({
                "filepath": allowed.path().join("M1.PcbLib").to_string_lossy(),
                "footprints": [{
                    "name": "FP",
                    "pads": [pad("1")],
                    "model_3d": { "filepath": model.to_string_lossy() },
                }],
            }));
            assert!(gated.is_error, "{}", get_result_text(&gated));

            // ...while the same key pointing inside the allow-list is accepted
            // and lands on the footprint, so a read -> write replay keeps its
            // model instead of dropping it.
            let permitted = allowed.path().join("inside.step");
            std::fs::write(&permitted, b"ISO-10303-21;\n").unwrap();
            let replayed = server.call_write_pcblib(&json!({
                "filepath": allowed.path().join("M2.PcbLib").to_string_lossy(),
                "footprints": [{
                    "name": "FP",
                    "pads": [pad("1")],
                    "model_3d": { "filepath": permitted.to_string_lossy(), "z_offset": 0.5 },
                }],
            }));
            assert!(!replayed.is_error, "{}", get_result_text(&replayed));
            assert_eq!(
                parse_result_json(&replayed)["bodies"][0]["source"],
                "step-embedded"
            );
        }

        #[test]
        fn write_pcblib_rejects_out_of_range_coordinates() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let r = server.call_write_pcblib(&json!({
                "filepath": dir.path().join("Far.PcbLib").to_string_lossy(),
                "footprints": [{
                    "name": "FP",
                    "pads": [{ "designator": "1", "x": 99_999.0, "y": 0.0, "width": 1.0, "height": 1.0 }],
                }],
            }));
            assert_error_mentions(&r, "exceeds the maximum safe range");
        }

        #[test]
        fn write_pcblib_reports_backup_and_save_failures() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            // A directory sitting where the library should be: it exists, so a
            // backup is attempted, and copying a directory fails.
            let as_dir = dir.path().join("Blocked.PcbLib");
            std::fs::create_dir(&as_dir).unwrap();
            let backup = server.call_write_pcblib(&json!({
                "filepath": as_dir.to_string_lossy(),
                "footprints": [footprint("A")],
            }));
            assert_error_mentions(&backup, "backup");

            // With the save temp path blocked, the backup still succeeds —
            // it is a plain copy — so the save is what fails, and the failure
            // is reported as a structured result rather than a panic.
            let locked = dir.path().join("ReadOnly.PcbLib");
            server.call_write_pcblib(&json!({
                "filepath": locked.to_string_lossy(),
                "footprints": [footprint("A")],
            }));
            block_save(&locked, true);
            let save = server.call_write_pcblib(&json!({
                "filepath": locked.to_string_lossy(),
                "footprints": [footprint("B")],
            }));
            block_save(&locked, false); // frees the squatted temp path
            assert!(save.is_error, "{}", get_result_text(&save));
            assert_eq!(parse_result_json(&save)["status"], "error");
        }

        // ---- read_pcblib / read_schlib ---------------------------------------

        #[test]
        fn read_pcblib_compact_downgrades_a_uniform_full_stack_pad() {
            // A FullStack pad whose per-layer values all match the primary pair
            // carries no information: compact mode strips the arrays and
            // reports the pad as simple. The reader always materialises all 32
            // layers, so every one of them has to match or the pad is genuinely
            // non-uniform and must keep its stack.
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Stack.PcbLib");
            let uniform: Vec<Value> = (0..32)
                .map(|_| json!({ "width": 1.0, "height": 1.0 }))
                .collect();
            let written = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [{
                    "name": "FP",
                    "pads": [{
                        "designator": "1", "x": 0.0, "y": 0.0, "width": 1.0, "height": 1.0,
                        "stack_mode": "full_stack",
                        "per_layer_sizes": uniform,
                    }],
                }],
            }));
            assert!(!written.is_error, "{}", get_result_text(&written));

            let r = server.call_read_pcblib(&json!({
                "filepath": path.to_string_lossy(), "compact": true,
            }));
            let pad_json = &parse_result_json(&r)["footprints"][0]["pads"][0];
            assert_eq!(pad_json["stack_mode"], "simple");
            assert!(pad_json.get("per_layer_sizes").is_none());
        }

        #[test]
        fn read_schlib_single_component_fetch_reports_no_more_pages() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Two.SchLib");
            server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(),
                "symbols": [symbol("A"), symbol("B")],
            }));
            let r = server.call_read_schlib(&json!({
                "filepath": path.to_string_lossy(), "component_name": "A",
            }));
            let p = parse_result_json(&r);
            assert_eq!(p["returned_count"], 1);
            assert_eq!(p["total_count"], 2);
            // Filtering is not pagination, so there is never a next page.
            assert_eq!(p["has_more"], false);
        }

        #[test]
        fn read_schlib_reports_an_unreadable_library() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Corrupt.SchLib");
            write_garbage(&path);
            let r = server.call_read_schlib(&json!({ "filepath": path.to_string_lossy() }));
            assert!(r.is_error, "{}", get_result_text(&r));
            assert_eq!(parse_result_json(&r)["status"], "error");
        }

        // ---- write_schlib -----------------------------------------------------

        #[test]
        fn write_schlib_rejects_a_path_outside_the_allowlist() {
            let allowed = test_temp_dir();
            let outside = test_temp_dir();
            let server = create_test_server(allowed.path());
            let r = server.call_write_schlib(&json!({
                "filepath": outside.path().join("Escape.SchLib").to_string_lossy(),
                "symbols": [symbol("A")],
            }));
            assert!(r.is_error, "{}", get_result_text(&r));
        }

        #[test]
        fn write_schlib_rejects_duplicate_empty_and_invalid_names() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Names.SchLib");

            let dup = server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(),
                "symbols": [symbol("SAME"), symbol("SAME")],
            }));
            assert_error_mentions(&dup, "Duplicate symbol name");

            let empty = server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(),
                "symbols": [symbol("")],
            }));
            assert_error_mentions(&empty, "cannot be empty");

            let invalid = server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(),
                "symbols": [symbol("BAD|NAME")],
            }));
            assert_error_mentions(&invalid, "invalid character");
        }

        #[test]
        fn write_schlib_append_reports_an_unreadable_existing_library() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Corrupt.SchLib");
            write_garbage(&path);
            let r = server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(),
                "append": true,
                "symbols": [symbol("A")],
            }));
            assert_error_mentions(&r, "Failed to read existing library");
        }

        #[test]
        fn write_schlib_append_rejects_a_name_already_in_the_library() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Existing.SchLib");
            server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(), "symbols": [symbol("A")],
            }));
            let r = server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(), "append": true, "symbols": [symbol("A")],
            }));
            assert_error_mentions(&r, "already exists in the library");
        }

        #[test]
        fn write_schlib_keeps_the_supplied_designator_placement_and_identity() {
            // A read-modify-write replays these three fields, so they must
            // survive rather than reset to the AD24 defaults.
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Desig.SchLib");
            let mut sym = symbol("A");
            sym["designator_x"] = json!(-12.0);
            sym["designator_y"] = json!(18.0);
            sym["designator_unique_id"] = json!("ABCDEFGH");
            let w = server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(), "symbols": [sym],
            }));
            assert!(!w.is_error, "{}", get_result_text(&w));

            let r = server.call_read_schlib(&json!({ "filepath": path.to_string_lossy() }));
            let s = &parse_result_json(&r)["symbols"][0];
            assert_eq!(s["designator_x"], -12.0);
            assert_eq!(s["designator_y"], 18.0);
        }

        #[test]
        fn write_schlib_records_a_footprint_library_path() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Linked.SchLib");
            let mut sym = symbol("A");
            sym["footprints"] = json!([{
                "name": "CHIP_0402",
                "description": "0402 chip",
                "library_path": "Parts.PcbLib",
            }]);
            let w = server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(), "symbols": [sym],
            }));
            assert!(!w.is_error, "{}", get_result_text(&w));

            let r = server.call_read_schlib(&json!({ "filepath": path.to_string_lossy() }));
            let fp = &parse_result_json(&r)["symbols"][0]["footprints"][0];
            assert_eq!(fp["name"], "CHIP_0402");
        }

        #[test]
        fn write_schlib_rejects_out_of_range_coordinates() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let r = server.call_write_schlib(&json!({
                "filepath": dir.path().join("Far.SchLib").to_string_lossy(),
                "symbols": [{
                    "name": "A",
                    "pins": [{
                        "name": "1", "designator": "1",
                        "x": 999_999, "y": 0, "length": 10, "orientation": "left",
                    }],
                }],
            }));
            assert_error_mentions(&r, "exceeds the maximum safe range");
        }

        #[test]
        fn write_schlib_reports_backup_and_save_failures() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let as_dir = dir.path().join("Blocked.SchLib");
            std::fs::create_dir(&as_dir).unwrap();
            let backup = server.call_write_schlib(&json!({
                "filepath": as_dir.to_string_lossy(), "symbols": [symbol("A")],
            }));
            assert_error_mentions(&backup, "backup");

            let locked = dir.path().join("ReadOnly.SchLib");
            server.call_write_schlib(&json!({
                "filepath": locked.to_string_lossy(), "symbols": [symbol("A")],
            }));
            block_save(&locked, true);
            let save = server.call_write_schlib(&json!({
                "filepath": locked.to_string_lossy(), "symbols": [symbol("B")],
            }));
            block_save(&locked, false); // frees the squatted temp path
            assert!(save.is_error, "{}", get_result_text(&save));
            assert_eq!(parse_result_json(&save)["status"], "error");
        }

        // ---- write_libpkg -----------------------------------------------------

        #[test]
        fn write_libpkg_rejects_bad_requests() {
            let allowed = test_temp_dir();
            let outside = test_temp_dir();
            let server = create_test_server(allowed.path());

            let escaped = server.call_write_libpkg(&json!({
                "filepath": outside.path().join("P.LibPkg").to_string_lossy(),
                "documents": ["A.SchLib"],
            }));
            assert!(escaped.is_error, "{}", get_result_text(&escaped));

            let no_docs = server.call_write_libpkg(&json!({
                "filepath": allowed.path().join("P.LibPkg").to_string_lossy(),
            }));
            assert_error_mentions(&no_docs, "Missing required parameter: documents");

            // Present but carrying nothing usable: the array exists, so the
            // emptiness check is what rejects it.
            let empty_docs = server.call_write_libpkg(&json!({
                "filepath": allowed.path().join("P.LibPkg").to_string_lossy(),
                "documents": [],
            }));
            assert_error_mentions(&empty_docs, "at least one");
        }

        #[test]
        fn write_libpkg_reports_a_failed_write() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            // A directory occupying the target path: the extension check passes
            // and the write is what fails.
            let as_dir = dir.path().join("Blocked.LibPkg");
            std::fs::create_dir(&as_dir).unwrap();
            let r = server.call_write_libpkg(&json!({
                "filepath": as_dir.to_string_lossy(),
                "documents": ["A.SchLib"],
            }));
            assert_error_mentions(&r, "Failed to write LibPkg");
        }

        // ---- list_components / extract_style ---------------------------------

        #[test]
        fn list_components_reports_an_unreadable_schlib() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Corrupt.SchLib");
            write_garbage(&path);
            let r = server.call_list_components(&json!({ "filepath": path.to_string_lossy() }));
            assert!(r.is_error, "{}", get_result_text(&r));
            assert_eq!(parse_result_json(&r)["status"], "error");
        }

        #[test]
        fn extract_style_rejects_a_path_outside_the_allowlist() {
            let allowed = test_temp_dir();
            let outside = test_temp_dir();
            let server = create_test_server(allowed.path());
            let r = server.call_extract_style(&json!({
                "filepath": outside.path().join("X.PcbLib").to_string_lossy(),
            }));
            assert!(r.is_error, "{}", get_result_text(&r));
        }

        #[test]
        fn extract_style_pcblib_counts_the_layer_a_region_sits_on() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Region.PcbLib");
            let w = server.call_write_pcblib(&json!({
                "filepath": path.to_string_lossy(),
                "footprints": [{
                    "name": "FP",
                    "pads": [pad("1")],
                    "regions": [{
                        "layer": "Mechanical 1",
                        "vertices": [
                            { "x": -1.0, "y": -1.0 }, { "x": 1.0, "y": -1.0 },
                            { "x": 1.0, "y": 1.0 }, { "x": -1.0, "y": 1.0 },
                        ],
                    }],
                }],
            }));
            assert!(!w.is_error, "{}", get_result_text(&w));

            let r = server.call_extract_style(&json!({ "filepath": path.to_string_lossy() }));
            let p = parse_result_json(&r);
            assert!(
                p["style"]["layers_used"].get("Mechanical 1").is_some(),
                "region layer missing from the tally: {p}"
            );
        }

        #[test]
        fn extract_style_schlib_reports_null_stats_for_a_bare_symbol() {
            // A symbol with no pins and no lines has nothing to average, and the
            // stats read null rather than a zero-count block.
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Bare.SchLib");
            let w = server.call_write_schlib(&json!({
                "filepath": path.to_string_lossy(),
                "symbols": [{ "name": "BARE" }],
            }));
            assert!(!w.is_error, "{}", get_result_text(&w));

            let r = server.call_extract_style(&json!({ "filepath": path.to_string_lossy() }));
            let p = parse_result_json(&r);
            assert!(p["style"]["pin_lengths"].is_null(), "{p}");
            assert!(p["style"]["line_widths"].is_null(), "{p}");
        }

        #[test]
        fn extract_style_reports_an_unreadable_schlib() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Corrupt.SchLib");
            write_garbage(&path);
            let r = server.call_extract_style(&json!({ "filepath": path.to_string_lossy() }));
            assert!(r.is_error, "{}", get_result_text(&r));
            assert_eq!(parse_result_json(&r)["status"], "error");
        }
    }

    #[test]
    fn reading_a_schlib_outside_the_allowed_directories_is_refused() {
        use crate::mcp::tools::test_support::{
            create_test_schlib, create_test_server, get_result_text, test_temp_dir,
        };
        use serde_json::json;

        let dir = test_temp_dir();
        let other = test_temp_dir();
        let server = create_test_server(dir.path());

        let outside = other.path().join("Outside.SchLib");
        create_test_schlib(&outside);

        let r = server.call_read_schlib(&json!({ "filepath": outside.to_string_lossy() }));
        assert!(r.is_error);
        assert!(
            get_result_text(&r).contains("Access denied"),
            "{}",
            get_result_text(&r)
        );
    }
}
