use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::error::Error;

/// Version to commit mapping structure
#[derive(Debug, Deserialize)]
struct VersionMapping {
    #[serde(flatten)]
    versions: HashMap<String, String>,
}

/// Resolve rmk-template version to a commit hash
///
/// # Arguments
/// * `version` - Optional version string (e.g., "0.7", "0.8")
///
/// # Returns
/// * Result with commit hash or "main" for latest, or error if version is invalid
pub async fn resolve_template_version(version: Option<&str>) -> Result<String, Box<dyn Error>> {
    match version {
        Some(v) => {
            if v == "latest" || v == "main" {
                return Ok("main".to_string());
            }

            // User provided a version, validate it
            let mapping = fetch_all_versions().await?;

            match mapping.versions.get(v) {
                Some(commit) => {
                    println!("📌 Using rmk-template version {} (commit: {})", v, commit);
                    Ok(commit.clone())
                }
                None => {
                    // Version not found, show available versions
                    let mut versions: Vec<String> = mapping.versions.keys().cloned().collect();
                    versions.sort();
                    versions.push("main".to_string());
                    Err(format!(
                        "Invalid version '{}'. Available versions: {}",
                        v,
                        versions.join(", ")
                    )
                    .into())
                }
            }
        }
        None => {
            // No version provided, use main branch
            println!("📌 Using latest template from main branch");
            Ok("main".to_string())
        }
    }
}

/// Fetch all available versions from remote config
async fn fetch_all_versions() -> Result<VersionMapping, Box<dyn Error>> {
    let config_url =
        "https://raw.githubusercontent.com/rmk-rs/rmk-template/main/version-mapping.json";

    let client = Client::new();
    let response = client.get(config_url).send().await?;

    if !response.status().is_success() {
        return Err(format!("Failed to fetch version mapping: {}", response.status()).into());
    }

    let mapping: VersionMapping = response.json().await?;
    Ok(mapping)
}

/// Build GitHub archive URL based on commit hash or "main"
///
/// # Arguments
/// * `user` - GitHub username
/// * `repo` - Repository name
/// * `commit_or_branch` - Commit hash or "main" for the main branch
///
/// # Returns
/// * GitHub archive URL
pub fn build_github_archive_url(user: &str, repo: &str, commit_or_branch: &str) -> String {
    if commit_or_branch == "main" {
        format!(
            "https://github.com/{}/{}/archive/refs/heads/main.zip",
            user, repo
        )
    } else {
        format!(
            "https://github.com/{}/{}/archive/{}.zip",
            user, repo, commit_or_branch
        )
    }
}
