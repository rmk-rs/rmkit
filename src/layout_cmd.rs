//! `rmkit layout` — CLI over the `rynk-kle` conversion library.
//!
//! `convert` turns a physical keyboard layout — a raw
//! [KLE](http://keyboard-layout-editor.com) JSON export or a Vial keyboard
//! definition (`vial.json`) — into the `[layout]` section of an RMK/Rynk
//! `keyboard.toml`, or the reverse with `--to-vial`. Neither input carries
//! keycodes, so no `[keymap]` is emitted — author it yourself against the
//! generated map.
//!
//! `show` renders the physical layout as Unicode box-drawing text, for
//! checking the geometry in a terminal. It takes a keyboard.toml — or a
//! vial.json / raw KLE export directly, converting it on the fly.
//!
//! All parsing, conversion, and blob decoding live in `rynk_kle`; this module
//! only adds file IO, stderr reporting, and the terminal renderer.

use rynk_kle::layout::{LayoutInfo, Variant};
use serde_json::Value;

use crate::render;

/// `rmkit layout convert`: KLE export or vial.json → `[layout]` (or, with
/// `to_vial`, a keyboard.toml's `[layout]` → a minimal vial.json).
pub fn convert(
    input: &str,
    output: Option<&str>,
    to_vial: bool,
    validate: bool,
) -> Result<(), String> {
    if to_vial {
        let text =
            std::fs::read_to_string(input).map_err(|e| format!("cannot read {input}: {e}"))?;
        let vial = rynk_kle::to_kle::keyboard_toml_to_vial(&text)?;
        let out = format!(
            "{}\n",
            serde_json::to_string_pretty(&vial).map_err(|e| e.to_string())?
        );
        match output {
            Some(path) => {
                std::fs::write(path, &out).map_err(|e| format!("cannot write {path}: {e}"))?;
                eprintln!("wrote {path}");
            }
            None => print!("{out}"),
        }
        return Ok(());
    }

    let text = std::fs::read_to_string(input).map_err(|e| format!("cannot read {input}: {e}"))?;
    let root: Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid JSON in {input}: {e}"))?;
    let generated = rynk_kle::convert_kle(&root)?;

    // Round-trip the generated [layout] through RMK's own builder.
    let mut validation_error = None;
    if validate {
        match rmk_config::layout_blob_from_toml(&generated.inner_layout_toml) {
            Ok(blob) => eprintln!("validation: OK ({} byte layout blob)", blob.len()),
            Err(e) => validation_error = Some(e),
        }
    }

    for w in &generated.warnings {
        eprintln!("warning: {w}");
    }

    let header = format!(
        "# Generated from {input} by rmkit — review the geometry. The source\n\
         # carries no keycodes, so author the [keymap] yourself: its `keys` follow the\n\
         # map's key order, plus one [\"cw\", \"ccw\"] pair per encoder in `encoders`.\n\n"
    );
    let out = header + &generated.display_toml;
    match output {
        Some(path) => {
            std::fs::write(path, &out).map_err(|e| format!("cannot write {path}: {e}"))?;
            eprintln!("wrote {path}");
        }
        None => print!("{out}"),
    }

    if let Some(e) = validation_error {
        return Err(format!(
            "the generated layout did not pass rmk-config validation: {e}\n\
             (output was still written; please report this as a converter bug)"
        ));
    }
    Ok(())
}

/// Any supported `show` input → decoded [`LayoutInfo`]. Content that parses as
/// JSON is a vial.json / raw KLE export and goes through the same conversion
/// `convert` performs; anything else is treated as keyboard.toml.
fn decode_input(input: &str, text: &str) -> Result<LayoutInfo, String> {
    match serde_json::from_str::<Value>(text) {
        Ok(root) => {
            let generated = rynk_kle::convert_kle(&root)?;
            for w in &generated.warnings {
                eprintln!("warning: {w}");
            }
            rynk_kle::decode_layout(&generated.inner_layout_toml)
        }
        // Not JSON but named like it: report the JSON error, not a TOML one.
        Err(e) if input.ends_with(".json") => Err(format!("invalid JSON in {input}: {e}")),
        Err(_) => rynk_kle::decode_layout(text),
    }
}

/// `rmkit layout show`: print the physical layout of a keyboard.toml — or,
/// directly, of a vial.json / raw KLE export — as box-drawing art. Keys are
/// labeled with their matrix position `row,col`; encoder knobs are drawn with
/// rounded corners and labeled `E<id>` plus `↺ ↻`. A character grid cannot draw
/// rotation, so rotated keys render axis-aligned with the angle printed inside.
pub fn show(input: &str, variant: Option<&str>) -> Result<(), String> {
    let text = std::fs::read_to_string(input).map_err(|e| format!("cannot read {input}: {e}"))?;
    let info = decode_input(input, &text)?;

    let variants: Vec<(usize, &Variant)> = match variant {
        Some(name) => {
            let idx = info
                .variants
                .iter()
                .position(|v| v.name == name)
                .ok_or_else(|| {
                    let names: Vec<&str> = info.variants.iter().map(|v| v.name.as_str()).collect();
                    format!("no variant '{name}' (available: {})", names.join(", "))
                })?;
            vec![(idx, &info.variants[idx])]
        }
        None => info.variants.iter().enumerate().collect(),
    };

    for (idx, v) in variants {
        // The default marker only means something when there's a choice.
        let is_default = info.variants.len() > 1 && idx == info.default_variant as usize;
        print!("{}", render::render_variant(v, is_default));
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rynk_kle::decode_layout;

    #[test]
    fn two_keys_snapshot() {
        let info = decode_layout("rows = 1\ncols = 2\nmap = \"(0,0) (0,1,@2u)\"").unwrap();
        let art = render::render_variant(&info.variants[0], false);
        let expected = "\
variant 'default': 2 keys, 3u × 1u

┌─────┬───────────┐
│ 0,0 │    0,1    │
│     │           │
└─────┴───────────┘
";
        assert_eq!(art, expected);
    }

    #[test]
    fn numpad_snapshot() {
        // The nrf52840_ble example numpad: 2u-tall Plus/Enter (@2uv) spanning
        // two rows and a 2u-wide zero — pins the junction merging.
        let toml = r#"
rows = 5
cols = 4
map = """
(0,0) (0,1) (0,2) (0,3)
(1,0) (1,1) (1,2) (1,3,@2uv)
(2,0) (2,1) (2,2)
(3,0) (3,1) (3,2) (3,3,@2uv)
(4,0,@2u) (4,1)
"""
"#;
        let art = render::render_variant(&decode_layout(toml).unwrap().variants[0], false);
        let expected = "\
variant 'default': 17 keys, 4u × 5u

┌─────┬─────┬─────┬─────┐
│ 0,0 │ 0,1 │ 0,2 │ 0,3 │
│     │     │     │     │
├─────┼─────┼─────┼─────┤
│ 1,0 │ 1,1 │ 1,2 │     │
│     │     │     │     │
├─────┼─────┼─────┤ 1,3 │
│ 2,0 │ 2,1 │ 2,2 │     │
│     │     │     │     │
├─────┼─────┼─────┼─────┤
│ 3,0 │ 3,1 │ 3,2 │     │
│     │     │     │     │
├─────┴─────┼─────┤ 3,3 │
│    4,0    │ 4,1 │     │
│           │     │     │
└───────────┴─────┴─────┘
";
        assert_eq!(art, expected);
    }

    #[test]
    fn encoder_and_rotation_render() {
        // The [0.5] gap keeps the encoder freestanding — adjacent boxes would
        // merge its left corners into ┬/┴ junctions instead of ╭/╰.
        let info = decode_layout(
            "rows = 1\ncols = 1\nmap = \"(0,0,@t) [0.5] (e,0)\"\n[shapes]\nt = { r = 15.0 }",
        )
        .unwrap();
        let art = render::render_variant(&info.variants[0], false);
        assert!(art.contains("15°"), "rotation angle shown:\n{art}");
        assert!(art.contains("E0"), "encoder label shown:\n{art}");
        assert!(art.contains("↺ ↻"), "rotary arrows shown:\n{art}");
        assert!(
            art.contains('╭') && art.contains('╯'),
            "encoder corners rounded:\n{art}"
        );
    }

    #[test]
    fn iso_enter_two_rects_render() {
        // @iso_enter pokes above its row (negative y) — exercises the origin offset.
        let info = decode_layout("rows = 1\ncols = 2\nmap = \"(0,0) (0,1,@iso_enter)\"").unwrap();
        let v = &info.variants[0];
        assert!(v.keys[1].rect2.is_some());
        let art = render::render_variant(v, false);
        assert!(art.contains("0,0") && art.contains("0,1"), "{art}");
    }

    #[test]
    fn show_accepts_vial_and_raw_kle_json() {
        // A vial.json-shaped input: `row,col` legends, matrix dims, a 2u cap.
        let vial = r#"{"matrix": {"rows": 1, "cols": 2},
                       "layouts": {"keymap": [["0,0", {"w": 2.0}, "0,1"]]}}"#;
        let info = decode_input("vial.json", vial).unwrap();
        let keys = &info.variants[0].keys;
        assert_eq!(keys.len(), 2);
        assert!((keys[1].w - 2.0).abs() < 1e-3);

        // A raw KLE "Download JSON" export: metadata object first, label
        // legends only — positions are assigned row-major.
        let kle = r#"[{"name": "plain"}, ["Esc", "Q"], [{"w": 1.5}, "Tab"]]"#;
        let info = decode_input("kle_export.json", kle).unwrap();
        let v = &info.variants[0];
        assert_eq!(v.keys.len(), 3);
        assert_eq!((v.keys[2].row, v.keys[2].col), (1, 0));
        assert!((v.keys[2].w - 1.5).abs() < 1e-3);
    }

    #[test]
    fn show_input_errors_match_the_format() {
        // Broken content named .json reports the JSON error, not a TOML one.
        let e = decode_input("x.json", "{not json").unwrap_err();
        assert!(e.contains("invalid JSON"), "{e}");
        // Anything else falls back to the TOML path.
        let e = decode_input("x.toml", "= bad").unwrap_err();
        assert!(e.contains("invalid TOML"), "{e}");
        // Valid JSON that isn't a layout still gets the shape hint.
        let e = decode_input("x.json", "{\"a\": 1}").unwrap_err();
        assert!(e.contains("layouts.keymap"), "{e}");
    }
}
