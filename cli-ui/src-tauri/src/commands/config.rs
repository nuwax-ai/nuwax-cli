use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, command};
use tauri_plugin_dialog::DialogExt;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// 获取当前RFC3339格式时间戳
fn get_current_rfc3339_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown_time".to_string())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DirectoryValidationResult {
    pub valid: bool,
    pub error: Option<String>,
    pub exists: bool,
    pub readable: bool,
    pub writable: bool,
    pub is_empty: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkingDirectoryConfig {
    pub path: String,
    pub last_updated: String,
    pub user_selected: bool,
}

/// 选择工作目录  
#[command]
pub async fn select_directory(app: AppHandle) -> Result<Option<String>, String> {
    // 使用 Tauri Dialog 插件打开目录选择对话框
    let file_path = app
        .dialog()
        .file()
        .set_title("选择工作目录")
        .blocking_pick_folder();

    match file_path {
        Some(path) => {
            // FilePath 类型需要转换为字符串
            let path_str = path.to_string();
            println!("用户选择的目录: {path_str}");
            Ok(Some(path_str))
        }
        None => {
            println!("用户取消了目录选择");
            Ok(None)
        }
    }
}

/// 验证工作目录
#[command]
pub async fn validate_working_directory(
    _app: AppHandle,
    path: String,
) -> Result<DirectoryValidationResult, String> {
    let path_buf = PathBuf::from(&path);

    // 检查目录是否存在
    let exists = path_buf.exists();

    if !exists {
        return Ok(DirectoryValidationResult {
            valid: false,
            error: Some("目录不存在".to_string()),
            exists: false,
            readable: false,
            writable: false,
            is_empty: false,
        });
    }

    // 检查是否为目录
    let is_dir = path_buf.is_dir();
    if !is_dir {
        return Ok(DirectoryValidationResult {
            valid: false,
            error: Some("选择的路径不是目录".to_string()),
            exists: true,
            readable: false,
            writable: false,
            is_empty: false,
        });
    }

    // 检查读权限
    let readable = fs::read_dir(&path_buf).is_ok();

    // 检查写权限 - 尝试创建临时文件
    let test_file_path = path_buf.join(".duck_cli_test_write");
    let writable = fs::File::create(&test_file_path).is_ok();
    if writable {
        let _ = fs::remove_file(&test_file_path); // 清理测试文件
    }

    // 检查目录是否为空
    let is_empty = if readable {
        fs::read_dir(&path_buf)
            .map(|entries| entries.count() == 0)
            .unwrap_or(false)
    } else {
        false
    };

    let valid = readable && writable;
    let error = if !valid {
        if !readable {
            Some("目录不可读".to_string())
        } else if !writable {
            Some("目录不可写".to_string())
        } else {
            Some("未知错误".to_string())
        }
    } else {
        None
    };

    Ok(DirectoryValidationResult {
        valid,
        error,
        exists,
        readable,
        writable,
        is_empty,
    })
}

/// 设置工作目录
#[command]
pub async fn set_working_directory(app: AppHandle, path: String) -> Result<(), String> {
    // 首先验证目录
    let validation = validate_working_directory(app.clone(), path.clone()).await?;

    if !validation.valid {
        return Err(validation.error.unwrap_or("目录无效".to_string()));
    }

    // 保存配置到应用数据目录
    let config = WorkingDirectoryConfig {
        path: path.clone(),
        last_updated: get_current_rfc3339_timestamp(),
        user_selected: true,
    };

    let config_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;

    // 确保配置目录存在
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }

    let config_file = config_dir.join("working_directory.json");
    let config_json =
        serde_json::to_string_pretty(&config).map_err(|e| format!("序列化配置失败: {e}"))?;

    fs::write(&config_file, &config_json).map_err(|e| format!("保存配置文件失败: {e}"))?;

    println!("工作目录已设置为: {path}");
    Ok(())
}

/// 获取工作目录
#[command]
pub async fn get_working_directory(app: AppHandle) -> Result<Option<String>, String> {
    let config_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;

    let config_file = config_dir.join("working_directory.json");

    if !config_file.exists() {
        return Ok(None);
    }

    let config_content =
        fs::read_to_string(&config_file).map_err(|e| format!("读取配置文件失败: {e}"))?;

    let config: WorkingDirectoryConfig =
        serde_json::from_str(&config_content).map_err(|e| format!("解析配置文件失败: {e}"))?;

    // 验证保存的目录是否仍然有效
    let validation = validate_working_directory(app.clone(), config.path.clone()).await?;

    if validation.valid {
        Ok(Some(config.path))
    } else {
        // 如果目录无效，清除配置
        let _ = fs::remove_file(&config_file);
        Ok(None)
    }
}
