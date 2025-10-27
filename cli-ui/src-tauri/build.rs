use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // 监听nuwax-cli源代码变化
    println!("cargo:rerun-if-changed=../../nuwax-cli/src");
    println!("cargo:rerun-if-changed=../../nuwax-cli/Cargo.toml");
    println!("cargo:rerun-if-changed=../../client-core/src");
    println!("cargo:rerun-if-changed=../../client-core/Cargo.toml");

    // 检查并自动构建nuwax-cli二进制文件
    ensure_nuwax_cli_binary();

    // 运行Tauri的构建过程
    tauri_build::build()
}

fn ensure_nuwax_cli_binary() {
    // 获取当前目标三元组
    let target_triple = get_target_triple();
    let binary_name = get_binary_name(&target_triple);
    let binary_path = Path::new("binaries").join(&binary_name);

    // 检查是否需要重新编译
    let needs_rebuild = check_if_rebuild_needed(&binary_path, &target_triple);

    if binary_path.exists() && !needs_rebuild {
        println!(
            "cargo:warning=Nuwax Cli  binary is up to date: {}",
            binary_path.display()
        );
        return;
    }

    if binary_path.exists() {
        println!(
            "cargo:warning=Nuwax Cli  binary found but needs rebuild: {}",
            binary_path.display()
        );
    } else {
        println!(
            "cargo:warning=Nuwax Cli  binary not found: {}",
            binary_path.display()
        );
    }

    println!("cargo:warning=Building nuwax-cli for target: {target_triple}");

    // 自动构建 nuwax-cli
    if build_duck_cli(&target_triple) {
        // 复制构建好的二进制文件到正确位置
        copy_binary_to_sidecar_location(&target_triple, &binary_name);
    } else {
        println!("cargo:warning=Failed to build nuwax-cli automatically");
        println!(
            "cargo:warning=Please build manually: cargo build --release -p nuwax-cli --target {target_triple}"
        );
    }
}

fn check_if_rebuild_needed(binary_path: &Path, _target_triple: &str) -> bool {
    // 如果二进制文件不存在，需要重新编译
    if !binary_path.exists() {
        return true;
    }

    // 获取二进制文件的修改时间
    let binary_mtime = match std::fs::metadata(binary_path) {
        Ok(metadata) => metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
        Err(_) => return true, // 如果无法获取元数据，强制重新编译
    };

    // 检查源代码目录的修改时间
    let workspace_root = get_workspace_root();
    let source_dirs = [
        workspace_root.join("nuwax-cli/src"),
        workspace_root.join("client-core/src"),
    ];

    for dir in &source_dirs {
        if is_directory_newer_than(dir, binary_mtime) {
            println!(
                "cargo:warning=Source code changed in {}, rebuilding...",
                dir.display()
            );
            return true;
        }
    }

    false
}

fn is_directory_newer_than(dir: &Path, timestamp: std::time::SystemTime) -> bool {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if is_directory_newer_than(&path, timestamp) {
                    return true;
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                if let Ok(metadata) = std::fs::metadata(&path) {
                    if let Ok(mtime) = metadata.modified() {
                        if mtime > timestamp {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn get_workspace_root() -> PathBuf {
    if let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") {
        Path::new(&manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    } else {
        PathBuf::from("../..")
    }
}

fn get_target_triple() -> String {
    // 获取目标三元组，优先使用环境变量，回退到主机三元组
    std::env::var("CARGO_CFG_TARGET_TRIPLE")
        .or_else(|_| std::env::var("TARGET"))
        .unwrap_or_else(|_| {
            // 如果环境变量不可用，使用编译时检测
            if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
                "aarch64-apple-darwin".to_string()
            } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
                "x86_64-apple-darwin".to_string()
            } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                "x86_64-unknown-linux-gnu".to_string()
            } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
                "aarch64-unknown-linux-gnu".to_string()
            } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
                "x86_64-pc-windows-msvc".to_string()
            } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
                "aarch64-pc-windows-msvc".to_string()
            } else {
                "aarch64-apple-darwin".to_string() // 默认回退
            }
        })
}

fn get_binary_name(target_triple: &str) -> String {
    if target_triple.contains("windows") {
        format!("nuwax-cli-{target_triple}.exe")
    } else {
        format!("nuwax-cli-{target_triple}")
    }
}

fn build_duck_cli(target_triple: &str) -> bool {
    println!(
        "cargo:warning=Building nuwax-cli with: cargo build --release -p nuwax-cli --target {target_triple}"
    );

    let mut cmd = Command::new("cargo");
    cmd.args([
        "build",
        "--release",
        "-p",
        "nuwax-cli",
        "--target",
        target_triple,
    ]);

    // 确保在工作区根目录执行
    let workspace_root = get_workspace_root();
    cmd.current_dir(workspace_root);

    match cmd.status() {
        Ok(status) if status.success() => {
            println!("cargo:warning=Successfully built nuwax-cli");
            true
        }
        Ok(status) => {
            println!(
                "cargo:warning=Failed to build nuwax-cli (exit code: {})",
                status.code().unwrap_or(-1)
            );
            false
        }
        Err(e) => {
            println!("cargo:warning=Failed to execute cargo build: {e}");
            false
        }
    }
}

fn copy_binary_to_sidecar_location(target_triple: &str, binary_name: &str) {
    let workspace_root = get_workspace_root();

    let source_binary_name = if target_triple.contains("windows") {
        "nuwax-cli.exe"
    } else {
        "nuwax-cli"
    };

    let source_path = workspace_root
        .join("target")
        .join(target_triple)
        .join("release")
        .join(source_binary_name);

    let dest_dir = Path::new("binaries");
    let dest_path = dest_dir.join(binary_name);

    // 确保目标目录存在
    if let Err(e) = std::fs::create_dir_all(dest_dir) {
        println!("cargo:warning=Failed to create binaries directory: {e}");
        return;
    }

    // 复制文件
    match std::fs::copy(&source_path, &dest_path) {
        Ok(_) => {
            println!(
                "cargo:warning=Copied {} to {}",
                source_path.display(),
                dest_path.display()
            );
        }
        Err(e) => {
            println!("cargo:warning=Failed to copy binary: {e}");
            println!("cargo:warning=Source: {}", source_path.display());
            println!("cargo:warning=Dest: {}", dest_path.display());
        }
    }
}
