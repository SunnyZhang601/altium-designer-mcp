//! Delete/validate/export/import tools. Split from `server.rs`.

use serde_json::{json, Value};

use crate::mcp::server::{McpServer, ToolCallResult};

impl McpServer {
    // ==================== Library Management Tools ====================

    /// Deletes one or more components from a library file.
    ///
    /// Supports both `.PcbLib` and `.SchLib` files. The file type is auto-detected
    /// from the extension. Returns per-component status (`deleted`, `not_found`, or `error`).
    pub(crate) fn call_delete_component(&self, arguments: &Value) -> ToolCallResult {
        let Some(filepath) = arguments.get("filepath").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: filepath");
        };

        // Validate path is within allowed directories
        if let Err(e) = self.validate_path(filepath) {
            return ToolCallResult::error(e);
        }

        let Some(component_names) = arguments.get("component_names").and_then(Value::as_array)
        else {
            return ToolCallResult::error("Missing required parameter: component_names");
        };

        let names: Vec<&str> = component_names.iter().filter_map(Value::as_str).collect();

        if names.is_empty() {
            return ToolCallResult::error("component_names array is empty or contains no strings");
        }

        let dry_run = arguments
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Determine file type from extension
        let path = std::path::Path::new(filepath);
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase);

        match extension.as_deref() {
            Some("pcblib") => Self::delete_from_pcblib(filepath, &names, dry_run),
            Some("schlib") => Self::delete_from_schlib(filepath, &names, dry_run),
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

    /// Deletes components from a `PcbLib` file.
    pub(crate) fn delete_from_pcblib(
        filepath: &str,
        names: &[&str],
        dry_run: bool,
    ) -> ToolCallResult {
        use crate::altium::PcbLib;

        // Read the library
        let mut library = match PcbLib::open(filepath) {
            Ok(lib) => lib,
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                return ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap());
            }
        };

        let original_count = library.len();
        let mut results: Vec<Value> = Vec::with_capacity(names.len());
        let mut deleted_count = 0;
        let mut seen = std::collections::HashSet::new();

        // Check which components exist (for dry_run) or remove them
        for name in names {
            if dry_run {
                // In dry-run mode, just check if component exists
                if library.get(name).is_some() {
                    results.push(json!({
                        "name": name,
                        "status": "would_delete"
                    }));
                    // Count each distinct existing name once so duplicate names
                    // can't over-count (which underflowed remaining_count).
                    if seen.insert(*name) {
                        deleted_count += 1;
                    }
                } else {
                    results.push(json!({
                        "name": name,
                        "status": "not_found"
                    }));
                }
            } else if library.remove(name).is_some() {
                results.push(json!({
                    "name": name,
                    "status": "deleted"
                }));
                deleted_count += 1;
            } else {
                results.push(json!({
                    "name": name,
                    "status": "not_found"
                }));
            }
        }

        // Clean up orphaned embedded models after deleting footprints
        let orphaned_models_removed = if deleted_count > 0 && !dry_run {
            library.remove_orphaned_models()
        } else {
            0
        };

        // Only write if something was deleted (and not dry-run)
        if deleted_count > 0 && !dry_run {
            // Create backup before destructive operation
            if let Err(e) = Self::create_backup(filepath) {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e,
                    "results": results,
                });
                return ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap());
            }

            if let Err(e) = library.save(filepath) {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": format!("Failed to write library: {e}"),
                    "results": results,
                });
                return ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap());
            }
        }

        let mut result = json!({
            "status": if dry_run { "dry_run" } else { "success" },
            "filepath": filepath,
            "file_type": "PcbLib",
            "dry_run": dry_run,
            "original_count": original_count,
            "deleted_count": deleted_count,
            "remaining_count": if dry_run { original_count.saturating_sub(deleted_count) } else { library.len() },
            "orphaned_models_removed": orphaned_models_removed,
            "results": results,
        });

        // Run post-write validation (only if actual changes were made)
        if deleted_count > 0 && !dry_run {
            if let Some(validation) = Self::post_write_validation_pcblib(filepath) {
                result["validation"] = validation;
            }
        }

        ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
    }

    /// Deletes components from a `SchLib` file.
    pub(crate) fn delete_from_schlib(
        filepath: &str,
        names: &[&str],
        dry_run: bool,
    ) -> ToolCallResult {
        use crate::altium::SchLib;

        // Read the library
        let mut library = match SchLib::open(filepath) {
            Ok(lib) => lib,
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                return ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap());
            }
        };

        let original_count = library.len();
        let mut results: Vec<Value> = Vec::with_capacity(names.len());
        let mut deleted_count = 0;
        let mut seen = std::collections::HashSet::new();

        // Check which components exist (for dry_run) or remove them
        for name in names {
            if dry_run {
                // In dry-run mode, just check if component exists
                if library.get(name).is_some() {
                    results.push(json!({
                        "name": name,
                        "status": "would_delete"
                    }));
                    // Count each distinct existing name once so duplicate names
                    // can't over-count (which underflowed remaining_count).
                    if seen.insert(*name) {
                        deleted_count += 1;
                    }
                } else {
                    results.push(json!({
                        "name": name,
                        "status": "not_found"
                    }));
                }
            } else if library.remove(name).is_some() {
                results.push(json!({
                    "name": name,
                    "status": "deleted"
                }));
                deleted_count += 1;
            } else {
                results.push(json!({
                    "name": name,
                    "status": "not_found"
                }));
            }
        }

        // Only write if something was deleted (and not dry-run)
        if deleted_count > 0 && !dry_run {
            // Create backup before destructive operation
            if let Err(e) = Self::create_backup(filepath) {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e,
                    "results": results,
                });
                return ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap());
            }

            if let Err(e) = library.save(filepath) {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": format!("Failed to write library: {e}"),
                    "results": results,
                });
                return ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap());
            }
        }

        let mut result = json!({
            "status": if dry_run { "dry_run" } else { "success" },
            "filepath": filepath,
            "file_type": "SchLib",
            "dry_run": dry_run,
            "original_count": original_count,
            "deleted_count": deleted_count,
            "remaining_count": if dry_run { original_count.saturating_sub(deleted_count) } else { library.len() },
            "results": results,
        });

        // Run post-write validation (only if actual changes were made)
        if deleted_count > 0 && !dry_run {
            if let Some(validation) = Self::post_write_validation_schlib(filepath) {
                result["validation"] = validation;
            }
        }

        ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
    }

    // ==================== Library Validation Tools ====================

    /// Validates an Altium library file for common issues.
    ///
    /// Checks for empty components, duplicate designators, invalid coordinates,
    /// zero-size primitives, and other integrity problems.
    pub(crate) fn call_validate_library(&self, arguments: &Value) -> ToolCallResult {
        let Some(filepath) = arguments.get("filepath").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: filepath");
        };

        // Validate path is within allowed directories
        if let Err(e) = self.validate_path(filepath) {
            return ToolCallResult::error(e);
        }

        // Determine file type from extension
        let path = std::path::Path::new(filepath);
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase);

        match extension.as_deref() {
            Some("pcblib") => Self::validate_pcblib(filepath),
            Some("schlib") => Self::validate_schlib(filepath),
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

    /// Validates a `PcbLib` file.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn validate_pcblib(filepath: &str) -> ToolCallResult {
        use crate::altium::pcblib::MAX_REPORTED_PAD_OVERLAPS;
        use crate::altium::PcbLib;
        use std::collections::HashSet;

        // Read the library
        let library = match PcbLib::open(filepath) {
            Ok(lib) => lib,
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                return ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap());
            }
        };

        let mut issues: Vec<Value> = Vec::new();
        let component_count = library.len();

        // Check for empty library
        if component_count == 0 {
            issues.push(json!({
                "severity": "warning",
                "component": null,
                "issue": "Library is empty (no footprints)"
            }));
        }

        // Validate each footprint
        for fp in library.iter() {
            let name = &fp.name;

            // Check for empty name
            if name.is_empty() {
                issues.push(json!({
                    "severity": "error",
                    "component": name,
                    "issue": "Footprint has empty name"
                }));
            }

            // Check for no pads
            if fp.pads.is_empty() {
                issues.push(json!({
                    "severity": "warning",
                    "component": name,
                    "issue": "Footprint has no pads"
                }));
            }

            // Check for duplicate pad designators
            let mut seen_designators: HashSet<&str> = HashSet::new();
            for pad in &fp.pads {
                if !seen_designators.insert(&pad.designator) {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Duplicate pad designator: '{}'", pad.designator)
                    }));
                }

                // Check for empty designator
                if pad.designator.is_empty() {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": "Pad has empty designator"
                    }));
                }

                // Check for zero or negative dimensions
                if pad.width <= 0.0 || pad.height <= 0.0 {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Pad '{}' has invalid dimensions (width: {}, height: {})",
                            pad.designator, pad.width, pad.height)
                    }));
                }

                // Check for invalid coordinates
                if !pad.x.is_finite() || !pad.y.is_finite() {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Pad '{}' has invalid coordinates (x: {}, y: {})",
                            pad.designator, pad.x, pad.y)
                    }));
                }
            }

            // Check tracks for invalid values
            for (i, track) in fp.tracks.iter().enumerate() {
                if track.width <= 0.0 {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Track {} has invalid width: {}", i, track.width)
                    }));
                }
                if !track.x1.is_finite()
                    || !track.y1.is_finite()
                    || !track.x2.is_finite()
                    || !track.y2.is_finite()
                {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Track {} has invalid coordinates", i)
                    }));
                }
            }

            // Check arcs for invalid values
            for (i, arc) in fp.arcs.iter().enumerate() {
                if arc.radius <= 0.0 {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Arc {} has invalid radius: {}", i, arc.radius)
                    }));
                }
                if !arc.x.is_finite() || !arc.y.is_finite() {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Arc {} has invalid centre coordinates", i)
                    }));
                }
            }

            // Overlapping pad copper shorts nets together. This is the only
            // electrical check here: everything else in this function is
            // integrity, and a footprint whose exposed pad welds every pin
            // together passes all of it. Warning rather than error - stacked
            // same-designator pads are legal and are already excluded by
            // overlapping_pad_pairs().
            //
            // Truncated like the write_pcblib warning: pairs are quadratic in
            // pad count, so a library holding one systematically broken BGA
            // would otherwise emit tens of thousands of issues here.
            let overlaps = fp.overlapping_pad_pairs();
            for &(i, j, ox, oy) in overlaps.iter().take(MAX_REPORTED_PAD_OVERLAPS) {
                issues.push(json!({
                    "severity": "warning",
                    "component": name,
                    "issue": format!(
                        "Pads '{}' and '{}' overlap by {:.3} x {:.3} mm on {} - overlapping copper merges into one net",
                        fp.pads[i].designator,
                        fp.pads[j].designator,
                        ox,
                        oy,
                        fp.pads[i].layer.as_str()
                    )
                }));
            }
            if overlaps.len() > MAX_REPORTED_PAD_OVERLAPS {
                issues.push(json!({
                    "severity": "warning",
                    "component": name,
                    "issue": format!(
                        "{} overlapping pad pairs total; {MAX_REPORTED_PAD_OVERLAPS} shown",
                        overlaps.len()
                    )
                }));
            }

            // Check regions for minimum vertices
            for (i, region) in fp.regions.iter().enumerate() {
                if region.vertices.len() < 3 {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Region {} has fewer than 3 vertices", i)
                    }));
                }
            }
        }

        let error_count = issues.iter().filter(|i| i["severity"] == "error").count();
        let warning_count = issues.iter().filter(|i| i["severity"] == "warning").count();

        let result = json!({
            "status": if error_count > 0 { "invalid" } else if warning_count > 0 { "warnings" } else { "valid" },
            "filepath": filepath,
            "file_type": "PcbLib",
            "component_count": component_count,
            "error_count": error_count,
            "warning_count": warning_count,
            "issues": issues,
        });

        ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
    }

    /// Validates a `SchLib` file.
    pub(crate) fn validate_schlib(filepath: &str) -> ToolCallResult {
        use crate::altium::SchLib;
        use std::collections::HashSet;

        // Read the library
        let library = match SchLib::open(filepath) {
            Ok(lib) => lib,
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                return ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap());
            }
        };

        let mut issues: Vec<Value> = Vec::new();
        let component_count = library.len();

        // Check for empty library
        if component_count == 0 {
            issues.push(json!({
                "severity": "warning",
                "component": null,
                "issue": "Library is empty (no symbols)"
            }));
        }

        // Validate each symbol
        for symbol in library.iter() {
            let name = &symbol.name;
            // Check for empty name
            if name.is_empty() {
                issues.push(json!({
                    "severity": "error",
                    "component": name,
                    "issue": "Symbol has empty name"
                }));
            }

            // Check for no pins
            if symbol.pins.is_empty() {
                issues.push(json!({
                    "severity": "warning",
                    "component": name,
                    "issue": "Symbol has no pins"
                }));
            }

            // Check for duplicate pin designators
            let mut seen_designators: HashSet<&str> = HashSet::new();
            for pin in &symbol.pins {
                if !seen_designators.insert(&pin.designator) {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Duplicate pin designator: '{}'", pin.designator)
                    }));
                }

                // Check for empty designator
                if pin.designator.is_empty() {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": "Pin has empty designator"
                    }));
                }

                // Check for zero or negative pin length
                if pin.length <= 0 {
                    issues.push(json!({
                        "severity": "warning",
                        "component": name,
                        "issue": format!("Pin '{}' has zero or negative length: {}",
                            pin.designator, pin.length)
                    }));
                }
            }

            // Check rectangles for inverted corners
            for (i, rect) in symbol.rectangles.iter().enumerate() {
                if rect.x1 > rect.x2 || rect.y1 > rect.y2 {
                    issues.push(json!({
                        "severity": "warning",
                        "component": name,
                        "issue": format!("Rectangle {} has inverted corners (x1={}, y1={}, x2={}, y2={})",
                            i, rect.x1, rect.y1, rect.x2, rect.y2)
                    }));
                }
            }

            // Check for symbols with no body (no rectangles, lines, or other graphics)
            let has_body = !symbol.rectangles.is_empty()
                || !symbol.lines.is_empty()
                || !symbol.polylines.is_empty()
                || !symbol.arcs.is_empty()
                || !symbol.ellipses.is_empty();

            if !has_body && !symbol.pins.is_empty() {
                issues.push(json!({
                    "severity": "warning",
                    "component": name,
                    "issue": "Symbol has pins but no body graphics (rectangles, lines, etc.)"
                }));
            }
        }

        let error_count = issues.iter().filter(|i| i["severity"] == "error").count();
        let warning_count = issues.iter().filter(|i| i["severity"] == "warning").count();

        let result = json!({
            "status": if error_count > 0 { "invalid" } else if warning_count > 0 { "warnings" } else { "valid" },
            "filepath": filepath,
            "file_type": "SchLib",
            "component_count": component_count,
            "error_count": error_count,
            "warning_count": warning_count,
            "issues": issues,
        });

        ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
    }

    /// Runs post-write validation on a `PcbLib` file and returns validation info.
    ///
    /// Returns a JSON value with validation results that can be included in write operation responses.
    /// Returns `None` if the file cannot be read (which would indicate a serious write failure).
    pub(crate) fn post_write_validation_pcblib(filepath: &str) -> Option<Value> {
        use crate::altium::PcbLib;
        use std::collections::HashSet;

        let library = PcbLib::open(filepath).ok()?;
        let mut issues: Vec<Value> = Vec::new();

        for fp in library.iter() {
            let name = &fp.name;

            // Check for empty name
            if name.is_empty() {
                issues.push(json!({
                    "severity": "error",
                    "component": name,
                    "issue": "Footprint has empty name"
                }));
            }

            // Check for duplicate pad designators
            let mut seen_designators: HashSet<&str> = HashSet::new();
            for pad in &fp.pads {
                if !seen_designators.insert(&pad.designator) {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Duplicate pad designator: '{}'", pad.designator)
                    }));
                }

                // Check for zero or negative dimensions
                if pad.width <= 0.0 || pad.height <= 0.0 {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Pad '{}' has invalid dimensions", pad.designator)
                    }));
                }
            }

            // Check tracks for invalid values
            for (i, track) in fp.tracks.iter().enumerate() {
                if track.width <= 0.0 {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Track {} has invalid width", i)
                    }));
                }
            }

            // Check arcs for invalid values
            for (i, arc) in fp.arcs.iter().enumerate() {
                if arc.radius <= 0.0 {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Arc {} has invalid radius", i)
                    }));
                }
            }

            // Check regions for minimum vertices
            for (i, region) in fp.regions.iter().enumerate() {
                if region.vertices.len() < 3 {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Region {} has fewer than 3 vertices", i)
                    }));
                }
            }
        }

        let error_count = issues.iter().filter(|i| i["severity"] == "error").count();
        let warning_count = issues.iter().filter(|i| i["severity"] == "warning").count();

        Some(json!({
            "status": if error_count > 0 { "invalid" } else if warning_count > 0 { "warnings" } else { "valid" },
            "error_count": error_count,
            "warning_count": warning_count,
            "issues": issues,
        }))
    }

    /// Runs post-write validation on a `SchLib` file and returns validation info.
    ///
    /// Returns a JSON value with validation results that can be included in write operation responses.
    /// Returns `None` if the file cannot be read (which would indicate a serious write failure).
    pub(crate) fn post_write_validation_schlib(filepath: &str) -> Option<Value> {
        use crate::altium::SchLib;
        use std::collections::HashSet;

        let library = SchLib::open(filepath).ok()?;
        let mut issues: Vec<Value> = Vec::new();

        for symbol in library.iter() {
            let name = &symbol.name;

            // Check for empty name
            if name.is_empty() {
                issues.push(json!({
                    "severity": "error",
                    "component": name,
                    "issue": "Symbol has empty name"
                }));
            }

            // Check for duplicate pin designators
            let mut seen_designators: HashSet<&str> = HashSet::new();
            for pin in &symbol.pins {
                if !seen_designators.insert(&pin.designator) {
                    issues.push(json!({
                        "severity": "error",
                        "component": name,
                        "issue": format!("Duplicate pin designator: '{}'", pin.designator)
                    }));
                }
            }

            // Check rectangles for inverted corners
            for (i, rect) in symbol.rectangles.iter().enumerate() {
                if rect.x1 > rect.x2 || rect.y1 > rect.y2 {
                    issues.push(json!({
                        "severity": "warning",
                        "component": name,
                        "issue": format!("Rectangle {} has inverted corners", i)
                    }));
                }
            }
        }

        let error_count = issues.iter().filter(|i| i["severity"] == "error").count();
        let warning_count = issues.iter().filter(|i| i["severity"] == "warning").count();

        Some(json!({
            "status": if error_count > 0 { "invalid" } else if warning_count > 0 { "warnings" } else { "valid" },
            "error_count": error_count,
            "warning_count": warning_count,
            "issues": issues,
        }))
    }

    // ==================== Library Export Tools ====================

    /// Exports an Altium library to JSON or CSV format.
    pub(crate) fn call_export_library(&self, arguments: &Value) -> ToolCallResult {
        let Some(filepath) = arguments.get("filepath").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: filepath");
        };

        // Validate path is within allowed directories
        if let Err(e) = self.validate_path(filepath) {
            return ToolCallResult::error(e);
        }

        let Some(format) = arguments.get("format").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: format");
        };

        let format_lower = format.to_lowercase();
        if format_lower != "json" && format_lower != "csv" {
            return ToolCallResult::error("Invalid format. Expected 'json' or 'csv'.");
        }

        // Parse compact parameter (default: true)
        let compact = arguments
            .get("compact")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        // Determine file type from extension
        let path = std::path::Path::new(filepath);
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase);

        match extension.as_deref() {
            Some("pcblib") => Self::export_pcblib(filepath, &format_lower, compact),
            Some("schlib") => Self::export_schlib(filepath, &format_lower),
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

    /// Exports a `PcbLib` file to JSON or CSV.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn export_pcblib(filepath: &str, format: &str, compact: bool) -> ToolCallResult {
        use crate::altium::pcblib::primitives::PadStackMode;
        use crate::altium::PcbLib;

        // Read the library
        let library = match PcbLib::open(filepath) {
            Ok(lib) => lib,
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                return ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap());
            }
        };

        if format == "json" {
            // Full JSON export
            let footprints: Vec<Value> = library
                .iter()
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
                                            obj.insert("stack_mode".to_string(), json!("simple"));
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

            let result = json!({
                "status": "success",
                "filepath": filepath,
                "file_type": "PcbLib",
                "format": "json",
                "units": "mm",
                "component_count": library.len(),
                "footprints": footprints,
            });

            ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
        } else {
            // CSV export - summary table
            let mut csv_lines: Vec<String> = Vec::new();
            csv_lines.push("name,description,pad_count,track_count,arc_count,region_count,text_count,external_3d_model,embedded_3d_bodies".to_string());

            for fp in library.iter() {
                let has_external_model = if fp.model_3d.is_some() { "yes" } else { "no" };
                let embedded_body_count = fp.component_bodies.len();
                csv_lines.push(format!(
                    "{},{},{},{},{},{},{},{},{}",
                    crate::util::escape_csv_field(&fp.name),
                    crate::util::escape_csv_field(&fp.description),
                    fp.pads.len(),
                    fp.tracks.len(),
                    fp.arcs.len(),
                    fp.regions.len(),
                    fp.text.len(),
                    has_external_model,
                    embedded_body_count
                ));
            }

            let csv_content = csv_lines.join("\n");

            let result = json!({
                "status": "success",
                "filepath": filepath,
                "file_type": "PcbLib",
                "format": "csv",
                "component_count": library.len(),
                "csv": csv_content,
            });

            ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
        }
    }

    /// Exports a `SchLib` file to JSON or CSV.
    pub(crate) fn export_schlib(filepath: &str, format: &str) -> ToolCallResult {
        use crate::altium::SchLib;

        // Read the library
        let library = match SchLib::open(filepath) {
            Ok(lib) => lib,
            Err(e) => {
                let result = json!({
                    "status": "error",
                    "filepath": filepath,
                    "error": e.to_string(),
                });
                return ToolCallResult::error(serde_json::to_string_pretty(&result).unwrap());
            }
        };

        if format == "json" {
            // Full JSON export
            let symbols: Vec<Value> = library
                .iter()
                .map(|symbol| {
                    json!({
                        "name": symbol.name,
                        "description": symbol.description,
                        "designator": symbol.designator,
                        "part_count": symbol.part_count,
                        "display_mode_count": symbol.display_mode_count,
                        "current_part_id": symbol.current_part_id,
                        "part_id_locked": symbol.part_id_locked,
                        "source_library_name": symbol.source_library_name,
                        "target_file_name": symbol.target_file_name,
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

            let result = json!({
                "status": "success",
                "filepath": filepath,
                "file_type": "SchLib",
                "format": "json",
                "units": "schematic units (10 = 1 grid)",
                "component_count": library.len(),
                "symbols": symbols,
            });

            ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
        } else {
            // CSV export - summary table
            let mut csv_lines: Vec<String> = Vec::new();
            csv_lines.push(
                "name,description,designator,pin_count,rectangle_count,line_count,footprint_count"
                    .to_string(),
            );

            for symbol in library.iter() {
                csv_lines.push(format!(
                    "{},{},{},{},{},{},{}",
                    crate::util::escape_csv_field(&symbol.name),
                    crate::util::escape_csv_field(&symbol.description),
                    crate::util::escape_csv_field(&symbol.designator),
                    symbol.pins.len(),
                    symbol.rectangles.len(),
                    symbol.lines.len(),
                    symbol.footprints.len()
                ));
            }

            let csv_content = csv_lines.join("\n");

            let result = json!({
                "status": "success",
                "filepath": filepath,
                "file_type": "SchLib",
                "format": "csv",
                "component_count": library.len(),
                "csv": csv_content,
            });

            ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
        }
    }

    // ==================== Library Import ====================

    /// Imports components from JSON data into an Altium library file.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn call_import_library(&self, arguments: &Value) -> ToolCallResult {
        let Some(output_path) = arguments.get("output_path").and_then(Value::as_str) else {
            return ToolCallResult::error("Missing required parameter: output_path");
        };

        // Validate output path
        if let Err(e) = self.validate_path(output_path) {
            return ToolCallResult::error(e);
        }

        let Some(json_data) = arguments.get("json_data") else {
            return ToolCallResult::error("Missing required parameter: json_data");
        };

        let append = arguments
            .get("append")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Detect file type from JSON data or output path extension
        let file_type = json_data
            .get("file_type")
            .and_then(Value::as_str)
            .map(str::to_lowercase);

        let ext = std::path::Path::new(output_path)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase);

        // Determine library type - prefer JSON file_type, fall back to extension
        let library_type = match (file_type.as_deref(), ext.as_deref()) {
            (Some("pcblib"), _) | (None, Some("pcblib")) => "pcblib",
            (Some("schlib"), _) | (None, Some("schlib")) => "schlib",
            _ => {
                return ToolCallResult::error(
                    "Cannot determine library type. Provide 'file_type' in JSON or use .PcbLib/.SchLib extension.",
                );
            }
        };

        match library_type {
            "pcblib" => Self::import_pcblib(output_path, json_data, append),
            "schlib" => Self::import_schlib(output_path, json_data, append),
            _ => unreachable!(),
        }
    }

    /// Imports footprints from JSON into a `PcbLib` file.
    pub(crate) fn import_pcblib(
        output_path: &str,
        json_data: &Value,
        append: bool,
    ) -> ToolCallResult {
        use crate::altium::pcblib::{Footprint, PcbLib};

        // Get footprints array
        let Some(footprints_json) = json_data.get("footprints").and_then(Value::as_array) else {
            return ToolCallResult::error("JSON data must contain 'footprints' array");
        };

        // If append mode and file exists, read existing library; otherwise create new
        let mut library = if append && std::path::Path::new(output_path).exists() {
            match PcbLib::open(output_path) {
                Ok(lib) => lib,
                Err(e) => {
                    return ToolCallResult::error(format!(
                        "Failed to read existing library for append: {e}"
                    ));
                }
            }
        } else {
            PcbLib::new()
        };

        let mut imported_count = 0;

        // Parse and add each footprint
        for (idx, fp_json) in footprints_json.iter().enumerate() {
            let name = fp_json
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Unnamed");

            // Check for duplicate
            if library.get(name).is_some() {
                return ToolCallResult::error(format!(
                    "Component '{name}' already exists in library"
                ));
            }

            // Use write_pcblib parsing logic via serde
            match serde_json::from_value::<Footprint>(fp_json.clone()) {
                Ok(footprint) => {
                    // Validate fresh geometry before it reaches save (serde
                    // bypasses the create-path validators).
                    if let Err(e) = Self::validate_footprint_coordinates(&footprint) {
                        return ToolCallResult::error(format!("Footprint {idx} ('{name}'): {e}"));
                    }
                    library.add(footprint);
                    imported_count += 1;
                }
                Err(e) => {
                    return ToolCallResult::error(format!(
                        "Failed to parse footprint {idx} ('{name}'): {e}"
                    ));
                }
            }
        }

        if let Err(resp) = Self::backup_then_save(output_path, || library.save(output_path)) {
            return resp;
        }

        let total_count = library.len();
        let mut result = json!({
            "status": "success",
            "output_path": output_path,
            "file_type": "PcbLib",
            "imported_count": imported_count,
            "total_count": total_count,
            "append": append,
            "message": if append {
                format!("Imported {imported_count} footprints (library now has {total_count} total)")
            } else {
                format!("Created library with {imported_count} footprints")
            },
        });

        // Run post-write validation
        if let Some(validation) = Self::post_write_validation_pcblib(output_path) {
            result["validation"] = validation;
        }

        ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
    }

    /// Imports symbols from JSON into a `SchLib` file.
    /// Validates a symbol JSON structure before serde parsing to provide clearer error messages.
    ///
    /// Returns `Ok(())` if validation passes, or an error message with context about
    /// which specific field is missing and in which primitive.
    pub(crate) fn validate_symbol_json(sym_json: &Value) -> Result<(), String> {
        let name = sym_json
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Unnamed");

        // Validate pins have required x/y
        if let Some(pins) = sym_json.get("pins").and_then(Value::as_array) {
            for (pin_idx, pin) in pins.iter().enumerate() {
                let pin_name = pin.get("name").and_then(Value::as_str).unwrap_or("?");
                let pin_designator = pin.get("designator").and_then(Value::as_str).unwrap_or("?");

                if pin.get("x").is_none() {
                    return Err(format!(
                        "Symbol '{name}' pin {pin_idx} (name='{pin_name}', designator='{pin_designator}') missing required field 'x'"
                    ));
                }
                if pin.get("y").is_none() {
                    return Err(format!(
                        "Symbol '{name}' pin {pin_idx} (name='{pin_name}', designator='{pin_designator}') missing required field 'y'"
                    ));
                }
                if pin.get("length").is_none() {
                    return Err(format!(
                        "Symbol '{name}' pin {pin_idx} (name='{pin_name}', designator='{pin_designator}') missing required field 'length'"
                    ));
                }
            }
        }

        // Validate rectangles have required coordinates
        if let Some(rects) = sym_json.get("rectangles").and_then(Value::as_array) {
            for (rect_idx, rect) in rects.iter().enumerate() {
                for field in ["x1", "y1", "x2", "y2"] {
                    if rect.get(field).is_none() {
                        return Err(format!(
                            "Symbol '{name}' rectangle {rect_idx} missing required field '{field}'"
                        ));
                    }
                }
            }
        }

        // Validate lines have required coordinates
        if let Some(lines) = sym_json.get("lines").and_then(Value::as_array) {
            for (line_idx, line) in lines.iter().enumerate() {
                for field in ["x1", "y1", "x2", "y2"] {
                    if line.get(field).is_none() {
                        return Err(format!(
                            "Symbol '{name}' line {line_idx} missing required field '{field}'"
                        ));
                    }
                }
            }
        }

        // Validate arcs have required fields
        if let Some(arcs) = sym_json.get("arcs").and_then(Value::as_array) {
            for (arc_idx, arc) in arcs.iter().enumerate() {
                for field in ["x", "y", "radius"] {
                    if arc.get(field).is_none() {
                        return Err(format!(
                            "Symbol '{name}' arc {arc_idx} missing required field '{field}'"
                        ));
                    }
                }
            }
        }

        // Validate ellipses have required fields
        if let Some(ellipses) = sym_json.get("ellipses").and_then(Value::as_array) {
            for (ellipse_idx, ellipse) in ellipses.iter().enumerate() {
                for field in ["x", "y", "radius_x", "radius_y"] {
                    if ellipse.get(field).is_none() {
                        return Err(format!(
                            "Symbol '{name}' ellipse {ellipse_idx} missing required field '{field}'"
                        ));
                    }
                }
            }
        }

        // Validate labels have required fields
        if let Some(labels) = sym_json.get("labels").and_then(Value::as_array) {
            for (label_idx, label) in labels.iter().enumerate() {
                let label_text = label.get("text").and_then(Value::as_str).unwrap_or("?");
                for field in ["x", "y", "text"] {
                    if label.get(field).is_none() {
                        return Err(format!(
                            "Symbol '{name}' label {label_idx} (text='{label_text}') missing required field '{field}'"
                        ));
                    }
                }
            }
        }

        // Note: parameters now have defaults for x/y/value, so no validation needed

        Ok(())
    }

    pub(crate) fn import_schlib(
        output_path: &str,
        json_data: &Value,
        append: bool,
    ) -> ToolCallResult {
        use crate::altium::schlib::Symbol;
        use crate::altium::SchLib;

        // Get symbols array
        let Some(symbols_json) = json_data.get("symbols").and_then(Value::as_array) else {
            return ToolCallResult::error("JSON data must contain 'symbols' array");
        };

        // If append mode and file exists, read existing library; otherwise create new
        let mut library = if append && std::path::Path::new(output_path).exists() {
            match SchLib::open(output_path) {
                Ok(lib) => lib,
                Err(e) => {
                    return ToolCallResult::error(format!(
                        "Failed to read existing library for append: {e}"
                    ));
                }
            }
        } else {
            SchLib::new()
        };

        let mut imported_count = 0;

        // Parse and add each symbol
        for (idx, sym_json) in symbols_json.iter().enumerate() {
            let name = sym_json
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Unnamed");

            // Check for duplicate
            if library.get(name).is_some() {
                return ToolCallResult::error(format!(
                    "Component '{name}' already exists in library"
                ));
            }

            // Validate symbol structure before serde parsing for better error messages
            if let Err(e) = Self::validate_symbol_json(sym_json) {
                return ToolCallResult::error(e);
            }

            // Parse symbol via serde
            match serde_json::from_value::<Symbol>(sym_json.clone()) {
                Ok(symbol) => {
                    // Range-validate fresh geometry (validate_symbol_json only
                    // checks presence; serde bypasses the create validators).
                    if let Err(e) = Self::validate_symbol_coordinates(&symbol) {
                        return ToolCallResult::error(format!("Symbol {idx} ('{name}'): {e}"));
                    }
                    library.add(symbol);
                    imported_count += 1;
                }
                Err(e) => {
                    return ToolCallResult::error(format!(
                        "Failed to parse symbol {idx} ('{name}'): {e}"
                    ));
                }
            }
        }

        if let Err(resp) = Self::backup_then_save(output_path, || library.save(output_path)) {
            return resp;
        }

        let total_count = library.len();
        let mut result = json!({
            "status": "success",
            "output_path": output_path,
            "file_type": "SchLib",
            "imported_count": imported_count,
            "total_count": total_count,
            "append": append,
            "message": if append {
                format!("Imported {imported_count} symbols (library now has {total_count} total)")
            } else {
                format!("Created library with {imported_count} symbols")
            },
        });

        // Run post-write validation
        if let Some(validation) = Self::post_write_validation_schlib(output_path) {
            result["validation"] = validation;
        }

        ToolCallResult::text(serde_json::to_string_pretty(&result).unwrap())
    }
}

#[cfg(test)]
mod tests {

    use crate::altium::pcblib::{Footprint, PcbLib};
    use crate::altium::schlib::{Pin, PinOrientation, SchLib, Symbol};
    use crate::mcp::server::McpServer;
    use crate::mcp::tools::test_support::{
        create_test_pcblib, create_test_schlib, create_test_server, get_result_text,
        parse_result_json, test_temp_dir,
    };
    use serde_json::json;

    // ==================== delete_component ====================

    #[test]
    fn delete_component_pcblib_success_and_not_found() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());
        let path = dir.path().join("Del.PcbLib");
        create_test_pcblib(&path);

        let result = server.call_delete_component(&json!({
            "filepath": path.to_string_lossy(),
            "component_names": ["CHIP_0402", "GHOST"],
        }));
        assert!(!result.is_error, "{}", get_result_text(&result));
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["file_type"], "PcbLib");
        assert_eq!(parsed["original_count"], 2);
        assert_eq!(parsed["deleted_count"], 1);
        assert_eq!(parsed["remaining_count"], 1);
        assert_eq!(parsed["results"][0]["name"], "CHIP_0402");
        assert_eq!(parsed["results"][0]["status"], "deleted");
        assert_eq!(parsed["results"][1]["name"], "GHOST");
        assert_eq!(parsed["results"][1]["status"], "not_found");

        let lib = PcbLib::open(&path).unwrap();
        assert!(lib.get("CHIP_0402").is_none());
        assert!(lib.get("CHIP_0603").is_some());
    }

    #[test]
    fn delete_component_dry_run_leaves_file_untouched() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());
        let path = dir.path().join("DelDry.PcbLib");
        create_test_pcblib(&path);

        let result = server.call_delete_component(&json!({
            "filepath": path.to_string_lossy(),
            "component_names": ["CHIP_0402"],
            "dry_run": true,
        }));
        assert!(!result.is_error, "{}", get_result_text(&result));
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["status"], "dry_run");
        assert_eq!(parsed["results"][0]["status"], "would_delete");
        assert_eq!(parsed["remaining_count"], 1);

        let lib = PcbLib::open(&path).unwrap();
        assert_eq!(lib.len(), 2, "dry run must not modify the library");
    }

    #[test]
    fn delete_component_schlib_success() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());
        let path = dir.path().join("Del.SchLib");
        create_test_schlib(&path);

        let result = server.call_delete_component(&json!({
            "filepath": path.to_string_lossy(),
            "component_names": ["CAPACITOR"],
        }));
        assert!(!result.is_error, "{}", get_result_text(&result));
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["file_type"], "SchLib");
        assert_eq!(parsed["deleted_count"], 1);
        assert_eq!(parsed["remaining_count"], 1);

        let lib = SchLib::open(&path).unwrap();
        assert!(lib.get("CAPACITOR").is_none());
    }

    #[test]
    fn delete_component_rejects_bad_arguments() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());

        let result = server.call_delete_component(&json!({}));
        assert!(result.is_error);
        assert_eq!(
            get_result_text(&result),
            "Missing required parameter: filepath"
        );

        let path = dir.path().join("DelBad.PcbLib");
        create_test_pcblib(&path);
        let result = server.call_delete_component(&json!({
            "filepath": path.to_string_lossy(),
            "component_names": [],
        }));
        assert!(result.is_error);
        assert!(get_result_text(&result).contains("empty"));

        let txt = dir.path().join("x.txt");
        let result = server.call_delete_component(&json!({
            "filepath": txt.to_string_lossy(),
            "component_names": ["A"],
        }));
        assert!(result.is_error);
        assert!(get_result_text(&result).contains("Unknown file type"));
    }

    // ==================== validate_library ====================

    #[test]
    fn validate_library_pcblib_clean_fixture_is_valid() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());
        let path = dir.path().join("Valid.PcbLib");
        create_test_pcblib(&path);

        let result = server.call_validate_library(&json!({
            "filepath": path.to_string_lossy(),
        }));
        assert!(!result.is_error, "{}", get_result_text(&result));
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["status"], "valid");
        assert_eq!(parsed["component_count"], 2);
        assert_eq!(parsed["error_count"], 0);
        assert_eq!(parsed["warning_count"], 0);
        assert_eq!(parsed["issues"], json!([]));
    }

    #[test]
    fn validate_library_pcblib_reports_warnings() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());

        // Footprint with no pads at all → warning.
        let mut lib = PcbLib::new();
        lib.add(Footprint::new("BARE"));
        let path = dir.path().join("Warn.PcbLib");
        lib.save(&path).unwrap();

        let result = server.call_validate_library(&json!({
            "filepath": path.to_string_lossy(),
        }));
        assert!(!result.is_error);
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["status"], "warnings");
        assert_eq!(parsed["warning_count"], 1);
        assert_eq!(parsed["issues"][0]["severity"], "warning");
        assert_eq!(parsed["issues"][0]["issue"], "Footprint has no pads");
    }

    #[test]
    fn validate_library_schlib_reports_duplicate_pin_designators() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());

        let mut lib = SchLib::new();
        let mut sym = Symbol::new("BROKEN");
        sym.add_pin(Pin::new("1", "1", -20, 0, 10, PinOrientation::Left));
        sym.add_pin(Pin::new("1", "1", 20, 0, 10, PinOrientation::Right));
        lib.add(sym);
        let path = dir.path().join("Dup.SchLib");
        lib.save(&path).unwrap();

        let result = server.call_validate_library(&json!({
            "filepath": path.to_string_lossy(),
        }));
        assert!(!result.is_error);
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["status"], "invalid");
        assert!(parsed["error_count"].as_u64().unwrap() >= 1);
        let issues = parsed["issues"].as_array().unwrap();
        assert!(issues
            .iter()
            .any(|i| i["issue"] == "Duplicate pin designator: '1'"));
        // The symbol has pins but no body graphics → also a warning.
        assert!(issues.iter().any(|i| i["severity"] == "warning"));
    }

    #[test]
    fn validate_library_unreadable_file_is_an_error() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());
        let missing = dir.path().join("Missing.SchLib");

        let result = server.call_validate_library(&json!({
            "filepath": missing.to_string_lossy(),
        }));
        assert!(result.is_error);
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["status"], "error");
    }

    // ==================== export_library ====================

    #[test]
    fn export_library_pcblib_json_and_csv() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());
        let path = dir.path().join("Export.PcbLib");
        create_test_pcblib(&path);
        let filepath = path.to_string_lossy().to_string();

        let result = server.call_export_library(&json!({
            "filepath": filepath,
            "format": "json",
        }));
        assert!(!result.is_error, "{}", get_result_text(&result));
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["format"], "json");
        assert_eq!(parsed["units"], "mm");
        assert_eq!(parsed["component_count"], 2);
        let footprints = parsed["footprints"].as_array().unwrap();
        assert_eq!(footprints.len(), 2);
        assert_eq!(footprints[0]["name"], "CHIP_0402");
        assert_eq!(footprints[0]["pads"].as_array().unwrap().len(), 2);

        let result = server.call_export_library(&json!({
            "filepath": filepath,
            "format": "csv",
        }));
        assert!(!result.is_error);
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["format"], "csv");
        let csv = parsed["csv"].as_str().unwrap();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3, "header + 2 rows");
        assert!(lines[0].starts_with("name,description,pad_count"));
        assert!(lines[1].starts_with("CHIP_0402,0402 chip resistor,2,"));
    }

    #[test]
    fn export_library_schlib_json_and_csv() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());
        let path = dir.path().join("Export.SchLib");
        create_test_schlib(&path);
        let filepath = path.to_string_lossy().to_string();

        let result = server.call_export_library(&json!({
            "filepath": filepath,
            "format": "json",
        }));
        assert!(!result.is_error, "{}", get_result_text(&result));
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["file_type"], "SchLib");
        assert_eq!(parsed["component_count"], 2);
        let symbols = parsed["symbols"].as_array().unwrap();
        assert_eq!(symbols[0]["name"], "RESISTOR");
        assert_eq!(symbols[0]["designator"], "R?");
        assert_eq!(symbols[0]["pins"].as_array().unwrap().len(), 2);

        let result = server.call_export_library(&json!({
            "filepath": filepath,
            "format": "csv",
        }));
        assert!(!result.is_error);
        let parsed = parse_result_json(&result);
        let csv = parsed["csv"].as_str().unwrap();
        assert!(csv
            .lines()
            .any(|l| l.starts_with("RESISTOR,Generic resistor,R?,2,1,")));
    }

    #[test]
    fn export_library_rejects_bad_arguments() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());
        let path = dir.path().join("ExportBad.PcbLib");
        create_test_pcblib(&path);

        let result = server.call_export_library(&json!({
            "filepath": path.to_string_lossy(),
        }));
        assert!(result.is_error);
        assert_eq!(
            get_result_text(&result),
            "Missing required parameter: format"
        );

        let result = server.call_export_library(&json!({
            "filepath": path.to_string_lossy(),
            "format": "xml",
        }));
        assert!(result.is_error);
        assert!(get_result_text(&result).contains("Expected 'json' or 'csv'"));
    }

    // ==================== import_library ====================

    #[test]
    fn import_library_pcblib_round_trips_an_export() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());
        let source = dir.path().join("ImpSource.PcbLib");
        create_test_pcblib(&source);

        // Export the fixture and import the payload into a new library.
        let exported = parse_result_json(&server.call_export_library(&json!({
            "filepath": source.to_string_lossy(),
            "format": "json",
        })));

        let output = dir.path().join("ImpTarget.PcbLib");
        let result = server.call_import_library(&json!({
            "output_path": output.to_string_lossy(),
            "json_data": {
                "file_type": "PcbLib",
                "footprints": exported["footprints"],
            },
        }));
        assert!(!result.is_error, "{}", get_result_text(&result));
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["status"], "success");
        assert_eq!(parsed["imported_count"], 2);
        assert_eq!(parsed["total_count"], 2);
        assert_eq!(parsed["append"], false);

        let lib = PcbLib::open(&output).unwrap();
        let fp = lib.get("CHIP_0402").unwrap();
        assert_eq!(fp.pads.len(), 2);
        assert_eq!(fp.description, "0402 chip resistor");
    }

    #[test]
    fn import_library_schlib_append_mode() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());
        let output = dir.path().join("ImpAppend.SchLib");
        create_test_schlib(&output);

        let result = server.call_import_library(&json!({
            "output_path": output.to_string_lossy(),
            "json_data": {
                "file_type": "SchLib",
                "symbols": [{
                    "name": "DIODE",
                    "description": "Generic diode",
                    "designator": "D?",
                    "pins": [
                        { "designator": "1", "name": "A", "x": -20, "y": 0, "length": 10 },
                        { "designator": "2", "name": "K", "x": 20, "y": 0, "length": 10 }
                    ],
                }],
            },
            "append": true,
        }));
        assert!(!result.is_error, "{}", get_result_text(&result));
        let parsed = parse_result_json(&result);
        assert_eq!(parsed["imported_count"], 1);
        assert_eq!(parsed["total_count"], 3);
        assert_eq!(parsed["append"], true);

        let lib = SchLib::open(&output).unwrap();
        assert_eq!(lib.len(), 3);
        assert_eq!(lib.get("DIODE").unwrap().pins.len(), 2);
    }

    #[test]
    fn import_library_error_paths() {
        let dir = test_temp_dir();
        let server = create_test_server(dir.path());

        let result = server.call_import_library(&json!({}));
        assert!(result.is_error);
        assert_eq!(
            get_result_text(&result),
            "Missing required parameter: output_path"
        );

        let output = dir.path().join("ImpErr.PcbLib");
        let result = server.call_import_library(&json!({
            "output_path": output.to_string_lossy(),
        }));
        assert!(result.is_error);
        assert_eq!(
            get_result_text(&result),
            "Missing required parameter: json_data"
        );

        // Payload without a footprints array.
        let result = server.call_import_library(&json!({
            "output_path": output.to_string_lossy(),
            "json_data": { "file_type": "PcbLib" },
        }));
        assert!(result.is_error);
        assert!(get_result_text(&result).contains("'footprints' array"));

        // Indeterminate library type.
        let unknown = dir.path().join("ImpErr.dat");
        let result = server.call_import_library(&json!({
            "output_path": unknown.to_string_lossy(),
            "json_data": { "footprints": [] },
        }));
        assert!(result.is_error);
        assert!(get_result_text(&result).contains("Cannot determine library type"));

        // Symbol JSON validation catches a pin without coordinates.
        let sch_out = dir.path().join("ImpErr.SchLib");
        let result = server.call_import_library(&json!({
            "output_path": sch_out.to_string_lossy(),
            "json_data": {
                "symbols": [{
                    "name": "BAD",
                    "pins": [{ "designator": "1", "name": "A", "y": 0, "length": 10 }],
                }],
            },
        }));
        assert!(result.is_error);
        assert!(get_result_text(&result).contains("missing required field 'x'"));
    }

    // ==================== validate / export / import success paths ====================

    mod more_coverage {
        use super::*;
        use crate::altium::pcblib::Pad;
        use crate::altium::schlib::Rectangle;

        #[test]
        fn validate_pcblib_reports_pad_errors() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut lib = PcbLib::new();
            let mut fp = Footprint::new("BAD");
            fp.add_pad(Pad::smd("1", -0.5, 0.0, 0.6, 0.5));
            let mut dup = Pad::smd("1", 0.5, 0.0, 0.6, 0.5); // duplicate designator
            dup.width = 0.0; // invalid dimensions
            fp.add_pad(dup);
            fp.add_pad(Pad::smd("", 0.0, 0.5, 0.6, 0.5)); // empty designator
            lib.add(fp);
            let path = dir.path().join("PcbErrors.PcbLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(
                &server.call_validate_library(&json!({ "filepath": path.to_string_lossy() })),
            );
            assert_eq!(parsed["status"], "invalid");
            let issues = parsed["issues"].as_array().unwrap();
            assert!(issues
                .iter()
                .any(|i| i["issue"] == "Duplicate pad designator: '1'"));
            assert!(issues
                .iter()
                .any(|i| i["issue"] == "Pad has empty designator"));
            assert!(issues.iter().any(|i| i["issue"]
                .as_str()
                .unwrap_or("")
                .contains("invalid dimensions")));
            assert!(parsed["error_count"].as_u64().unwrap() >= 3);
        }

        #[test]
        fn validate_pcblib_caps_pad_overlap_issues() {
            use crate::altium::pcblib::MAX_REPORTED_PAD_OVERLAPS;

            // Overlapping pairs are quadratic in pad count, so a systematically
            // broken part must not bury the response: 30 mutually overlapping
            // pads are 435 pairs, which has to collapse to the cap plus one
            // summary line carrying the true total.
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut lib = PcbLib::new();
            let mut fp = Footprint::new("SHORTED");
            for n in 0..30 {
                fp.add_pad(Pad::smd(format!("{n}"), f64::from(n) * 0.01, 0.0, 1.0, 1.0));
            }
            lib.add(fp);
            let path = dir.path().join("Shorted.PcbLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(
                &server.call_validate_library(&json!({ "filepath": path.to_string_lossy() })),
            );
            let overlap_issues: Vec<&str> = parsed["issues"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|i| i["issue"].as_str())
                .filter(|s| s.contains("overlap"))
                .collect();
            assert_eq!(
                overlap_issues.len(),
                MAX_REPORTED_PAD_OVERLAPS + 1,
                "expected {MAX_REPORTED_PAD_OVERLAPS} pairs plus a summary, got: {overlap_issues:?}"
            );
            assert!(
                overlap_issues
                    .last()
                    .unwrap()
                    .starts_with("435 overlapping pad pairs total"),
                "summary must carry the true total: {overlap_issues:?}"
            );
        }

        #[test]
        fn validate_pcblib_empty_library_is_warning() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Empty.PcbLib");
            PcbLib::new().save(&path).unwrap();

            let parsed = parse_result_json(
                &server.call_validate_library(&json!({ "filepath": path.to_string_lossy() })),
            );
            assert_eq!(parsed["status"], "warnings");
            assert_eq!(
                parsed["issues"][0]["issue"],
                "Library is empty (no footprints)"
            );
        }

        #[test]
        fn validate_schlib_reports_pin_length_and_inverted_rect() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut lib = SchLib::new();
            let mut sym = Symbol::new("W");
            sym.add_pin(Pin::new("A", "1", -20, 0, 0, PinOrientation::Left)); // length 0
            sym.add_rectangle(Rectangle::new(10, 5, -10, -5)); // inverted corners
            lib.add(sym);
            let path = dir.path().join("SchWarn.SchLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(
                &server.call_validate_library(&json!({ "filepath": path.to_string_lossy() })),
            );
            assert_eq!(parsed["status"], "warnings");
            let issues = parsed["issues"].as_array().unwrap();
            assert!(issues.iter().any(|i| i["issue"]
                .as_str()
                .unwrap_or("")
                .contains("zero or negative length")));
            assert!(issues.iter().any(|i| i["issue"]
                .as_str()
                .unwrap_or("")
                .contains("inverted corners")));
        }

        #[test]
        fn export_pcblib_json_non_compact_keeps_detail() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Exp.PcbLib");
            create_test_pcblib(&path);

            let parsed = parse_result_json(&server.call_export_library(&json!({
                "filepath": path.to_string_lossy(),
                "format": "json",
                "compact": false,
            })));
            assert_eq!(parsed["status"], "success");
            assert_eq!(parsed["footprints"].as_array().unwrap().len(), 2);
        }

        #[test]
        fn import_schlib_rejects_incomplete_rectangle() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let out = dir.path().join("ImpRect.SchLib");
            let result = server.call_import_library(&json!({
                "output_path": out.to_string_lossy(),
                "json_data": {
                    "file_type": "SchLib",
                    "symbols": [{
                        "name": "S",
                        "pins": [{ "designator": "1", "name": "A", "x": -20, "y": 0, "length": 10 }],
                        "rectangles": [{ "x1": 0, "y1": 0, "x2": 10 }],
                    }],
                },
            }));
            assert!(result.is_error);
            assert!(get_result_text(&result).contains("rectangle 0 missing required field 'y2'"));
        }

        #[test]
        fn import_schlib_creates_new_library() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let out = dir.path().join("NewSch.SchLib");
            let result = server.call_import_library(&json!({
                "output_path": out.to_string_lossy(),
                "json_data": {
                    "file_type": "SchLib",
                    "symbols": [{
                        "name": "LED",
                        "designator": "D?",
                        "pins": [
                            { "designator": "1", "name": "A", "x": -20, "y": 0, "length": 10 },
                            { "designator": "2", "name": "K", "x": 20, "y": 0, "length": 10 },
                        ],
                    }],
                },
            }));
            assert!(!result.is_error, "{}", get_result_text(&result));
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["imported_count"], 1);
            assert_eq!(parsed["total_count"], 1);
            assert_eq!(parsed["append"], false);
            assert!(parsed.get("validation").is_some());
            assert!(SchLib::open(&out).unwrap().get("LED").is_some());
        }

        #[test]
        fn delete_component_surfaces_post_write_validation() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut lib = PcbLib::new();
            lib.add(Footprint::new("GOOD")); // deleted below
            let mut bad = Footprint::new("BAD");
            bad.add_pad(Pad::smd("1", 0.0, 0.0, 0.6, 0.5));
            bad.add_pad(Pad::smd("1", 0.5, 0.0, 0.6, 0.5)); // duplicate designator remains
            lib.add(bad);
            let path = dir.path().join("PostVal.PcbLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(&server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["GOOD"],
            })));
            assert_eq!(parsed["status"], "success");
            assert_eq!(parsed["validation"]["status"], "invalid");
            assert!(parsed["validation"]["error_count"].as_u64().unwrap() >= 1);
        }
    }
    /// `validate_symbol_json` gates every graphic primitive `write_schlib`
    /// accepts, and none of its rejection branches had coverage: a symbol whose
    /// pins or shapes were missing required coordinates would reach the writer
    /// on the strength of untested guards. Walks every family and every required
    /// field, so a guard that is dropped or mis-keyed fails here.
    #[test]
    fn validate_symbol_json_rejects_every_missing_required_field() {
        use serde_json::json;

        let ok = |v: &serde_json::Value| McpServer::validate_symbol_json(v);

        // A complete symbol passes.
        assert!(ok(&json!({
            "name": "GOOD",
            "pins": [{"designator": "1", "name": "A", "x": 0, "y": 0, "length": 20}],
            "rectangles": [{"x1": 0, "y1": 0, "x2": 1, "y2": 1}],
            "lines": [{"x1": 0, "y1": 0, "x2": 1, "y2": 1}],
            "arcs": [{"x": 0, "y": 0, "radius": 1}],
            "ellipses": [{"x": 0, "y": 0, "radius_x": 1, "radius_y": 1}],
            "labels": [{"x": 0, "y": 0, "text": "L"}],
        }))
        .is_ok());

        // Each family, each required field: dropping it must be reported, and the
        // message must name the symbol, the primitive index and the field.
        let cases: Vec<(&str, serde_json::Value, &str)> = vec![
            (
                "pins",
                json!({"designator": "1", "name": "A", "y": 0, "length": 20}),
                "x",
            ),
            (
                "pins",
                json!({"designator": "1", "name": "A", "x": 0, "length": 20}),
                "y",
            ),
            (
                "pins",
                json!({"designator": "1", "name": "A", "x": 0, "y": 0}),
                "length",
            ),
            ("rectangles", json!({"y1": 0, "x2": 1, "y2": 1}), "x1"),
            ("rectangles", json!({"x1": 0, "x2": 1, "y2": 1}), "y1"),
            ("rectangles", json!({"x1": 0, "y1": 0, "y2": 1}), "x2"),
            ("rectangles", json!({"x1": 0, "y1": 0, "x2": 1}), "y2"),
            ("lines", json!({"y1": 0, "x2": 1, "y2": 1}), "x1"),
            ("lines", json!({"x1": 0, "x2": 1, "y2": 1}), "y1"),
            ("lines", json!({"x1": 0, "y1": 0, "y2": 1}), "x2"),
            ("lines", json!({"x1": 0, "y1": 0, "x2": 1}), "y2"),
            ("arcs", json!({"y": 0, "radius": 1}), "x"),
            ("arcs", json!({"x": 0, "radius": 1}), "y"),
            ("arcs", json!({"x": 0, "y": 0}), "radius"),
            (
                "ellipses",
                json!({"y": 0, "radius_x": 1, "radius_y": 1}),
                "x",
            ),
            (
                "ellipses",
                json!({"x": 0, "radius_x": 1, "radius_y": 1}),
                "y",
            ),
            (
                "ellipses",
                json!({"x": 0, "y": 0, "radius_y": 1}),
                "radius_x",
            ),
            (
                "ellipses",
                json!({"x": 0, "y": 0, "radius_x": 1}),
                "radius_y",
            ),
            ("labels", json!({"y": 0, "text": "L"}), "x"),
            ("labels", json!({"x": 0, "text": "L"}), "y"),
            ("labels", json!({"x": 0, "y": 0}), "text"),
        ];
        for (family, primitive, missing) in cases {
            let sym = json!({ "name": "SYM", family: [primitive] });
            let err = ok(&sym).expect_err(&format!("{family} missing {missing} must fail"));
            // Match the quoted field name, not the bare one: the message text
            // itself contains most single letters, so `contains("y")` is
            // satisfied by the 'y' in "Symbol" and `contains("x")` by the 'x'
            // in a label's "(text='L')" — both would pass even if the guard
            // named the wrong field.
            assert!(
                err.contains("SYM") && err.contains(&format!("'{missing}'")),
                "{family}/{missing}: unhelpful message {err:?}"
            );
        }
    }

    /// The identifier fallbacks in the same function: an unnamed symbol reports
    /// as 'Unnamed' and an unlabelled pin as '?', so a failure is still
    /// attributable when the caller omitted the descriptive fields.
    #[test]
    fn validate_symbol_json_falls_back_to_placeholder_identifiers() {
        use serde_json::json;

        let err = McpServer::validate_symbol_json(&json!({
            "pins": [{"y": 0, "length": 20}]
        }))
        .expect_err("missing x must fail");
        assert!(err.contains("Unnamed"), "symbol name fallback: {err}");
        assert!(err.contains("name='?'"), "pin name fallback: {err}");
        assert!(err.contains("designator='?'"), "designator fallback: {err}");

        // Label text fallback is reported the same way.
        let err = McpServer::validate_symbol_json(&json!({
            "name": "S", "labels": [{"y": 0, "text": "T"}]
        }))
        .expect_err("missing x must fail");
        assert!(err.contains("text='T'"), "label text echoed: {err}");
    }

    /// Empty primitive arrays and absent keys are both valid - a symbol need not
    /// carry every family. Guards against a future `is_empty()` check turning an
    /// ordinary pins-only symbol into an error.
    #[test]
    fn validate_symbol_json_accepts_absent_and_empty_families() {
        use serde_json::json;

        assert!(McpServer::validate_symbol_json(&json!({"name": "BARE"})).is_ok());
        assert!(McpServer::validate_symbol_json(&json!({
            "name": "EMPTY",
            "pins": [], "rectangles": [], "lines": [],
            "arcs": [], "ellipses": [], "labels": [],
        }))
        .is_ok());
    }

    // ==================== rejection and failure paths ====================

    mod error_paths {
        use super::*;
        use crate::altium::pcblib::primitives::{Arc, Layer, PadStackMode, Region, Track, Vertex};
        use crate::altium::pcblib::Pad;
        use crate::altium::schlib::Rectangle;
        use std::path::{Path, PathBuf};

        /// Writes bytes that are not an OLE compound document, standing in for
        /// a truncated or transfer-mangled library file.
        fn write_corrupt_file(path: &Path) {
            std::fs::write(path, b"not an OLE compound document").expect("write corrupt file");
        }

        /// Occupies the timestamped backup names `create_backup` is about to
        /// try, so the copy fails. Covers this second and the next two, which
        /// the call under test cannot outrun.
        fn block_backup_paths(path: &Path) {
            let now = chrono::Local::now();
            for offset in 0..3_i64 {
                let stamp = (now + chrono::Duration::seconds(offset)).format("%Y%m%d_%H%M%S");
                let _ = std::fs::create_dir(format!("{}.{stamp}.bak", path.display()));
            }
        }

        /// Clears or sets the read-only attribute, ignoring failures on paths
        /// that have already gone away.
        #[allow(clippy::permissions_set_readonly_false)] // cleanup: the temp dir must stay deletable
        fn set_readonly(path: &Path, readonly: bool) {
            if let Ok(meta) = std::fs::metadata(path) {
                let mut perms = meta.permissions();
                perms.set_readonly(readonly);
                let _ = std::fs::set_permissions(path, perms);
            }
        }

        /// Makes a library file read-only for the guard's lifetime, so the next
        /// write fails the way it does when Altium holds the file open or the
        /// library lives on a read-only share. Write access is restored on
        /// drop — including on the backup copies, which inherit the attribute
        /// on Windows and would otherwise block temp-directory cleanup.
        struct ReadOnlyFile(PathBuf);

        impl ReadOnlyFile {
            fn new(path: &Path) -> Self {
                set_readonly(path, true);
                assert!(
                    std::fs::metadata(path)
                        .expect("metadata")
                        .permissions()
                        .readonly(),
                    "test setup: could not make the library read-only"
                );
                Self(path.to_path_buf())
            }
        }

        impl Drop for ReadOnlyFile {
            fn drop(&mut self) {
                let prefix = self.0.to_string_lossy().into_owned();
                let parent = self
                    .0
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();
                for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
                    if entry.path().to_string_lossy().starts_with(&prefix) {
                        set_readonly(&entry.path(), false);
                    }
                }
            }
        }

        // ---------- delete_component ----------

        // The sandbox check has to run before the library is opened. Were it
        // dropped, any MCP client could name a .PcbLib anywhere on the machine
        // and have footprints deleted out of it.
        #[test]
        fn delete_component_rejects_path_outside_allowed_roots() {
            let allowed = test_temp_dir();
            let elsewhere = test_temp_dir();
            let server = create_test_server(allowed.path());
            let outside = elsewhere.path().join("Outside.PcbLib");
            create_test_pcblib(&outside);

            let result = server.call_delete_component(&json!({
                "filepath": outside.to_string_lossy(),
                "component_names": ["CHIP_0402"],
            }));
            assert!(result.is_error);
            assert_eq!(
                get_result_text(&result),
                "Access denied: path is outside the configured allowed directories"
            );
            assert_eq!(
                PcbLib::open(&outside).unwrap().len(),
                2,
                "a refused delete must leave the library untouched"
            );
        }

        // component_names is the whole instruction. A request that omits it
        // must be rejected by name, not silently treated as "delete nothing"
        // (an operator would think the delete ran) or "delete everything".
        #[test]
        fn delete_component_requires_component_names() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("NoNames.PcbLib");
            create_test_pcblib(&path);

            let result = server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
            }));
            assert!(result.is_error);
            assert_eq!(
                get_result_text(&result),
                "Missing required parameter: component_names"
            );
        }

        // A .PcbLib the reader cannot parse must be reported as unreadable. If
        // it were treated as an empty library, the delete would "succeed" and
        // the save that follows would overwrite the file with nothing.
        #[test]
        fn delete_component_pcblib_reports_a_library_it_cannot_read() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Corrupt.PcbLib");
            write_corrupt_file(&path);

            let result = server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["CHIP_0402"],
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
            assert_eq!(
                parsed["filepath"], "Corrupt.PcbLib",
                "the response must name the library that failed, by file name only"
            );
            assert!(
                !parsed["error"].as_str().unwrap().is_empty(),
                "the reader's reason must be carried through, got: {parsed}"
            );
        }

        // Same guard on the SchLib side, which reads through a separate path: a
        // symbol library that cannot be parsed must fail loudly rather than be
        // rewritten from an empty in-memory library.
        #[test]
        fn delete_component_schlib_reports_a_library_it_cannot_read() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Corrupt.SchLib");
            write_corrupt_file(&path);

            let result = server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["RESISTOR"],
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
            assert_eq!(parsed["filepath"], "Corrupt.SchLib");
            assert!(!parsed["error"].as_str().unwrap().is_empty());
        }

        // Dry-run is what an operator reads before approving a delete. A name
        // that does not exist has to come back as not_found: reported as
        // would_delete, they would approve a list that then removes less than
        // they were shown.
        #[test]
        fn delete_component_pcblib_dry_run_flags_names_that_do_not_exist() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("DryGhost.PcbLib");
            create_test_pcblib(&path);

            let parsed = parse_result_json(&server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["CHIP_0402", "GHOST"],
                "dry_run": true,
            })));
            assert_eq!(parsed["status"], "dry_run");
            assert_eq!(parsed["results"][0]["name"], "CHIP_0402");
            assert_eq!(parsed["results"][0]["status"], "would_delete");
            assert_eq!(parsed["results"][1]["name"], "GHOST");
            assert_eq!(parsed["results"][1]["status"], "not_found");
            assert_eq!(parsed["deleted_count"], 1);
            assert_eq!(parsed["remaining_count"], 1);
        }

        // The SchLib dry-run keeps its own copy of the counting logic. A name
        // repeated in the request must count once, or the previewed
        // remaining_count undershoots what the real delete actually leaves.
        #[test]
        fn delete_component_schlib_dry_run_counts_repeated_names_once() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("DryDup.SchLib");
            create_test_schlib(&path);

            let parsed = parse_result_json(&server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["RESISTOR", "RESISTOR", "GHOST"],
                "dry_run": true,
            })));
            assert_eq!(parsed["status"], "dry_run");
            assert_eq!(parsed["file_type"], "SchLib");
            assert_eq!(parsed["results"][0]["status"], "would_delete");
            assert_eq!(parsed["results"][1]["status"], "would_delete");
            assert_eq!(parsed["results"][2]["name"], "GHOST");
            assert_eq!(parsed["results"][2]["status"], "not_found");
            assert_eq!(
                parsed["deleted_count"], 1,
                "the repeat must not double-count"
            );
            assert_eq!(parsed["remaining_count"], 1);

            assert_eq!(
                SchLib::open(&path).unwrap().len(),
                2,
                "dry run must not modify the library"
            );
        }

        // A SchLib delete has to distinguish a symbol it removed from one it
        // never found. Reporting "deleted" for a name that was not there tells
        // the operator an obsolete symbol is gone while it is still shipped.
        #[test]
        fn delete_component_schlib_reports_names_that_do_not_exist() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("SchGhost.SchLib");
            create_test_schlib(&path);

            let parsed = parse_result_json(&server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["GHOST"],
            })));
            assert_eq!(parsed["status"], "success");
            assert_eq!(parsed["results"][0]["name"], "GHOST");
            assert_eq!(parsed["results"][0]["status"], "not_found");
            assert_eq!(parsed["deleted_count"], 0);
            assert_eq!(
                SchLib::open(&path).unwrap().len(),
                2,
                "a delete that found nothing must not rewrite the file"
            );
        }

        // The timestamped backup is the only recovery point before a
        // destructive delete. If a failed backup stopped aborting the
        // operation, the delete would go ahead with nothing to restore from.
        #[test]
        fn delete_component_pcblib_aborts_when_the_backup_cannot_be_written() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("NoBackup.PcbLib");
            create_test_pcblib(&path);
            block_backup_paths(&path);

            let result = server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["CHIP_0402"],
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
            assert!(
                parsed["error"]
                    .as_str()
                    .unwrap()
                    .starts_with("Failed to create backup of 'NoBackup.PcbLib'"),
                "the abort must name the library it could not back up, got: {parsed}"
            );
            assert_eq!(
                PcbLib::open(&path).unwrap().len(),
                2,
                "no backup means no delete"
            );
        }

        // Same guard on the SchLib path, which has its own copy of the
        // backup-then-save sequence.
        #[test]
        fn delete_component_schlib_aborts_when_the_backup_cannot_be_written() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("NoBackup.SchLib");
            create_test_schlib(&path);
            block_backup_paths(&path);

            let result = server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["RESISTOR"],
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
            assert!(
                parsed["error"]
                    .as_str()
                    .unwrap()
                    .starts_with("Failed to create backup of 'NoBackup.SchLib'"),
                "the abort must name the library it could not back up, got: {parsed}"
            );
            assert_eq!(
                SchLib::open(&path).unwrap().len(),
                2,
                "no backup, no delete"
            );
        }

        // A library Altium has open (or one on a read-only share) cannot be
        // written. The handler must report the failure and still list what it
        // tried to remove — a bare error leaves the operator unable to tell
        // whether the file on disk changed.
        #[test]
        fn delete_component_pcblib_reports_a_failed_write() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Locked.PcbLib");
            create_test_pcblib(&path);
            let _readonly = ReadOnlyFile::new(&path);

            let result = server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["CHIP_0402"],
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
            assert!(
                parsed["error"]
                    .as_str()
                    .unwrap()
                    .starts_with("Failed to write library"),
                "got: {parsed}"
            );
            assert_eq!(parsed["results"][0]["name"], "CHIP_0402");
            assert_eq!(
                parsed["results"][0]["status"], "deleted",
                "the attempted deletions must stay in the report"
            );
        }

        // Same for SchLib: the write failure must not be reported as success,
        // or a user would delete their symbol and only find out at the next
        // open that it is still there.
        #[test]
        fn delete_component_schlib_reports_a_failed_write() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Locked.SchLib");
            create_test_schlib(&path);
            let _readonly = ReadOnlyFile::new(&path);

            let result = server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["RESISTOR"],
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
            assert!(
                parsed["error"]
                    .as_str()
                    .unwrap()
                    .starts_with("Failed to write library"),
                "got: {parsed}"
            );
            assert_eq!(parsed["results"][0]["name"], "RESISTOR");
            assert_eq!(parsed["results"][0]["status"], "deleted");
        }

        // ---------- validate_library ----------

        // validate_library opens whatever path it is handed, so the sandbox
        // check doubles as the guard that stops it being used to probe for
        // files elsewhere on the machine.
        #[test]
        fn validate_library_rejects_path_outside_allowed_roots() {
            let allowed = test_temp_dir();
            let elsewhere = test_temp_dir();
            let server = create_test_server(allowed.path());
            let outside = elsewhere.path().join("Outside.PcbLib");
            create_test_pcblib(&outside);

            let result = server.call_validate_library(&json!({
                "filepath": outside.to_string_lossy(),
            }));
            assert!(result.is_error);
            assert_eq!(
                get_result_text(&result),
                "Access denied: path is outside the configured allowed directories"
            );
        }

        // The extension picks the parser. An unrecognised one has to be named
        // as unsupported; guessing instead would feed a .bak or .lib to the
        // PcbLib reader and report "corrupt library" for a file that simply
        // was never one.
        #[test]
        fn validate_library_rejects_unknown_extension() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Notes.txt");

            let result = server.call_validate_library(&json!({
                "filepath": path.to_string_lossy(),
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
            assert_eq!(
                parsed["error"],
                "Unknown file type. Expected .PcbLib or .SchLib extension."
            );
        }

        // A PcbLib that cannot be parsed must come back as an error rather than
        // as a library with zero footprints: "valid, 0 components" would tell
        // the operator a corrupt file is fine to release.
        #[test]
        fn validate_pcblib_reports_a_library_it_cannot_read() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Corrupt.PcbLib");
            write_corrupt_file(&path);

            let result = server.call_validate_library(&json!({
                "filepath": path.to_string_lossy(),
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
            assert_eq!(parsed["filepath"], "Corrupt.PcbLib");
            assert!(!parsed["error"].as_str().unwrap().is_empty());
        }

        // Zero-width tracks and zero-radius arcs are silkscreen and courtyard
        // geometry Altium draws as nothing: the outline simply vanishes on the
        // fab drawing. The report has to carry the primitive index, or it
        // cannot be found in a footprint with dozens of them.
        #[test]
        fn validate_pcblib_reports_degenerate_tracks_and_arcs() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut fp = Footprint::new("DEGENERATE");
            fp.add_pad(Pad::smd("1", 0.0, 0.0, 0.6, 0.5));
            fp.add_track(Track::new(-1.0, -0.5, 1.0, -0.5, 0.15, Layer::TopOverlay));
            fp.add_track(Track::new(-1.0, 0.5, 1.0, 0.5, 0.0, Layer::TopOverlay));
            fp.add_arc(Arc::circle(0.0, 0.0, 0.5, 0.15, Layer::TopOverlay));
            fp.add_arc(Arc::circle(0.0, 0.0, 0.0, 0.15, Layer::TopOverlay));
            let mut lib = PcbLib::new();
            lib.add(fp);
            let path = dir.path().join("Degenerate.PcbLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(
                &server.call_validate_library(&json!({ "filepath": path.to_string_lossy() })),
            );
            assert_eq!(parsed["status"], "invalid");
            let issues = parsed["issues"].as_array().unwrap();
            assert!(
                issues
                    .iter()
                    .any(|i| i["issue"] == "Track 1 has invalid width: 0"
                        && i["component"] == "DEGENERATE"),
                "the zero-width track must be reported by index: {issues:?}"
            );
            assert!(
                issues
                    .iter()
                    .any(|i| i["issue"] == "Arc 1 has invalid radius: 0"
                        && i["component"] == "DEGENERATE"),
                "the zero-radius arc must be reported by index: {issues:?}"
            );
        }

        // A region with fewer than three vertices encloses no area, so the
        // copper it was meant to pour simply is not there. Validation must
        // flag it by index rather than let a missing thermal pad reach fab.
        #[test]
        fn validate_pcblib_reports_region_with_too_few_vertices() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut fp = Footprint::new("SLIVER");
            fp.add_pad(Pad::smd("1", 0.0, 0.0, 0.6, 0.5));
            fp.add_region(Region {
                vertices: vec![Vertex { x: 0.0, y: 0.0 }, Vertex { x: 1.0, y: 0.0 }],
                layer: Layer::TopLayer,
                ..Region::default()
            });
            let mut lib = PcbLib::new();
            lib.add(fp);
            let path = dir.path().join("Sliver.PcbLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(
                &server.call_validate_library(&json!({ "filepath": path.to_string_lossy() })),
            );
            assert_eq!(parsed["status"], "invalid");
            assert!(
                parsed["issues"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|i| i["issue"] == "Region 0 has fewer than 3 vertices"
                        && i["component"] == "SLIVER"),
                "got: {parsed}"
            );
        }

        // An empty SchLib is a warning rather than an error, but it has to be
        // reported: a symbol library whose write silently produced nothing
        // would otherwise pass validation and ship as an empty file.
        #[test]
        fn validate_schlib_reports_empty_library() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Empty.SchLib");
            SchLib::new().save(&path).unwrap();

            let parsed = parse_result_json(
                &server.call_validate_library(&json!({ "filepath": path.to_string_lossy() })),
            );
            assert_eq!(parsed["status"], "warnings");
            assert_eq!(parsed["component_count"], 0);
            assert_eq!(
                parsed["issues"][0]["issue"],
                "Library is empty (no symbols)"
            );
        }

        // A symbol with no pins cannot be wired to anything: it drops out of
        // the netlist without warning. Validation must surface it and name the
        // symbol, so it can be found in a library of hundreds.
        #[test]
        fn validate_schlib_reports_symbol_without_pins() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut sym = Symbol::new("BODY_ONLY");
            sym.add_rectangle(Rectangle::new(-10, -5, 10, 5));
            let mut lib = SchLib::new();
            lib.add(sym);
            let path = dir.path().join("NoPins.SchLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(
                &server.call_validate_library(&json!({ "filepath": path.to_string_lossy() })),
            );
            assert_eq!(parsed["status"], "warnings");
            assert!(
                parsed["issues"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|i| i["issue"] == "Symbol has no pins" && i["component"] == "BODY_ONLY"),
                "got: {parsed}"
            );
        }

        // A pin with no designator has nothing to map onto a footprint pad, so
        // the schematic-to-layout sync silently drops the connection. This has
        // to be an error, not a warning.
        #[test]
        fn validate_schlib_reports_empty_pin_designator() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut sym = Symbol::new("NO_DESIGNATOR");
            sym.add_pin(Pin::new("A", "", -20, 0, 10, PinOrientation::Left));
            sym.add_rectangle(Rectangle::new(-10, -5, 10, 5));
            let mut lib = SchLib::new();
            lib.add(sym);
            let path = dir.path().join("NoDesignator.SchLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(
                &server.call_validate_library(&json!({ "filepath": path.to_string_lossy() })),
            );
            assert_eq!(parsed["status"], "invalid");
            assert!(
                parsed["issues"].as_array().unwrap().iter().any(|i| {
                    i["issue"] == "Pin has empty designator"
                        && i["severity"] == "error"
                        && i["component"] == "NO_DESIGNATOR"
                }),
                "got: {parsed}"
            );
        }

        // ---------- post-write validation ----------

        // Post-write validation is the last look at the file after it has been
        // rewritten. If it stopped noticing degenerate geometry, a delete would
        // report clean success on a library it had just left with invisible
        // silkscreen, empty copper regions and shorted pad designators.
        #[test]
        fn delete_component_pcblib_post_write_validation_flags_degenerate_primitives() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut lib = PcbLib::new();
            lib.add(Footprint::new("GOOD")); // deleted below, to trigger the write
            let mut bad = Footprint::new("BAD");
            bad.add_pad(Pad::smd("1", 0.0, 0.0, 0.6, 0.5));
            let mut zero_width = Pad::smd("2", 1.0, 0.0, 0.6, 0.5);
            zero_width.width = 0.0;
            bad.add_pad(zero_width);
            bad.add_track(Track::new(-1.0, 0.0, 1.0, 0.0, 0.0, Layer::TopOverlay));
            bad.add_arc(Arc::circle(0.0, 0.0, 0.0, 0.15, Layer::TopOverlay));
            bad.add_region(Region {
                vertices: vec![Vertex { x: 0.0, y: 0.0 }],
                layer: Layer::TopLayer,
                ..Region::default()
            });
            lib.add(bad);
            let path = dir.path().join("PostWriteDefects.PcbLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(&server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["GOOD"],
            })));
            assert_eq!(parsed["status"], "success");
            assert_eq!(parsed["validation"]["status"], "invalid");
            let reported: Vec<&str> = parsed["validation"]["issues"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|i| i["issue"].as_str())
                .collect();
            for expected in [
                "Pad '2' has invalid dimensions",
                "Track 0 has invalid width",
                "Arc 0 has invalid radius",
                "Region 0 has fewer than 3 vertices",
            ] {
                assert!(
                    reported.contains(&expected),
                    "post-write validation must report {expected:?}, got: {reported:?}"
                );
            }
        }

        // The SchLib delete path runs its own post-write validation. Without
        // it, a delete that leaves two pins claiming the same designator —
        // which silently shorts two nets on import — reads as a clean success.
        #[test]
        fn delete_component_schlib_post_write_validation_flags_symbol_defects() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut lib = SchLib::new();
            lib.add(Symbol::new("GOOD")); // deleted below, to trigger the write
            let mut bad = Symbol::new("BAD");
            bad.add_pin(Pin::new("A", "1", -20, 0, 10, PinOrientation::Left));
            bad.add_pin(Pin::new("K", "1", 20, 0, 10, PinOrientation::Right));
            bad.add_rectangle(Rectangle::new(10, 5, -10, -5)); // inverted corners
            lib.add(bad);
            let path = dir.path().join("PostWriteSch.SchLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(&server.call_delete_component(&json!({
                "filepath": path.to_string_lossy(),
                "component_names": ["GOOD"],
            })));
            assert_eq!(parsed["status"], "success");
            assert_eq!(parsed["validation"]["status"], "invalid");
            let reported: Vec<&str> = parsed["validation"]["issues"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|i| i["issue"].as_str())
                .collect();
            assert!(
                reported.contains(&"Duplicate pin designator: '1'"),
                "got: {reported:?}"
            );
            assert!(
                reported.contains(&"Rectangle 0 has inverted corners"),
                "got: {reported:?}"
            );
        }

        // ---------- export_library ----------

        // Export reads the whole library and hands it back to the client, so a
        // missing sandbox check would turn it into an arbitrary-file reader.
        #[test]
        fn export_library_rejects_path_outside_allowed_roots() {
            let allowed = test_temp_dir();
            let elsewhere = test_temp_dir();
            let server = create_test_server(allowed.path());
            let outside = elsewhere.path().join("Outside.PcbLib");
            create_test_pcblib(&outside);

            let result = server.call_export_library(&json!({
                "filepath": outside.to_string_lossy(),
                "format": "json",
            }));
            assert!(result.is_error);
            assert_eq!(
                get_result_text(&result),
                "Access denied: path is outside the configured allowed directories"
            );
        }

        // Export has its own extension switch. An unsupported one must say so,
        // rather than silently producing an empty export the caller then trusts
        // as the library's real contents.
        #[test]
        fn export_library_rejects_unknown_extension() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Parts.csv");

            let result = server.call_export_library(&json!({
                "filepath": path.to_string_lossy(),
                "format": "json",
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(
                parsed["error"],
                "Unknown file type. Expected .PcbLib or .SchLib extension."
            );
        }

        // A corrupt PcbLib must fail the export instead of yielding an empty
        // footprint list — a caller feeding that back through import_library
        // would replace a real library with an empty one.
        #[test]
        fn export_pcblib_reports_a_library_it_cannot_read() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Corrupt.PcbLib");
            write_corrupt_file(&path);

            let result = server.call_export_library(&json!({
                "filepath": path.to_string_lossy(),
                "format": "json",
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
            assert_eq!(parsed["filepath"], "Corrupt.PcbLib");
            assert!(!parsed["error"].as_str().unwrap().is_empty());
        }

        // Same for the SchLib exporter, which reads through a separate path.
        #[test]
        fn export_schlib_reports_a_library_it_cannot_read() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let path = dir.path().join("Corrupt.SchLib");
            write_corrupt_file(&path);

            let result = server.call_export_library(&json!({
                "filepath": path.to_string_lossy(),
                "format": "csv",
            }));
            assert!(result.is_error);
            let parsed = parse_result_json(&result);
            assert_eq!(parsed["status"], "error");
            assert_eq!(parsed["filepath"], "Corrupt.SchLib");
            assert!(!parsed["error"].as_str().unwrap().is_empty());
        }

        // Compact export strips the per-layer arrays when every layer holds the
        // same geometry — so it must also downgrade stack_mode to "simple".
        // Left saying "full_stack", the payload describes a pad with
        // independent per-layer geometry whose per-layer data is gone, and
        // re-importing it loses the pad's real size.
        #[test]
        fn export_pcblib_compact_downgrades_uniform_full_stack_pads() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());

            let mut uniform = Pad::smd("1", 0.0, 0.0, 0.6, 0.5);
            uniform.stack_mode = PadStackMode::FullStack;
            uniform.per_layer_sizes = Some(vec![(0.6, 0.5); 32]);

            // A genuinely tapered stack: layer 0 is wider than the rest.
            let mut tapered = Pad::smd("2", 2.0, 0.0, 0.6, 0.5);
            tapered.stack_mode = PadStackMode::FullStack;
            let mut tapered_sizes = vec![(0.6, 0.5); 32];
            tapered_sizes[0] = (1.2, 0.5);
            tapered.per_layer_sizes = Some(tapered_sizes);

            let mut fp = Footprint::new("FULLSTACK");
            fp.add_pad(uniform);
            fp.add_pad(tapered);
            let mut lib = PcbLib::new();
            lib.add(fp);
            let path = dir.path().join("FullStack.PcbLib");
            lib.save(&path).unwrap();

            let parsed = parse_result_json(&server.call_export_library(&json!({
                "filepath": path.to_string_lossy(),
                "format": "json",
                "compact": true,
            })));
            let exported = &parsed["footprints"][0]["pads"][0];
            assert_eq!(
                exported["stack_mode"], "simple",
                "a stripped pad must not still claim a full stack: {exported}"
            );
            for stripped in [
                "per_layer_sizes",
                "per_layer_shapes",
                "per_layer_corner_radii",
                "per_layer_offsets",
            ] {
                assert!(
                    exported.get(stripped).is_none(),
                    "uniform {stripped} must be stripped: {exported}"
                );
            }
            // The pad's real geometry has to survive the strip, or the compact
            // form describes a pad of unknown size.
            let width = exported["width"].as_f64().unwrap();
            assert!((width - 0.6).abs() < 1e-3, "width lost: {exported}");

            // The tapered pad's per-layer sizes are not redundant: stripping
            // them would export a plain 0.6 mm pad and silently lose the wider
            // layer-0 land the part actually needs.
            let tapered = &parsed["footprints"][0]["pads"][1];
            assert_eq!(
                tapered["stack_mode"], "full_stack",
                "a non-uniform stack must keep its mode: {tapered}"
            );
            let sizes = tapered["per_layer_sizes"]
                .as_array()
                .unwrap_or_else(|| panic!("per-layer sizes must be kept: {tapered}"));
            assert!(
                (sizes[0][0].as_f64().unwrap() - 1.2).abs() < 1e-3,
                "the wider layer-0 size must survive: {tapered}"
            );
        }

        // ---------- import_library ----------

        // Import writes a file. Without the sandbox check it would be an
        // arbitrary-file writer, which is the one thing the server must never
        // become.
        #[test]
        fn import_library_rejects_path_outside_allowed_roots() {
            let allowed = test_temp_dir();
            let elsewhere = test_temp_dir();
            let server = create_test_server(allowed.path());
            let outside = elsewhere.path().join("Outside.PcbLib");

            let result = server.call_import_library(&json!({
                "output_path": outside.to_string_lossy(),
                "json_data": { "file_type": "PcbLib", "footprints": [] },
            }));
            assert!(result.is_error);
            assert_eq!(
                get_result_text(&result),
                "Access denied: path is outside the configured allowed directories"
            );
            assert!(!outside.exists(), "a refused import must write nothing");
        }

        // Append merges into whatever is already on disk. If an unreadable
        // existing library were treated as absent, the append would quietly
        // become an overwrite and destroy every footprint already there.
        #[test]
        fn import_pcblib_append_reports_an_existing_library_it_cannot_read() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let output = dir.path().join("Corrupt.PcbLib");
            write_corrupt_file(&output);
            let before = std::fs::read(&output).unwrap();

            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": { "file_type": "PcbLib", "footprints": [{ "name": "NEW" }] },
                "append": true,
            }));
            assert!(result.is_error);
            let text = get_result_text(&result);
            assert!(
                text.starts_with("Failed to read existing library for append"),
                "got: {text}"
            );
            assert_eq!(
                std::fs::read(&output).unwrap(),
                before,
                "a failed append must not overwrite the existing file"
            );
        }

        // Two footprints under one name make the second unreachable — Altium
        // resolves the name to whichever it indexed first. The import must
        // refuse and say which name collided.
        #[test]
        fn import_pcblib_rejects_a_duplicate_footprint_name() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let source = dir.path().join("DupSource.PcbLib");
            create_test_pcblib(&source);
            let exported = parse_result_json(&server.call_export_library(&json!({
                "filepath": source.to_string_lossy(),
                "format": "json",
            })));
            let fp = exported["footprints"][0].clone();

            let output = dir.path().join("Dup.PcbLib");
            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": { "file_type": "PcbLib", "footprints": [fp.clone(), fp] },
            }));
            assert!(result.is_error);
            assert_eq!(
                get_result_text(&result),
                "Component 'CHIP_0402' already exists in library"
            );
        }

        // serde deserialisation bypasses the create-path validators, so import
        // re-checks ranges itself. A coordinate past the internal-unit limit
        // wraps an i32 on write and lands the pad somewhere else entirely; the
        // message has to name the footprint and the field.
        #[test]
        fn import_pcblib_rejects_out_of_range_coordinates() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let source = dir.path().join("RangeSource.PcbLib");
            create_test_pcblib(&source);
            let exported = parse_result_json(&server.call_export_library(&json!({
                "filepath": source.to_string_lossy(),
                "format": "json",
            })));
            let mut fp = exported["footprints"][0].clone();
            fp["pads"][0]["x"] = json!(9999.0);

            let output = dir.path().join("Range.PcbLib");
            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": { "file_type": "PcbLib", "footprints": [fp] },
            }));
            assert!(result.is_error);
            let text = get_result_text(&result);
            assert!(
                text.starts_with("Footprint 0 ('CHIP_0402'):"),
                "the offending footprint must be named: {text}"
            );
            assert!(
                text.contains("pad 0 x") && text.contains("exceeds the maximum safe range"),
                "the offending field must be named: {text}"
            );
            assert!(!output.exists(), "a rejected import must write nothing");
        }

        // A footprint the parser cannot read has to be reported with its index
        // and name. A payload of fifty footprints is otherwise impossible to
        // debug, and silently skipping it would ship a library missing a part.
        #[test]
        fn import_pcblib_rejects_an_unparseable_footprint() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let output = dir.path().join("Unparseable.PcbLib");

            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": {
                    "file_type": "PcbLib",
                    "footprints": [{ "name": "MANGLED", "pads": "not-an-array" }],
                },
            }));
            assert!(result.is_error);
            let text = get_result_text(&result);
            assert!(
                text.starts_with("Failed to parse footprint 0 ('MANGLED')"),
                "got: {text}"
            );
            assert!(!output.exists(), "a rejected import must write nothing");
        }

        // If the save fails the import must not claim success: a caller that
        // believes the library was written deletes its source data and loses
        // the footprints.
        #[test]
        fn import_pcblib_reports_a_failed_write() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let output = dir.path().join("LockedImport.PcbLib");
            create_test_pcblib(&output);
            let _readonly = ReadOnlyFile::new(&output);

            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": { "file_type": "PcbLib", "footprints": [{ "name": "NEW" }] },
                "append": true,
            }));
            assert!(result.is_error);
            let text = get_result_text(&result);
            assert!(text.starts_with("Failed to write library"), "got: {text}");
        }

        // Append mode has to report the running total, not just what this call
        // added; the message is what an operator uses to confirm the merge
        // landed on top of the existing library rather than replacing it.
        #[test]
        fn import_pcblib_append_adds_to_the_existing_library() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let output = dir.path().join("AppendPcb.PcbLib");
            create_test_pcblib(&output);

            let parsed = parse_result_json(&server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": { "file_type": "PcbLib", "footprints": [{ "name": "CHIP_0805" }] },
                "append": true,
            })));
            assert_eq!(parsed["status"], "success");
            assert_eq!(parsed["imported_count"], 1);
            assert_eq!(parsed["total_count"], 3);
            assert_eq!(
                parsed["message"],
                "Imported 1 footprints (library now has 3 total)"
            );

            let lib = PcbLib::open(&output).unwrap();
            assert!(
                lib.get("CHIP_0402").is_some(),
                "append must keep what was there"
            );
            assert!(lib.get("CHIP_0805").is_some());
        }

        // A SchLib import payload without a symbols array must be refused. Left
        // to fall through it would write an empty library over the output path.
        #[test]
        fn import_schlib_requires_a_symbols_array() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let output = dir.path().join("NoSymbols.SchLib");

            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": { "file_type": "SchLib" },
            }));
            assert!(result.is_error);
            assert_eq!(
                get_result_text(&result),
                "JSON data must contain 'symbols' array"
            );
            assert!(!output.exists());
        }

        // As on the PcbLib side, an unreadable existing library must abort the
        // append rather than let it silently turn into an overwrite.
        #[test]
        fn import_schlib_append_reports_an_existing_library_it_cannot_read() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let output = dir.path().join("Corrupt.SchLib");
            write_corrupt_file(&output);
            let before = std::fs::read(&output).unwrap();

            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": {
                    "file_type": "SchLib",
                    "symbols": [{ "name": "NEW", "pins": [] }],
                },
                "append": true,
            }));
            assert!(result.is_error);
            let text = get_result_text(&result);
            assert!(
                text.starts_with("Failed to read existing library for append"),
                "got: {text}"
            );
            assert_eq!(std::fs::read(&output).unwrap(), before);
        }

        // Duplicate symbol names make the second symbol unreachable from a
        // schematic; the import must refuse and name the collision.
        #[test]
        fn import_schlib_rejects_a_duplicate_symbol_name() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let output = dir.path().join("DupSym.SchLib");
            let symbol = json!({
                "name": "DIODE",
                "designator": "D?",
                "pins": [{ "designator": "1", "name": "A", "x": -20, "y": 0, "length": 10 }],
            });

            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": { "file_type": "SchLib", "symbols": [symbol.clone(), symbol] },
            }));
            assert!(result.is_error);
            assert_eq!(
                get_result_text(&result),
                "Component 'DIODE' already exists in library"
            );
        }

        // validate_symbol_json only checks that fields are present, so the
        // range check afterwards is the only thing standing between an
        // out-of-range pin and a coordinate that wraps on write, putting the
        // pin at the wrong end of the sheet.
        #[test]
        fn import_schlib_rejects_out_of_range_coordinates() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let output = dir.path().join("RangeSym.SchLib");

            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": {
                    "file_type": "SchLib",
                    "symbols": [{
                        "name": "FAR_AWAY",
                        "pins": [{ "designator": "1", "name": "A", "x": 99999, "y": 0, "length": 10 }],
                    }],
                },
            }));
            assert!(result.is_error);
            let text = get_result_text(&result);
            assert!(
                text.starts_with("Symbol 0 ('FAR_AWAY'):"),
                "the offending symbol must be named: {text}"
            );
            assert!(
                text.contains("pin 0 x") && text.contains("exceeds the maximum safe range"),
                "the offending field must be named: {text}"
            );
            assert!(!output.exists(), "a rejected import must write nothing");
        }

        // A symbol serde cannot deserialise must be reported with its index and
        // name, not skipped: a library silently missing a part passes every
        // later check and only fails on the schematic.
        #[test]
        fn import_schlib_rejects_an_unparseable_symbol() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let output = dir.path().join("BadSym.SchLib");

            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": {
                    "file_type": "SchLib",
                    "symbols": [{
                        "name": "MANGLED",
                        "pins": [{
                            "designator": "1", "name": "A",
                            "x": 0, "y": 0, "length": 10,
                            "orientation": "sideways",
                        }],
                    }],
                },
            }));
            assert!(result.is_error);
            let text = get_result_text(&result);
            assert!(
                text.starts_with("Failed to parse symbol 0 ('MANGLED')"),
                "got: {text}"
            );
            assert!(!output.exists(), "a rejected import must write nothing");
        }

        // A failed SchLib save must be reported. Reporting success would have
        // the caller discard the source JSON while the file on disk still holds
        // the old symbols.
        #[test]
        fn import_schlib_reports_a_failed_write() {
            let dir = test_temp_dir();
            let server = create_test_server(dir.path());
            let output = dir.path().join("LockedImport.SchLib");
            create_test_schlib(&output);
            let _readonly = ReadOnlyFile::new(&output);

            let result = server.call_import_library(&json!({
                "output_path": output.to_string_lossy(),
                "json_data": {
                    "file_type": "SchLib",
                    "symbols": [{
                        "name": "DIODE",
                        "pins": [{ "designator": "1", "name": "A", "x": -20, "y": 0, "length": 10 }],
                    }],
                },
                "append": true,
            }));
            assert!(result.is_error);
            let text = get_result_text(&result);
            assert!(text.starts_with("Failed to write library"), "got: {text}");
        }
    }
}
