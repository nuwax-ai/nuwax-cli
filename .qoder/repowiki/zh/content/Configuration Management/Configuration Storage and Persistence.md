# Configuration Storage and Persistence

<cite>
**Referenced Files in This Document**   
- [config_manager.rs](file://client-core/src/config_manager.rs)
- [config.rs](file://client-core/src/config.rs)
- [constants.rs](file://client-core/src/constants.rs)
</cite>

## Table of Contents
1. [Configuration Storage and Persistence](#configuration-storage-and-persistence)
2. [Configuration File Format and Structure](#configuration-file-format-and-structure)
3. [Configuration Persistence Mechanism](#configuration-persistence-mechanism)
4. [ConfigManager Implementation Details](#configmanager-implementation-details)
5. [Configuration Synchronization and Caching](#configuration-synchronization-and-caching)
6. [Error Handling and Validation](#error-handling-and-validation)
7. [Cross-Component Access and Thread Safety](#cross-component-access-and-thread-safety)
8. [Code Examples](#code-examples)

## Configuration File Format and Structure

The configuration system uses TOML (Tom's Obvious, Minimal Language) as the primary file format for configuration storage. The main configuration file is named `config.toml` and follows a structured format with multiple sections for different configuration domains.

The configuration structure is defined by the `AppConfig` struct in `config.rs`, which contains the following top-level sections:

- **versions**: Version management configuration including Docker service version, patch version, and upgrade history
- **docker**: Docker-related configuration including compose and environment file paths
- **backup**: Backup configuration including storage directory
- **cache**: Cache configuration including cache and download directories
- **updates**: Update-related configuration including check frequency

```toml
# Example config.toml structure
[versions]
docker_service = "0.0.1"
patch_version = "0.0.0"
local_patch_level = 0
full_version_with_patches = "0.0.1.0"

[docker]
compose_file = "docker/docker-compose.yml"
env_file = "docker/.env"

[backup]
storage_dir = "backups"

[cache]
cache_dir = "cacheDuckData"
download_dir = "cacheDuckData/download"

[updates]
check_frequency = "daily"
```

The configuration file is generated with comments using a template system, where the `to_toml_with_comments()` method in `config.rs` replaces placeholders in a template file with actual configuration values.

**Section sources**
- [config.rs](file://client-core/src/config.rs#L1-L661)

## Configuration Persistence Mechanism

Configuration persistence is implemented through a dual-layer approach: a file-based configuration system for initial setup and defaults, and a database-backed configuration system for runtime configuration that needs to be persisted across application restarts.

### File-Based Configuration

The file-based configuration system uses TOML format and is managed by the `AppConfig` struct. The system follows a priority-based loading mechanism:

1. First attempts to load from `config.toml` in the current directory
2. If not found, attempts to load from `/app/config.toml`
3. If neither file exists, creates a default configuration file

The default paths and values are defined in `constants.rs` and include:

- **Configuration file**: `data/config.toml`
- **Database file**: `data/duck_client.db`
- **Cache directory**: `cacheDuckData`
- **Download directory**: `cacheDuckData/download`
- **Backup directory**: `backups`

```rust
// Default configuration paths from constants.rs
pub mod config {
    pub const CONFIG_FILE_NAME: &str = "config.toml";
    pub const DATABASE_FILE_NAME: &str = "duck_client.db";
    pub const CACHE_DIR_NAME: &str = "cacheDuckData";
    pub const DOWNLOAD_DIR_NAME: &str = "download";
    
    pub fn get_config_file_path() -> PathBuf {
        Path::new(".").join(DATA_DIR_NAME).join(CONFIG_FILE_NAME)
    }
    
    pub fn get_database_path() -> PathBuf {
        Path::new(".").join(DATA_DIR_NAME).join(DATABASE_FILE_NAME)
    }
    
    pub fn get_default_cache_dir() -> PathBuf {
        Path::new(".").join(CACHE_DIR_NAME)
    }
    
    pub fn get_default_download_dir() -> PathBuf {
        get_default_cache_dir().join(DOWNLOAD_DIR_NAME)
    }
}
```

When no configuration file is found, the system creates a default configuration using the `Default` trait implementation, which sets up sensible defaults for all configuration values.

**Section sources**
- [config.rs](file://client-core/src/config.rs#L1-L661)
- [constants.rs](file://client-core/src/constants.rs#L1-L522)

## ConfigManager Implementation Details

The `ConfigManager` struct in `config_manager.rs` provides the primary interface for configuration management and persistence. It uses a database (DuckDB) to store configuration data, with a memory cache for performance optimization.

### Database Schema

Configuration data is stored in a table named `app_config` with the following schema:

- `config_key`: Unique identifier for the configuration item (TEXT, PRIMARY KEY)
- `config_value`: JSON-encoded value of the configuration (TEXT)
- `config_type`: Type of the configuration value (STRING, NUMBER, BOOLEAN, OBJECT, ARRAY)
- `category`: Category grouping for the configuration item (TEXT)
- `description`: Human-readable description of the configuration (TEXT)
- `is_system_config`: Flag indicating if the configuration is a system setting (BOOLEAN)
- `is_user_editable`: Flag indicating if the configuration can be modified by users (BOOLEAN)
- `validation_rule`: Optional validation rule for the configuration value (TEXT)
- `default_value`: Default value for the configuration (TEXT, JSON-encoded)
- `updated_at`: Timestamp of the last update (TIMESTAMP)

### ConfigManager Structure

```rust
pub struct ConfigManager {
    db: DatabaseConnection,
    cache: Arc<RwLock<HashMap<String, ConfigItem>>>,
    cache_initialized: Arc<RwLock<bool>>,
}
```

The `ConfigManager` maintains an in-memory cache of configuration items using a thread-safe `RwLock` wrapped in an `Arc` for shared ownership. This allows multiple threads to read the configuration concurrently while ensuring exclusive access for writes.

### Configuration Types

The system supports five configuration types through the `ConfigType` enum:

- **String**: Text values
- **Number**: Numeric values (floating-point)
- **Boolean**: True/false values
- **Object**: JSON objects
- **Array**: JSON arrays

Each configuration item is strongly typed, and type validation is performed when updating configuration values to ensure type safety.

**Section sources**
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L810)

## Configuration Synchronization and Caching

The configuration system implements a sophisticated caching mechanism to balance performance and consistency. The `ConfigManager` uses lazy loading and caching to minimize database access while ensuring configuration data is up-to-date.

### Cache Initialization

The cache is initialized lazily when the first configuration access occurs. The `ensure_cache_initialized()` method checks if the cache has been initialized and, if not, calls `initialize_cache()` to load all configuration data from the database:

```rust
async fn ensure_cache_initialized(&self) -> Result<()> {
    let initialized = *self.cache_initialized.read().await;
    if !initialized {
        self.initialize_cache().await?;
    }
    Ok(())
}
```

The `initialize_cache()` method performs the following steps:
1. Queries the `app_config` table to retrieve all configuration items
2. Parses the JSON-encoded values and default values
3. Converts the configuration type strings to `ConfigType` enum values
4. Stores the configuration items in the in-memory cache
5. Marks the cache as initialized

### Cache Synchronization

Configuration updates are synchronized between memory and disk through a write-through caching strategy. When a configuration value is updated:

1. The update is written to the database using a transaction
2. Upon successful database write, the in-memory cache is updated
3. This ensures that the cache always reflects the latest persisted state

The `update_config()` method handles single configuration updates:

```rust
pub async fn update_config(&self, key: &str, value: Value) -> Result<()> {
    // Ensure cache is initialized
    self.ensure_cache_initialized().await?;
    
    // Check permissions and validate type
    // ...
    
    // Update database
    let value_json = serde_json::to_string(&value)?;
    self.db.write_with_retry(|conn| {
        conn.execute(
            "UPDATE app_config SET config_value = ?, updated_at = CURRENT_TIMESTAMP WHERE config_key = ?",
            [&value_json, key]
        )?;
        Ok(())
    }).await?;
    
    // Update cache
    let mut cache = self.cache.write().await;
    if let Some(config) = cache.get_mut(key) {
        config.value = value;
    }
    
    debug!("Configuration item {} updated successfully", key);
    Ok(())
}
```

For batch updates, the `update_configs()` method processes multiple configuration updates in a single database transaction to ensure atomicity:

```rust
pub async fn update_configs(&self, updates: Vec<ConfigUpdateRequest>) -> Result<()> {
    // Validate all updates first
    // ...
    
    // Batch update database
    self.db.batch_write_with_retry(|conn| {
        for update in &updates {
            let value_json = serde_json::to_string(&update.value)?;
            conn.execute(
                "UPDATE app_config SET config_value = ?, updated_at = CURRENT_TIMESTAMP WHERE config_key = ?",
                [&value_json, &update.key]
            )?;
        }
        Ok(())
    }).await?;
    
    // Batch update cache
    let mut cache = self.cache.write().await;
    for update in updates {
        if let Some(config) = cache.get_mut(&update.key) {
            config.value = update.value;
        }
    }
    
    debug!("Batch configuration update successful");
    Ok(())
}
```

### Cache Refresh

The system provides a `refresh_cache()` method to force a reload of all configuration data from the database, which is useful when external processes may have modified the configuration:

```rust
pub async fn refresh_cache(&self) -> Result<()> {
    *self.cache_initialized.write().await = false;
    self.initialize_cache().await
}
```

This method marks the cache as uninitialized and then reinitializes it, effectively clearing and repopulating the cache from the database.

**Section sources**
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L810)

## Error Handling and Validation

The configuration system implements comprehensive error handling and validation to ensure data integrity and provide meaningful error messages.

### Type Validation

When updating a configuration value, the system validates that the new value matches the expected type for that configuration item:

```rust
fn validate_value_type(&self, value: &Value, expected_type: &ConfigType) -> bool {
    match (value, expected_type) {
        (Value::String(_), ConfigType::String) => true,
        (Value::Number(_), ConfigType::Number) => true,
        (Value::Bool(_), ConfigType::Boolean) => true,
        (Value::Object(_), ConfigType::Object) => true,
        (Value::Array(_), ConfigType::Array) => true,
        _ => false,
    }
}
```

If the type does not match, an error is returned with details about the expected and actual types.

### Permission Validation

The system distinguishes between system configurations and user-editable configurations. When attempting to update a configuration, it checks the `is_user_editable` flag:

```rust
// Check permissions
let is_editable = {
    let cache = self.cache.read().await;
    if let Some(config) = cache.get(key) {
        if !config.is_user_editable {
            return Err(anyhow::anyhow!("Configuration item {key} is not editable"));
        }
        config.is_user_editable
    } else {
        return Err(anyhow::anyhow!("Configuration item {key} does not exist"));
    }
};
```

This prevents unauthorized modification of system-critical settings.

### I/O Error Handling

Database operations are wrapped in retry mechanisms to handle transient failures:

```rust
pub async fn write_with_retry<F, R>(&self, operation: F) -> Result<R>
where
    F: Fn(&duckdb::Connection) -> duckdb::Result<R> + Send + Sync,
    R: Send,
{
    match self {
        DatabaseConnection::DatabaseManager(db) => db.write_with_retry(operation).await,
        DatabaseConnection::Database(_db) => {
            Err(anyhow::anyhow!("Traditional database connection does not support configuration management"))
        }
    }
}
```

The `read_with_retry` and `write_with_retry` methods on `DatabaseConnection` handle database-level retries for read and write operations, respectively.

### Default Value Management

Configuration items can have default values that can be used to reset the configuration to its original state:

```rust
pub async fn reset_config_to_default(&self, key: &str) -> Result<()> {
    self.ensure_cache_initialized().await?;

    let default_value = {
        let cache = self.cache.read().await;
        if let Some(config) = cache.get(key) {
            if !config.is_user_editable {
                return Err(anyhow::anyhow!("Configuration item {key} is not editable"));
            }
            config.default_value.clone()
        } else {
            return Err(anyhow::anyhow!("Configuration item {key} does not exist"));
        }
    };

    if let Some(default_value) = default_value {
        self.update_config(key, default_value).await
    } else {
        Err(anyhow::anyhow!("Configuration item {key} has no default value"))
    }
}
```

This allows for safe recovery from invalid configuration states.

**Section sources**
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L810)

## Cross-Component Access and Thread Safety

The configuration system is designed for safe concurrent access across multiple components and threads.

### Thread Safety

The `ConfigManager` uses `Arc<RwLock<T>>` for its internal data structures, providing thread-safe access:

- **Arc (Atomically Reference Counted)**: Allows multiple owners of the same data, enabling the `ConfigManager` to be shared across threads
- **RwLock (Read-Write Lock)**: Allows multiple readers simultaneously but only one writer at a time, optimizing for the common read-heavy workload of configuration access

```rust
pub struct ConfigManager {
    db: DatabaseConnection,
    cache: Arc<RwLock<HashMap<String, ConfigItem>>>,
    cache_initialized: Arc<RwLock<bool>>,
}
```

This design ensures that:
- Multiple threads can read configuration values concurrently without blocking each other
- Write operations (configuration updates) are serialized to prevent race conditions
- The cache state remains consistent even under heavy concurrent access

### Component Integration

The `ConfigManager` is integrated with various components through dependency injection. It can be created with either a `DatabaseManager` or a `Database` connection:

```rust
pub fn new(db: Arc<DatabaseManager>) -> Self {
    Self {
        db: DatabaseConnection::DatabaseManager(db),
        cache: Arc::new(RwLock::new(HashMap::new())),
        cache_initialized: Arc::new(RwLock::new(false)),
    }
}

pub fn new_with_database(db: Arc<Database>) -> Self {
    Self {
        db: DatabaseConnection::Database(db),
        cache: Arc::new(RwLock::new(HashMap::new())),
        cache_initialized: Arc::new(RwLock::new(false)),
    }
}
```

This flexibility allows different parts of the system to use the appropriate database connection type.

### Business-Specific Configuration Methods

The `ConfigManager` provides higher-level methods for specific business domains, such as auto-backup and auto-upgrade configurations:

```rust
// Auto-backup configuration methods
pub async fn set_auto_backup_enabled(&self, enabled: bool) -> Result<()> {
    let value = Value::Bool(enabled);
    self.update_config("auto_backup_enabled", value).await
}

pub async fn get_auto_backup_config(&self) -> Result<AutoBackupConfig> {
    let enabled = self.get_bool("auto_backup_enabled").await?.unwrap_or(false);
    let cron_expr = self.get_string("auto_backup_schedule").await?.unwrap_or("0 2 * * *".to_string());
    // ... other fields
    Ok(AutoBackupConfig { /* ... */ })
}

// Auto-upgrade task methods
pub async fn create_auto_upgrade_task(&self, task: &AutoUpgradeTask) -> Result<()> {
    // Store task in database
    // ...
}

pub async fn get_pending_upgrade_tasks(&self) -> Result<Vec<AutoUpgradeTask>> {
    // Query pending tasks from database
    // ...
}
```

These methods abstract the underlying key-value storage and provide domain-specific interfaces that are easier to use and less error-prone.

**Section sources**
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L810)

## Code Examples

### Initialization and Configuration Loading

```rust
// Initialize configuration from file
let mut app_config = AppConfig::find_and_load_config()
    .expect("Failed to load configuration");

// Ensure cache directories exist
app_config.ensure_cache_dirs()
    .expect("Failed to create cache directories");

// Create database connection
let db_manager = DatabaseManager::new(config::get_database_path())
    .await
    .expect("Failed to initialize database");

// Create configuration manager
let config_manager = ConfigManager::new(Arc::new(db_manager));

// Initialize cache (loads from database)
config_manager.initialize_cache().await
    .expect("Failed to initialize configuration cache");
```

### Reading Configuration Values

```rust
// Get string configuration
let backup_dir = config_manager.get_string("auto_backup_directory").await;
match backup_dir {
    Ok(Some(dir)) => println!("Backup directory: {}", dir),
    Ok(None) => println!("Backup directory not set"),
    Err(e) => eprintln!("Error reading backup directory: {}", e),
}

// Get boolean configuration
let auto_backup_enabled = config_manager.get_bool("auto_backup_enabled").await;
if let Ok(Some(enabled)) &auto_backup_enabled {
    if *enabled {
        println!("Auto-backup is enabled");
    }
}

// Get numeric configuration
let retention_days = config_manager.get_integer("auto_backup_retention_days").await;
if let Ok(Some(days)) = retention_days {
    println!("Backup retention: {} days", days);
}
```

### Updating Configuration Values

```rust
// Update single configuration value
let cron_value = Value::String("0 3 * * *".to_string());
match config_manager.update_config("auto_backup_schedule", cron_value).await {
    Ok(()) => println!("Backup schedule updated successfully"),
    Err(e) => eprintln!("Failed to update backup schedule: {}", e),
}

// Batch update multiple configuration values
let updates = vec![
    ConfigUpdateRequest {
        key: "auto_backup_enabled".to_string(),
        value: Value::Bool(true),
        validate: true,
    },
    ConfigUpdateRequest {
        key: "auto_backup_retention_days".to_string(),
        value: Value::Number(14.into()),
        validate: true,
    },
];

match config_manager.update_configs(updates).await {
    Ok(()) => println!("Batch configuration update successful"),
    Err(e) => eprintln!("Batch update failed: {}", e),
}
```

### Handling Missing Configuration Files

```rust
// The system automatically handles missing configuration files
let app_config = match AppConfig::load_from_file("config.toml") {
    Ok(config) => {
        println!("Configuration loaded successfully");
        config
    },
    Err(e) => {
        println!("Configuration file not found or invalid: {}. Creating default configuration.", e);
        let default_config = AppConfig::default();
        if let Err(save_err) = default_config.save_to_file("config.toml") {
            eprintln!("Failed to save default configuration: {}", save_err);
        }
        default_config
    }
};
```

### Error Handling in Configuration Operations

```rust
// Example of comprehensive error handling
async fn update_backup_settings(
    config_manager: &ConfigManager,
    schedule: &str,
    retention_days: i64,
    enabled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Validate inputs
    if retention_days <= 0 {
        return Err("Retention days must be positive".into());
    }

    // Start transaction for atomic update
    let updates = vec![
        ConfigUpdateRequest {
            key: "auto_backup_schedule".to_string(),
            value: Value::String(schedule.to_string()),
            validate: true,
        },
        ConfigUpdateRequest {
            key: "auto_backup_retention_days".to_string(),
            value: Value::Number(retention_days.into()),
            validate: true,
        },
        ConfigUpdateRequest {
            key: "auto_backup_enabled".to_string(),
            value: Value::Bool(enabled),
            validate: true,
        },
    ];

    // Perform batch update
    match config_manager.update_configs(updates).await {
        Ok(()) => {
            println!("Backup settings updated successfully");
            Ok(())
        },
        Err(e) => {
            eprintln!("Failed to update backup settings: {}", e);
            Err(e.into())
        }
    }
}
```

These examples demonstrate the complete workflow for configuration management, from initialization and error recovery to reading, updating, and handling configuration data in a production environment.

**Section sources**
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L810)
- [config.rs](file://client-core/src/config.rs#L1-L661)
- [constants.rs](file://client-core/src/constants.rs#L1-L522)