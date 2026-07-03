# rmkit

rmkit is a toolkit set for [RMK keyboard firmware](https://github.com/haobogu/rmk).

Now rmkit can be used to generate RMK project directly from `keyboard.toml` and `vial.json`, or interactively.

## Usage

1. Install rmkit:
   
   If you have Rust installed in your machine, you can use Cargo to install rmkit

    ```shell
    cargo install rmkit

    # If you have cargo-binstall, you can use it to speedup the installation:
    cargo binstall rmkit
    ```
    
   rmkit also provides install script that you can use:

   ```shell
    # macOS/linux
    curl --proto '=https' --tlsv1.2 -LsSf https://github.com/haobogu/rmkit/releases/download/v0.0.1/rmkit-installer.sh | sh

    # Windows(powershell)
    powershell -ExecutionPolicy ByPass -c "irm https://github.com/haobogu/rmkit/releases/download/v0.0.1/rmkit-installer.ps1 | iex"
   ```

2. Create RMK project from `keyboard.toml` and `vial.json`:

    ```
    rmkit create --keyboard-toml-path keyboard.toml --vial-json-path vial.json
    ```

3. Or, you can create RMK project from project template

    ```
    rmkit init
    ```

    The available project template can be found at [rmk-template](https://github.com/HaoboGu/rmk-template)

## Layout tools

`rmkit layout` works with the physical `[layout]` section of a `keyboard.toml`. The conversion engine is the [`rynk-kle`](https://github.com/haobogu/rmk/tree/main/rynk/rynk-kle) library crate (which also compiles to a wasm package for the web); this CLI wraps it.

Convert a [KLE](http://www.keyboard-layout-editor.com/) JSON export or a [Vial](https://get.vial.today/) definition (`vial.json`) into RMK's `[layout]` — key positions, cap sizes, split gaps, rotation, ISO/L-shaped caps, encoders, and VIA layout options are all converted, and the result is validated against RMK's own layout builder. KLE carries no keycodes, so author the `[keymap]` yourself:

```shell
rmkit layout convert path/to/vial.json -o layout.toml   # vial.json → [layout]
rmkit layout convert path/to/kle_export.json            # raw KLE "Download JSON" export too
rmkit layout convert --to-vial keyboard.toml            # reverse: [layout] → vial.json
```

Render a physical layout as box-drawing art, to check the geometry in a terminal without flashing anything. The input can be a `keyboard.toml`, or a `vial.json` / raw KLE export directly (converted on the fly); `--variant` picks one `[[layout.variant]]`:

```shell
$ rmkit layout show keyboard.toml
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
```
