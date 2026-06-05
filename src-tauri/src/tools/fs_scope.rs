use std::path::{Component, Path, PathBuf};

use crate::agent::AgentError;

const MAX_TEXT_FILE_BYTES: u64 = 1024 * 1024;

pub fn allowed_file(path: &str) -> Result<PathBuf, AgentError> {
    allowed_file_with_roots(path, &[])
}

pub fn allowed_file_with_roots(path: &str, extra_roots: &[PathBuf]) -> Result<PathBuf, AgentError> {
    let path = resolve_existing_with_roots(path, extra_roots)?;
    if !path.is_file() {
        return Err(AgentError::Tool("目标不是文件。".to_string()));
    }
    if std::fs::metadata(&path)
        .map_err(|error| AgentError::Tool(format!("无法读取文件元数据: {error}")))?
        .len()
        > MAX_TEXT_FILE_BYTES
    {
        return Err(AgentError::Tool(
            "文件超过 1MB，旧版本地工具不会读取这么大的文本。".to_string(),
        ));
    }
    Ok(path)
}

pub fn allowed_directory(path: &str) -> Result<PathBuf, AgentError> {
    allowed_directory_with_roots(path, &[])
}

pub fn allowed_directory_with_roots(
    path: &str,
    extra_roots: &[PathBuf],
) -> Result<PathBuf, AgentError> {
    let path = resolve_existing_with_roots(path, extra_roots)?;
    if !path.is_dir() {
        return Err(AgentError::Tool("目标不是目录。".to_string()));
    }
    Ok(path)
}

pub fn allowed_existing(path: &str) -> Result<PathBuf, AgentError> {
    allowed_existing_with_roots(path, &[])
}

pub fn allowed_existing_with_roots(
    path: &str,
    extra_roots: &[PathBuf],
) -> Result<PathBuf, AgentError> {
    resolve_existing_with_roots(path, extra_roots)
}

pub fn allowed_new_path(path: &str) -> Result<PathBuf, AgentError> {
    allowed_new_path_with_roots(path, &[])
}

pub fn allowed_new_path_with_roots(
    path: &str,
    extra_roots: &[PathBuf],
) -> Result<PathBuf, AgentError> {
    let expanded = expand_home(path);
    let raw = PathBuf::from(expanded);
    let absolute = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir()
            .map_err(|error| AgentError::Tool(format!("无法读取当前目录: {error}")))?
            .join(raw)
    };
    if absolute.exists() {
        let real = absolute
            .canonicalize()
            .map_err(|error| AgentError::Tool(format!("无法解析目标路径: {error}")))?;
        if real.is_dir() {
            return Err(AgentError::Tool(
                "目标路径是目录，不能作为文件写入。".to_string(),
            ));
        }
        ensure_in_allowed_scope_with_roots(&real, extra_roots)?;
        return Ok(real);
    }
    if let Some(parent) = absolute.parent() {
        let parent = parent
            .canonicalize()
            .map_err(|error| AgentError::Tool(format!("无法解析父目录: {error}")))?;
        ensure_in_allowed_scope_with_roots(&parent, extra_roots)?;
    } else {
        return Err(AgentError::Tool("目标路径缺少父目录。".to_string()));
    }
    if is_sensitive(&absolute) {
        return Err(AgentError::Tool(
            "路径属于系统、密钥或敏感应用目录。".to_string(),
        ));
    }
    Ok(absolute)
}

pub fn allowed_new_directory(path: &str) -> Result<PathBuf, AgentError> {
    allowed_new_directory_with_roots(path, &[])
}

pub fn allowed_new_directory_with_roots(
    path: &str,
    extra_roots: &[PathBuf],
) -> Result<PathBuf, AgentError> {
    let expanded = expand_home(path);
    let raw = PathBuf::from(expanded);
    let absolute = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir()
            .map_err(|error| AgentError::Tool(format!("无法读取当前目录: {error}")))?
            .join(raw)
    };

    if absolute
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(AgentError::Tool(
            "目录路径不能包含上级目录跳转。".to_string(),
        ));
    }

    if absolute.exists() {
        let real = absolute
            .canonicalize()
            .map_err(|error| AgentError::Tool(format!("无法解析目标目录: {error}")))?;
        if !real.is_dir() {
            return Err(AgentError::Tool(
                "目标路径是文件，不能作为目录创建。".to_string(),
            ));
        }
        ensure_in_allowed_scope_with_roots(&real, extra_roots)?;
        return Ok(real);
    }

    let existing_ancestor = nearest_existing_ancestor(&absolute)?;
    ensure_in_allowed_scope_with_roots(&existing_ancestor, extra_roots)?;
    if is_sensitive(&absolute) {
        return Err(AgentError::Tool(
            "路径属于系统、密钥或敏感应用目录。".to_string(),
        ));
    }
    Ok(absolute)
}

fn resolve_existing_with_roots(path: &str, extra_roots: &[PathBuf]) -> Result<PathBuf, AgentError> {
    let expanded = expand_home(path);
    let raw = PathBuf::from(expanded);
    let absolute = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir()
            .map_err(|error| AgentError::Tool(format!("无法读取当前目录: {error}")))?
            .join(raw)
    };
    let real = absolute
        .canonicalize()
        .map_err(|error| AgentError::Tool(format!("无法解析路径: {error}")))?;
    ensure_in_allowed_scope_with_roots(&real, extra_roots)?;
    Ok(real)
}

fn nearest_existing_ancestor(path: &Path) -> Result<PathBuf, AgentError> {
    let mut current = path
        .parent()
        .ok_or_else(|| AgentError::Tool("目标目录缺少父目录。".to_string()))?;
    loop {
        if current.exists() {
            return current
                .canonicalize()
                .map_err(|error| AgentError::Tool(format!("无法解析父目录: {error}")));
        }
        current = current
            .parent()
            .ok_or_else(|| AgentError::Tool("无法找到已存在的父目录。".to_string()))?;
    }
}

fn ensure_in_allowed_scope_with_roots(
    path: &Path,
    extra_roots: &[PathBuf],
) -> Result<(), AgentError> {
    let roots = allowed_roots(extra_roots);
    if !roots.iter().any(|root| path.starts_with(root)) {
        return Err(AgentError::Tool(
            "路径不在当前项目或常用用户目录内。".to_string(),
        ));
    }
    if is_sensitive(path) {
        return Err(AgentError::Tool(
            "路径属于系统、密钥或敏感应用目录。".to_string(),
        ));
    }
    Ok(())
}

pub fn normalize_extra_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut normalized = Vec::new();
    for root in roots {
        if let Ok(real) = root.canonicalize() {
            if real.is_dir()
                && !is_sensitive(&real)
                && !normalized
                    .iter()
                    .any(|existing: &PathBuf| existing == &real)
            {
                normalized.push(real);
            }
        }
    }
    normalized
}

fn allowed_roots(extra_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for root in extra_roots {
        if let Ok(real) = root.canonicalize() {
            if real.is_dir()
                && !is_sensitive(&real)
                && !roots.iter().any(|existing: &PathBuf| existing == &real)
            {
                roots.push(real);
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir().and_then(|path| path.canonicalize()) {
        roots.push(cwd);
    }
    for root in [
        dirs::home_dir(),
        dirs::desktop_dir(),
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::audio_dir(),
        dirs::video_dir(),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(real) = root.canonicalize() {
            roots.push(real);
        }
    }
    roots
}

fn expand_home(path: &str) -> String {
    if (path.starts_with("~/") || path.starts_with("~\\")) && dirs::home_dir().is_some() {
        let home = dirs::home_dir().expect("checked above");
        return home
            .join(path.trim_start_matches("~/").trim_start_matches("~\\"))
            .to_string_lossy()
            .to_string();
    }
    path.to_string()
}

pub fn is_sensitive_path(path: &Path) -> bool {
    let text = path.to_string_lossy().to_ascii_lowercase();
    [
        "\\windows\\",
        "\\program files\\",
        "\\program files (x86)\\",
        "\\appdata\\local\\",
        "\\appdata\\roaming\\",
        "\\.ssh\\",
        "\\.gnupg\\",
        "\\.aws\\",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn is_sensitive(path: &Path) -> bool {
    is_sensitive_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn new_path_rejects_existing_target_outside_allowed_scope() {
        let target =
            std::env::temp_dir().join(format!("atlas_scope_existing_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, "outside").unwrap();

        let result = allowed_new_path(&target.to_string_lossy());

        assert!(result.is_err());
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn extra_roots_allow_current_project_files_outside_default_scope() {
        let base = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let project = base.join(format!("atlas_scope_project_{}", Uuid::new_v4()));
        let target = project.join("notes.txt");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(&target, "project").unwrap();

        assert!(allowed_file(&target.to_string_lossy()).is_err());
        assert!(
            allowed_file_with_roots(&target.to_string_lossy(), std::slice::from_ref(&project))
                .is_ok()
        );

        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn new_path_rejects_symlink_that_resolves_outside_allowed_scope() {
        let outside =
            std::env::temp_dir().join(format!("atlas_scope_symlink_{}.txt", Uuid::new_v4()));
        let link = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("atlas_scope_link_{}.txt", Uuid::new_v4()));
        std::fs::write(&outside, "outside").unwrap();

        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(&outside, &link).is_ok();
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_file(&outside, &link).is_ok();

        if created {
            let result = allowed_new_path(&link.to_string_lossy());
            assert!(result.is_err());
        }

        let _ = std::fs::remove_file(link);
        let _ = std::fs::remove_file(outside);
    }

    #[test]
    fn new_directory_allows_nested_path_under_allowed_scope() {
        let base = std::env::current_dir().unwrap().join("target");
        std::fs::create_dir_all(&base).unwrap();
        let target = base
            .join(format!("atlas_scope_dir_{}", Uuid::new_v4()))
            .join("child")
            .join("grandchild");

        let result = allowed_new_directory(&target.to_string_lossy()).unwrap();

        assert_eq!(result, target);
    }

    #[test]
    fn new_directory_allows_existing_directory_under_allowed_scope() {
        let base = std::env::current_dir().unwrap().join("target");
        std::fs::create_dir_all(&base).unwrap();
        let target = base.join(format!("atlas_scope_existing_dir_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&target).unwrap();

        let result = allowed_new_directory(&target.to_string_lossy()).unwrap();

        assert!(result.is_dir());
        let _ = std::fs::remove_dir_all(target);
    }

    #[test]
    fn new_directory_rejects_existing_file() {
        let base = std::env::current_dir().unwrap().join("target");
        std::fs::create_dir_all(&base).unwrap();
        let target = base.join(format!("atlas_scope_file_{}.txt", Uuid::new_v4()));
        std::fs::write(&target, "file").unwrap();

        let result = allowed_new_directory(&target.to_string_lossy());

        assert!(result.is_err());
        let _ = std::fs::remove_file(target);
    }

    #[test]
    fn existing_home_directory_is_inside_allowed_scope() {
        let Some(home) = dirs::home_dir() else {
            return;
        };

        let result = allowed_directory(&home.to_string_lossy());

        assert!(result.is_ok());
    }
}
