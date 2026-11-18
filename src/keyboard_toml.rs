use crate::chip::get_board_chip_map;
use rmk_config::resolved::hardware::BoardConfig;
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

    let chip = hardware.chip.chip;
    let template = hardware
        .chip
        .board
        .as_deref()
        .and_then(|board| get_board_chip_map().get(board).copied())
        .unwrap_or(&chip);
    let folder = match hardware.board {
        BoardConfig::Split(_) => format!("{template}_split"),
        BoardConfig::UniBody(_) => template.to_owned(),
    };

    let uf2_key = if chip.starts_with("stm32") {
        chip[..7].to_string()
    } else {
        chip.clone()
    };

    Ok(ProjectInfo {
        project_name,
        target_dir: project_dir,
        remote_folder: folder,
        chip,
        uf2_key,
        disabled_default_feature,
        enabled_feature,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_aliases_resolve_to_chip_and_split_template() {
        let dir = env::temp_dir().join(format!("rmkit-board-aliases-{}", process::id()));
        fs::create_dir(&dir).unwrap();
        let config_path = dir.join("keyboard.toml").to_str().unwrap().to_owned();
        let target_dir = dir.join("project").to_str().unwrap().to_owned();
        let fixture = include_str!("../keyboard.toml");

        for (board, chip, template) in [
            ("XIAO BLE", "nrf52840", "nrf52840_split"),
            ("Pi Pico W", "rp2040", "pico_w_split"),
        ] {
            let config = fixture.replace("chip = \"nrf52840\"", &format!("board = \"{board}\""));
            fs::write(&config_path, config).unwrap();
            let project = parse_keyboard_toml(&config_path, Some(target_dir.clone())).unwrap();

            assert_eq!(
                (project.chip.as_str(), project.remote_folder.as_str()),
                (chip, template),
                "{board}"
            );
        }

        fs::remove_dir_all(dir).unwrap();
    }
}
