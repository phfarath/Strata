use std::path::Path;
use serde::{Deserialize, Serialize};

use strata_core::errors::StrataError;
use strata_core::state::Scope;

/// Type of package or workspace manifest detected in the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageType {
    CargoWorkspace,
    CargoCrate,
    NpmWorkspace,
    NpmPackage,
    PythonWorkspace,
    PythonPackage,
    GoWorkspace,
    GoModule,
    SingleProject,
}

impl std::fmt::Display for PackageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageType::CargoWorkspace => write!(f, "cargo_workspace"),
            PackageType::CargoCrate => write!(f, "cargo_crate"),
            PackageType::NpmWorkspace => write!(f, "npm_workspace"),
            PackageType::NpmPackage => write!(f, "npm_package"),
            PackageType::PythonWorkspace => write!(f, "python_workspace"),
            PackageType::PythonPackage => write!(f, "python_package"),
            PackageType::GoWorkspace => write!(f, "go_workspace"),
            PackageType::GoModule => write!(f, "go_module"),
            PackageType::SingleProject => write!(f, "single_project"),
        }
    }
}

/// Metadata describing a distinct package/crate within a monorepo workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonorepoPackage {
    pub name: String,
    pub package_type: PackageType,
    pub root_path: String,
    pub manifest_path: String,
    pub version: Option<String>,
    pub internal_dependencies: Vec<String>,
}

impl MonorepoPackage {
    pub fn new(
        name: impl Into<String>,
        package_type: PackageType,
        root_path: impl Into<String>,
        manifest_path: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            package_type,
            root_path: root_path.into(),
            manifest_path: manifest_path.into(),
            version: None,
            internal_dependencies: Vec::new(),
        }
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn with_internal_deps(mut self, deps: Vec<String>) -> Self {
        self.internal_dependencies = deps;
        self
    }
}

/// Representation of monorepo boundaries and member package isolation hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBoundary {
    pub root_path: String,
    pub workspace_type: PackageType,
    pub packages: Vec<MonorepoPackage>,
}

fn normalize_path_str(p: &str) -> String {
    let mut s = p.replace('\\', "/");
    if let Some(stripped) = s.strip_prefix("//?/") {
        s = stripped.to_string();
    } else if let Some(stripped) = s.strip_prefix("/?/") {
        s = stripped.to_string();
    } else if let Some(stripped) = s.strip_prefix("?/") {
        s = stripped.to_string();
    }
    s
}

impl WorkspaceBoundary {
    pub fn new(root_path: impl Into<String>, workspace_type: PackageType) -> Self {
        Self {
            root_path: root_path.into(),
            workspace_type,
            packages: Vec::new(),
        }
    }

    pub fn add_package(&mut self, pkg: MonorepoPackage) {
        self.packages.push(pkg);
    }

    /// Finds the specific member package enclosing a given file path.
    pub fn find_package_for_file(&self, file_path: &str) -> Option<&MonorepoPackage> {
        let normalized_target = normalize_path_str(file_path);
        let absolute_target = if Path::new(file_path).is_absolute() {
            normalized_target.clone()
        } else {
            let root = normalize_path_str(&self.root_path);
            format!("{}/{}", root.trim_end_matches('/'), normalized_target.trim_start_matches('/'))
        };

        let mut best_match: Option<&MonorepoPackage> = None;
        let mut best_len = 0;

        for pkg in &self.packages {
            let normalized_pkg_root = normalize_path_str(&pkg.root_path);
            let prefix = if normalized_pkg_root.ends_with('/') {
                normalized_pkg_root.clone()
            } else {
                format!("{normalized_pkg_root}/")
            };

            if absolute_target.starts_with(&prefix) || absolute_target == normalized_pkg_root
                || normalized_target.starts_with(&prefix) || normalized_target == normalized_pkg_root
            {
                if normalized_pkg_root.len() > best_len {
                    best_len = normalized_pkg_root.len();
                    best_match = Some(pkg);
                }
            }
        }

        best_match
    }

    /// Resolves the primary memory Scope for a given file.
    pub fn resolve_package_scope(&self, file_path: &str) -> Scope {
        if let Some(pkg) = self.find_package_for_file(file_path) {
            Scope::Project(pkg.name.clone())
        } else {
            Scope::Global
        }
    }

    /// Returns the hierarchical search scopes in order of priority:
    /// 1. Package Scope (`Scope::Project(package_name)`)
    /// 2. Workspace Root Scope (if different)
    /// 3. Global Scope (`Scope::Global`)
    pub fn get_hierarchical_scopes(&self, file_path: &str) -> Vec<Scope> {
        let mut scopes = Vec::new();
        if let Some(pkg) = self.find_package_for_file(file_path) {
            scopes.push(Scope::Project(pkg.name.clone()));
            // Also include internal dependencies as fallback scopes
            for dep in &pkg.internal_dependencies {
                scopes.push(Scope::Project(dep.clone()));
            }
        } else {
            scopes.push(Scope::Global);
        }
        scopes.dedup();
        scopes
    }
}

fn strip_unc_prefix(path: std::path::PathBuf) -> std::path::PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        std::path::PathBuf::from(stripped)
    } else {
        path
    }
}

/// Automatic detector for monorepo workspaces and package boundaries.
pub struct WorkspaceBoundaryDetector;

impl WorkspaceBoundaryDetector {
    /// Detects monorepo workspace boundaries starting from a given directory.
    pub fn detect(dir: &Path) -> Result<WorkspaceBoundary, StrataError> {
        let canonical_dir = strip_unc_prefix(dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()));

        // 1. Search upwards for enclosing workspace root (Cargo workspace, pnpm/npm workspace)
        let mut curr = Some(canonical_dir.as_path());
        while let Some(current) = curr {
            let cargo_toml = current.join("Cargo.toml");
            if cargo_toml.exists() {
                if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                    if content.contains("[workspace]") {
                        return Self::parse_cargo_workspace(current, &content);
                    }
                }
            }

            let pnpm_workspace = current.join("pnpm-workspace.yaml");
            let package_json = current.join("package.json");
            if pnpm_workspace.exists() || package_json.exists() {
                if let Ok(content) = std::fs::read_to_string(&package_json) {
                    if content.contains("\"workspaces\"") || pnpm_workspace.exists() {
                        return Self::parse_npm_workspace(current, &content);
                    }
                }
            }

            // Stop at .git repository root
            if current.join(".git").exists() {
                break;
            }

            curr = current.parent();
        }

        // 2. Check for single Cargo crate
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                if content.contains("[package]") {
                    let name = Self::extract_cargo_package_name(&content).unwrap_or_else(|| "unnamed_crate".to_string());
                    let mut boundary = WorkspaceBoundary::new(dir.to_string_lossy(), PackageType::CargoCrate);
                    boundary.add_package(MonorepoPackage::new(
                        name,
                        PackageType::CargoCrate,
                        dir.to_string_lossy(),
                        cargo_toml.to_string_lossy(),
                    ));
                    return Ok(boundary);
                }
            }
        }

        // 3. Check for single NPM package
        let package_json = dir.join("package.json");
        if package_json.exists() {
            if let Ok(content) = std::fs::read_to_string(&package_json) {
                let name = Self::extract_json_field(&content, "name").unwrap_or_else(|| "unnamed_package".to_string());
                let mut boundary = WorkspaceBoundary::new(dir.to_string_lossy(), PackageType::NpmPackage);
                boundary.add_package(MonorepoPackage::new(
                    name,
                    PackageType::NpmPackage,
                    dir.to_string_lossy(),
                    package_json.to_string_lossy(),
                ));
                return Ok(boundary);
            }
        }

        // 4. Check for Python workspace / package
        let pyproject_toml = dir.join("pyproject.toml");
        if pyproject_toml.exists() {
            let mut boundary = WorkspaceBoundary::new(dir.to_string_lossy(), PackageType::PythonPackage);
            boundary.add_package(MonorepoPackage::new(
                "python_app",
                PackageType::PythonPackage,
                dir.to_string_lossy(),
                pyproject_toml.to_string_lossy(),
            ));
            return Ok(boundary);
        }

        // Default Single Project fallback
        let mut boundary = WorkspaceBoundary::new(dir.to_string_lossy(), PackageType::SingleProject);
        boundary.add_package(MonorepoPackage::new(
            "default",
            PackageType::SingleProject,
            dir.to_string_lossy(),
            dir.to_string_lossy(),
        ));
        Ok(boundary)
    }

    fn parse_cargo_workspace(root_dir: &Path, manifest_content: &str) -> Result<WorkspaceBoundary, StrataError> {
        let mut boundary = WorkspaceBoundary::new(root_dir.to_string_lossy(), PackageType::CargoWorkspace);

        // Extract member globs/directories
        let mut member_paths = Vec::new();
        let mut in_workspace_members = false;

        for line in manifest_content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("members") && trimmed.contains('=') {
                in_workspace_members = true;
            }
            if in_workspace_members {
                if let Some(start) = trimmed.find('"') {
                    if let Some(end) = trimmed[start + 1..].find('"') {
                        let member = &trimmed[start + 1..start + 1 + end];
                        member_paths.push(member.to_string());
                    }
                }
                if trimmed.contains(']') {
                    in_workspace_members = false;
                }
            }
        }

        // Walk member directories and discover crate packages
        for member_pattern in member_paths {
            // Handle crates/* or specific folders
            if member_pattern.ends_with("/*") {
                let base_dir = root_dir.join(member_pattern.trim_end_matches("/*"));
                if let Ok(entries) = std::fs::read_dir(base_dir) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            let crate_manifest = entry.path().join("Cargo.toml");
                            if crate_manifest.exists() {
                                if let Ok(crate_content) = std::fs::read_to_string(&crate_manifest) {
                                    let name = Self::extract_cargo_package_name(&crate_content)
                                        .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
                                    let internal_deps = Self::extract_cargo_internal_deps(&crate_content);
                                    let mut pkg = MonorepoPackage::new(
                                        name,
                                        PackageType::CargoCrate,
                                        entry.path().to_string_lossy(),
                                        crate_manifest.to_string_lossy(),
                                    );
                                    pkg.internal_dependencies = internal_deps;
                                    boundary.add_package(pkg);
                                }
                            }
                        }
                    }
                }
            } else {
                let crate_dir = root_dir.join(&member_pattern);
                let crate_manifest = crate_dir.join("Cargo.toml");
                if crate_manifest.exists() {
                    if let Ok(crate_content) = std::fs::read_to_string(&crate_manifest) {
                        let name = Self::extract_cargo_package_name(&crate_content).unwrap_or(member_pattern);
                        let internal_deps = Self::extract_cargo_internal_deps(&crate_content);
                        let mut pkg = MonorepoPackage::new(
                            name,
                            PackageType::CargoCrate,
                            crate_dir.to_string_lossy(),
                            crate_manifest.to_string_lossy(),
                        );
                        pkg.internal_dependencies = internal_deps;
                        boundary.add_package(pkg);
                    }
                }
            }
        }

        Ok(boundary)
    }

    fn parse_npm_workspace(root_dir: &Path, _manifest_content: &str) -> Result<WorkspaceBoundary, StrataError> {
        let mut boundary = WorkspaceBoundary::new(root_dir.to_string_lossy(), PackageType::NpmWorkspace);

        // Check common monorepo package dirs: packages/*, apps/*, services/*
        let search_dirs = ["packages", "apps", "services", "crates", "libs"];
        for dir_name in search_dirs {
            let parent_dir = root_dir.join(dir_name);
            if parent_dir.exists() && parent_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(parent_dir) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            let pkg_json = entry.path().join("package.json");
                            if pkg_json.exists() {
                                if let Ok(content) = std::fs::read_to_string(&pkg_json) {
                                    let name = Self::extract_json_field(&content, "name")
                                        .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
                                    boundary.add_package(MonorepoPackage::new(
                                        name,
                                        PackageType::NpmPackage,
                                        entry.path().to_string_lossy(),
                                        pkg_json.to_string_lossy(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(boundary)
    }

    fn extract_cargo_package_name(content: &str) -> Option<String> {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("name") && trimmed.contains('=') && !trimmed.contains("workspace") {
                if let Some(first_quote) = trimmed.find('"') {
                    if let Some(second_quote) = trimmed[first_quote + 1..].find('"') {
                        return Some(trimmed[first_quote + 1..first_quote + 1 + second_quote].to_string());
                    }
                }
                if let Some(first_quote) = trimmed.find('\'') {
                    if let Some(second_quote) = trimmed[first_quote + 1..].find('\'') {
                        return Some(trimmed[first_quote + 1..first_quote + 1 + second_quote].to_string());
                    }
                }
            }
        }
        None
    }

    fn extract_cargo_internal_deps(content: &str) -> Vec<String> {
        let mut deps = Vec::new();
        let mut in_deps = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("[dependencies") {
                in_deps = true;
                continue;
            }
            if in_deps && trimmed.starts_with('[') && !trimmed.starts_with("[dependencies") {
                in_deps = false;
            }
            if in_deps && (trimmed.contains("workspace = true") || trimmed.contains("path =")) {
                if let Some(eq_pos) = trimmed.find('=') {
                    let dep_name = trimmed[..eq_pos].trim();
                    if !dep_name.is_empty() {
                        deps.push(dep_name.to_string());
                    }
                }
            }
        }
        deps
    }

    fn extract_json_field(json_str: &str, field: &str) -> Option<String> {
        let key = format!("\"{field}\"");
        if let Some(pos) = json_str.find(&key) {
            let rest = &json_str[pos + key.len()..];
            if let Some(colon) = rest.find(':') {
                let val_part = rest[colon + 1..].trim();
                if val_part.starts_with('"') {
                    if let Some(end) = val_part[1..].find('"') {
                        return Some(val_part[1..1 + end].to_string());
                    }
                }
            }
        }
        None
    }
}

// =========================================================================
// Unit Tests (TDD)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_cargo_workspace_monorepo() {
        let temp_dir = std::env::temp_dir().join("strata_test_cargo_workspace");
        let _ = std::fs::create_dir_all(&temp_dir);

        // Root Cargo.toml
        std::fs::write(
            temp_dir.join("Cargo.toml"),
            r#"
[workspace]
members = [
    "crates/core",
    "crates/server"
]
"#,
        )
        .unwrap();

        // Crates
        let core_dir = temp_dir.join("crates/core");
        let server_dir = temp_dir.join("crates/server");
        let _ = std::fs::create_dir_all(&core_dir);
        let _ = std::fs::create_dir_all(&server_dir);

        std::fs::write(
            core_dir.join("Cargo.toml"),
            r#"
[package]
name = "my-core"
version = "0.1.0"
"#,
        )
        .unwrap();

        std::fs::write(
            server_dir.join("Cargo.toml"),
            r#"
[package]
name = "my-server"
version = "0.1.0"

[dependencies]
my-core = { path = "../core" }
"#,
        )
        .unwrap();

        let boundary = WorkspaceBoundaryDetector::detect(&temp_dir).expect("detect workspace");
        assert_eq!(boundary.workspace_type, PackageType::CargoWorkspace);
        assert_eq!(boundary.packages.len(), 2);

        let core_pkg = boundary.packages.iter().find(|p| p.name == "my-core").unwrap();
        assert_eq!(core_pkg.package_type, PackageType::CargoCrate);

        let server_pkg = boundary.packages.iter().find(|p| p.name == "my-server").unwrap();
        assert!(server_pkg.internal_dependencies.contains(&"my-core".to_string()));

        // Test finding package for file
        let file_in_core = core_dir.join("src/lib.rs");
        let found = boundary.find_package_for_file(file_in_core.to_str().unwrap()).unwrap();
        assert_eq!(found.name, "my-core");

        // Test resolving hierarchical scopes
        let scopes = boundary.get_hierarchical_scopes(server_dir.join("src/main.rs").to_str().unwrap());
        assert_eq!(scopes.len(), 2);
        assert_eq!(scopes[0], Scope::Project("my-server".to_string()));
        assert_eq!(scopes[1], Scope::Project("my-core".to_string()));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_npm_pnpm_workspace_monorepo() {
        let temp_dir = std::env::temp_dir().join("strata_test_npm_workspace");
        let _ = std::fs::create_dir_all(&temp_dir);

        std::fs::write(
            temp_dir.join("package.json"),
            r#"{ "name": "my-monorepo", "workspaces": ["packages/*"] }"#,
        )
        .unwrap();

        let web_dir = temp_dir.join("packages/web");
        let api_dir = temp_dir.join("packages/api");
        let _ = std::fs::create_dir_all(&web_dir);
        let _ = std::fs::create_dir_all(&api_dir);

        std::fs::write(web_dir.join("package.json"), r#"{ "name": "@org/web" }"#).unwrap();
        std::fs::write(api_dir.join("package.json"), r#"{ "name": "@org/api" }"#).unwrap();

        let boundary = WorkspaceBoundaryDetector::detect(&temp_dir).expect("detect npm workspace");
        assert_eq!(boundary.workspace_type, PackageType::NpmWorkspace);
        assert_eq!(boundary.packages.len(), 2);

        let found_web = boundary.find_package_for_file(web_dir.join("src/App.tsx").to_str().unwrap()).unwrap();
        assert_eq!(found_web.name, "@org/web");

        let scope = boundary.resolve_package_scope(api_dir.join("src/server.ts").to_str().unwrap());
        assert_eq!(scope, Scope::Project("@org/api".to_string()));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
