# rmkit

rmkit is a toolkit set for [RMK keyboard firmware](https://github.com/rmk-rs/rmk).

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
    curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rmk-rs/rmkit/releases/latest/download/rmkit-installer.sh | sh

    # Windows(powershell)
    powershell -ExecutionPolicy ByPass -c "irm https://github.com/rmk-rs/rmkit/releases/latest/download/rmkit-installer.ps1 | iex"
   ```

2. Create RMK project from `keyboard.toml` and `vial.json`:

    ```
    rmkit create --keyboard-toml-path keyboard.toml --vial-json-path vial.json
    ```

3. Or, you can create RMK project from project template

    ```
    rmkit init
    ```

    The available project template can be found at [rmk-template](https://github.com/rmk-rs/rmk-template)
