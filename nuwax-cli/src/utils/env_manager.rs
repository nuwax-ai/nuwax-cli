use anyhow::{Context, Result};
use client_core::atomic_file::{PermissionsPolicy, write_atomic};
use log::{debug, info};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Appended records. Existing records retain their original source bytes.
#[derive(Debug, Clone)]
pub enum LineType {
    Variable(Variable),
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum QuoteType {
    None,
    Single,
    Double,
}

#[derive(Debug, Clone)]
pub struct Variable {
    pub key: String,
    pub value: String,
    pub quote_type: QuoteType,
    /// Parsed metadata retained for callers; saving uses the original source span.
    #[allow(dead_code)]
    pub inline_comment: Option<String>,
}

struct VariableSpan {
    key: String,
    value: Range<usize>,
}

struct SourceLine {
    range: Range<usize>,
    variable: Option<VariableSpan>,
}

/// Edits only selected values; it never serializes unrelated assignments.
/// In particular, loading a file does not interpolate its values against the
/// process environment or turn escaped credentials into different source text.
pub struct EnvManager {
    file_path: Option<PathBuf>,
    source: String,
    lines: Vec<SourceLine>,
    appended: Vec<LineType>,
    variables: HashMap<String, Variable>,
    changed: HashSet<String>,
    removed: HashSet<String>,
    newline: &'static str,
}

impl Default for EnvManager {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvManager {
    pub fn new() -> Self {
        Self {
            file_path: None,
            source: String::new(),
            lines: Vec::new(),
            appended: Vec::new(),
            variables: HashMap::new(),
            changed: HashSet::new(),
            removed: HashSet::new(),
            newline: "\n",
        }
    }

    pub fn load<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read .env file: {}", path.display()))?;
        self.parse_content(&content)?;
        self.file_path = Some(path.to_path_buf());
        Ok(())
    }

    fn parse_content(&mut self, content: &str) -> Result<()> {
        self.source = content.to_string();
        self.lines.clear();
        self.appended.clear();
        self.variables.clear();
        self.changed.clear();
        self.removed.clear();
        self.newline = if content.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let assignment = Regex::new(r"^[ \t]*(?:export[ \t]+)?([\w.]+)[ \t]*=")?;
        // Windows editors can add a UTF-8 BOM. It belongs to the document,
        // not to its first key, and must survive even if that key is removed.
        let mut start = if content.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        };
        while start < content.len() {
            let physical_end = content[start..]
                .find('\n')
                .map_or(content.len(), |offset| start + offset + 1);
            let first_line = content[start..physical_end].trim_end_matches(['\r', '\n']);
            if let Some(captures) = assignment.captures(first_line) {
                let key = captures
                    .get(1)
                    .context("Missing assignment key")?
                    .as_str()
                    .to_string();
                let rhs = start + captures.get(0).context("Missing assignment prefix")?.end();
                let (rhs_end, end, comment) = scan_rhs(content, rhs)?;
                let raw_rhs = &content[rhs..rhs_end];
                let trimmed = raw_rhs.trim();
                // An empty assignment inserts its new value before the existing
                // whitespace/comment, so KEY= # note becomes KEY=value # note.
                let value_start = if trimmed.is_empty() {
                    rhs
                } else {
                    rhs + raw_rhs.len() - raw_rhs.trim_start().len()
                };
                let value_end = if trimmed.is_empty() {
                    rhs
                } else {
                    value_start + trimmed.len()
                };
                let (value, quote_type) = parse_value(trimmed);
                let comment =
                    comment.map(|comment_end| content[value_end..comment_end].to_string());
                self.variables.insert(
                    key.clone(),
                    Variable {
                        key: key.clone(),
                        value,
                        quote_type,
                        inline_comment: comment,
                    },
                );
                self.lines.push(SourceLine {
                    range: start..end,
                    variable: Some(VariableSpan {
                        key,
                        value: value_start..value_end,
                    }),
                });
                start = end;
            } else {
                self.lines.push(SourceLine {
                    range: start..physical_end,
                    variable: None,
                });
                start = physical_end;
            }
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        let path = self
            .file_path
            .as_ref()
            .context("File path is not set; cannot save .env changes")?;
        let output = self.render()?;
        if output == self.source {
            return Ok(());
        }
        write_atomic(path, output.as_bytes(), PermissionsPolicy::Preserve)
            .with_context(|| format!("Failed to write .env file: {}", path.display()))
    }

    fn render(&self) -> Result<String> {
        let mut output = if self.source.starts_with('\u{feff}') {
            String::from("\u{feff}")
        } else {
            String::new()
        };
        for line in &self.lines {
            if let Some(span) = &line.variable {
                if self.removed.contains(&span.key) {
                    continue;
                }
                if self.changed.contains(&span.key) {
                    let variable = self
                        .variables
                        .get(&span.key)
                        .context("Missing edited variable")?;
                    output.push_str(&self.source[line.range.start..span.value.start]);
                    output.push_str(&render_value(&variable.value, &variable.quote_type)?);
                    output.push_str(&self.source[span.value.end..line.range.end]);
                    continue;
                }
            }
            output.push_str(&self.source[line.range.clone()]);
        }
        let mut additions = Vec::new();
        for line in &self.appended {
            match line {
                LineType::Other(text) => additions.push(text.clone()),
                LineType::Variable(template) => {
                    if let Some(variable) = self.variables.get(&template.key) {
                        additions.push(format!(
                            "{}={}",
                            variable.key,
                            render_value(&variable.value, &variable.quote_type)?
                        ));
                    }
                }
            }
        }
        if !additions.is_empty() {
            if !output.is_empty() && output != "\u{feff}" && !output.ends_with('\n') {
                output.push_str(self.newline);
            }
            output.push_str(&additions.join(self.newline));
            if self.source.ends_with('\n') {
                output.push_str(self.newline);
            }
        }
        Ok(output)
    }

    #[allow(dead_code)]
    pub fn get_variable(&self, key: &str) -> Option<&Variable> {
        self.variables.get(key)
    }

    pub fn set_variable(&mut self, key: &str, value: &str) -> Result<()> {
        let variable = self
            .variables
            .get_mut(key)
            .with_context(|| format!("Variable '{key}' does not exist"))?;
        debug!("Setting variable: {key}");
        variable.value = value.to_string();
        self.changed.insert(key.to_string());
        Ok(())
    }

    pub fn upsert_variable(&mut self, key: &str, value: &str, quote: QuoteType) -> Result<()> {
        // Validate before mutating the document. Existing unrelated syntax is
        // retained as-is, while generated values must form a safe assignment.
        render_value(value, &quote)?;
        if let Some(variable) = self.variables.get_mut(key) {
            variable.value = value.to_string();
            variable.quote_type = quote;
            self.changed.insert(key.to_string());
        } else {
            let variable = Variable {
                key: key.to_string(),
                value: value.to_string(),
                quote_type: quote,
                inline_comment: None,
            };
            self.appended.push(LineType::Variable(variable.clone()));
            self.variables.insert(key.to_string(), variable);
        }
        Ok(())
    }

    /// Remove every definition of a key, including duplicate or appended rows.
    pub fn remove_variable(&mut self, key: &str) -> bool {
        let existed = self.variables.remove(key).is_some();
        self.removed.insert(key.to_string());
        self.changed.remove(key);
        self.appended
            .retain(|line| !matches!(line, LineType::Variable(variable) if variable.key == key));
        existed
    }

    pub fn ensure_marker(&mut self, marker: &str) {
        let exists = self.lines.iter().any(|line| {
            line.variable.is_none() && self.source[line.range.clone()].trim() == marker.trim()
        }) || self
            .appended
            .iter()
            .any(|line| matches!(line, LineType::Other(text) if text.trim() == marker.trim()));
        if !exists {
            self.appended.push(LineType::Other(marker.to_string()));
        }
    }

    #[allow(dead_code)]
    pub fn get_all_variables(&self) -> &HashMap<String, Variable> {
        &self.variables
    }
}

/// Locate a logical assignment, including newlines inside quoted values.
/// Return value end, record end including newline, and optional comment end.
fn scan_rhs(content: &str, start: usize) -> Result<(usize, usize, Option<usize>)> {
    let mut quote = None;
    let mut escaped = false;
    let mut previous_whitespace = true;
    for (offset, character) in content[start..].char_indices() {
        let index = start + offset;
        if character == '\n' && quote.is_none() {
            return Ok((index, index + 1, None));
        }
        if escaped {
            escaped = false;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
        } else if Some(character) == quote {
            quote = None;
        } else if quote.is_none() {
            match character {
                '\'' | '"' => quote = Some(character),
                '#' if previous_whitespace => {
                    let end = content[index..]
                        .find('\n')
                        .map_or(content.len(), |n| index + n + 1);
                    let comment_end = if content[..end].ends_with('\n') {
                        end - 1
                    } else {
                        end
                    };
                    let comment_end = if content[..comment_end].ends_with('\r') {
                        comment_end - 1
                    } else {
                        comment_end
                    };
                    return Ok((index, end, Some(comment_end)));
                }
                _ => {}
            }
        }
        previous_whitespace = character.is_whitespace();
    }
    anyhow::ensure!(
        quote.is_none(),
        "Unterminated quoted .env value at byte {start}"
    );
    Ok((content.len(), content.len(), None))
}

/// Decode display values without evaluating ${...} against process state.
/// Original source is always retained separately for untouched assignments.
fn parse_value(raw: &str) -> (String, QuoteType) {
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        return (raw[1..raw.len() - 1].to_string(), QuoteType::Single);
    }
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        let mut decoded = String::new();
        let mut characters = raw[1..raw.len() - 1].chars();
        while let Some(character) = characters.next() {
            if character != '\\' {
                decoded.push(character);
                continue;
            }
            match characters.next() {
                Some('n') => decoded.push('\n'),
                Some('r') => decoded.push('\r'),
                Some('t') => decoded.push('\t'),
                Some(next @ ('\\' | '"' | '\'' | '$' | ' ')) => decoded.push(next),
                Some(other) => {
                    decoded.push('\\');
                    decoded.push(other);
                }
                None => decoded.push('\\'),
            }
        }
        return (decoded, QuoteType::Double);
    }
    (raw.to_string(), QuoteType::None)
}

fn render_value(value: &str, quote: &QuoteType) -> Result<String> {
    match quote {
        QuoteType::None => {
            anyhow::ensure!(
                !value.contains(['\r', '\n']),
                "Unquoted .env values cannot contain newlines"
            );
            Ok(value.to_string())
        }
        QuoteType::Single => {
            anyhow::ensure!(
                !value.contains('\''),
                "Single-quoted .env values cannot contain a single quote"
            );
            Ok(format!("'{value}'"))
        }
        QuoteType::Double => {
            anyhow::ensure!(
                !value.contains('`'),
                "Use single quotes for literal backticks in .env values"
            );
            let mut encoded = String::from("\"");
            for character in value.chars() {
                match character {
                    '\\' | '"' | '$' => {
                        encoded.push('\\');
                        encoded.push(character);
                    }
                    '\n' => encoded.push_str("\\n"),
                    // dotenvy does not recognize \\r or \\t escapes. Literal
                    // controls inside quotes retain their meaning for both
                    // dotenvy and Compose (managed text is sanitized upstream).
                    other => encoded.push(other),
                }
            }
            encoded.push('"');
            Ok(encoded)
        }
    }
}

/// 便捷函数：更新前端端口
/// 在指定的 .env 文件中更新 FRONTEND_HOST_PORT 变量
pub fn update_frontend_port(env_path: &Path, new_port: u16) -> Result<()> {
    info!("env_path: {}, new_port: {}", env_path.display(), new_port);
    let mut env_manager = EnvManager::new();
    env_manager.load(env_path)?;

    let port_str = new_port.to_string();

    // 尝试更新变量
    if env_manager
        .set_variable("FRONTEND_HOST_PORT", &port_str)
        .is_ok()
    {
        env_manager.save()?;
        info!("Successfully updated FRONTEND_HOST_PORT in .env to {new_port}");
    } else {
        info!("FRONTEND_HOST_PORT not found in .env, no update needed.");
    }

    Ok(())
}

/// 便捷函数：从 .env 文件读取所有变量
///
/// # Arguments
///
/// * `env_path`: .env 文件的路径
///
/// # Returns
///
/// 返回一个包含所有环境变量的 HashMap
#[allow(dead_code)]
pub fn load_env_variables(env_path: &Path) -> Result<HashMap<String, String>> {
    let mut env_manager = EnvManager::new();
    env_manager.load(env_path)?;

    let mut result = HashMap::new();
    for (key, var) in env_manager.get_all_variables() {
        // 检查值是否为空，如果为空则不插入
        if !var.value.is_empty() {
            result.insert(key.clone(), var.value.clone());
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn injection_preserves_quoted_secrets_and_source_bytes() {
        let file = NamedTempFile::new().unwrap();
        let original = concat!(
            "  export MYSQL_PASSWORD = \"pa # ss\\\"C:\\\\tmp\\n\\$TOKEN\"  # keep me\r\n",
            "WINDOWS_PATH='C:\\work\\nuwax'\r\n",
            "REFERENCE=\"${SOME_EXTERNAL_VALUE}\"\r\n",
            "MULTILINE='first # literal\r\nDEVICE_ID=inside-value\r\nlast'\r\n",
            "# Original comment\r\n"
        );
        fs::write(file.path(), original).unwrap();
        let mut manager = EnvManager::new();
        manager.load(file.path()).unwrap();
        assert!(manager.get_variable("DEVICE_ID").is_none());
        manager
            .upsert_variable("DEVICE_ID", "v1:new", QuoteType::None)
            .unwrap();
        manager.save().unwrap();
        assert_eq!(
            fs::read_to_string(file.path()).unwrap(),
            format!("{original}DEVICE_ID=v1:new\r\n")
        );
        manager.load(file.path()).unwrap();
        manager
            .upsert_variable("DEVICE_ID", "v1:new", QuoteType::None)
            .unwrap();
        manager.save().unwrap();
        assert_eq!(
            fs::read_to_string(file.path()).unwrap(),
            format!("{original}DEVICE_ID=v1:new\r\n")
        );
    }

    #[test]
    fn updates_duplicate_keys_with_their_own_comments_and_prefixes() {
        let mut manager = EnvManager::new();
        manager.parse_content(" export DEVICE_ID = 'old # literal' # first\r\nDEVICE_ID=old2  # second\r\nOTHER=keep\r\n").unwrap();
        manager
            .upsert_variable("DEVICE_ID", "v1:new", QuoteType::None)
            .unwrap();
        assert_eq!(
            manager.render().unwrap(),
            " export DEVICE_ID = v1:new # first\r\nDEVICE_ID=v1:new  # second\r\nOTHER=keep\r\n"
        );
    }

    #[test]
    fn removal_deletes_all_definitions_and_supports_reinsertion() {
        let mut manager = EnvManager::new();
        manager.parse_content("DEVICE_FIELDS_DISK_SERIAL=stale\nOTHER=keep\nexport DEVICE_FIELDS_DISK_SERIAL=also_stale\n").unwrap();
        assert!(manager.remove_variable("DEVICE_FIELDS_DISK_SERIAL"));
        assert!(!manager.remove_variable("NOT_PRESENT"));
        assert_eq!(manager.render().unwrap(), "OTHER=keep\n");
        manager
            .upsert_variable("DEVICE_FIELDS_DISK_SERIAL", "current", QuoteType::None)
            .unwrap();
        assert_eq!(
            manager.render().unwrap(),
            "OTHER=keep\nDEVICE_FIELDS_DISK_SERIAL=current\n"
        );
        assert!(manager.remove_variable("DEVICE_FIELDS_DISK_SERIAL"));
        assert_eq!(manager.render().unwrap(), "OTHER=keep\n");
    }

    #[test]
    fn empty_assignment_keeps_comment_and_missing_final_newline() {
        let mut manager = EnvManager::new();
        manager
            .parse_content("A= # explanation\nLAST='unchanged'")
            .unwrap();
        manager.set_variable("A", "80").unwrap();
        assert_eq!(
            manager.render().unwrap(),
            "A=80 # explanation\nLAST='unchanged'"
        );
    }

    #[test]
    fn bom_first_assignment_updates_without_creating_duplicate() {
        let mut manager = EnvManager::new();
        manager
            .parse_content("\u{feff}FRONTEND_HOST_PORT=80 # keep\r\nOTHER='unchanged'\r\n")
            .unwrap();
        manager.set_variable("FRONTEND_HOST_PORT", "8090").unwrap();
        assert_eq!(
            manager.render().unwrap(),
            "\u{feff}FRONTEND_HOST_PORT=8090 # keep\r\nOTHER='unchanged'\r\n"
        );
    }

    #[test]
    fn bom_survives_first_key_removal_and_reinsertion() {
        let mut manager = EnvManager::new();
        manager
            .parse_content("\u{feff}DEVICE_FIELDS_DISK_SERIAL=stale\r\n")
            .unwrap();
        assert!(manager.remove_variable("DEVICE_FIELDS_DISK_SERIAL"));
        assert_eq!(manager.render().unwrap(), "\u{feff}");
        manager
            .upsert_variable("DEVICE_ID", "v1:current", QuoteType::None)
            .unwrap();
        assert_eq!(
            manager.render().unwrap(),
            "\u{feff}DEVICE_ID=v1:current\r\n"
        );
    }

    #[test]
    fn edited_double_quotes_round_trip_with_dotenvy() {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), "VALUE=\"old\"\n").unwrap();
        let value = "quote \" slash \\ dollar $TOKEN\nnext line";
        let mut manager = EnvManager::new();
        manager.load(file.path()).unwrap();
        manager.set_variable("VALUE", value).unwrap();
        manager.save().unwrap();
        let decoded: HashMap<_, _> = dotenvy::from_path_iter(file.path())
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(decoded.get("VALUE").map(String::as_str), Some(value));
    }

    #[test]
    fn updating_a_quoted_value_keeps_hashes_in_the_value() {
        let mut manager = EnvManager::new();
        manager
            .parse_content("HOSTNAME='prod # one' # note\n")
            .unwrap();
        assert_eq!(
            manager.get_variable("HOSTNAME").unwrap().value,
            "prod # one"
        );
        manager
            .upsert_variable("HOSTNAME", "prod # two", QuoteType::Single)
            .unwrap();
        assert_eq!(manager.render().unwrap(), "HOSTNAME='prod # two' # note\n");
    }

    #[test]
    fn test_env_parsing() {
        let content = r#"
# This is a comment
FRONTEND_HOST_PORT=80
BACKEND_PORT="3000"
DB_HOST='localhost'
API_URL=http://localhost:3000 # inline comment
EMPTY_VAR=
ESCAPED_VAR="hello\nworld"
"#;

        let mut manager = EnvManager::new();
        manager.parse_content(content).unwrap();

        assert_eq!(manager.variables.len(), 6);
        assert_eq!(
            manager.get_variable("FRONTEND_HOST_PORT").unwrap().value,
            "80"
        );
        assert_eq!(manager.get_variable("BACKEND_PORT").unwrap().value, "3000");
        assert_eq!(
            manager.get_variable("BACKEND_PORT").unwrap().quote_type,
            QuoteType::Double
        );
        assert_eq!(
            manager.get_variable("DB_HOST").unwrap().quote_type,
            QuoteType::Single
        );
        assert_eq!(
            manager
                .get_variable("API_URL")
                .unwrap()
                .inline_comment
                .as_deref(),
            Some(" # inline comment")
        );
        assert_eq!(
            manager.get_variable("ESCAPED_VAR").unwrap().value,
            "hello\nworld"
        );
    }

    #[test]
    fn test_save_and_load() {
        let temp_file = NamedTempFile::new().unwrap();
        let initial_content = r#"
KEY1=VALUE1
# A comment
KEY2="old_value"
KEY3='single_quoted'
"#;
        fs::write(temp_file.path(), initial_content).unwrap();

        let mut manager = EnvManager::new();
        manager.load(temp_file.path()).unwrap();

        // 修改一个变量
        manager.set_variable("KEY2", "new_value").unwrap();

        // 保存
        manager.save().unwrap();

        // 读回并验证
        let final_content = fs::read_to_string(temp_file.path()).unwrap();
        let expected_content = r#"
KEY1=VALUE1
# A comment
KEY2="new_value"
KEY3='single_quoted'"#;

        // 比较时忽略由于实现细节可能产生的尾部换行符差异
        assert_eq!(final_content.trim(), expected_content.trim());

        // 验证修改是否正确应用
        let mut final_manager = EnvManager::new();
        final_manager.parse_content(&final_content).unwrap();
        assert_eq!(
            final_manager.get_variable("KEY2").unwrap().value,
            "new_value"
        );
        assert_eq!(
            final_manager.get_variable("KEY2").unwrap().quote_type,
            QuoteType::Double
        );
    }

    #[test]
    fn test_set_variable_preserves_inline_comment() {
        // 回归测试：改值不丢行内注释（旧行为依赖 line_index 反查原始行，
        // get_original_line_str 恒返回空串，注释会静默丢失）
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(
            temp_file.path(),
            "FRONTEND_HOST_PORT=80 # default http port\nMYSQL_USER=agent\n",
        )
        .unwrap();

        let mut manager = EnvManager::new();
        manager.load(temp_file.path()).unwrap();
        manager.set_variable("FRONTEND_HOST_PORT", "8090").unwrap();
        manager.save().unwrap();

        let content = fs::read_to_string(temp_file.path()).unwrap();
        assert_eq!(
            content.trim(),
            "FRONTEND_HOST_PORT=8090 # default http port\nMYSQL_USER=agent"
        );
    }

    #[test]
    fn test_upsert_appends_new_key_with_marker() {
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(
            temp_file.path(),
            "FRONTEND_HOST_PORT=80\nMYSQL_USER=agent\n",
        )
        .unwrap();

        let mut manager = EnvManager::new();
        manager.load(temp_file.path()).unwrap();
        manager.ensure_marker("# --- managed ---");
        manager
            .upsert_variable("DEVICE_ID", "v1:abc123", QuoteType::None)
            .unwrap();
        manager
            .upsert_variable(
                "DEVICE_FIELDS",
                "{\"machine_id\":\"h1\"}",
                QuoteType::Single,
            )
            .unwrap();
        manager.save().unwrap();

        let content = fs::read_to_string(temp_file.path()).unwrap();
        let expected = "FRONTEND_HOST_PORT=80\nMYSQL_USER=agent\n# --- managed ---\nDEVICE_ID=v1:abc123\nDEVICE_FIELDS='{\"machine_id\":\"h1\"}'";
        assert_eq!(content.trim(), expected);

        // 读回验证解析结果
        let mut reloaded = EnvManager::new();
        reloaded.load(temp_file.path()).unwrap();
        assert_eq!(
            reloaded.get_variable("DEVICE_ID").unwrap().value,
            "v1:abc123"
        );
        assert_eq!(
            reloaded.get_variable("DEVICE_FIELDS").unwrap().value,
            "{\"machine_id\":\"h1\"}"
        );
    }

    #[test]
    fn test_upsert_updates_existing_key_and_quote() {
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(temp_file.path(), "DEVICE_ID=old\nOTHER=1\n").unwrap();

        let mut manager = EnvManager::new();
        manager.load(temp_file.path()).unwrap();
        manager
            .upsert_variable("DEVICE_ID", "v1:new", QuoteType::None)
            .unwrap();
        manager.save().unwrap();

        let content = fs::read_to_string(temp_file.path()).unwrap();
        // 更新在原位置，OTHER 不受影响
        assert_eq!(content.trim(), "DEVICE_ID=v1:new\nOTHER=1");
    }

    #[test]
    fn test_upsert_idempotent_and_marker_not_duplicated() {
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(temp_file.path(), "A=1\n").unwrap();

        for _ in 0..2 {
            let mut manager = EnvManager::new();
            manager.load(temp_file.path()).unwrap();
            manager.ensure_marker("# --- managed ---");
            manager
                .upsert_variable("DEVICE_ID", "v1:same", QuoteType::None)
                .unwrap();
            manager.save().unwrap();
        }

        let content = fs::read_to_string(temp_file.path()).unwrap();
        assert_eq!(content.trim(), "A=1\n# --- managed ---\nDEVICE_ID=v1:same");
        assert_eq!(content.matches("# --- managed ---").count(), 1);
    }
}
