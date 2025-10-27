# Update Command

<cite>
**Referenced Files in This Document**   
- [upgrade.rs](file://client-core/src/upgrade.rs)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs)
- [backup.rs](file://client-core/src/backup.rs)
- [config_manager.rs](file://client-core/src/config_manager.rs)
- [version.rs](file://client-core/src/version.rs)
- [backup.rs](file://nuwax-cli/src/commands/backup.rs)
</cite>

## Table of Contents
1. [Introduction](#introduction)
2. [Update Command Overview](#update-command-overview)
3. [Core Components](#core-components)
4. [Update Process Flow](#update-process-flow)
5. [Upgrade Strategy Management](#upgrade-strategy-management)
6. [Backup and Rollback System](#backup-and-rollback-system)
7. [Configuration Management](#configuration-management)
8. [Version Management System](#version-management-system)
9. [Error Handling and Recovery](#error-handling-and-recovery)
10. [User Experience Features](#user-experience-features)
11. [Architecture Overview](#architecture-overview)

## Introduction
The Update Command is a comprehensive system designed to manage the complete update process for Docker-based services. It orchestrates a sequence of operations including version checking, backup creation, service management, package downloading, deployment, and verification. The system is designed with atomicity, resilience, and user experience in mind, ensuring safe and reliable updates while providing clear feedback and rollback capabilities.

## Update Command Overview

The Update Command serves as the central orchestration mechanism for service updates, coordinating multiple subsystems to ensure a seamless and reliable update process. It manages the entire lifecycle from initial version checking to final service verification.

The command follows a structured workflow:
1. Check for available updates
2. Determine appropriate upgrade strategy
3. Create pre-update backup
4. Download update package
5. Stop running services
6. Deploy update
7. Start services
8. Verify successful update
9. Clean up temporary files

This systematic approach ensures that updates are performed safely and can be rolled back in case of failures.

**Section sources**
- [upgrade.rs](file://client-core/src/upgrade.rs#L1-L90)

## Core Components

The update system comprises several key components that work together to deliver a robust update experience:

- **UpgradeManager**: Orchestrates the entire update process
- **UpgradeStrategyManager**: Determines the optimal update approach
- **BackupManager**: Handles backup creation and restoration
- **ConfigManager**: Manages configuration persistence
- **Version**: Handles version parsing and comparison

These components are designed with clear separation of concerns, allowing each to focus on specific aspects of the update process while maintaining loose coupling.

```mermaid
classDiagram
class UpgradeManager {
+check_for_updates() Result~UpgradeStrategy~
+execute_update() Result~UpgradeResult~
}
class UpgradeStrategyManager {
+determine_strategy() Result~UpgradeStrategy~
+select_full_upgrade_strategy() Result~UpgradeStrategy~
+select_patch_upgrade_strategy() Result~UpgradeStrategy~
}
class BackupManager {
+create_backup() Result~BackupRecord~
+restore_backup() Result~()~
+list_backups() Result~Vec~BackupRecord~~
}
class ConfigManager {
+get_string() Result~Option~String~~
+update_config() Result~()~
+get_configs_by_category() Result~Vec~ConfigItem~~
}
class Version {
+major : u32
+minor : u32
+patch : u32
+build : u32
+compare_detailed() VersionComparison
+base_version() Version
}
UpgradeManager --> UpgradeStrategyManager : "uses"
UpgradeManager --> BackupManager : "uses"
UpgradeManager --> ConfigManager : "uses"
UpgradeManager --> Version : "uses"
```

**Diagram sources**
- [upgrade.rs](file://client-core/src/upgrade.rs#L1-L90)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L462)
- [backup.rs](file://client-core/src/backup.rs#L1-L623)
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L799)
- [version.rs](file://client-core/src/version.rs#L1-L409)

**Section sources**
- [upgrade.rs](file://client-core/src/upgrade.rs#L1-L90)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L462)
- [backup.rs](file://client-core/src/backup.rs#L1-L623)
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L799)
- [version.rs](file://client-core/src/version.rs#L1-L409)

## Update Process Flow

The update process follows a well-defined sequence of steps, each with specific responsibilities and error handling mechanisms. The flow ensures that updates are performed safely and can be rolled back if necessary.

```mermaid
sequenceDiagram
participant CLI as "CLI Command"
participant UpgradeManager as "UpgradeManager"
participant StrategyManager as "UpgradeStrategyManager"
participant BackupManager as "BackupManager"
participant DockerService as "DockerService"
participant Downloader as "Downloader"
CLI->>UpgradeManager : execute_update()
UpgradeManager->>StrategyManager : check_for_updates()
StrategyManager-->>UpgradeManager : UpgradeStrategy
alt Skip Backup
UpgradeManager->>UpgradeManager : Proceed without backup
else
UpgradeManager->>BackupManager : create_backup()
BackupManager-->>UpgradeManager : BackupRecord
end
UpgradeManager->>DockerService : stop_services()
DockerService-->>UpgradeManager : Success
UpgradeManager->>Downloader : download_update()
Downloader-->>UpgradeManager : Downloaded Package
UpgradeManager->>UpgradeManager : extract_and_deploy()
UpgradeManager->>DockerService : start_services()
DockerService-->>UpgradeManager : Success
UpgradeManager->>UpgradeManager : verify_services()
alt Verification Success
UpgradeManager->>UpgradeManager : cleanup()
UpgradeManager-->>CLI : Success Result
else Verification Failed
UpgradeManager->>BackupManager : restore_backup()
BackupManager-->>UpgradeManager : Restoration Complete
UpgradeManager-->>CLI : Failure Result
end
```

**Diagram sources**
- [upgrade.rs](file://client-core/src/upgrade.rs#L1-L90)
- [backup.rs](file://client-core/src/backup.rs#L1-L623)

**Section sources**
- [upgrade.rs](file://client-core/src/upgrade.rs#L1-L90)
- [backup.rs](file://client-core/src/backup.rs#L1-L623)

## Upgrade Strategy Management

The upgrade strategy system intelligently determines the most appropriate update approach based on various factors including version differences, architecture compatibility, and user preferences.

### Strategy Determination Logic

The system evaluates several conditions to determine the optimal upgrade strategy:

1. **Version Comparison**: Compares current and target versions to determine if update is needed
2. **Architecture Detection**: Identifies system architecture for platform-specific packages
3. **Force Flag**: Respects user preference for full upgrades
4. **Environment Check**: Validates presence of required directories and files

```mermaid
flowchart TD
Start([Start]) --> ParseVersion["Parse Current Version"]
ParseVersion --> CompareVersion["Compare with Server Version"]
CompareVersion --> VersionEqual{"Versions Equal?"}
VersionEqual --> |Yes| NoUpgrade["No Upgrade Needed"]
VersionEqual --> |No| ForceFull{"Force Full Upgrade?"}
ForceFull --> |Yes| FullUpgrade["Select Full Upgrade"]
ForceFull --> |No| CheckEnvironment["Check Docker Environment"]
CheckEnvironment --> EnvironmentValid{"Environment Valid?"}
EnvironmentValid --> |No| FullUpgrade
EnvironmentValid --> |Yes| CheckBaseVersion["Check Base Version"]
CheckBaseVersion --> SameBase{"Same Base Version?"}
SameBase --> |Yes| CheckPatch["Patch Available?"]
CheckPatch --> |Yes| PatchUpgrade["Select Patch Upgrade"]
CheckPatch --> |No| FullUpgrade
SameBase --> |No| FullUpgrade
NoUpgrade --> End([End])
FullUpgrade --> End
PatchUpgrade --> End
```

**Diagram sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L462)

**Section sources**
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L462)

## Backup and Rollback System

The backup system provides critical protection against update failures, ensuring data integrity and enabling safe rollback to previous states.

### Backup Creation Process

The backup process follows these steps:
1. Validate Docker service state
2. Create timestamped backup file
3. Compress and archive specified directories
4. Record backup metadata in database
5. Return backup record

```mermaid
sequenceDiagram
participant User as "User"
participant BackupCommand as "run_backup()"
participant HealthCheck as "DockerService.health_check()"
participant BackupManager as "BackupManager.create_backup()"
participant Database as "Database"
User->>BackupCommand : Execute backup
BackupCommand->>HealthCheck : Check service status
HealthCheck-->>BackupCommand : Service report
alt Services Running
BackupCommand-->>User : Error - Stop services first
else Services Stopped
BackupCommand->>BackupManager : Create backup with options
BackupManager->>BackupManager : Generate filename
BackupManager->>BackupManager : Perform compression
BackupManager->>Database : Record backup metadata
Database-->>BackupManager : Record ID
BackupManager-->>BackupCommand : BackupRecord
BackupCommand-->>User : Success message
end
```

**Diagram sources**
- [backup.rs](file://nuwax-cli/src/commands/backup.rs#L1-L799)
- [backup.rs](file://client-core/src/backup.rs#L1-L623)

**Section sources**
- [backup.rs](file://nuwax-cli/src/commands/backup.rs#L1-L799)
- [backup.rs](file://client-core/src/backup.rs#L1-L623)

## Configuration Management

The configuration management system handles persistent settings and upgrade-related configuration, ensuring settings are preserved across updates.

### Configuration Structure

The system manages various configuration aspects:

- **Auto Backup Settings**: Schedule, retention, directory
- **Upgrade Tasks**: Scheduled upgrades, status tracking
- **System Parameters**: Various application settings

```mermaid
erDiagram
CONFIG ||--o{ BACKUP_CONFIG : "contains"
CONFIG ||--o{ UPGRADE_TASKS : "contains"
CONFIG ||--o{ SYSTEM_PARAMS : "contains"
CONFIG {
string config_key PK
string config_value
string config_type
string category
boolean is_user_editable
boolean is_system_config
string validation_rule
string default_value
timestamp created_at
timestamp updated_at
}
BACKUP_CONFIG {
string auto_backup_enabled
string auto_backup_schedule
string auto_backup_last_time
string auto_backup_last_status
string auto_backup_directory
integer auto_backup_retention_days
}
UPGRADE_TASKS {
string task_id PK
string task_name
timestamp schedule_time
string upgrade_type
string target_version
string status
integer progress
string error_message
timestamp created_at
timestamp updated_at
}
SYSTEM_PARAMS {
string param_name PK
string param_value
string description
}
```

**Diagram sources**
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L799)

**Section sources**
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L799)

## Version Management System

The version management system provides robust version parsing, comparison, and validation capabilities, enabling intelligent upgrade decisions.

### Version Structure

The system uses a four-segment version format:
- **Major**: Major version number
- **Minor**: Minor version number
- **Patch**: Patch/revision number
- **Build**: Build/patch level

```mermaid
classDiagram
class Version {
+major : u32
+minor : u32
+patch : u32
+build : u32
+from_str() Result~Version~
+to_string() String
+base_version() Version
+compare_detailed() VersionComparison
+can_apply_patch() bool
+is_compatible_with_patch() bool
}
class VersionComparison {
+Equal
+Newer
+PatchUpgradeable
+FullUpgradeRequired
}
```

**Diagram sources**
- [version.rs](file://client-core/src/version.rs#L1-L409)

**Section sources**
- [version.rs](file://client-core/src/version.rs#L1-L409)

## Error Handling and Recovery

The system implements comprehensive error handling and recovery mechanisms to ensure update integrity and data safety.

### Failure Recovery Process

When an update fails, the system follows a structured recovery process:

1. **Immediate Stop**: Halt the update process
2. **Rollback Initiation**: Trigger automatic rollback
3. **Service Restoration**: Restore from backup
4. **Status Reporting**: Report failure and recovery status

The system ensures atomicity by treating the entire update process as a transaction that either completes successfully or rolls back completely.

**Section sources**
- [upgrade.rs](file://client-core/src/upgrade.rs#L1-L90)
- [backup.rs](file://client-core/src/backup.rs#L1-L623)

## User Experience Features

The update system incorporates several user experience features to provide clear feedback and control:

- **Progress Reporting**: Detailed step-by-step progress updates
- **Confirmation Prompts**: Safety checks before critical operations
- **Interactive Selection**: Backup selection interface
- **Comprehensive Logging**: Detailed operation logs
- **JSON Output**: Machine-readable output for integration

These features ensure users have full visibility into the update process and can make informed decisions.

**Section sources**
- [backup.rs](file://nuwax-cli/src/commands/backup.rs#L1-L799)

## Architecture Overview

The update system follows a modular architecture with clear separation of concerns, enabling maintainability and extensibility.

```mermaid
graph TB
subgraph "CLI Interface"
CLI[CLI Command]
UI[User Interface]
end
subgraph "Core Logic"
UpgradeManager[UpgradeManager]
StrategyManager[UpgradeStrategyManager]
BackupManager[BackupManager]
ConfigManager[ConfigManager]
Version[Version]
end
subgraph "External Services"
Docker[Docker Service]
API[Update Server API]
Database[Configuration Database]
end
CLI --> UpgradeManager
UpgradeManager --> StrategyManager
UpgradeManager --> BackupManager
UpgradeManager --> ConfigManager
UpgradeManager --> Version
UpgradeManager --> Docker
UpgradeManager --> API
UpgradeManager --> Database
StrategyManager --> API
BackupManager --> Docker
ConfigManager --> Database
```

**Diagram sources**
- [upgrade.rs](file://client-core/src/upgrade.rs#L1-L90)
- [upgrade_strategy.rs](file://client-core/src/upgrade_strategy.rs#L1-L462)
- [backup.rs](file://client-core/src/backup.rs#L1-L623)
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L799)
- [version.rs](file://client-core/src/version.rs#L1-L409)