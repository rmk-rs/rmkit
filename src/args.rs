use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create a new RMK project from keyboard.toml and vial.json
    Create {
        /// Path to keyboard.toml file
        #[arg(long)]
        keyboard_toml_path: Option<String>,

        /// Path to vial.json file
        #[arg(long)]
        vial_json_path: Option<String>,

        /// Target dir
        #[arg(long)]
        target_dir: Option<String>,

        /// (Optional) RMK version
        #[arg(long)]
        version: Option<String>,
    },

    /// Initialize a new RMK project with basic configuration
    Init {
        /// Name of the project
        #[arg(long)]
        project_name: Option<String>,

        /// Target chip (e.g., nrf52840)
        #[arg(long)]
        chip: Option<String>,

        /// Whether the keyboard is split
        #[arg(long)]
        split: Option<bool>,

        /// (Optional) Local project template path
        #[arg(long)]
        local_path: Option<String>,

        /// (Optional) RMK version
        #[arg(long)]
        version: Option<String>,
    },
    /// Get chip name from keyboard.toml
    GetChip {
        /// Path to keyboard.toml file
        #[arg(long)]
        keyboard_toml_path: String,
    },
    /// Get project name from keyboard.toml
    GetProjectName {
        /// Path to keyboard.toml file
        #[arg(long)]
        keyboard_toml_path: String,
    },
    /// Physical-layout tools: convert KLE/Vial layouts and preview [layout] geometry
    Layout {
        #[command(subcommand)]
        command: LayoutCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum LayoutCommands {
    /// Convert a KLE JSON export or vial.json into an RMK [layout] section (or back with --to-vial)
    Convert {
        /// Path to a KLE JSON export or a vial.json (a keyboard.toml with --to-vial)
        input: String,

        /// Write the result to FILE instead of stdout
        #[arg(short, long)]
        output: Option<String>,

        /// Reverse: a keyboard.toml's [layout] → a minimal vial.json
        #[arg(long)]
        to_vial: bool,

        /// Skip the rmk-config round-trip check (forward only)
        #[arg(long)]
        no_validate: bool,
    },
    /// Render a physical layout (keyboard.toml, vial.json, or KLE export) as box-drawing art
    Show {
        /// Path to a keyboard.toml (or bare [layout] snippet), a vial.json, or a raw KLE JSON export
        input: String,

        /// Render only this [[layout.variant]] (default: all)
        #[arg(long)]
        variant: Option<String>,
    },
}
