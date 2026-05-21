/// 环境检测模块
///
/// 此模块提供环境检测功能，支持通过环境变量 NUWAX_CLI_ENV 来区分不同的运行环境
///
/// 支持的环境：
/// - Production: 生产环境（默认）
/// - Test: 测试环境
///
/// 使用方式：
/// - 不设置环境变量或设置 NUWAX_CLI_ENV=production → 生产环境
/// - 设置 NUWAX_CLI_ENV=testing 或 NUWAX_CLI_ENV=test → 测试环境
///
/// 环境类型枚举
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Environment {
    /// 生产环境
    #[default]
    Production,
    /// 测试环境
    Test,
}

impl Environment {
    /// 从环境变量检测当前环境
    ///
    /// 环境变量优先级：
    /// 1. NUWAX_CLI_ENV=testing 或 NUWAX_CLI_ENV=test → Test
    /// 2. NUWAX_CLI_ENV=production 或未设置 → Production
    ///
    /// # Examples
    /// ```
    /// use client_core::environment::Environment;
    ///
    /// // 获取当前环境
    /// let env = Environment::from_env();
    /// println!("Current environment: {:?}", env);
    /// ```
    pub fn from_env() -> Self {
        match std::env::var("NUWAX_CLI_ENV")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "testing" | "test" => Environment::Test,
            "production" | "prod" | "" => Environment::Production,
            // 对于未知值，默认为生产环境以确保安全
            _ => {
                tracing::warn!("Unknown NUWAX_CLI_ENV value, defaulting to Production environment");
                Environment::Production
            }
        }
    }

    /// 检查是否为测试环境
    pub fn is_testing(&self) -> bool {
        matches!(self, Environment::Test)
    }

    /// 检查是否为生产环境
    pub fn is_production(&self) -> bool {
        matches!(self, Environment::Production)
    }

    /// 获取环境的字符串表示
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Production => "production",
            Environment::Test => "testing",
        }
    }

    /// 获取环境的显示名称（用户友好）
    pub fn display_name(&self) -> &'static str {
        match self {
            Environment::Production => "Production",
            Environment::Test => "Testing",
        }
    }
}

impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_env_production() {
        // 测试未设置环境变量时默认为生产环境
        unsafe {
            std::env::remove_var("NUWAX_CLI_ENV");
        }
        assert_eq!(Environment::from_env(), Environment::Production);

        // 测试显式设置生产环境
        unsafe {
            std::env::set_var("NUWAX_CLI_ENV", "production");
        }
        assert_eq!(Environment::from_env(), Environment::Production);

        // 测试生产环境简写
        unsafe {
            std::env::set_var("NUWAX_CLI_ENV", "prod");
        }
        assert_eq!(Environment::from_env(), Environment::Production);
    }

    #[test]
    fn test_from_env_testing() {
        // 测试测试环境
        unsafe {
            std::env::set_var("NUWAX_CLI_ENV", "testing");
        }
        assert_eq!(Environment::from_env(), Environment::Test);

        // 测试测试环境简写
        unsafe {
            std::env::set_var("NUWAX_CLI_ENV", "test");
        }
        assert_eq!(Environment::from_env(), Environment::Test);

        // 测试大小写不敏感
        unsafe {
            std::env::set_var("NUWAX_CLI_ENV", "TESTING");
        }
        assert_eq!(Environment::from_env(), Environment::Test);

        unsafe {
            std::env::set_var("NUWAX_CLI_ENV", "Test");
        }
        assert_eq!(Environment::from_env(), Environment::Test);
    }

    #[test]
    fn test_from_env_unknown() {
        // 测试未知值时默认为生产环境
        unsafe {
            std::env::set_var("NUWAX_CLI_ENV", "staging");
        }
        assert_eq!(Environment::from_env(), Environment::Production);

        unsafe {
            std::env::set_var("NUWAX_CLI_ENV", "development");
        }
        assert_eq!(Environment::from_env(), Environment::Production);
    }

    #[test]
    fn test_environment_methods() {
        let prod = Environment::Production;
        let test = Environment::Test;

        assert!(prod.is_production());
        assert!(!prod.is_testing());
        assert_eq!(prod.as_str(), "production");
        assert_eq!(prod.display_name(), "Production");

        assert!(test.is_testing());
        assert!(!test.is_production());
        assert_eq!(test.as_str(), "testing");
        assert_eq!(test.display_name(), "Testing");
    }

    #[test]
    fn test_default() {
        assert_eq!(Environment::default(), Environment::Production);
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", Environment::Production), "Production");
        assert_eq!(format!("{}", Environment::Test), "Testing");
    }
}
