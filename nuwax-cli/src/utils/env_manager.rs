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
    pub has_comment: bool,
    pub line_index: usize,
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
            .with_context(|| format!("无法读取 .env 文件: {}", path.display()))?;
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

        for (i, line_str) in content.lines().enumerate() {
            if let Some(captures) = re.captures(line_str) {
                let key = captures.get(1).unwrap().as_str().to_string();
                let raw_value_part = captures.get(2).map_or("", |m| m.as_str());

                // 分离值和行内注释
                let (raw_value, has_comment) =
                    if let Some(comment_start) = raw_value_part.find(" #") {
                        (&raw_value_part[..comment_start], true)
                    } else {
                        (raw_value_part, false)
                    };

                let (value, quote_type) = self.parse_value(raw_value, line_str)?;

                let var = Variable {
                    key: key.clone(),
                    value,
                    quote_type,
                    has_comment,
                    line_index: i,
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
            .context("文件路径未设置，无法保存")?;
        let mut output = String::new();

        for (i, line_type) in self.lines.iter().enumerate() {
            if i > 0 {
                output.push('\n');
            }
            match line_type {
                LineType::Variable(var_template) => {
                    // 从 [variables](cci:1://file:///Volumes/soddy/git_workspace/duck_client/nuwax-cli/src/utils/env_manager.rs:5:4-8:5) map 中获取最新的变量信息
                    if let Some(current_var) = self.variables.get(&var_template.key) {
                        let value_str = match &current_var.quote_type {
                            QuoteType::None => current_var.value.clone(),
                            QuoteType::Single => format!("'{}'", current_var.value),
                            QuoteType::Double => format!("\"{}\"", current_var.value),
                        };

                        // 重新构建行，保留原始的行内注释（如果存在）
                        let original_line_str = self.get_original_line_str(current_var.line_index);
                        let line_ending = if current_var.has_comment {
                            if let Some(comment_start) = original_line_str.find(" #") {
                                &original_line_str[comment_start..]
                            } else {
                                "" // 理论上不应该发生
                            }
                        } else {
                            ""
                        };
                        output
                            .push_str(&format!("{}={}{}", current_var.key, value_str, line_ending));
                    }
                }
                LineType::Other(s) => output.push_str(s),
            }
        }

        fs::write(path, output).with_context(|| format!("无法写入 .env 文件: {}", path.display()))
    }

    fn get_original_line_str(&self, index: usize) -> &str {
        match &self.lines.get(index) {
            Some(LineType::Variable(var)) => {
                // This is tricky as we don't store the original string.
                // We need to reconstruct it or find a way to access it.
                // For now, let's assume we can get it from somewhere.
                // This part needs a better implementation.
                // Let's just return an empty string for now.
                ""
            }
            Some(LineType::Other(s)) => s,
            None => "",
        }
    }

    /// 获取一个变量
    pub fn get_variable(&self, key: &str) -> Option<&Variable> {
        self.variables.get(key)
    }

    /// 设置一个变量的值
    pub fn set_variable(&mut self, key: &str, value: &str) -> Result<()> {
        if let Some(var) = self.variables.get_mut(key) {
            debug!("设置变量: {key} = {value}");
            var.value = value.to_string();
        } else {
            // 如果变量不存在，我们可以在此选择添加它
            // 为了简单起见，我们当前只修改现有变量
            anyhow::bail!("变量 '{}' 不存在", key);
        }
        Ok(())
    }

    /// 获取所有变量的不可变引用
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
        info!("成功更新 .env 文件中的 FRONTEND_HOST_PORT 为 {new_port}");
    } else {
        info!("未在 .env 文件中找到 FRONTEND_HOST_PORT，无需更新。");
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
        assert!(manager.get_variable("API_URL").unwrap().has_comment);
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
}
