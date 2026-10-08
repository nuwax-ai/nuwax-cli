use anyhow::{Context, Result};
use log::{debug, info};
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// 表示 .env 文件中的一行
#[derive(Debug, Clone)]
pub enum LineType {
    Variable(Variable),
    Other(String), // 用于注释或空行
}

/// 变量的引号类型
#[derive(Debug, Clone, PartialEq)]
pub enum QuoteType {
    None,
    Single,
    Double,
}

/// 表示一个环境变量
#[derive(Debug, Clone)]
pub struct Variable {
    pub key: String,
    pub value: String,
    pub quote_type: QuoteType,
    /// 行内注释（自 " #" 起，含前导空格）；保存时原样回写，改值不丢注释
    pub inline_comment: Option<String>,
}

/// 管理 .env 文件的结构
pub struct EnvManager {
    file_path: Option<PathBuf>,
    lines: Vec<LineType>,
    variables: HashMap<String, Variable>,
}

impl EnvManager {
    /// 创建一个新的 EnvManager 实例
    pub fn new() -> Self {
        EnvManager {
            file_path: None,
            lines: Vec::new(),
            variables: HashMap::new(),
        }
    }

    /// 从文件加载 .env 内容
    pub fn load<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref();
        self.file_path = Some(path.to_path_buf());
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read .env file: {}", path.display()))?;
        self.parse_content(&content)?;
        Ok(())
    }

    /// 解析 .env 文件内容
    fn parse_content(&mut self, content: &str) -> Result<()> {
        self.lines.clear();
        self.variables.clear();

        // 正则表达式，用于从行中捕获键和值部分
        // 1. `^s*`: 行首的任意空白
        // 2. [(?:export\s+)?](cci:1://file:///Volumes/soddy/git_workspace/duck_client/nuwax-cli/src/utils/env_manager.rs:64:4-73:5): 可选的 `export` 关键字
        // 3. [([\w.]+)](cci:1://file:///Volumes/soddy/git_workspace/duck_client/nuwax-cli/src/utils/env_manager.rs:64:4-73:5): 捕获组 1, 变量的键 (字母, 数字, _, .)
        // 4. `\s*=\s*`: 等号，前后可有空白
        // 5. [(.*?)](cci:1://file:///Volumes/soddy/git_workspace/duck_client/nuwax-cli/src/utils/env_manager.rs:64:4-73:5): 捕获组 2, 值以及可能的行内注释 (非贪婪)
        // 6. `\s*$`: 行尾的任意空白
        let re = Regex::new(r"^\s*(?:export\s+)?([\w.]+)\s*=\s*(.*?)?\s*$").unwrap();

        for line_str in content.lines() {
            if let Some(captures) = re.captures(line_str) {
                let key = captures.get(1).unwrap().as_str().to_string();
                let raw_value_part = captures.get(2).map_or("", |m| m.as_str());

                // 分离值和行内注释（注释自 " #" 起原样保留，保存时回写）
                let (raw_value, inline_comment) =
                    if let Some(comment_start) = raw_value_part.find(" #") {
                        (
                            &raw_value_part[..comment_start],
                            Some(raw_value_part[comment_start..].to_string()),
                        )
                    } else {
                        (raw_value_part, None)
                    };

                let (value, quote_type) = self.parse_value(raw_value, line_str)?;

                let var = Variable {
                    key: key.clone(),
                    value,
                    quote_type,
                    inline_comment,
                };

                self.lines.push(LineType::Variable(var.clone()));
                self.variables.insert(key, var);
            } else {
                // 处理空行或注释
                self.lines.push(LineType::Other(line_str.to_string()));
            }
        }
        Ok(())
    }

    /// 使用 dotenvy 解析值以处理转义
    fn parse_value(&self, raw_value: &str, _original_line: &str) -> Result<(String, QuoteType)> {
        let trimmed_value = raw_value.trim();
        let quote_type = if trimmed_value.starts_with('\'') && trimmed_value.ends_with('\'') {
            QuoteType::Single
        } else if trimmed_value.starts_with('"') && trimmed_value.ends_with('"') {
            QuoteType::Double
        } else {
            QuoteType::None
        };

        // 对于无引号或单引号的值，我们直接使用原始值，因为dotenvy的行为可能不完全符合我们的需求
        // 只有双引号的值才需要dotenvy来处理复杂的转义序列
        if quote_type == QuoteType::Double {
            // 我们需要给dotenvy一个完整的 "KEY=VALUE" 行来进行解析
            let fake_line_for_parser = format!("_DUMMY_KEY_={trimmed_value}");
            let mut iter = dotenvy::Iter::new(Cursor::new(fake_line_for_parser));

            if let Some(item) = iter.next() {
                let (_key, value) = item?;
                return Ok((value, quote_type));
            }
        }

        // 对于 None 和 Single quote，我们手动去除引号
        let value = match quote_type {
            QuoteType::None => trimmed_value.to_string(),
            QuoteType::Single => trimmed_value
                .strip_prefix('\'')
                .unwrap()
                .strip_suffix('\'')
                .unwrap()
                .to_string(),
            QuoteType::Double => unreachable!(), // 已在上面处理
        };

        Ok((value, quote_type))
    }

    /// 保存对 .env 文件的更改
    pub fn save(&self) -> Result<()> {
        let path = self
            .file_path
            .as_ref()
            .context("File path is not set; cannot save .env changes")?;
        let mut output = String::new();

        for (i, line_type) in self.lines.iter().enumerate() {
            if i > 0 {
                output.push('\n');
            }
            match line_type {
                LineType::Variable(var_template) => {
                    // 从 variables map 中获取最新的变量信息
                    if let Some(current_var) = self.variables.get(&var_template.key) {
                        let value_str = match &current_var.quote_type {
                            QuoteType::None => current_var.value.clone(),
                            QuoteType::Single => format!("'{}'", current_var.value),
                            QuoteType::Double => format!("\"{}\"", current_var.value),
                        };
                        // 回写解析时保留的行内注释，改值不丢注释
                        let comment = current_var.inline_comment.as_deref().unwrap_or("");
                        output.push_str(&format!("{}={}{}", current_var.key, value_str, comment));
                    }
                }
                LineType::Other(s) => output.push_str(s),
            }
        }

        fs::write(path, output)
            .with_context(|| format!("Failed to write .env file: {}", path.display()))
    }

    /// 获取一个变量
    #[allow(dead_code)]
    pub fn get_variable(&self, key: &str) -> Option<&Variable> {
        self.variables.get(key)
    }

    /// 设置一个变量的值
    pub fn set_variable(&mut self, key: &str, value: &str) -> Result<()> {
        if let Some(var) = self.variables.get_mut(key) {
            debug!("Setting variable: {key} = {value}");
            var.value = value.to_string();
        } else {
            // 如果变量不存在，我们可以在此选择添加它
            // 为了简单起见，我们当前只修改现有变量
            anyhow::bail!("Variable '{}' does not exist", key);
        }
        Ok(())
    }

    /// 键存在则更新（含引号风格），不存在则追加到文件末尾。
    /// 与 `set_variable` 的区别：新增键不需要预先存在于 .env 中
    /// （设备指纹等 CLI 托管键由工具追加，设计文档 §6.2）。
    pub fn upsert_variable(&mut self, key: &str, value: &str, quote: QuoteType) -> Result<()> {
        if let Some(var) = self.variables.get_mut(key) {
            debug!("Upserting existing variable: {key}");
            var.value = value.to_string();
            var.quote_type = quote;
            return Ok(());
        }
        debug!("Appending new variable: {key}");
        let var = Variable {
            key: key.to_string(),
            value: value.to_string(),
            quote_type: quote,
            inline_comment: None,
        };
        self.lines.push(LineType::Variable(var.clone()));
        self.variables.insert(key.to_string(), var);
        Ok(())
    }

    /// 确保托管区块的注释标记存在（不存在则追加一行注释）
    pub fn ensure_marker(&mut self, marker: &str) {
        let exists = self
            .lines
            .iter()
            .any(|line| matches!(line, LineType::Other(s) if s.trim() == marker.trim()));
        if !exists {
            self.lines.push(LineType::Other(marker.to_string()));
        }
    }

    /// 获取所有变量的不可变引用
    #[allow(dead_code)]
    pub fn get_all_variables(&self) -> &HashMap<String, Variable> {
        &self.variables
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
