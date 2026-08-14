use rmk_config::KeyboardTomlConfig;
use std::{env, fs, path::PathBuf, process};

/// All info needed to create a RMK project
#[derive(Debug)]
pub(crate) struct ProjectInfo {
    /// Project name
    pub(crate) project_name: String,
    /// Local directory of created RMK project
    pub(crate) target_dir: PathBuf,
    /// Remote folder name which contains the template
    pub(crate) remote_folder: String,
    /// Chip name
    pub(crate) chip: String,
    /// Key for uf2 generation
    pub(crate) uf2_key: String,
    /// List of disabled default features
    pub(crate) disabled_default_feature: Vec<String>,
    /// List of enabled non-default features
    pub(crate) enabled_feature: Vec<String>,
}

/// Parse `keyboard.toml`, get all needed project info for creating a new RMK project
pub(crate) fn parse_keyboard_toml(
    keyboard_toml: &String,
    target_dir: Option<String>,
) -> Result<ProjectInfo, Box<dyn std::error::Error>> {
    let keyboard_toml_config = KeyboardTomlConfig::new_from_toml_path(keyboard_toml);
    let hardware = keyboard_toml_config.hardware()?;
    let host = keyboard_toml_config.host();

    let project_name = keyboard_toml_config.identity()?.name.replace(" ", "_");
    let target_dir = if let Some(dir) = target_dir {
        dir
    } else {
        project_name.clone()
    };
    let project_dir = env::current_dir()?.join(&target_dir);

    if let Err(e) = fs::create_dir_all(&project_dir) {
        eprintln!("Failed to create project directory {}: {}", project_name, e);
        process::exit(1);
    }

    let mut disabled_default_feature = vec![];
    let mut enabled_feature = vec![];

    // Check keyboard.toml

    if hardware.storage.is_none() {
        disabled_default_feature.push("storage".to_string());
    }

    if !hardware.dependency.defmt_log {
        disabled_default_feature.push("defmt".to_string());
    }

    // Vial and Rynk are mutually exclusive, and `vial` is a default feature.
    if host.rynk_enabled {
        disabled_default_feature.push("vial".to_string());
        enabled_feature.push("rynk".to_string());
    } else if !host.vial_enabled {
        // No host protocol at all, so the lock gate has nothing to guard either.
        disabled_default_feature.push("vial".to_string());
        disabled_default_feature.push("host_lock".to_string());
    }

    let matrix_type = match hardware.board {
        rmk_config::resolved::hardware::BoardConfig::Split(_) => "split".to_string(),
        rmk_config::resolved::hardware::BoardConfig::UniBody(_) => "normal".to_string(),
    };

    let chip_model = hardware.chip;
    let chip_or_board = if let Some(board) = chip_model.board {
        board
    } else {
        chip_model.chip.clone()
    };
    let folder = if matrix_type == "split" {
        format!("{}_{}", chip_or_board, matrix_type)
    } else {
        chip_or_board.clone()
    };

    let uf2_key = if chip_model.chip.starts_with("stm32") {
        chip_model.chip[..7].to_string()
    } else {
        chip_model.chip.clone()
    };

    Ok(ProjectInfo {
        project_name,
        target_dir: project_dir,
        remote_folder: folder,
        chip: chip_or_board,
        uf2_key,
        disabled_default_feature,
        enabled_feature,
    })
}
