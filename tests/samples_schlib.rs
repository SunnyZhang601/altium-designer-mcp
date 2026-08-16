//! Sample-library tests for `SchLib`.
//!
//! Unlike the round-trip tests in `file_io_roundtrip.rs` (which write a library
//! with our own writer and read it back), these tests open a *real*,
//! Altium-authored sample library from `scripts/samples/` with our reader and
//! assert the parsed values against the file's authored intent. This is the
//! reference pattern for the rest of the `samples_*` test files.

use altium_designer_mcp::altium::schlib::{
    Ellipse, Label, Parameter, Pin, PinElectricalType, PinOrientation, PinSymbol, Polygon,
    Rectangle, RoundRect, SchLib, ShapeDisplayFlags, Symbol, TextJustification,
};
use std::path::PathBuf;

/// Resolves a sample fixture by name under `scripts/samples/`.
fn sample(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("samples")
        .join(name)
}

#[test]
fn samples_exist() {
    let path = sample("symbols.SchLib");
    assert!(
        path.exists(),
        "missing sample fixture: {} — the samples_schlib tests read a real \
         Altium-authored library that must be present on disk",
        path.display()
    );
}

/// Compares two angles (degrees) within a tolerance. Arc angles are stored as
/// `f64`, so they are compared approximately rather than bit-for-bit.
fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// Looks up a pin by designator within a symbol, panicking with context if
/// it is absent. Sample tests match primitives by stable fields, never index.
fn pin_by_designator<'a>(symbol: &'a Symbol, designator: &str) -> &'a Pin {
    symbol
        .pins
        .iter()
        .find(|p| p.designator == designator)
        .unwrap_or_else(|| panic!("{}: no pin with designator {designator:?}", symbol.name))
}

#[test]
fn samples_schlib_structure() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");

    // Fifteen per-primitive-family symbols plus the coverage-enrichment symbols
    // (SHAPESTYLE, SHAPESTYLE2, SHAPECOLOR, LOCKFLAGS, JUSTIFY, FRACPINS,
    // BEZIERSYM, PIESYM, IMAGESYM, TEXTFRAMESYM, EMBIMGSYM, SWAPPIN, FRACSHAPES,
    // DISPMODE) added to GenerateSamples.pas and regenerated on-site.
    assert_eq!(lib.len(), 31, "expected exactly thirty-one symbols");

    let names = lib.names();
    for expected in [
        "PINS_ETYPE",
        "PINS_ORIENT",
        "PINS_VIS",
        "PINS_DECOR",
        "LINES",
        "ARCS",
        "LABELS",
        "PARAMS",
        "DUALPART",
        "RECTS",
        "ELLIPSES",
        "POLYLINES",
        "ROUNDRECTS",
        "POLYGONS",
        "EDGE",
        "SHAPESTYLE",
        "LOCKFLAGS",
        "JUSTIFY",
        "FRACPINS",
        "BEZIERSYM",
        "PIESYM",
        "IMAGESYM",
        "TEXTFRAMESYM",
        "EMBIMGSYM",
        "SWAPPIN",
        "FRACSHAPES",
        "DISPMODE",
        "SHAPECOLOR",
        "SHAPESTYLE2",
        "LOCKFLAGS2",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing symbol {expected:?}; got {names:?}",
        );
    }
}

#[test]
fn samples_schlib_edge() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");

    let symbol = lib.get("EDGE").expect("symbol EDGE not found");
    assert_eq!(symbol.name, "EDGE");
    assert_eq!(symbol.pins.len(), 3, "EDGE has 3 pins");

    // Boundary-case pins, matched by designator. Pins 1 and 2 push the
    // coordinate extremes (large and negative positions); pin 3 is the headline
    // case — a 35-character name that must survive the round-trip intact.
    let pin1 = pin_by_designator(symbol, "1");
    assert_eq!(pin1.name, "BIG", "pin 1 name");
    assert_eq!(pin1.x, 50, "pin 1 x");
    assert_eq!(pin1.y, 30, "pin 1 y");

    let pin2 = pin_by_designator(symbol, "2");
    assert_eq!(pin2.name, "NEG", "pin 2 name");
    assert_eq!(pin2.x, -50, "pin 2 x");
    assert_eq!(pin2.y, -30, "pin 2 y");

    let pin3 = pin_by_designator(symbol, "3");
    assert_eq!(
        pin3.name, "VERY_LONG_PIN_NAME_0123456789ABCDEF",
        "pin 3 long name survives intact",
    );
    assert_eq!(pin3.x, 0, "pin 3 x");
    assert_eq!(pin3.y, 20, "pin 3 y");
}

#[test]
fn samples_schlib_pins_etype() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");

    let symbol = lib.get("PINS_ETYPE").expect("symbol PINS_ETYPE not found");
    assert_eq!(symbol.name, "PINS_ETYPE");
    assert_eq!(symbol.part_count, 1, "PINS_ETYPE is a single-part symbol");
    assert_eq!(symbol.pins.len(), 8, "PINS_ETYPE has 8 pins");

    // Authored pins: each is oriented Left with length 20 (reader units), at
    // x = 0 and y stepping down by 10 (0, -10, -20, … -70). We assert the
    // load-bearing fields per pin; volatile identity (unique_id) is not checked.
    let expected: [(&str, &str, PinElectricalType, i32); 8] = [
        ("1", "IN", PinElectricalType::Input, 0),
        ("2", "IO", PinElectricalType::Bidirectional, -10),
        ("3", "OUT", PinElectricalType::Output, -20),
        ("4", "OC", PinElectricalType::OpenCollector, -30),
        ("5", "PAS", PinElectricalType::Passive, -40),
        ("6", "HIZ", PinElectricalType::HiZ, -50),
        ("7", "OE", PinElectricalType::OpenEmitter, -60),
        ("8", "PWR", PinElectricalType::Power, -70),
    ];

    for (i, &(designator, name, electrical_type, y)) in expected.iter().enumerate() {
        let pin = &symbol.pins[i];
        assert_eq!(pin.designator, designator, "pin[{i}] designator");
        assert_eq!(pin.name, name, "pin[{i}] name");
        assert_eq!(
            pin.electrical_type, electrical_type,
            "pin[{i}] ({designator}) electrical type",
        );
        assert_eq!(
            pin.orientation,
            PinOrientation::Left,
            "pin[{i}] ({designator}) orientation",
        );
        assert_eq!(pin.length, 20, "pin[{i}] ({designator}) length");
        assert_eq!(pin.x, 0, "pin[{i}] ({designator}) x");
        assert_eq!(pin.y, y, "pin[{i}] ({designator}) y");
        // PR-R3 pin auxiliary data. The Altium-authored golden pins are on-grid
        // with a default symbol line width and display mode, so all three read
        // back at their defaults — the byte-identity anchor for the aux streams
        // (a from-scratch default pin therefore writes no PinFrac /
        // PinSymbolLineWidth stream, keeping the storage identical).
        assert_eq!(
            pin.owner_part_display_mode, 0,
            "pin[{i}] ({designator}) OwnerPartDisplayMode reads the golden byte (0)"
        );
        assert_eq!(
            pin.symbol_line_width, 0,
            "pin[{i}] ({designator}) has default symbol line width"
        );
        assert_eq!(
            pin.frac, None,
            "pin[{i}] ({designator}) is on-grid (no PinFrac remainder)"
        );
    }

    // One Altium-default parameter (a `Comment` = "*").
    assert_eq!(symbol.parameters.len(), 1, "expected one parameter");
}

#[test]
fn samples_schlib_pins_orient() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib
        .get("PINS_ORIENT")
        .expect("symbol PINS_ORIENT not found");
    assert_eq!(symbol.pins.len(), 4, "PINS_ORIENT has 4 pins");

    // One pin per orientation, matched by designator (not index).
    let expected: [(&str, &str, PinOrientation); 4] = [
        ("1", "R", PinOrientation::Right),
        ("2", "U", PinOrientation::Up),
        ("3", "L", PinOrientation::Left),
        ("4", "D", PinOrientation::Down),
    ];
    for (designator, name, orientation) in expected {
        let pin = pin_by_designator(symbol, designator);
        assert_eq!(pin.name, name, "pin {designator} name");
        assert_eq!(pin.orientation, orientation, "pin {designator} orientation");
        assert!(pin.show_name, "pin {designator} show_name");
        assert!(pin.show_designator, "pin {designator} show_designator");
        assert_eq!(pin.owner_part_id, 1, "pin {designator} owner_part_id");
    }
}

#[test]
fn samples_schlib_pins_vis() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("PINS_VIS").expect("symbol PINS_VIS not found");
    assert_eq!(symbol.pins.len(), 4, "PINS_VIS has 4 pins");

    // (designator, name, show_name, show_designator, hidden).
    let expected: [(&str, &str, bool, bool, bool); 4] = [
        ("1", "BOTH", true, true, false),
        ("2", "NONLY", true, false, false),
        ("3", "DONLY", false, true, false),
        ("4", "HIDE", true, true, true),
    ];
    for (designator, name, show_name, show_designator, hidden) in expected {
        let pin = pin_by_designator(symbol, designator);
        assert_eq!(pin.name, name, "pin {designator} name");
        assert_eq!(pin.show_name, show_name, "pin {designator} show_name");
        assert_eq!(
            pin.show_designator, show_designator,
            "pin {designator} show_designator",
        );
        assert_eq!(pin.hidden, hidden, "pin {designator} hidden");
    }
}

#[test]
fn samples_schlib_pins_decor() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("PINS_DECOR").expect("symbol PINS_DECOR not found");
    assert_eq!(symbol.pins.len(), 4, "PINS_DECOR has 4 pins");

    // One pin per IEEE decoration slot: each sets exactly one slot, the other
    // three stay None. Confirms all four DelphiScript slot properties round-trip
    // (Symbol_InnerEdge / Symbol_OuterEdge / Symbol_Inner / Symbol_Outer).
    let expected: [(&str, &str, PinSymbol, PinSymbol, PinSymbol, PinSymbol); 4] = [
        (
            "1",
            "IECLK",
            PinSymbol::Clock,
            PinSymbol::None,
            PinSymbol::None,
            PinSymbol::None,
        ),
        (
            "2",
            "OEDOT",
            PinSymbol::None,
            PinSymbol::Dot,
            PinSymbol::None,
            PinSymbol::None,
        ),
        (
            "3",
            "INCLK",
            PinSymbol::None,
            PinSymbol::None,
            PinSymbol::Clock,
            PinSymbol::None,
        ),
        (
            "4",
            "OUTDOT",
            PinSymbol::None,
            PinSymbol::None,
            PinSymbol::None,
            PinSymbol::Dot,
        ),
    ];
    for (designator, name, inner_edge, outer_edge, inside, outside) in expected {
        let pin = pin_by_designator(symbol, designator);
        assert_eq!(pin.name, name, "pin {designator} name");
        assert_eq!(
            pin.symbol_inner_edge, inner_edge,
            "pin {designator} symbol_inner_edge"
        );
        assert_eq!(
            pin.symbol_outer_edge, outer_edge,
            "pin {designator} symbol_outer_edge"
        );
        assert_eq!(pin.symbol_inside, inside, "pin {designator} symbol_inside");
        assert_eq!(
            pin.symbol_outside, outside,
            "pin {designator} symbol_outside"
        );
    }
}

#[test]
fn samples_schlib_lines() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("LINES").expect("symbol LINES not found");
    assert_eq!(symbol.lines.len(), 3, "LINES has 3 lines");

    // Match each line by its (x1, y1, x2, y2) endpoints (reader units). Coords are
    // f64; the integer-grid sample reads back as exact whole values.
    for endpoints in [
        (0.0, 0.0, 10.0, 0.0),
        (0.0, 0.0, 0.0, 10.0),
        (0.0, 0.0, 10.0, 10.0),
    ] {
        let (x1, y1, x2, y2) = endpoints;
        assert!(
            symbol.lines.iter().any(|l| {
                (l.x1 - x1).abs() < 1e-9
                    && (l.y1 - y1).abs() < 1e-9
                    && (l.x2 - x2).abs() < 1e-9
                    && (l.y2 - y2).abs() < 1e-9
            }),
            "missing line {endpoints:?}",
        );
    }

    // The golden authors the designator record at Location.X=-5|Location.Y=5
    // with a stable UniqueID; position and identity must read back rather than
    // being re-hardcoded or regenerated on write.
    assert!(
        approx_eq(symbol.designator_x, -5.0) && approx_eq(symbol.designator_y, 5.0),
        "golden designator position must read back as (-5, 5), got ({}, {})",
        symbol.designator_x,
        symbol.designator_y
    );
    let uid = symbol
        .designator_unique_id
        .as_deref()
        .expect("golden designator UniqueID must be preserved on read");
    assert_eq!(uid.len(), 8, "designator UniqueID is an 8-char Altium id");
}

#[test]
fn samples_schlib_arcs() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("ARCS").expect("symbol ARCS not found");
    assert_eq!(symbol.arcs.len(), 2, "ARCS has 2 arcs");

    // Full circle at the origin.
    let circle = symbol
        .arcs
        .iter()
        .find(|a| approx_eq(a.x, 0.0) && approx_eq(a.y, 0.0))
        .expect("full-circle arc at origin not found");
    assert!(approx_eq(circle.radius, 5.0), "circle radius");
    assert!(approx_eq(circle.start_angle, 0.0), "circle start angle");
    assert!(approx_eq(circle.end_angle, 360.0), "circle end angle");

    // Quarter arc centred below the origin.
    let quarter = symbol
        .arcs
        .iter()
        .find(|a| approx_eq(a.x, 0.0) && approx_eq(a.y, -20.0))
        .expect("quarter arc at (0,-20) not found");
    assert!(approx_eq(quarter.radius, 5.0), "quarter-arc radius");
    assert!(
        approx_eq(quarter.start_angle, 0.0),
        "quarter-arc start angle"
    );
    assert!(approx_eq(quarter.end_angle, 90.0), "quarter-arc end angle");
}

#[test]
fn samples_schlib_labels() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("LABELS").expect("symbol LABELS not found");
    assert_eq!(symbol.labels.len(), 3, "LABELS has 3 labels");

    let by_text = |text: &str| -> &Label {
        symbol
            .labels
            .iter()
            .find(|l| l.text == text)
            .unwrap_or_else(|| panic!("label {text:?} not found"))
    };

    // Match by text; assert the authored justification (rotation is not part of
    // the contract here and is left unchecked).
    assert_eq!(
        by_text("LBL_BL").justification,
        TextJustification::BottomLeft,
        "LBL_BL justification",
    );
    assert_eq!(
        by_text("LBL_TR").justification,
        TextJustification::TopRight,
        "LBL_TR justification",
    );
    assert_eq!(
        by_text("LBL_ROT90").justification,
        TextJustification::BottomLeft,
        "LBL_ROT90 justification",
    );
}

#[test]
fn samples_schlib_params() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("PARAMS").expect("symbol PARAMS not found");

    // Every symbol also carries an Altium-default `Comment` = "*", so we locate
    // the authored parameters by (name, value) rather than asserting a count.
    let find = |name: &str, value: &str| -> &Parameter {
        symbol
            .parameters
            .iter()
            .find(|p| p.name == name && p.value == value)
            .unwrap_or_else(|| panic!("parameter {name:?} = {value:?} not found"))
    };

    let value = find("Value", "10k");
    assert!(!value.hidden, "authored Value parameter is visible");
    // The golden's parameters carry neither SHOWNAME nor HIDENAME, so both
    // name-visibility toggles read back as their omit-when-default `false`.
    // Visibility is driven solely by IsHidden.
    assert!(!value.show_name, "golden Value parameter has no ShowName");
    assert!(!value.hide_name, "golden Value parameter has no HideName");
    assert_eq!(value.orientation, 0, "golden Value parameter Orientation=0");
    assert!(
        value.description.is_empty(),
        "golden Value parameter has no Description"
    );
    assert!(
        !value.is_configurable,
        "golden Value parameter is not configurable"
    );

    let comment = find("Comment", "100nF");
    assert!(comment.hidden, "authored Comment parameter is hidden");
    assert!(
        !comment.show_name,
        "golden Comment parameter has no ShowName"
    );
    assert!(
        !comment.hide_name,
        "golden Comment parameter has no HideName"
    );
}

#[test]
fn samples_schlib_no_utf8_key_for_win1252_golden() {
    // Promotion must be reserved for values that need it: a Windows-1252
    // symbol must NOT gain a `%UTF8%` key, so the common case stays
    // byte-identical and the readability oracle sees no change. The Cyrillic
    // symbol is excluded — it is the one that legitimately requires promotion.
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    for symbol in lib.iter().filter(|s| s.name.is_ascii()) {
        let data = altium_designer_mcp::altium::schlib::writer::encode_data_stream(symbol)
            .expect("encode");
        assert!(
            !data.windows(6).any(|w| w == b"%UTF8%"),
            "symbol {:?}: golden (all Windows-1252) must not gain a %UTF8% key",
            symbol.name
        );
    }

    // And the specific text-bearing symbols round-trip their values unchanged.
    let labels = lib.get("LABELS").expect("LABELS symbol");
    assert!(
        labels.labels.iter().all(|l| l.text.is_ascii()),
        "golden labels are ASCII, so their values must read back verbatim"
    );
    let params = lib.get("PARAMS").expect("PARAMS symbol");
    assert!(
        params.parameters.iter().any(|p| p.value == "10k"),
        "golden Value parameter reads back as the plain Windows-1252 value"
    );
}

#[test]
fn samples_schlib_dualpart() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("DUALPART").expect("symbol DUALPART not found");

    assert_eq!(symbol.part_count, 2, "DUALPART is a two-part symbol");
    assert_eq!(symbol.pins.len(), 4, "DUALPART has 4 pins");

    // Pins split across the two parts by owner_part_id, matched by designator.
    let expected: [(&str, &str, i32); 4] = [
        ("1", "INA", 1),
        ("2", "OUTA", 1),
        ("3", "INB", 2),
        ("4", "OUTB", 2),
    ];
    for (designator, name, owner_part_id) in expected {
        let pin = pin_by_designator(symbol, designator);
        assert_eq!(pin.name, name, "pin {designator} name");
        assert_eq!(
            pin.owner_part_id, owner_part_id,
            "pin {designator} owner_part_id",
        );
    }
}

#[test]
fn samples_schlib_rects() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("RECTS").expect("symbol RECTS not found");
    assert_eq!(symbol.rectangles.len(), 2, "RECTS has 2 rectangles");

    // Match by left edge (x1); both share line_color 0 / fill_color 65535.
    let by_x1 = |x1: f64| -> &Rectangle {
        symbol
            .rectangles
            .iter()
            .find(|r| approx_eq(r.x1, x1))
            .unwrap_or_else(|| panic!("rectangle with x1 = {x1} not found"))
    };

    let filled = by_x1(-10.0);
    assert_eq!(
        (filled.y1, filled.x2, filled.y2),
        (0.0, 10.0, 10.0),
        "filled rect geometry"
    );
    assert!(filled.filled, "filled rect is filled");
    assert_eq!(filled.fill_color, 65535, "filled rect fill_color");
    assert_eq!(filled.line_color, 0, "filled rect line_color");

    let unfilled = by_x1(15.0);
    assert_eq!(
        (unfilled.y1, unfilled.x2, unfilled.y2),
        (0.0, 35.0, 10.0),
        "unfilled rect geometry"
    );
    assert!(!unfilled.filled, "unfilled rect is not filled");
    assert_eq!(unfilled.fill_color, 65535, "unfilled rect fill_color");
    assert_eq!(unfilled.line_color, 0, "unfilled rect line_color");
}

#[test]
fn samples_schlib_display_flags_default_on_golden_shapes() {
    // The Altium-authored golden library carries no GraphicallyLocked / Disabled
    // / Dimmed / OwnerPartDisplayMode on its graphic shapes, so the reader must
    // decode each as its default (false / 0) — the read half of the
    // omit-when-default contract. Exercises one shape per graphic family.
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let def = ShapeDisplayFlags::default();

    for r in &lib.get("RECTS").unwrap().rectangles {
        assert_eq!(r.display_flags, def, "golden rectangle flags default");
    }
    for r in &lib.get("ROUNDRECTS").unwrap().round_rects {
        assert_eq!(r.display_flags, def, "golden round_rect flags default");
    }
    for e in &lib.get("ELLIPSES").unwrap().ellipses {
        assert_eq!(e.display_flags, def, "golden ellipse flags default");
    }
    for l in &lib.get("LINES").unwrap().lines {
        assert_eq!(l.display_flags, def, "golden line flags default");
    }
    for p in &lib.get("POLYLINES").unwrap().polylines {
        assert_eq!(p.display_flags, def, "golden polyline flags default");
    }
    for p in &lib.get("POLYGONS").unwrap().polygons {
        assert_eq!(p.display_flags, def, "golden polygon flags default");
    }
    for a in &lib.get("ARCS").unwrap().arcs {
        assert_eq!(a.display_flags, def, "golden arc flags default");
    }
    for l in &lib.get("LABELS").unwrap().labels {
        assert_eq!(l.display_flags, def, "golden label flags default");
    }
    for p in &lib.get("PARAMS").unwrap().parameters {
        assert_eq!(p.display_flags, def, "golden parameter flags default");
    }
}

#[test]
fn samples_schlib_ellipses() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("ELLIPSES").expect("symbol ELLIPSES not found");
    assert_eq!(symbol.ellipses.len(), 2, "ELLIPSES has 2 ellipses");

    // Match by horizontal radius (radius_x), which is unique here.
    let by_radius_x = |radius_x: f64| -> &Ellipse {
        symbol
            .ellipses
            .iter()
            .find(|e| approx_eq(e.radius_x, radius_x))
            .unwrap_or_else(|| panic!("ellipse with radius_x = {radius_x} not found"))
    };

    let circle = by_radius_x(5.0);
    assert_eq!(
        (circle.x, circle.y, circle.radius_y),
        (0.0, 0.0, 5.0),
        "circle geometry"
    );
    assert!(circle.filled, "circle is filled");

    let ellipse = by_radius_x(8.0);
    assert_eq!(
        (ellipse.x, ellipse.y, ellipse.radius_y),
        (20.0, 0.0, 4.0),
        "ellipse geometry"
    );
    assert!(!ellipse.filled, "ellipse is not filled");

    // The golden tags every ellipse IsNotAccesible=T; the field must read back.
    for e in &symbol.ellipses {
        assert!(
            e.is_not_accessible,
            "golden ellipse must read IsNotAccesible=T"
        );
    }
}

#[test]
fn samples_schlib_polylines() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("POLYLINES").expect("symbol POLYLINES not found");
    assert_eq!(symbol.polylines.len(), 1, "POLYLINES has 1 polyline");

    let polyline = &symbol.polylines[0];
    assert_eq!(
        polyline.points,
        vec![(0.0, 0.0), (10.0, 5.0), (0.0, 10.0)],
        "polyline points",
    );

    // The golden tags every polyline IsNotAccesible=T; the field must read back.
    assert!(
        polyline.is_not_accessible,
        "golden polyline must read IsNotAccesible=T"
    );
}

#[test]
fn samples_schlib_roundrects() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("ROUNDRECTS").expect("symbol ROUNDRECTS not found");
    assert_eq!(symbol.round_rects.len(), 1, "ROUNDRECTS has 1 rounded rect");

    let rr: &RoundRect = &symbol.round_rects[0];
    assert_eq!(
        (rr.x1, rr.y1, rr.x2, rr.y2),
        (-10.0, 0.0, 10.0, 10.0),
        "round rect geometry"
    );
    assert_eq!(
        (rr.corner_x_radius, rr.corner_y_radius),
        (2.0, 2.0),
        "round rect corner radii"
    );
    assert!(rr.filled, "round rect is filled");
}

#[test]
fn samples_schlib_polygons() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let symbol = lib.get("POLYGONS").expect("symbol POLYGONS not found");
    assert_eq!(symbol.polygons.len(), 2, "POLYGONS has 2 polygons");

    // Both are 4-vertex boxes; match each by its first vertex x (unique here).
    let by_first_x = |x: f64| -> &Polygon {
        symbol
            .polygons
            .iter()
            .find(|p| p.points.first().is_some_and(|&(px, _)| approx_eq(px, x)))
            .unwrap_or_else(|| panic!("polygon with first vertex x = {x} not found"))
    };

    let left = by_first_x(-10.0);
    assert_eq!(
        left.points,
        vec![(-10.0, 0.0), (10.0, 0.0), (10.0, 10.0), (-10.0, 10.0)],
        "left polygon points",
    );
    assert!(left.filled, "left polygon is filled");

    let right = by_first_x(15.0);
    assert_eq!(
        right.points,
        vec![(15.0, 0.0), (35.0, 0.0), (35.0, 10.0), (15.0, 10.0)],
        "right polygon points",
    );
    assert!(right.filled, "right polygon is filled");
}

// ---------------------------------------------------------------------------
// Coverage-enrichment tests (docs/FIXTURE_COVERAGE.md).
//
// These assert the NON-default property values authored by the enrichment block
// in GenerateSamples.pas, read from the real Altium-regenerated fixture. This is
// the whole point of the enrichment: values a self-round-trip cannot vouch
// for (line style, transparency, non-default justification,
// off-grid PinFrac coordinates) verified against a genuine Altium file.
// ---------------------------------------------------------------------------

#[test]
fn samples_schlib_shapestyle() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("SHAPESTYLE").expect("SHAPESTYLE symbol not found");

    // Two lines, one dashed (line_style 1) and one dotted (line_style 2).
    assert_eq!(sym.lines.len(), 2, "SHAPESTYLE has two lines");
    let styles: Vec<u8> = sym.lines.iter().map(|l| l.line_style).collect();
    assert!(
        styles.contains(&1) && styles.contains(&2),
        "SHAPESTYLE must carry a dashed (1) and a dotted (2) line, got {styles:?}"
    );

    // Two rectangles: one solid-opaque, one transparent.
    assert_eq!(sym.rectangles.len(), 2, "SHAPESTYLE has two rectangles");
    assert_eq!(
        sym.rectangles.iter().filter(|r| r.transparent).count(),
        1,
        "exactly one SHAPESTYLE rectangle is transparent"
    );

    // One transparent polygon (ISch_Polygon.Transparent round-trips from Altium).
    assert_eq!(sym.polygons.len(), 1, "SHAPESTYLE has one polygon");
    assert!(sym.polygons[0].transparent, "the polygon is transparent");

    // One transparent ellipse (ISch_Ellipse.Transparent round-trips). Note:
    // RoundRectangle.Transparent is deliberately NOT authored — Altium does not
    // persist it on a library round-rect (reads back false), so it is not testable.
    assert_eq!(sym.ellipses.len(), 1, "SHAPESTYLE has one ellipse");
    assert!(sym.ellipses[0].transparent, "the ellipse is transparent");
}

#[test]
fn samples_schlib_shape_colours() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("SHAPECOLOR").expect("SHAPECOLOR symbol not found");

    // Every other symbol authors Color := $000000, which is Altium's default and
    // is therefore omitted from the record — so without this symbol no shape
    // parser's colour arm is ever exercised against a real file. SHAPECOLOR
    // carries one of each shape in a DISTINCT colour, so a mismatched read
    // cannot pass by picking up a neighbour's value.
    assert_eq!(sym.lines.len(), 1, "SHAPECOLOR has one line");
    assert_eq!(sym.lines[0].color, 255, "line is red ($0000FF)");

    assert_eq!(sym.rectangles.len(), 1, "SHAPECOLOR has one rectangle");
    assert_eq!(
        sym.rectangles[0].line_color, 65280,
        "rectangle border is green"
    );
    assert_eq!(sym.rectangles[0].fill_color, 16_776_960, "rectangle fill");
    assert!(sym.rectangles[0].filled, "rectangle is solid");

    assert_eq!(sym.round_rects.len(), 1, "SHAPECOLOR has one round rect");
    assert_eq!(
        sym.round_rects[0].line_color, 16_711_680,
        "round rect border is blue"
    );

    assert_eq!(sym.arcs.len(), 1, "SHAPECOLOR has one arc");
    assert_eq!(sym.arcs[0].color, 65535, "arc is yellow");
    // A non-zero StartAngle: every other arc in the library starts at 0, which
    // Altium omits, leaving the start-angle read path uncovered.
    assert!(approx_eq(sym.arcs[0].start_angle, 45.0), "arc start angle");
    assert!(approx_eq(sym.arcs[0].end_angle, 315.0), "arc end angle");

    assert_eq!(sym.ellipses.len(), 1, "SHAPECOLOR has one ellipse");
    assert_eq!(
        sym.ellipses[0].line_color, 16_711_935,
        "ellipse border is magenta"
    );

    assert_eq!(sym.polylines.len(), 1, "SHAPECOLOR has one polyline");
    assert_eq!(sym.polylines[0].color, 8_421_376, "polyline is teal");

    assert_eq!(sym.polygons.len(), 1, "SHAPECOLOR has one polygon");
    assert_eq!(
        sym.polygons[0].line_color, 128,
        "polygon border is dark red"
    );

    assert_eq!(sym.pies.len(), 1, "SHAPECOLOR has one pie");
    assert_eq!(sym.pies[0].line_color, 32896, "pie border is olive");
    assert_eq!(sym.pies[0].fill_color, 42495, "pie fill");

    assert_eq!(sym.beziers.len(), 1, "SHAPECOLOR has one bezier");
    assert_eq!(sym.beziers[0].color, 8_404_992, "bezier colour");
    assert_eq!(sym.beziers[0].line_width, 2, "bezier is eMedium (2)");

    let label = sym
        .labels
        .iter()
        .find(|l| l.text == "COLOURED")
        .expect("SHAPECOLOR label not found");
    assert_eq!(label.color, 4_227_327, "label is orange");
}

#[test]
fn samples_schlib_polyline_styling() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib
        .get("SHAPESTYLE2")
        .expect("SHAPESTYLE2 symbol not found");

    // A polyline carries four styling properties no other primitive family has,
    // and AD24 persists all four. Authored as dashed with an open arrow at the
    // start, a solid arrow at the end, and the large end-shape size.
    // Two polylines: this styled one, and an unstyled control that documents the
    // fill properties AD24 refuses to persist.
    assert_eq!(sym.polylines.len(), 2, "SHAPESTYLE2 has two polylines");
    let pl = sym
        .polylines
        .iter()
        .find(|p| p.start_line_shape != 0)
        .expect("the styled polyline");
    assert_eq!(pl.line_style, 1, "dashed");
    assert_eq!(pl.start_line_shape, 1, "start is an arrow");
    assert_eq!(pl.end_line_shape, 2, "end is a solid arrow");
    assert_eq!(pl.line_shape_size, 3, "end shapes are eLarge");
}

#[test]
fn samples_schlib_label_and_parameter_display_props() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib
        .get("SHAPESTYLE2")
        .expect("SHAPESTYLE2 symbol not found");

    // A mirrored label. AD24 writes `IsMirrored=T` before `UniqueID` here, and
    // *after* it on the parameter below — the orders genuinely differ.
    let label = sym
        .labels
        .iter()
        .find(|l| l.text == "MIRRORED")
        .expect("SHAPESTYLE2 label not found");
    assert!(label.is_mirrored, "the label is mirrored");

    // The parameter display properties, none of which any other symbol sets.
    let param = sym
        .parameters
        .iter()
        .find(|p| p.name == "Rating")
        .expect("SHAPESTYLE2 Rating parameter not found");
    assert_eq!(param.value, "10V", "parameter value");
    assert!(param.show_name, "the name is shown beside the value");
    assert_eq!(param.read_only_state, 1, "parameter is read-only");
    assert!(param.is_mirrored, "the parameter text is mirrored");

    // Authored `LineStyle := eLineStyleDotted` on the round rect, which AD24
    // accepts and then does not persist — the saved record carries no
    // LineStyle key at all, so it must read back as the 0 default.
    assert_eq!(sym.round_rects.len(), 1, "SHAPESTYLE2 has one round rect");
    assert_eq!(
        sym.round_rects[0].line_style, 0,
        "AD24 does not persist LineStyle on a library round rect"
    );
}

#[test]
fn samples_schlib_polyline_and_frame_fill_are_not_persisted() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib
        .get("SHAPESTYLE2")
        .expect("SHAPESTYLE2 symbol not found");

    // Both were authored with a fill and transparency and AD24 accepted both
    // without complaint, then wrote neither. Asserting the defaults keeps the
    // negative honest: if a later AD version starts writing them, this fails
    // rather than quietly passing.
    let plain = sym
        .polylines
        .iter()
        .find(|p| p.line_style == 0)
        .expect("the unstyled polyline");
    assert!(
        !plain.transparent,
        "AD24 does not persist polyline Transparent"
    );

    assert_eq!(sym.text_frames.len(), 1, "SHAPESTYLE2 has one text frame");
    let frame = &sym.text_frames[0];
    assert!(
        !frame.transparent,
        "AD24 does not persist text-frame Transparent"
    );
    // A whole-mil margin: every other frame's is sub-mil, so the record carries
    // only TextMargin_Frac and the integer key goes unread.
    assert!(
        approx_eq(frame.text_margin, 3.0),
        "whole-mil text margin, got {}",
        frame.text_margin
    );
}

#[test]
fn samples_schlib_parameter_type() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib
        .get("SHAPESTYLE2")
        .expect("SHAPESTYLE2 symbol not found");

    // eParameterType_Integer. Every other parameter in the library leaves the
    // type at its default, which Altium omits.
    let param = sym
        .parameters
        .iter()
        .find(|p| p.name == "Rating")
        .expect("Rating parameter not found");
    assert_eq!(param.param_type, 2, "eParameterType_Integer");
}

#[test]
fn samples_schlib_lockflags() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("LOCKFLAGS").expect("LOCKFLAGS symbol not found");

    // The rectangle was authored with GraphicallyLocked := True (a verified
    // ISch_GraphicalObject flag); it round-trips from the real Altium file.
    assert_eq!(sym.rectangles.len(), 1, "LOCKFLAGS has one rectangle");
    assert!(
        sym.rectangles[0].display_flags.graphically_locked,
        "the LOCKFLAGS rectangle must be graphically locked"
    );
}

#[test]
fn samples_schlib_locked_shapes() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("LOCKFLAGS2").expect("LOCKFLAGS2 symbol not found");

    // GraphicallyLocked lives on ISch_GraphicalObject, but whether AD24 WRITES an
    // inherited flag varies by record — Disabled and Dimmed are dropped from a
    // rectangle, a polyline's fill is dropped, a text frame's transparency is
    // dropped. So each shape type is authored and asserted rather than assumed
    // from the rectangle that LOCKFLAGS already covers.
    assert!(sym.lines[0].display_flags.graphically_locked, "line");
    assert!(sym.arcs[0].display_flags.graphically_locked, "arc");
    assert!(sym.ellipses[0].display_flags.graphically_locked, "ellipse");
    assert!(
        sym.round_rects[0].display_flags.graphically_locked,
        "round rect"
    );
    assert!(
        sym.polylines[0].display_flags.graphically_locked,
        "polyline"
    );
    assert!(sym.polygons[0].display_flags.graphically_locked, "polygon");
    assert!(sym.pies[0].display_flags.graphically_locked, "pie");
    assert!(sym.beziers[0].display_flags.graphically_locked, "bezier");
    let label = sym
        .labels
        .iter()
        .find(|l| l.text == "LOCKED")
        .expect("LOCKFLAGS2 label not found");
    assert!(label.display_flags.graphically_locked, "label");
}

#[test]
fn samples_schlib_justify() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("JUSTIFY").expect("JUSTIFY symbol not found");

    // Labels prove three distinct justifications round-trip from a real Altium
    // file: BottomLeft (default), MiddleCenter (authored eJustify_Center), and
    // TopRight.
    let has = |j: TextJustification| sym.labels.iter().any(|l| l.justification == j);
    assert!(
        has(TextJustification::TopRight),
        "JUSTIFY must carry a TopRight label, got {:?}",
        sym.labels
            .iter()
            .map(|l| l.justification)
            .collect::<Vec<_>>()
    );
    assert!(
        has(TextJustification::MiddleCenter),
        "JUSTIFY must carry a MiddleCenter label (authored eJustify_Center)"
    );
    assert!(
        has(TextJustification::BottomLeft),
        "JUSTIFY must carry a BottomLeft label"
    );

    // Parameter justification (the golden carries `Justification=8` on Value
    // and `Justification=4` on the hidden Tol), which must survive the read.
    let param = |name: &str| -> &Parameter {
        sym.parameters
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("JUSTIFY parameter {name:?} not found"))
    };
    assert_eq!(
        param("Value").justification,
        8,
        "Value parameter is top-right justified (Justification=8)"
    );
    let tol = param("Tol");
    assert_eq!(
        tol.justification, 4,
        "Tol parameter is centre justified (Justification=4)"
    );
    assert_eq!(tol.orientation, 1, "Tol parameter is rotated 90 degrees");
    assert!(tol.hidden, "Tol parameter is hidden");
}

#[test]
fn samples_schlib_fracpins() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("FRACPINS").expect("FRACPINS symbol not found");

    // Three pins: two off-grid (PinFrac stream) and one with a non-default symbol
    // line width (PinSymbolLineWidth stream). This is the FIRST real-Altium ground
    // truth for BOTH pin auxiliary streams, beyond a self-round-trip.
    assert_eq!(sym.pins.len(), 3, "FRACPINS has three pins");
    let pin = |d: &str| {
        sym.pins
            .iter()
            .find(|p| p.designator == d)
            .unwrap_or_else(|| panic!("pin {d} not found"))
    };
    // Pin 1 authored at (5, 3) mil + (0.5, 0.3) mil => frac (55000, 33000).
    assert_eq!(
        pin("1").frac.map(|f| (f.x, f.y)),
        Some((55_000, 33_000)),
        "pin 1 PinFrac"
    );
    // Pin 2 authored at (0, 97) mil + (0.5, 0.3) mil => frac (5000, 73000).
    assert_eq!(
        pin("2").frac.map(|f| (f.x, f.y)),
        Some((5_000, 73_000)),
        "pin 2 PinFrac"
    );
    // Pin 3 authored with Symbol_LineWidth := eLarge (index 3) — the PinSymbolLineWidth
    // aux stream; it is on-grid so carries no PinFrac.
    assert_eq!(
        pin("3").symbol_line_width,
        3,
        "pin 3 symbol_line_width (eLarge)"
    );
    assert_eq!(pin("3").frac, None, "pin 3 is on-grid (no PinFrac)");
}

#[test]
fn samples_schlib_bezier() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("BEZIERSYM").expect("BEZIERSYM symbol not found");

    // One cubic Bezier (four control points) authored via the verified
    // eBezier factory + SetState_Vertex path: AddBezier4(-100, 0, -50, 80,
    // 50, 80, 100, 0) in mils = (-10, 0) (-5, 8) (5, 8) (10, 0) in units.
    assert_eq!(sym.beziers.len(), 1, "BEZIERSYM has one Bezier curve");
    let bez = &sym.beziers[0];
    assert!(
        approx_eq(bez.x1, -10.0)
            && approx_eq(bez.y1, 0.0)
            && approx_eq(bez.x2, -5.0)
            && approx_eq(bez.y2, 8.0)
            && approx_eq(bez.x3, 5.0)
            && approx_eq(bez.y3, 8.0)
            && approx_eq(bez.x4, 10.0)
            && approx_eq(bez.y4, 0.0),
        "Bezier control points must match the authored values, got \
         ({}, {}) ({}, {}) ({}, {}) ({}, {})",
        bez.x1,
        bez.y1,
        bez.x2,
        bez.y2,
        bez.x3,
        bez.y3,
        bez.x4,
        bez.y4
    );
}

#[test]
fn samples_schlib_pie() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("PIESYM").expect("PIESYM symbol not found");

    // One filled pie sector (RECORD=9), authored 30..210 deg, radius 50 mil (=5
    // reader units), yellow fill. This is real-Altium ground truth for a primitive
    // the reader did not parse at all before this change — read as a Pie, NOT an Arc.
    assert!(
        sym.arcs.is_empty(),
        "PIESYM has no arcs (the sector is a Pie)"
    );
    assert_eq!(sym.pies.len(), 1, "PIESYM has one pie");
    let p = &sym.pies[0];
    assert!(
        (p.x - 0.0).abs() < 1e-6 && (p.y - 0.0).abs() < 1e-6,
        "pie centre"
    );
    assert!(
        (p.radius - 5.0).abs() < 1e-6,
        "pie radius (50 mil = 5 units)"
    );
    assert!((p.start_angle - 30.0).abs() < 1e-3, "pie start angle");
    assert!((p.end_angle - 210.0).abs() < 1e-3, "pie end angle");
    assert!(p.filled, "pie is filled (IsSolid)");
    assert_eq!(p.fill_color, 0x00_FF_FF, "pie fill colour (yellow)");
}

#[test]
fn samples_schlib_image() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("IMAGESYM").expect("IMAGESYM symbol not found");

    // One linked image (RECORD=30) — a 100x60 mil box (-5,-3)-(5,3 reader units)
    // referencing "logo.bmp", not embedded, aspect kept. Real-Altium ground truth
    // for a primitive the reader did not parse at all before this change.
    assert_eq!(sym.images.len(), 1, "IMAGESYM has one image");
    let im = &sym.images[0];
    assert!(
        (im.x1 - -5.0).abs() < 1e-6 && (im.y1 - -3.0).abs() < 1e-6,
        "image corner 1"
    );
    assert!(
        (im.x2 - 5.0).abs() < 1e-6 && (im.y2 - 3.0).abs() < 1e-6,
        "image corner 2"
    );
    assert_eq!(im.file_name, "logo.bmp", "image file name round-trips");
    assert!(!im.embed_image, "image is linked, not embedded");
    assert!(im.keep_aspect, "KeepAspect round-trips");
    assert_eq!(
        im.image_data, None,
        "a linked image carries no /Storage bytes"
    );
}

#[test]
fn samples_schlib_embedded_image() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("EMBIMGSYM").expect("EMBIMGSYM symbol not found");

    // One EMBEDDED image (RECORD=30, EmbedImage=T) whose raw bytes AD24 stored
    // in the library-level /Storage stream (one 0xD0 compressed entry, named
    // with the image's full source file path). The committed embed.bmp is
    // byte-identical to the bytes AD24 embedded, so this is real-Altium ground
    // truth for the /Storage read path — exact, all 70 bytes.
    assert_eq!(sym.images.len(), 1, "EMBIMGSYM has one image");
    let im = &sym.images[0];
    assert!(im.embed_image, "the image is embedded (EmbedImage=T)");
    assert_eq!(
        im.file_name, r"C:\Users\Public\altium_designer_mcp\samples\embed.bmp",
        "AD24 stores the full source file path as the FileName"
    );
    let expected: &[u8] = include_bytes!("../scripts/samples/embed.bmp");
    assert_eq!(expected.len(), 70, "the committed embed.bmp is 70 bytes");
    assert_eq!(
        im.image_data.as_deref(),
        Some(expected),
        "the /Storage bytes match the committed embed.bmp exactly"
    );
}

#[test]
fn samples_schlib_text_frame() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib
        .get("TEXTFRAMESYM")
        .expect("TEXTFRAMESYM symbol not found");

    // One text frame (RECORD=28) — a (-10,-5)-(10,5) box holding "Frame text",
    // light-yellow fill, dark-blue text, centred, word-wrapped and clipped, with
    // a 0.2-unit text margin (TextMargin_Frac=20000). Real-Altium ground truth
    // for a primitive the reader did not parse at all before this change.
    assert_eq!(sym.text_frames.len(), 1, "TEXTFRAMESYM has one text frame");
    let f = &sym.text_frames[0];
    assert!(
        (f.x1 - -10.0).abs() < 1e-6 && (f.y1 - -5.0).abs() < 1e-6,
        "frame corner 1"
    );
    assert!(
        (f.x2 - 10.0).abs() < 1e-6 && (f.y2 - 5.0).abs() < 1e-6,
        "frame corner 2"
    );
    assert_eq!(f.text, "Frame text", "frame text round-trips");
    assert_eq!(f.area_color, 11_599_871, "fill colour (light yellow)");
    assert_eq!(f.text_color, 8_388_608, "text colour (dark blue)");
    assert_eq!(f.line_width, 1, "border width");
    assert_eq!(f.font_id, 1, "font id");
    assert!(f.is_solid, "IsSolid round-trips");
    assert!(f.show_border, "ShowBorder round-trips");
    assert!(f.word_wrap, "WordWrap round-trips");
    assert!(f.clip_to_rect, "ClipToRect round-trips");
    assert_eq!(f.alignment, 1, "centred alignment");
    assert!(
        (f.text_margin - 0.2).abs() < 1e-6,
        "text margin (TextMargin_Frac=20000 = 0.2 units), got {}",
        f.text_margin
    );
    assert!(f.is_not_accessible, "IsNotAccesible round-trips");
}

#[test]
fn samples_schlib_fracshapes() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("FRACSHAPES").expect("FRACSHAPES symbol not found");

    // Off-grid shapes authored at MilsToCoord(n) + 5000 internal units (+0.5 mil
    // = +0.05 units per coordinate). AD24 stores negative off-grid coordinates
    // as truncation-toward-zero with a SIGNED fraction — the golden rectangle is
    // literally `Location.X=-5|Location.X_Frac=-45000` (= -5.45). Before the
    // signed-frac fix the reader parsed `_Frac` as u32, so -45000 failed to
    // parse and silently truncated the coordinate to -5.0.
    assert_eq!(sym.rectangles.len(), 1, "FRACSHAPES has one rectangle");
    let r = &sym.rectangles[0];
    assert!(
        approx_eq(r.x1, -5.45) && approx_eq(r.y1, -2.45),
        "rectangle corner 1 (authored -55-25 mil + 0.5 mil), got ({}, {})",
        r.x1,
        r.y1
    );
    assert!(
        approx_eq(r.x2, 5.55) && approx_eq(r.y2, 2.55),
        "rectangle corner 2 (authored 55/25 mil + 0.5 mil), got ({}, {})",
        r.x2,
        r.y2
    );

    // The arc centre sits at (+0.05, +0.05): AD24 omits the zero integer keys
    // entirely and stores only `Location.X_Frac=5000|Location.Y_Frac=5000`,
    // plus `Radius=4|Radius_Frac=5000` (= 4.05).
    assert_eq!(sym.arcs.len(), 1, "FRACSHAPES has one arc");
    let a = &sym.arcs[0];
    assert!(
        approx_eq(a.x, 0.05) && approx_eq(a.y, 0.05),
        "arc centre (integer keys omitted, frac-only), got ({}, {})",
        a.x,
        a.y
    );
    assert!(approx_eq(a.radius, 4.05), "arc radius, got {}", a.radius);
    assert!(approx_eq(a.end_angle, 270.0), "arc end angle");
}

#[test]
fn samples_schlib_swappin() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("SWAPPIN").expect("SWAPPIN symbol not found");

    // One pin authored with the swap-id tail: SwapId_Pin='A', SwapId_Part='1',
    // DefaultValue='3V3'. The binary pin record stores these as three trailing
    // Pascal short strings, mapped by the reader in order:
    //   SwapId_Pin  -> swap_id_group      ("A")
    //   SwapId_Part -> part_and_sequence  ("1" — replacing the "|&|" default)
    //   DefaultValue -> default_value     ("3V3")
    // Real-Altium ground truth for the tail, beyond a
    // self-round-trip.
    assert_eq!(sym.pins.len(), 1, "SWAPPIN has one pin");
    let pin = pin_by_designator(sym, "1");
    assert_eq!(pin.name, "SWP", "pin name");
    assert_eq!(pin.swap_id_group, "A", "SwapId_Pin lands in swap_id_group");
    assert_eq!(
        pin.part_and_sequence, "1",
        "SwapId_Part lands in part_and_sequence"
    );
    assert_eq!(
        pin.default_value, "3V3",
        "DefaultValue lands in default_value"
    );
}

#[test]
fn samples_schlib_dispmode() {
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let sym = lib.get("DISPMODE").expect("DISPMODE symbol not found");

    // A DisplayModeCount=2 symbol with one rectangle per display mode — the
    // first real-Altium golden for a non-default `OwnerPartDisplayMode` (it was
    // self-round-trip only until this fixture). The RECORD=1 header carries
    // `DisplayModeCount=2` verbatim.
    assert_eq!(sym.display_mode_count, 2, "DisplayModeCount round-trips");
    assert_eq!(sym.rectangles.len(), 2, "DISPMODE has two rectangles");

    // Mode-0 (normal view) rectangle: (-5, -2.5)..(5, 2.5) units. The ±2.5 y
    // coordinates are more signed-frac ground truth — the golden stores
    // `Location.Y=-2|Location.Y_Frac=-50000` (truncation toward zero with a
    // signed fraction) and `Corner.Y=2|Corner.Y_Frac=50000`.
    let mode0 = sym
        .rectangles
        .iter()
        .find(|r| r.display_flags.owner_part_display_mode == 0)
        .expect("DISPMODE has a mode-0 rectangle");
    assert!(
        approx_eq(mode0.x1, -5.0) && approx_eq(mode0.y1, -2.5),
        "mode-0 corner 1 (signed-frac y), got ({}, {})",
        mode0.x1,
        mode0.y1
    );
    assert!(
        approx_eq(mode0.x2, 5.0) && approx_eq(mode0.y2, 2.5),
        "mode-0 corner 2, got ({}, {})",
        mode0.x2,
        mode0.y2
    );

    // Mode-1 (alternate view) rectangle: (-6, -3)..(6, 3) units, carrying
    // `OwnerPartDisplayMode=1` in its RECORD=14 line.
    let mode1 = sym
        .rectangles
        .iter()
        .find(|r| r.display_flags.owner_part_display_mode == 1)
        .expect("DISPMODE has a mode-1 rectangle");
    assert!(
        approx_eq(mode1.x1, -6.0) && approx_eq(mode1.y1, -3.0),
        "mode-1 corner 1, got ({}, {})",
        mode1.x1,
        mode1.y1
    );
    assert!(
        approx_eq(mode1.x2, 6.0) && approx_eq(mode1.y2, 3.0),
        "mode-1 corner 2, got ({}, {})",
        mode1.x2,
        mode1.y2
    );

    // Both rectangles belong to part 1 — display modes are orthogonal to parts.
    assert_eq!(mode0.owner_part_id, 1, "mode-0 owner part");
    assert_eq!(mode1.owner_part_id, 1, "mode-1 owner part");
}

// ---------------------------------------------------------------------------
// Read-modify-write byte fidelity against the golden.
//
// These read a symbol from the Altium-authored golden and re-encode it with
// our writer, then compare the emitted records against the golden's exact
// record text (dumped byte-for-byte from scripts/samples/symbols.SchLib).
// Only the RECORD=1 component header is excluded (its UniqueID / AllPinCount
// fidelity is tracked separately in TODO §B); every content, designator and
// system-parameter record must match the golden token-for-token.
// ---------------------------------------------------------------------------

/// Replaces each `UniqueID` value with the `<UID>` placeholder.
///
/// Altium mints fresh random ids every time the samples are authored, so the
/// literal values cannot be asserted without the expectations breaking on every
/// regeneration. The shape still is: the key must be present, in the same
/// position, with exactly eight uppercase letters.
fn normalise_unique_ids(record: &str) -> String {
    let mut out = String::with_capacity(record.len());
    let mut rest = record;
    while let Some(at) = rest.find("UniqueID=") {
        let (before, tail) = rest.split_at(at + "UniqueID=".len());
        out.push_str(before);
        let id: String = tail.chars().take_while(char::is_ascii_uppercase).collect();
        assert_eq!(
            id.len(),
            8,
            "UniqueID must be 8 uppercase letters, got {id:?} in {record:?}"
        );
        out.push_str("<UID>");
        rest = &tail[id.len()..];
    }
    out.push_str(rest);
    out
}

/// Re-encodes `name` from the golden and returns its records as text (the
/// trailing NUL trimmed; binary pin records surface as `"<PIN>"`), excluding
/// the RECORD=1 component header, with `UniqueID` values normalised.
fn reencoded_records(lib: &SchLib, name: &str) -> Vec<String> {
    let symbol = lib.get(name).unwrap_or_else(|| panic!("{name} not found"));
    let data = altium_designer_mcp::altium::schlib::writer::encode_data_stream(symbol)
        .expect("re-encode golden symbol");
    let mut records = Vec::new();
    let mut off = 0;
    while off + 4 <= data.len() {
        let len =
            data[off] as usize | ((data[off + 1] as usize) << 8) | ((data[off + 2] as usize) << 16);
        let flags = data[off + 3];
        if flags == 1 {
            records.push("<PIN>".to_string());
        } else {
            records.push(
                String::from_utf8_lossy(&data[off + 4..off + 4 + len])
                    .trim_end_matches('\0')
                    .to_string(),
            );
        }
        off += 4 + len;
    }
    records.remove(0); // RECORD=1 header (see doc comment)
    records.iter().map(|r| normalise_unique_ids(r)).collect()
}

#[test]
fn samples_schlib_rmw_dispmode_matches_golden_records() {
    // The F1 headline proof: the system Comment keeps its IndexInSheet=-1
    // sentinel (no counter slot), the first rectangle stays at slot 0 (token
    // omitted) and the second at =1 — the full stream re-encodes exactly.
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    assert_eq!(
        reencoded_records(&lib, "DISPMODE"),
        [
            "|RECORD=14|IsNotAccesible=T|OwnerPartId=1|Location.X=-5|Location.Y=-2|Location.Y_Frac=-50000|Corner.X=5|Corner.Y=2|Corner.Y_Frac=50000|LineWidth=1|AreaColor=11599871|IsSolid=T|UniqueID=<UID>",
            "|RECORD=14|IsNotAccesible=T|IndexInSheet=1|OwnerPartId=1|OwnerPartDisplayMode=1|Location.X=-6|Location.Y=-3|Corner.X=6|Corner.Y=3|LineWidth=1|AreaColor=11599871|IsSolid=T|UniqueID=<UID>",
            "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
            "|RECORD=44",
        ]
    );
}

#[test]
fn samples_schlib_rmw_lines_matches_golden_records() {
    // F2 proof: every zero coordinate key stays omitted on re-encode (the
    // golden line (0,0)->(10,0) carries only Corner.X=10).
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    assert_eq!(
        reencoded_records(&lib, "LINES"),
        [
            "|RECORD=13|IsNotAccesible=T|OwnerPartId=1|Corner.X=10|LineWidth=1|UniqueID=<UID>",
            "|RECORD=13|IsNotAccesible=T|IndexInSheet=1|OwnerPartId=1|Corner.Y=10|LineWidth=1|UniqueID=<UID>",
            "|RECORD=13|IsNotAccesible=T|IndexInSheet=2|OwnerPartId=1|Corner.X=10|Corner.Y=10|LineWidth=1|UniqueID=<UID>",
            "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
            "|RECORD=44",
        ]
    );
}

#[test]
fn samples_schlib_rmw_arcs_and_fracshapes_match_golden_records() {
    // F3.6 proof: LineWidth precedes the 3-decimal angles and a zero
    // StartAngle is omitted (EndAngle=360.000 / 90.000 / 270.000), plus the
    // FRACSHAPES signed-frac coordinates re-encode exactly.
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    assert_eq!(
        reencoded_records(&lib, "ARCS"),
        [
            "|RECORD=12|IsNotAccesible=T|OwnerPartId=1|Radius=5|LineWidth=1|EndAngle=360.000|UniqueID=<UID>",
            "|RECORD=12|IsNotAccesible=T|IndexInSheet=1|OwnerPartId=1|Location.Y=-20|Radius=5|LineWidth=1|EndAngle=90.000|UniqueID=<UID>",
            "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
            "|RECORD=44",
        ]
    );
    assert_eq!(
        reencoded_records(&lib, "FRACSHAPES"),
        [
            "|RECORD=14|IsNotAccesible=T|OwnerPartId=1|Location.X=-5|Location.X_Frac=-45000|Location.Y=-2|Location.Y_Frac=-45000|Corner.X=5|Corner.X_Frac=55000|Corner.Y=2|Corner.Y_Frac=55000|LineWidth=1|AreaColor=11599871|IsSolid=T|UniqueID=<UID>",
            "|RECORD=12|IsNotAccesible=T|IndexInSheet=1|OwnerPartId=1|Location.X_Frac=5000|Location.Y_Frac=5000|Radius=4|Radius_Frac=5000|LineWidth=1|EndAngle=270.000|UniqueID=<UID>",
            "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
            "|RECORD=44",
        ]
    );
}

#[test]
fn samples_schlib_rmw_justify_and_params_match_golden_records() {
    // F3.2/F3.9 proof: user parameters keep the golden token order (Location,
    // Orientation, Justification, FontID, IsHidden, Text, Name), the zero
    // Color stays omitted, and they follow the labels on the shared content
    // counter (slots 4 and 5, exactly as the golden stores).
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    assert_eq!(
        reencoded_records(&lib, "JUSTIFY"),
        [
            "|RECORD=4|IsNotAccesible=T|OwnerPartId=1|Location.X=-10|Location.Y=10|FontID=1|Text=BL|UniqueID=<UID>",
            "|RECORD=4|IsNotAccesible=T|IndexInSheet=1|OwnerPartId=1|Location.X=-10|Location.Y=5|Justification=4|FontID=1|Text=CC|UniqueID=<UID>",
            "|RECORD=4|IsNotAccesible=T|IndexInSheet=2|OwnerPartId=1|Location.X=-10|Justification=8|FontID=1|Text=TR|UniqueID=<UID>",
            "|RECORD=4|IsNotAccesible=T|IndexInSheet=3|OwnerPartId=1|Location.X=-10|Location.Y=-5|Orientation=1|FontID=1|Text=ROT90|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=4|OwnerPartId=1|Location.X=10|Location.Y=10|Justification=8|FontID=1|Text=1k|Name=Value|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=5|OwnerPartId=1|Location.X=10|Location.Y=5|Orientation=1|Justification=4|FontID=1|IsHidden=T|Text=5%|Name=Tol|UniqueID=<UID>",
            "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
            "|RECORD=44",
        ]
    );
    assert_eq!(
        reencoded_records(&lib, "PARAMS"),
        [
            "|RECORD=41|OwnerPartId=1|Location.X=5|Location.Y=40|FontID=1|Text=10k|Name=Value|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=1|OwnerPartId=1|Location.X=5|Location.Y=45|FontID=1|IsHidden=T|Text=100nF|Name=Comment|UniqueID=<UID>",
            "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
            "|RECORD=44",
        ]
    );
}

#[test]
fn samples_schlib_rmw_polyline_ellipse_bezier_textframe_match_golden_records() {
    // F3.1 (IsNotAccesible on ellipse/polyline), F3.4 (zero polyline style
    // keys omitted), F2 (zero vertices omitted) and F4 (text frame token
    // order with unconditional AreaColor/FontID) all re-encode exactly.
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    assert_eq!(
        reencoded_records(&lib, "POLYLINES"),
        [
            "|RECORD=6|IsNotAccesible=T|OwnerPartId=1|LineWidth=1|LocationCount=3|X2=10|Y2=5|Y3=10|UniqueID=<UID>",
            "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
            "|RECORD=44",
        ]
    );
    assert_eq!(
        reencoded_records(&lib, "ELLIPSES"),
        [
            "|RECORD=8|IsNotAccesible=T|OwnerPartId=1|Radius=5|SecondaryRadius=5|LineWidth=1|AreaColor=65535|IsSolid=T|UniqueID=<UID>",
            "|RECORD=8|IsNotAccesible=T|IndexInSheet=1|OwnerPartId=1|Location.X=20|Radius=8|SecondaryRadius=4|LineWidth=1|AreaColor=65535|UniqueID=<UID>",
            "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
            "|RECORD=44",
        ]
    );
    assert_eq!(
        reencoded_records(&lib, "BEZIERSYM"),
        [
            "|RECORD=5|IsNotAccesible=T|OwnerPartId=1|LineWidth=1|LocationCount=4|X1=-10|X2=-5|Y2=8|X3=5|Y3=8|X4=10|UniqueID=<UID>",
            "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
            "|RECORD=44",
        ]
    );
    assert_eq!(
        reencoded_records(&lib, "TEXTFRAMESYM"),
        [
            "|RECORD=28|IsNotAccesible=T|OwnerPartId=1|Location.X=-10|Location.Y=-5|Corner.X=10|Corner.Y=5|LineWidth=1|AreaColor=11599871|TextColor=8388608|FontID=1|IsSolid=T|ShowBorder=T|Alignment=1|WordWrap=T|ClipToRect=T|Text=Frame text|TextMargin_Frac=20000|UniqueID=<UID>",
            "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
            "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
            "|RECORD=44",
        ]
    );
}

#[test]
fn samples_schlib_rmw_shapestyle_records_match_golden_ignoring_stream_order() {
    // SHAPESTYLE mixes shape families, and our writer groups families in a
    // fixed order while the golden stream interleaves them — so the shared
    // IndexInSheet slots differ by position. Every record must still match a
    // golden record exactly once with the positional IndexInSheet token
    // stripped (proving the F3.5 LineStyle+LineStyleExt dual-key emission and
    // the F3.3/polygon Transparent placement byte-exactly), and the emitted
    // slots must still be the golden's {omitted, 1..5}.
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");
    let strip = |s: &str| -> String {
        match (s.find("|IndexInSheet="), s.find("|OwnerPartId=")) {
            (Some(a), Some(b)) if a < b => format!("{}{}", &s[..a], &s[b..]),
            _ => s.to_string(),
        }
    };
    let mut ours: Vec<String> = reencoded_records(&lib, "SHAPESTYLE")
        .iter()
        .map(|s| strip(s))
        .collect();
    let mut golden: Vec<String> = [
        "|RECORD=13|IsNotAccesible=T|OwnerPartId=1|Location.X=-20|LineWidth=1|LineStyle=1|LineStyleExt=1|UniqueID=<UID>",
        "|RECORD=13|IsNotAccesible=T|IndexInSheet=1|OwnerPartId=1|Corner.X=20|LineWidth=1|LineStyle=2|LineStyleExt=2|UniqueID=<UID>",
        "|RECORD=14|IsNotAccesible=T|IndexInSheet=2|OwnerPartId=1|Location.X=-10|Location.Y=-10|Corner.X=10|Corner.Y=-5|LineWidth=1|AreaColor=65535|IsSolid=T|UniqueID=<UID>",
        "|RECORD=14|IsNotAccesible=T|IndexInSheet=3|OwnerPartId=1|Location.X=-10|Location.Y=5|Corner.X=10|Corner.Y=10|LineWidth=1|AreaColor=11599871|IsSolid=T|Transparent=T|UniqueID=<UID>",
        "|RECORD=7|IsNotAccesible=T|IndexInSheet=4|OwnerPartId=1|LineWidth=1|AreaColor=65280|IsSolid=T|Transparent=T|LocationCount=3|X1=-5|Y1=12|X2=5|Y2=12|X3=5|Y3=17|UniqueID=<UID>",
        "|RECORD=8|IsNotAccesible=T|IndexInSheet=5|OwnerPartId=1|Location.X=15|Location.Y=10|Radius=3|SecondaryRadius=2|LineWidth=1|AreaColor=11599871|IsSolid=T|Transparent=T|UniqueID=<UID>",
        "|RECORD=34|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=5|Color=8388608|FontID=1|Text=U?|Name=Designator|ReadOnlyState=1|UniqueID=<UID>",
        "|RECORD=41|IndexInSheet=-1|OwnerPartId=-1|Location.X=-5|Location.Y=-15|Color=8388608|FontID=1|Text=*|Name=Comment|UniqueID=<UID>",
        "|RECORD=44",
    ]
    .iter()
    .map(|s| strip(s))
    .collect();
    ours.sort();
    golden.sort();
    assert_eq!(ours, golden, "SHAPESTYLE records (IndexInSheet-stripped)");

    // The shared counter itself must still produce the golden slot set.
    let slots: Vec<Option<i32>> = reencoded_records(&lib, "SHAPESTYLE")
        .iter()
        .filter(|s| !s.contains("OwnerPartId=-1") && !s.ends_with("RECORD=44"))
        .map(|s| {
            s.find("|IndexInSheet=")
                .map(|a| s[a + 14..].split('|').next().unwrap().parse().unwrap())
        })
        .collect();
    let mut numbered: Vec<i32> = slots.iter().filter_map(|s| *s).collect();
    numbered.sort_unstable();
    assert_eq!(
        slots.iter().filter(|s| s.is_none()).count(),
        1,
        "exactly one content record sits at slot 0 (token omitted)"
    );
    assert_eq!(numbered, vec![1, 2, 3, 4, 5], "content slots 1..5");
}

#[test]
fn samples_schlib_unicode_symbol_name_and_description() {
    // A symbol whose name is outside Windows-1252, authored by Altium itself.
    //
    // Altium writes such a value as its raw UTF-8 bytes under the plain key and
    // widens the same bytes through the authoring machine's ANSI code page for
    // the CFB storage name, so the header's LibRef entry and the storage name
    // carry different bytes. Components are therefore located by walking the
    // storages rather than trusting that list, and the name is recovered from
    // the plain key's UTF-8 bytes — the `%UTF8%` companion Altium writes
    // alongside is locale-dependent and decodes to mojibake off that machine.
    let lib = SchLib::open(sample("symbols.SchLib")).expect("failed to open symbols.SchLib");

    let sym = lib
        .get("Резистор")
        .expect("the Cyrillic-named symbol must be found, not silently skipped");
    assert_eq!(sym.name, "Резистор");
    assert_eq!(sym.description, "описание Ω", "Greek omega survives too");
    assert_eq!(sym.rectangles.len(), 1, "its body shape is read normally");
}

#[test]
fn samples_schlib_manual_parameter_properties() {
    // Hand-authored fixture (scripts/samples/manual/parameters.SchLib) — AD24 exposes
    // neither of these on ISch_Parameter, so no DelphiScript can author them and the
    // generated golden cannot carry them. See that folder's README to rebuild it.
    //
    // NotAutoPosition is stored inverted and omit-when-default: Altium writes the key
    // only when the user turns auto-positioning OFF. This fixture is what proved that,
    // and that the key is not the `AUTOPOSITION` the docs had led us to expect.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("samples")
        .join("manual")
        .join("parameters.SchLib");
    let lib = SchLib::open(&path).expect("failed to open manual/parameters.SchLib");
    let sym = lib.get("PARAMPROPS").expect("symbol PARAMPROPS not found");

    let param = |n: &str| {
        sym.parameters
            .iter()
            .find(|p| p.name == n)
            .unwrap_or_else(|| panic!("parameter {n} not found"))
    };
    let test_param = param("TestParam");
    assert!(
        !test_param.auto_position,
        "TestParam was authored with auto-positioning turned off"
    );
    assert_eq!(test_param.justification, 7, "top-centre");

    // The control: an untouched parameter omits the key, which must read as ON rather
    // than defaulting to off — the inversion is easy to get backwards.
    assert!(
        param("Comment").auto_position,
        "an untouched parameter auto-positions"
    );

    // A rule parameter is identified by its name and payload, not by a flag: AD24
    // writes no IsRule key into a library.
    let rule = param("Rule");
    assert!(
        rule.value.contains("RULEKIND=Width"),
        "value: {}",
        rule.value
    );
    assert!(rule.hidden, "the rule parameter is hidden");
}
