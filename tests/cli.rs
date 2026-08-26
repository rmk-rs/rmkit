//! End-to-end: run the built binary (`rmkit layout convert`) on fixture inputs
//! (vial.json and raw KLE exports) and check the output (including that the
//! generated `[layout]` passes rmk-config validation, which the command reports
//! on stderr and exits non-zero on failure).

use std::process::Command;

fn run(fixture: &str) -> std::process::Output {
    let path = format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    Command::new(env!("CARGO_BIN_EXE_rmkit"))
        .args(["layout", "convert", &path])
        .output()
        .expect("failed to run rmkit")
}

#[test]
fn ansi60_split_backspace_converts_and_validates() {
    let out = run("ansi60_splitbs.json");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(out.status.success(), "converter failed:\n{stderr}");
    // Validation ran and passed (exit 0 already implies it, but be explicit).
    assert!(stderr.contains("validation: OK"), "stderr:\n{stderr}");

    // Matrix size and the stock-width caps come through without shape defs.
    assert!(stdout.contains("rows = 5"));
    assert!(stdout.contains("cols = 15"));
    assert!(stdout.contains("(0,13,@2u)")); // default 2u backspace
    assert!(stdout.contains("(1,0,@1.5u)")); // 1.5u tab
    assert!(stdout.contains("(2,0,@1.75u)")); // 1.75u caps
    assert!(stdout.contains("(4,3,@6.25u)")); // 6.25u space

    // Caps are all stock widths; the only generated shape is the 1u reset the
    // split-backspace variant uses to shrink (0,13).
    assert!(stdout.contains("s1 = { w = 1.0 }"));

    // The split-backspace option became a variant that hides the extra key by
    // default and shows/reshapes it in the alternate.
    assert!(stdout.contains("[[layout.variant]]"));
    assert!(stdout.contains("name = \"default\""));
    assert!(stdout.contains("name = \"Split_Backspace\""));
    assert!(stdout.contains("hidden = [\"(0,14)\"]"));

    // KLE carries no keycodes — the output is geometry only, no [keymap] section
    // (the header comment still points the user at authoring one).
    assert!(!stdout.contains("\n[keymap]"));
    assert!(!stdout.contains("[[keymap.layer]]"));
}

#[test]
fn raw_kle_export_auto_assigns_matrix() {
    // A KLE "Download JSON" file: bare array, metadata object first, label
    // legends only — the matrix is derived row-major with a warning.
    let out = run("kle_export.json");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(out.status.success(), "converter failed:\n{stderr}");
    assert!(stderr.contains("validation: OK"), "stderr:\n{stderr}");
    assert!(
        stderr.contains("row-major"),
        "expected the auto-assign warning, got:\n{stderr}"
    );

    assert!(stdout.contains("rows = 3"));
    assert!(stdout.contains("cols = 4"));
    assert!(stdout.contains("(0,0) (0,1) (0,2) (0,3)"));
    assert!(stdout.contains("(1,0,@1.5u)")); // 1.5u Tab
    assert!(stdout.contains("(2,1,@2u)")); // 2u space
}

#[test]
fn bare_kle_array_with_matrix_legends_converts() {
    // The same KLE array a vial.json would embed, passed directly: `row,col`
    // legends win over the row-major fallback, and the undeclared matrix is
    // derived silently.
    let out = run("kle_rowcol.json");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(out.status.success(), "converter failed:\n{stderr}");
    assert!(!stderr.contains("warning:"), "stderr:\n{stderr}");
    assert!(stdout.contains("rows = 2"));
    assert!(stdout.contains("cols = 3"));
    assert!(stdout.contains("(0,0) (0,1) (0,2)"));
    assert!(stdout.contains("(1,0,@2u) (1,2)")); // legends place (1,2) after the 2u key
}

#[test]
fn missing_keymap_is_a_clean_error() {
    // A JSON file that isn't a Vial definition should fail with a helpful message.
    let out = Command::new(env!("CARGO_BIN_EXE_rmkit"))
        .args(["layout", "convert"])
        .arg(format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("failed to run");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error:"), "stderr:\n{stderr}");
}

#[test]
fn converted_toml_fixtures_are_up_to_date() {
    // Every JSON fixture has a committed `.toml` golden next to it: the exact
    // `layout convert` output. After changing the converter, regenerate with
    //   for f in tests/fixtures/*.json; do
    //       cargo run -- layout convert "$f" -o "${f%.json}.toml"; done
    // (from the crate root, so the generated-from header stays relative).
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let json = entry.path();
        if !json.extension().is_some_and(|e| e == "json") {
            continue;
        }
        let golden = json.with_extension("toml");
        let expected = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
            panic!("missing golden {golden:?} — regenerate (see comment above)")
        });
        let name = json.file_name().unwrap().to_string_lossy();
        let out = Command::new(env!("CARGO_BIN_EXE_rmkit"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(["layout", "convert"])
            .arg(format!("tests/fixtures/{name}"))
            .output()
            .expect("failed to run rmkit");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            stdout, expected,
            "stale golden for {name} — regenerate (see comment above)"
        );
        checked += 1;
    }
    assert!(
        checked >= 5,
        "expected the committed fixture pairs, found {checked}"
    );
}

#[test]
fn layout_show_accepts_vial_and_kle_json() {
    let show = |fixture: &str, extra: &[&str]| {
        let path = format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
        Command::new(env!("CARGO_BIN_EXE_rmkit"))
            .args(["layout", "show", &path])
            .args(extra)
            .output()
            .expect("failed to run rmkit")
    };

    // A vial.json renders directly, without converting to keyboard.toml first.
    let out = show("corne.json", &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("42 keys"), "stdout:\n{stdout}");
    assert!(stdout.contains("│ 0,0 │"), "stdout:\n{stdout}");

    // VIA layout options become variants, so --variant works on a vial.json.
    let out = show("ansi60_splitbs.json", &["--variant", "Split_Backspace"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("variant 'Split_Backspace'"),
        "stdout:\n{stdout}"
    );

    // A raw KLE export renders too, with the row-major fallback warning.
    let out = show("kle_export.json", &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("row-major"), "stderr:\n{stderr}");
}

#[test]
fn layout_show_renders_box_art() {
    // `layout show` on a bare map snippet prints the box-drawing art.
    let dir = std::env::temp_dir().join("rmkit_cli_show_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("layout.toml");
    std::fs::write(&path, "rows = 1\ncols = 2\nmap = \"(0,0) (0,1)\"\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_rmkit"))
        .args(["layout", "show"])
        .arg(&path)
        .output()
        .expect("failed to run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "show failed:\n{stderr}");
    assert!(stdout.contains("2 keys"), "stdout:\n{stdout}");
    assert!(stdout.contains("┌─────┬─────┐"), "stdout:\n{stdout}");
}
