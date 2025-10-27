// duck_client/nuwax-cli/tests/test_helpers.rs
//! 测试辅助模块，包含共享的JSON测试数据

/// 完整的增强服务清单JSON字符串
pub const ENHANCED_MANIFEST_JSON: &str = r#"
{
    "version": "0.0.13",
    "release_notes": "测试发布说明",
    "release_date": "2025-07-12T13:49:59Z",
    "packages": {
        "full": {
            "url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/docker/20250715141736/docker.zip",
            "hash": "external",
            "signature": "",
            "size": 0
        }
    },
    "platforms": {
        "x86_64": {
            "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIGNsaSBzZWNyZXQga2V5CkNMSS1MSU5VWC1YNjQtdjEuMS4w",
            "url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/other/docker-x86_64.zip"
        },
        "aarch64": {
            "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBfcm9tIGNsaSBzZWNyZXQga2V5CkNMSS1XSU5ET1dTLVg2NC12MS4xLjA=",
            "url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/other/docker-aarch64.zip"
        }
    },
    "patch": {
        "x86_64": {
            "url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/patches/20250712133533/x86_64-patch.tar.gz",
            "hash": "sha256:patch_hash_x86_64",
            "signature": "patch_signature_x86_64",
            "operations": {
                "replace": {
                    "files": [
                        "app/app.jar",
                        "config/application.yml",
                        "docker-compose.yml",
                        "scripts/start.sh"
                    ],
                    "directories": [
                        "front/",
                        "plugins/",
                        "templates/"
                    ]
                },
                "delete": {
                    "files": [
                        "old.jar",
                        "config/application.yml",
                        "docker-compose.yml",
                        "scripts/start.sh"
                    ],
                    "directories": [
                        "front/",
                        "plugins/",
                        "templates/"
                    ]
                }
            },
            "notes": null
        },
        "aarch64": {
            "url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/patches/20250712133533/aarch64-patch.tar.gz",
            "hash": "sha256:patch_hash_aarch64",
            "signature": "patch_signature_aarch64",
            "operations": {
                "delete": {
                    "files": [
                        "app.jar",
                        "config/application.yml",
                        "docker-compose.yml",
                        "scripts/start.sh"
                    ],
                    "directories": [
                        "front/",
                        "plugins/",
                        "templates/"
                    ]
                },
                "replace": {
                    "directories": [
                        "front/",
                        "plugins/",
                        "templates/"
                    ],
                    "files": [
                        "app.jar",
                        "config/application.yml",
                        "docker-compose.yml",
                        "scripts/start.sh"
                    ]
                }
            },
            "notes": "patch包目录结构与docker.zip保持一致"
        }
    }
}"#;

/// 最小化的增强服务清单JSON字符串
pub const MINIMAL_MANIFEST_JSON: &str = r#"
{
    "version": "1.0.0",
    "release_notes": "最小化测试",
    "release_date": "2025-01-01T00:00:00Z",
    "packages": {
        "full": {
            "url": "https://example.com/package.zip",
            "hash": "sha256:abc123",
            "signature": "sig123",
            "size": 100
        }
    }
}"#;

/// 用于解析测试的增强服务清单JSON字符串
pub const PARSING_TEST_JSON: &str = r#"
{
    "version": "0.0.13",
    "release_notes": "测试发布说明",
    "release_date": "2025-07-12T13:49:59Z",
    "packages": {
        "full": {
            "url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/docker/20250715141736/docker.zip",
            "hash": "external",
            "signature": "",
            "size": 1024
        }
    },
    "platforms": {
        "x86_64": {
            "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIGNsaSBzZWNyZXQga2V5",
            "url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/other/docker-x86_64.zip"
        },
        "aarch64": {
            "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIGNsaSBzZWNyZXQ ga2V5",
            "url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/other/docker-aarch64.zip"
        }
    },
    "patch": {
        "x86_64": {
            "url": "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/patches/20250712133533/x86_64-patch.tar.gz",
            "hash": "sha256:patch_hash_x86_64",
            "signature": "patch_signature_x86_64",
            "operations": {
                "replace": {
                    "files": ["app/app.jar", "config/application.yml"],
                    "directories": ["front/", "plugins/"]
                },
                "delete": {
                    "files": ["old.jar"],
                    "directories": ["old/"]
                }
            }
        }
    }
}"#;
