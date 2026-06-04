use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub(crate) fn secret_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| secret_value_from(&load_project_env().values, key))
}

fn secret_value_from(values: &HashMap<String, String>, key: &str) -> Option<String> {
    values
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

struct EnvFile {
    values: HashMap<String, String>,
}

fn load_project_env() -> EnvFile {
    #[cfg(test)]
    if std::env::var("AURA_TEST_DISABLE_PROJECT_ENV")
        .ok()
        .as_deref()
        == Some("1")
    {
        return EnvFile {
            values: HashMap::new(),
        };
    }

    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join(".env"));
        candidates.push(cwd.join("src-tauri").join(".env"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env"));
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|path| path.join(".env"))
            .unwrap_or_else(|| PathBuf::from(".env")),
    );

    for path in candidates {
        if let Ok(content) = fs::read_to_string(&path) {
            return EnvFile {
                values: parse_env_file(&content),
            };
        }
    }

    EnvFile {
        values: HashMap::new(),
    }
}

fn parse_env_file(content: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        values.insert(key.to_string(), clean_env_value(value));
    }
    values
}

fn clean_env_value(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let wrapped_in_double = bytes.first() == Some(&b'"') && bytes.last() == Some(&b'"');
        let wrapped_in_single = bytes.first() == Some(&b'\'') && bytes.last() == Some(&b'\'');
        if wrapped_in_double || wrapped_in_single {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}
