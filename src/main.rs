use cargo_metadata::{Metadata, MetadataCommand};
use chip::{get_board_chip_map, get_chip_options};
use clap::Parser;
use futures::stream::StreamExt;
use inquire::ui::{Attributes, Color, RenderConfig, StyleSheet, Styled};
use inquire::{Select, Text};
use keyboard_toml::{parse_keyboard_toml, ProjectInfo};
use reqwest::Client;
use std::error::Error;
use std::fs;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

mod args;
mod chip;
mod keyboard_toml;
mod layout_cmd;
mod render;
mod version;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    inquire::set_global_render_config(get_render_config());
    let args = args::Args::parse();
    match args.command {
        args::Commands::Create {
            keyboard_toml_path,
            vial_json_path,
            target_dir,
            version,
        } => create_project(keyboard_toml_path, vial_json_path, target_dir, version).await,
        args::Commands::Init {
            project_name,
            chip,
            split,
            local_path,
            version,
        } => init_project(project_name, chip, split, local_path, version).await,
        args::Commands::GetChip { keyboard_toml_path } => {
            let project_info = parse_keyboard_toml(&keyboard_toml_path, None)?;
            println!("{}", project_info.chip);
            Ok(())
        }
        args::Commands::GetProjectName { keyboard_toml_path } => {
            let project_info = parse_keyboard_toml(&keyboard_toml_path, None)?;
            println!("{}", project_info.project_name);
            Ok(())
        }
        args::Commands::Layout { command } => {
            let result = match command {
                args::LayoutCommands::Convert {
                    input,
                    output,
                    to_vial,
                    no_validate,
                } => layout_cmd::convert(&input, output.as_deref(), to_vial, !no_validate),
                args::LayoutCommands::Show { input, variant } => layout_cmd::show(&input, variant.as_deref()),
            };
            // The layout tools speak plain stderr + exit code (their output is
            // piped/captured), not the interactive error style of create/init.
            if let Err(e) = result {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

async fn create_project(
    keyboard_toml_path: Option<String>,
    vial_json_path: Option<String>,
    target_dir: Option<String>,
    version: Option<String>,
) -> Result<(), Box<dyn Error>> {
    // Resolve version first for fast fail
    let commit_or_branch = version::resolve_template_version(version.as_deref()).await?;

    // Inquire paths interactively is no argument is specified
    let keyboard_toml_path = if let Some(path) = keyboard_toml_path {
        path
    } else {
        Text::new("Path to keyboard.toml:")
            .with_default("./keyboard.toml")
            .prompt()?
    };
    let vial_json_path = if let Some(path) = vial_json_path {
        path
    } else {
        Text::new("Path to vial.json")
            .with_default("./vial.json")
            .prompt()?
    };
    // Parse keyboard.toml to get project info
    let project_info = parse_keyboard_toml(&keyboard_toml_path, target_dir)?;

    // Download corresponding project template
    download_project_template(&project_info, &commit_or_branch).await?;

    // Copy keyboard.toml and vial.json to project_dir
    fs::copy(
        &keyboard_toml_path,
        project_info.target_dir.join("keyboard.toml"),
    )?;
    fs::copy(&vial_json_path, project_info.target_dir.join("vial.json"))?;

    // Post-process
    post_process(project_info)?;

    Ok(())
}

/// Postprocessing after generating project
fn post_process(project_info: ProjectInfo) -> Result<(), Box<dyn Error>> {
    // Replace {{ project_name }} in toml/json files
    replace_in_folder(
        &project_info,
        "toml",
        "{{ project_name }}",
        &project_info.project_name,
    )?;
    replace_in_folder(
        &project_info,
        "json",
        "{{ project_name }}",
        &project_info.project_name,
    )?;

    // Replace {{ chip_name }} in toml files
    replace_in_folder(&project_info, "toml", "{{ chip_name }}", &project_info.chip)?;

    // Replace {{ uf2_key }} in toml files
    replace_in_folder(
        &project_info,
        "toml",
        "{{ uf2_key }}",
        &project_info.uf2_key,
    )?;

    // Disable some default features
    if !project_info.disabled_default_feature.is_empty() {
        let metadata = MetadataCommand::new()
            .current_dir(&project_info.target_dir)
            .exec()?;
        disable_rmk_default_features(
            &project_info.target_dir,
            &metadata,
            project_info.disabled_default_feature,
        )?;
    }

    // Enable non-default features
    if !project_info.enabled_feature.is_empty() {
        enable_rmk_features(&project_info.target_dir, project_info.enabled_feature)?;
    }

    Ok(())
}

fn replace_in_folder(
    project_info: &ProjectInfo,
    ext: &str,
    from: &str,
    to: &str,
) -> Result<(), Box<dyn Error>> {
    let walker = walkdir::WalkDir::new(&project_info.target_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|e| e == ext));
    for entry in walker {
        let path = entry.path();
        let content = fs::read_to_string(path)?;
        let new_content = content.replace(from, to);
        fs::write(path, new_content)?;
    }
    Ok(())
}

async fn download_project_template(
    project_info: &ProjectInfo,
    commit_or_branch: &str,
) -> Result<(), Box<dyn Error>> {
    let user = "HaoboGu";
    let repo = "rmk-template";

    // Build download URL
    let url = version::build_github_archive_url(user, repo, commit_or_branch);

    download_with_progress(&url, &project_info.target_dir, &project_info.remote_folder).await
}

/// Initialize project from remote url
async fn init_project(
    project_name: Option<String>,
    chip: Option<String>,
    split: Option<bool>,
    local_path: Option<String>,
    version: Option<String>,
) -> Result<(), Box<dyn Error>> {
    // Resolve version first for fast fail (only when using remote template)
    let commit_or_branch = if local_path.is_none() {
        Some(version::resolve_template_version(version.as_deref()).await?)
    } else {
        None
    };

    let project_name = if let Some(name) = project_name {
        name.replace(" ", "_")
    } else {
        Text::new("Project Name:").prompt()?.replace(" ", "_")
    };
    let split = if let Some(s) = split {
        s
    } else {
        Select::new("Choose your keyboard type?", vec!["normal", "split"]).prompt()? == "split"
    };
    let mut chip_or_board = if let Some(c) = chip {
        c
    } else {
        Select::new(
            "Choose your microcontroller or board",
            get_chip_options(split),
        )
        .prompt()?
        .to_string()
    };

    // Get project info from parameters
    let target_dir = PathBuf::from(&project_name);
    fs::create_dir_all(&target_dir)?;

    // Convert board to chip first
    let board_chip_map = get_board_chip_map();
    if let Some(c) = board_chip_map.get(chip_or_board.as_str()) {
        chip_or_board = c.to_string();
    };
    let remote_folder = if split {
        format!("{}_{}", chip_or_board, "split")
    } else {
        chip_or_board.clone()
    };

    let uf2_key = if chip_or_board.starts_with("stm32") {
        chip_or_board[..7].to_string()
    } else if chip_or_board == "pico_w" {
        "rp2040".to_string()
    } else {
        chip_or_board.clone()
    };

    let project_info = ProjectInfo {
        project_name,
        target_dir,
        remote_folder,
        chip: chip_or_board,
        uf2_key,
        disabled_default_feature: Vec::new(),
        enabled_feature: Vec::new(),
    };

    // Download template
    match local_path {
        Some(p) => {
            // Copy local template to project_info.target_dir
            copy_dir_recursive(Path::new(&p), &project_info.target_dir)?;
        }
        None => {
            // Use remote template
            download_project_template(
                &project_info,
                commit_or_branch
                    .as_ref()
                    .expect("commit_or_branch should be resolved for remote template"),
            )
            .await?;
        }
    }

    // Post-process
    post_process(project_info)?;

    Ok(())
}

/// Download code from a GitHub repository link and extract it to the `repo` folder, using asynchronous download and a progress bar
///
/// # Parameters
/// - `download_url`: GitHub repository link
/// - `output_path`: Target extraction path
/// - `folder`: Specific subdirectory to extract
async fn download_with_progress<P>(
    download_url: &str,
    output_path: P,
    folder: &str,
) -> Result<(), Box<dyn Error>>
where
    P: AsRef<Path>,
{
    println!("download url: {}", download_url);
    let output_path = output_path.as_ref();

    // Ensure the output path is clean
    if output_path.exists() {
        fs::remove_dir_all(output_path)?;
    }
    fs::create_dir_all(output_path)?;

    println!("⇣ Download project template for {}...", folder);

    // Send request and get response
    let client = Client::new();
    let response = client.get(download_url).send().await?;
    if !response.status().is_success() {
        return Err(format!("Download failed: {}", response.status()).into());
    }

    // Temporary file to store the downloaded content
    let temp_file_path = output_path.join("temp.zip");
    let mut temp_file = File::create(&temp_file_path)?;

    // Ensure the temporary file is cleaned up on error
    struct TempFileCleanup<'a> {
        path: &'a Path,
    }
    impl<'a> Drop for TempFileCleanup<'a> {
        fn drop(&mut self) {
            if self.path.exists() {
                if let Err(e) = fs::remove_file(self.path) {
                    eprintln!(
                        "Failed to remove temp file '{}': {}",
                        self.path.display(),
                        e
                    );
                }
            }
        }
    }
    let _cleanup_guard = TempFileCleanup {
        path: &temp_file_path,
    };

    // Stream response bytes and write to temp file
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        temp_file.write_all(&chunk)?;
    }

    // Open the downloaded ZIP file and extract
    let zip_file = File::open(&temp_file_path)?;
    let mut zip = ZipArchive::new(zip_file)?;

    let mut folder_found = false;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let file_name = file.enclosed_name().ok_or("Invalid file path")?;

        // Find the root directory from the ZIP file
        let segments: Vec<_> = file_name.iter().collect();
        if segments.len() > 1 && segments[1] == folder {
            folder_found = true;
            let relative_name = file_name.iter().skip(2).collect::<PathBuf>();
            let out_path = output_path.join(relative_name);

            if file.is_dir() {
                fs::create_dir_all(&out_path)?;
            } else {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut outfile = File::create(&out_path)?;
                io::copy(&mut file, &mut outfile)?;
            }
        }
    }

    if !folder_found {
        // Check whether the remote_folder starts with stm32, do the second search using `stm32xx` and if there's still no matched template, use `stm32` template
        if folder.starts_with("stm32") {
            // Generate template for stm32
            if folder.len() > 7 {
                // Do the second search, use the stm32's family name
                let stm32_series = &folder[..7];
                for i in 0..zip.len() {
                    let mut file = zip.by_index(i)?;
                    let file_name = file.enclosed_name().ok_or("Invalid file path")?;

                    // Find the root directory from the ZIP file
                    let segments: Vec<_> = file_name.iter().collect();
                    if segments.len() > 1 && segments[1] == stm32_series {
                        folder_found = true;
                        let relative_name = file_name.iter().skip(2).collect::<PathBuf>();
                        let out_path = output_path.join(relative_name);

                        if file.is_dir() {
                            fs::create_dir_all(&out_path)?;
                        } else {
                            if let Some(parent) = out_path.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            let mut outfile = File::create(&out_path)?;
                            io::copy(&mut file, &mut outfile)?;
                        }
                    }
                }
            }
            if !folder_found {
                println!("️️🚨 There's no template available for [{folder}], using the default stm32 template. You may need to make further edit.");
                // Still not found, use the default stm32 template
                for i in 0..zip.len() {
                    let mut file = zip.by_index(i)?;
                    let file_name = file.enclosed_name().ok_or("Invalid file path")?;

                    // Find the root directory from the ZIP file
                    let segments: Vec<_> = file_name.iter().collect();
                    if segments.len() > 1 && segments[1] == "stm32" {
                        folder_found = true;
                        let relative_name = file_name.iter().skip(2).collect::<PathBuf>();
                        let out_path = output_path.join(relative_name);

                        if file.is_dir() {
                            fs::create_dir_all(&out_path)?;
                        } else {
                            if let Some(parent) = out_path.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            let mut outfile = File::create(&out_path)?;
                            io::copy(&mut file, &mut outfile)?;
                        }
                    }
                }
            }
        }

        // Check again
        if !folder_found {
            return Err(format!(
                "The specified chip/board '{}' does not exist in the template repo",
                folder
            )
            .into());
        }
    }

    println!("✅ Project created, path: {}", output_path.display());
    Ok(())
}

fn get_render_config() -> RenderConfig<'static> {
    let mut render_config = RenderConfig::default();
    render_config.prompt_prefix = Styled::new("?").with_fg(Color::LightRed);

    render_config.error_message = render_config
        .error_message
        .with_prefix(Styled::new("❌").with_fg(Color::LightRed));

    render_config.answer = StyleSheet::new()
        .with_attr(Attributes::ITALIC)
        .with_fg(Color::LightGreen);

    render_config
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
    if !src.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Source is not a directory",
        ));
    }

    // Create the target folder
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if file_type.is_dir() {
            // Recursively process
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            // Copy file
            fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

/// Update the rmk dependency configuration in the Cargo.toml file at the specified path
/// Replace rmk = { version = "...", features = ["..."] } with
/// rmk = { version = "...", default-features = false, features = ["..."] }
///
/// # Arguments
/// * `target_dir` - Target directory path containing Cargo.toml
///
/// # Returns
/// * `Result<(), String>` - Returns Ok on success, Err on failure
fn disable_rmk_default_features(
    target_dir: &PathBuf,
    metadata: &Metadata,
    features: Vec<String>,
) -> Result<(), String> {
    println!("Disabling default features: {:?}", features);
    // Define the path to Cargo.toml
    let cargo_toml_path = Path::new(target_dir).join("Cargo.toml");

    // Parse as Manifest using cargo_toml
    let mut manifest =
        cargo_toml::Manifest::from_path(&cargo_toml_path).map_err(|e| e.to_string())?;

    // Get dependencies and modify rmk configuration
    if let Some(cargo_toml::Dependency::Detailed(rmk_dep)) = manifest.dependencies.get_mut("rmk") {
        // Set default-features = false, and keep the original version and features
        let mut default_features = get_dependency_default_features("rmk", metadata)?;
        default_features.retain(|s| !features.contains(s));

        rmk_dep.features.append(&mut default_features);
        rmk_dep.features.sort_unstable();
        rmk_dep.features.dedup();

        rmk_dep.default_features = false;
    } else {
        return Err("No valid rmk dependency found".to_string());
    }

    // Convert the modified Manifest to a string
    let updated_toml = toml::to_string(&manifest)
        .map_err(|e| format!("Failed to serialize updated Cargo.toml: {}", e))?;

    // Write the updated content back to the file
    fs::write(&cargo_toml_path, updated_toml)
        .map_err(|e| format!("Failed to write updated Cargo.toml: {}", e))?;

    Ok(())
}

fn get_dependency_default_features(
    dependency: &str,
    metadata: &Metadata,
) -> Result<Vec<String>, String> {
    let dep = metadata
        .packages
        .iter()
        .find(|p| p.name.to_string() == dependency)
        .ok_or(format!("Failed to find {} in dependencies", dependency))?;
    dep.features
        .get("default")
        .cloned()
        .ok_or(format!("Failed to get default {} features", dependency))
}

/// Enable non-default features for rmk dependency in Cargo.toml
///
/// This function adds features to the rmk dependency's feature list
///
/// # Arguments
/// * `target_dir` - Target directory path containing Cargo.toml
/// * `features` - List of features to enable
///
/// # Returns
/// * `Result<(), String>` - Returns Ok on success, Err on failure
fn enable_rmk_features(target_dir: &PathBuf, features: Vec<String>) -> Result<(), String> {
    println!("Enabling features: {:?}", features);
    // Define the path to Cargo.toml
    let cargo_toml_path = Path::new(target_dir).join("Cargo.toml");

    // Parse as Manifest using cargo_toml
    let mut manifest =
        cargo_toml::Manifest::from_path(&cargo_toml_path).map_err(|e| e.to_string())?;

    // Get dependencies and modify rmk configuration
    if let Some(cargo_toml::Dependency::Detailed(rmk_dep)) = manifest.dependencies.get_mut("rmk") {
        // Add features to the existing feature list
        for feature in features {
            if !rmk_dep.features.contains(&feature) {
                rmk_dep.features.push(feature);
            }
        }
        rmk_dep.features.sort_unstable();
        rmk_dep.features.dedup();
    } else {
        return Err("No valid rmk dependency found".to_string());
    }

    // Convert the modified Manifest to a string
    let updated_toml = toml::to_string(&manifest)
        .map_err(|e| format!("Failed to serialize updated Cargo.toml: {}", e))?;

    // Write the updated content back to the file
    fs::write(&cargo_toml_path, updated_toml)
        .map_err(|e| format!("Failed to write updated Cargo.toml: {}", e))?;

    Ok(())
}
