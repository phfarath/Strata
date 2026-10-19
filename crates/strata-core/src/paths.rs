use std::path::Path;

/// Canonicalizes a resource identifier for use with Strata leases, tasks, and A2A locks.
///
/// Rules:
/// 1. Non-file URIs (e.g. `crate:strata-memory`, `task:123`, `res:crate:core`) are trimmed and preserved.
/// 2. File resources (whether starting with `file:`, relative paths like `src/main.rs`, `./src/main.rs`,
///    or absolute paths like `C:\Dev\project\src\main.rs`) are:
///    - Converted to forward slashes (`/`).
///    - Stripped of Windows UNC prefixes (`\\?\`).
///    - Normalized relative to the current workspace root if within it.
///    - Stripped of leading `./` and redundant slash/dot segments (`/./`, `//`).
///    - Prepended with the canonical `file:` prefix.
pub fn canonical_resource_id(raw: &str) -> String {
    canonical_resource_id_with_root(raw, None)
}

/// Canonicalizes a resource identifier with an explicit workspace root override.
pub fn canonical_resource_id_with_root(raw: &str, workspace_root: Option<&Path>) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Check scheme
    let file_str = if let Some(stripped) = trimmed.strip_prefix("file:") {
        stripped.trim()
    } else if let Some((scheme, _rest)) = trimmed.split_once(':') {
        // Windows drive letter check: e.g. "C:\" or "c:/" has scheme.len() == 1
        if scheme.len() == 1
            && scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
        {
            trimmed
        } else {
            // Non-file URI scheme: preserve as-is
            return trimmed.to_string();
        }
    } else {
        trimmed
    };

    let mut p = file_str.replace('\\', "/");

    // Strip UNC prefixes
    if let Some(stripped) = p.strip_prefix("//?/") {
        p = stripped.to_string();
    } else if let Some(stripped) = p.strip_prefix("/?/") {
        p = stripped.to_string();
    } else if let Some(stripped) = p.strip_prefix("?/") {
        p = stripped.to_string();
    }

    // If it starts with multiple slashes before a Windows drive letter: ///C:/...
    let trimmed_leading = p.trim_start_matches('/');
    if trimmed_leading.len() >= 2
        && trimmed_leading
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && trimmed_leading.chars().nth(1) == Some(':')
    {
        p = trimmed_leading.to_string();
    }

    // Try stripping workspace root if absolute
    let root_buf = workspace_root
        .map(|r| r.to_path_buf())
        .or_else(|| std::env::current_dir().ok());

    if let Some(root) = root_buf {
        let mut root_str = root.to_string_lossy().replace('\\', "/");
        if let Some(stripped) = root_str.strip_prefix("//?/") {
            root_str = stripped.to_string();
        }
        let root_clean = root_str.trim_end_matches('/');

        let matched_len = if p.starts_with(root_clean)
            || (p.len() >= root_clean.len()
                && p[..root_clean.len()].eq_ignore_ascii_case(root_clean))
        {
            Some(root_clean.len())
        } else {
            None
        };

        if let Some(len) = matched_len {
            let remainder = &p[len..];
            p = remainder.trim_start_matches('/').to_string();
        }
    }

    // Distinguish drive-letter absolute vs unix absolute vs relative
    let (prefix, path_to_clean) = if p.len() >= 2
        && p.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && p.chars().nth(1) == Some(':')
    {
        let drive = &p[..2];
        let rest = p[2..].trim_start_matches('/');
        (format!("{drive}/"), rest)
    } else if p.starts_with('/') {
        ("/".to_string(), p.trim_start_matches('/'))
    } else {
        (String::new(), p.as_str())
    };

    // Normalize path segments (remove '.', handle '..', remove empty)
    let mut segments = Vec::new();
    for seg in path_to_clean.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        } else if seg == ".." {
            if !segments.is_empty() {
                segments.pop();
            }
        } else {
            segments.push(seg);
        }
    }

    let clean_path = segments.join("/");
    format!("file:{prefix}{clean_path}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_canonical_resource_id_schemes() {
        assert_eq!(
            canonical_resource_id("crate:strata-memory"),
            "crate:strata-memory"
        );
        assert_eq!(
            canonical_resource_id("task:orphan-task-1"),
            "task:orphan-task-1"
        );
        assert_eq!(
            canonical_resource_id("res:crate:strata-core"),
            "res:crate:strata-core"
        );
    }

    #[test]
    fn test_canonical_resource_id_relative_files() {
        assert_eq!(canonical_resource_id("src/lib.rs"), "file:src/lib.rs");
        assert_eq!(canonical_resource_id("./src/lib.rs"), "file:src/lib.rs");
        assert_eq!(canonical_resource_id(r".\src\lib.rs"), "file:src/lib.rs");
        assert_eq!(canonical_resource_id("file:src/lib.rs"), "file:src/lib.rs");
        assert_eq!(
            canonical_resource_id("file:./src/lib.rs"),
            "file:src/lib.rs"
        );
        assert_eq!(
            canonical_resource_id("crates/strata-cli/../strata-memory/src/lib.rs"),
            "file:crates/strata-memory/src/lib.rs"
        );
    }

    #[test]
    fn test_canonical_resource_id_with_workspace_root() {
        let root = Path::new("C:/Dev/mem-research");
        assert_eq!(
            canonical_resource_id_with_root(r"C:\Dev\mem-research\src\auth.rs", Some(root)),
            "file:src/auth.rs"
        );
        assert_eq!(
            canonical_resource_id_with_root(
                "file:C:/Dev/mem-research/crates/strata-cli/src/main.rs",
                Some(root)
            ),
            "file:crates/strata-cli/src/main.rs"
        );
        // Case-insensitivity on Windows
        assert_eq!(
            canonical_resource_id_with_root(r"c:\dev\mem-research\src\auth.rs", Some(root)),
            "file:src/auth.rs"
        );
    }

    #[test]
    fn test_canonical_resource_id_external_absolute() {
        let root = Path::new("C:/Dev/mem-research");
        assert_eq!(
            canonical_resource_id_with_root(r"D:\External\file.rs", Some(root)),
            "file:D:/External/file.rs"
        );
    }
}
