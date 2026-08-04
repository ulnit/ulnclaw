//! Skills — port of hermes' skills system (tools/skills_tool.py)
//!
//! A skill is a directory `<home>/skills/<name>/SKILL.md` with YAML-ish
//! frontmatter (name, description) plus optional linked files under
//! references/, templates/, scripts/.

pub mod blueprint;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub category: String,
    pub path: PathBuf,
}

/// Parse YAML-ish frontmatter between `---` fences.
fn parse_frontmatter(content: &str) -> (String, String) {
    let mut name = String::new();
    let mut description = String::new();
    let trimmed = content.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---") {
        for line in rest.lines() {
            let line = line.trim();
            if line == "---" {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
                match key.trim() {
                    "name" => name = value,
                    "description" => description = value,
                    _ => {}
                }
            }
        }
    }
    (name, description)
}

/// List all skills found in a skills directory.
pub fn list_skills(skills_dir: &Path) -> Vec<Skill> {
    let mut skills = Vec::new();
    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return skills;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&skill_md).unwrap_or_default();
        let (mut name, description) = parse_frontmatter(&content);
        if name.is_empty() {
            name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
        }
        skills.push(Skill {
            name,
            description,
            category: "general".to_string(),
            path,
        });
    }
    skills
}

/// Find a skill by name (case-insensitive).
pub fn find_skill(skills_dir: &Path, name: &str) -> Option<Skill> {
    list_skills(skills_dir)
        .into_iter()
        .find(|skill| skill.name.eq_ignore_ascii_case(name))
}

/// Collect linked files relative to the skill directory
/// (references/, templates/, scripts/, assets/).
pub fn linked_files(skill_path: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for subdir in ["references", "templates", "scripts", "assets"] {
        let dir = skill_path.join(subdir);
        if !dir.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&dir).into_iter().flatten() {
            if entry.file_type().is_file() {
                if let Ok(rel) = entry.path().strip_prefix(skill_path) {
                    files.push(rel.display().to_string());
                }
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_discovery() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("deploy-helper");
        std::fs::create_dir_all(skill_dir.join("references")).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: deploy-helper\ndescription: Helps with deployments\n---\n\n# Steps\n1. build\n",
        )
        .unwrap();
        std::fs::write(skill_dir.join("references/runbook.md"), "# runbook").unwrap();

        let skills = list_skills(dir.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "deploy-helper");
        assert_eq!(skills[0].description, "Helps with deployments");

        let files = linked_files(&skills[0].path);
        assert_eq!(files, vec!["references/runbook.md"]);

        assert!(find_skill(dir.path(), "DEPLOY-helper").is_some());
        assert!(find_skill(dir.path(), "nope").is_none());
    }
}
