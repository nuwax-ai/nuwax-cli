# One-Click Rollback

<cite>
**Referenced Files in This Document**   
- [backup.rs](file://client-core/src/backup.rs)
- [BackupSelectionModal.tsx](file://cli-ui/src/components/BackupSelectionModal.tsx)
- [service.rs](file://client-core/src/container/service.rs)
- [error.rs](file://client-core/src/error.rs)
</cite>

## Table of Contents
1. [Introduction](#introduction)
2. [Core Components](#core-components)
3. [Rollback Workflow](#rollback-workflow)
4. [Error Handling and Recovery](#error-handling-and-recovery)
5. [Common Issues and Troubleshooting](#common-issues-and-troubleshooting)
6. [Verification and Monitoring](#verification-and-monitoring)

## Introduction
The One-Click Rollback feature enables users to recover from failed upgrades by restoring the system to a previous stable state using saved backups. This functionality integrates a graphical user interface (GUI) for backup selection with a robust backend system that handles service shutdown, state restoration, and restart validation. The rollback process ensures data integrity by restoring Docker volumes, configuration files, and database snapshots from compressed archives. This document details the implementation, workflow, error handling, and best practices for using the rollback system effectively.

## Core Components

### Backup Management System
The core rollback logic is implemented in the `BackupManager` struct defined in `backup.rs`. This component orchestrates the entire restoration process, including service lifecycle management, file restoration, and post-recovery validation.

```mermaid
classDiagram
class BackupManager {
+storage_dir : PathBuf
+database : Arc~Database~
+docker_manager : Arc~DockerManager~
+create_backup(options : BackupOptions) Result~BackupRecord~
+restore_data_from_backup_with_exculde(backup_id : i64, target_dir : &Path, auto_start_service : bool, dirs_to_exculde : &[&str]) Result~()~
+restore_data_directory_only(backup_id : i64, target_dir : &Path, auto_start_service : bool, dirs_to_restore : &[&str]) Result~()~
+perform_selective_restore(backup_path : &Path, target_dir : &Path, dirs_to_restore : &[&str]) Result~()~
+perform_restore(backup_path : &Path, target_dir : &Path, dirs_to_exculde : &[&str]) Result~()~
+clear_data_directories(docker_dir : &Path, dirs_to_exculde : &[&str]) Result~()~
+clear_data_directory_only(docker_dir : &Path) Result~()~
}
class BackupRecord {
+id : i64
+backup_type : BackupType
+created_at : String
+service_version : String
+file_path : String
+file_size : Option~u64~
+file_exists : bool
}
class BackupOptions {
+backup_type : BackupType
+service_version : String
+work_dir : PathBuf
+source_paths : Vec~PathBuf~
+compression_level : u32
}
class RestoreOptions {
+target_dir : PathBuf
+force_overwrite : bool
}
class BackupType {
<<enumeration>>
Manual
PreUpgrade
}
class BackupStatus {
<<enumeration>>
Completed
Failed
}
BackupManager --> Database : "reads/writes"
BackupManager --> DockerManager : "controls"
BackupManager --> BackupRecord : "creates/retrieves"
BackupManager --> BackupOptions : "uses"
BackupManager --> RestoreOptions : "uses"
```

**Diagram sources**
- [backup.rs](file://client-core/src/backup.rs#L49-L155)

**Section sources**
- [backup.rs](file://client-core/src/backup.rs#L49-L155)

### GUI Interface: BackupSelectionModal
The `BackupSelectionModal.tsx` component provides a user-friendly interface for selecting backups to restore. It displays available backups with metadata such as creation time, version, size, and type, allowing users to make informed decisions about which backup to use.

```mermaid
classDiagram
class BackupSelectionModal {
+isOpen : boolean
+workingDirectory : string
+onConfirm : (backupId : number, backupInfo : BackupRecord) => void
+onCancel : () => void
+backups : BackupRecord[]
+selectedBackup : BackupRecord | null
+loading : boolean
+error : string
+fetchBackups() : Promise~void~
+formatFileSize(bytes? : number) : string
+formatBackupType(type : string) : string
+getBackupTypeColor(type : string) : string
+formatDateTime(dateTime : string) : string
+handleConfirm() : void
}
class BackupRecord {
+id : number
+backup_type : 'Manual' | 'PreUpgrade'
+created_at : string
+service_version : string
+file_path : string
+file_size? : number
+file_exists : boolean
}
BackupSelectionModal --> DuckCliManager : "calls getBackupList"
DuckCliManager --> BackupManager : "invokes list_backups"
BackupSelectionModal --> BackupRecord : "displays"
```

**Diagram sources**
- [BackupSelectionModal.tsx](file://cli-ui/src/components/BackupSelectionModal.tsx#L10-L302)

**Section sources**
- [BackupSelectionModal.tsx](file://cli-ui/src/components/BackupSelectionModal.tsx#L10-L302)

### Service Lifecycle Management
The `DockerManager` in `service.rs` handles the container lifecycle operations required during rollback. It provides methods to start, stop, and restart services, ensuring proper state transitions during the restoration process.

```mermaid
classDiagram
class DockerManager {
+start_services() : Result~()~
+stop_services() : Result~()~
+restart_services() : Result~()~
+restart_service(service_name : &str) : Result~()~
+get_services_status() : Result~Vec~ServiceInfo~~~
+check_services_health() : Result~()~
+verify_services_started(custom_timeout : Option~u64~) : Result~()~
}
class ServiceInfo {
+name : String
+status : ServiceStatus
+image : String
+ports : Vec~String~
}
class ServiceStatus {
<<enumeration>>
Running
Stopped
Unknown
Created
Restarting
}
BackupManager --> DockerManager : "invokes stop/start"
DockerManager --> DockerContainer : "manages"
```

**Diagram sources**
- [service.rs](file://client-core/src/container/service.rs#L11-L71)

**Section sources**
- [service.rs](file://client-core/src/container/service.rs#L11-L71)

## Rollback Workflow

### Complete Rollback Sequence
The rollback process follows a well-defined sequence of operations to ensure system stability and data integrity.

```mermaid
sequenceDiagram
participant User as "User"
participant GUI as "BackupSelectionModal"
participant CLI as "DuckCliManager"
participant BackupManager as "BackupManager"
participant DockerManager as "DockerManager"
participant Database as "Database"
User->>GUI : Open modal and select backup
GUI->>CLI : Call getBackupList(workingDirectory)
CLI->>BackupManager : list_backups()
BackupManager->>Database : Query all backup records
Database-->>BackupManager : Return backup list
BackupManager-->>CLI : Return backups
CLI-->>GUI : Return backup list
GUI->>User : Display available backups
User->>GUI : Select backup and confirm
GUI->>CLI : Call restore_from_backup(backupId, workingDirectory)
CLI->>BackupManager : restore_data_from_backup_with_exculde(backupId, target_dir, true, ["data"])
BackupManager->>Database : get_backup_by_id(backupId)
Database-->>BackupManager : Return backup record
BackupManager->>DockerManager : stop_services()
DockerManager-->>BackupManager : Services stopped
BackupManager->>BackupManager : clear_data_directories(target_dir, ["data"])
BackupManager->>BackupManager : perform_restore(backup_path, target_dir, ["data"])
BackupManager->>DockerManager : start_services()
DockerManager-->>BackupManager : Services started
BackupManager-->>CLI : Restoration complete
CLI-->>GUI : Success notification
GUI-->>User : Show success message
```

**Diagram sources**
- [backup.rs](file://client-core/src/backup.rs#L224-L284)
- [BackupSelectionModal.tsx](file://cli-ui/src/components/BackupSelectionModal.tsx#L130-L150)
- [service.rs](file://client-core/src/container/service.rs#L11-L47)

**Section sources**
- [backup.rs](file://client-core/src/backup.rs#L224-L284)
- [BackupSelectionModal.tsx](file://cli-ui/src/components/BackupSelectionModal.tsx#L130-L150)
- [service.rs](file://client-core/src/container/service.rs#L11-L47)

### Selective Restoration Logic
The system supports selective restoration, allowing specific directories to be excluded from the rollback process. This is particularly useful when preserving user data while restoring configuration files.

```mermaid
flowchart TD
Start([Start Restoration]) --> ValidateBackup["Validate Backup Record"]
ValidateBackup --> BackupExists{"Backup File Exists?"}
BackupExists --> |No| ReturnError["Return Error: Backup File Missing"]
BackupExists --> |Yes| StopServices["Stop Docker Services"]
StopServices --> ClearData["Clear Data Directories<br>(excluding specified dirs)"]
ClearData --> CheckRestoreType{"Selective Restore?"}
CheckRestoreType --> |Yes| PerformSelective["perform_selective_restore()<br>Only restore specified dirs"]
CheckRestoreType --> |No| PerformFull["perform_restore()<br>Restore all except excluded dirs"]
PerformSelective --> StartServices["Start Docker Services"]
PerformFull --> StartServices
StartServices --> ValidateRestart["Verify Services Started"]
ValidateRestart --> |Success| ReturnSuccess["Return Success"]
ValidateRestart --> |Failure| ReturnError
ReturnSuccess --> End([End])
ReturnError --> End
```

**Diagram sources**
- [backup.rs](file://client-core/src/backup.rs#L244-L284)

**Section sources**
- [backup.rs](file://client-core/src/backup.rs#L244-L284)

## Error Handling and Recovery

### Error Type Hierarchy
The system uses a comprehensive error handling mechanism centered around the `DuckError` enum, which categorizes different types of failures that can occur during rollback operations.

```mermaid
classDiagram
class DuckError {
<<enumeration>>
Config(toml : : de : : Error)
DuckDb(String)
Http(reqwest : : Error)
Io(std : : io : : Error)
Uuid(uuid : : Error)
Serde(serde_json : : Error)
Join(tokio : : task : : JoinError)
Zip(zip : : result : : ZipError)
WalkDir(walkdir : : Error)
StripPrefix(std : : path : : StripPrefixError)
Template(String)
Docker(String)
Backup(String)
Upgrade(String)
ClientNotRegistered
InvalidResponse(String)
Custom(String)
ConfigNotFound
Api(String)
DockerService(String)
BadRequest(String)
VersionParse(String)
ServiceUpgradeParse(String)
}
class Result[T, E] {
Ok(T)
Err(E)
}
BackupManager --> DuckError : "returns"
DockerManager --> DuckError : "returns"
Result --> DuckError : "specialization"
style DuckError fill : #f9f,stroke : #333,stroke-width : 1px
```

**Diagram sources**
- [error.rs](file://client-core/src/error.rs#L3-L109)

**Section sources**
- [error.rs](file://client-core/src/error.rs#L3-L109)

### Restoration Error Handling
When restoration fails, the system follows a structured error handling approach that provides meaningful feedback and maintains system stability.

```mermaid
flowchart TD
Start([Restore Operation]) --> UnpackArchive["Unpack Backup Archive"]
UnpackArchive --> EntryLoop["For each archive entry"]
EntryLoop --> ReadEntry{"Read Entry"}
ReadEntry --> |Success| GetPath{"Get Entry Path"}
ReadEntry --> |Failure| HandleReadError["Map to DuckError::Backup"]
GetPath --> |Success| CheckExclusion{"Should Exclude?"}
GetPath --> |Failure| HandlePathError["Map to DuckError::Backup"]
CheckExclusion --> |No| CreateParent["Create Parent Directory"]
CheckExclusion --> |Yes| NextEntry["Skip Entry"]
CreateParent --> UnpackFile["Unpack File to Target"]
UnpackFile --> |Success| LogRestore["Log: '恢复文件: {path}'"]
UnpackFile --> |Failure| HandleUnpackError["Map to DuckError::Backup"]
NextEntry --> EntryLoop
LogRestore --> EntryLoop
HandleReadError --> ReturnError["Return DuckError"]
HandlePathError --> ReturnError
HandleUnpackError --> ReturnError
ReturnError --> LogError["Log Error with Context"]
LogError --> UpdateStatus["Update Backup Status to Failed"]
UpdateStatus --> End([Return Error])
style ReturnError fill:#f96,stroke:#333
style LogError fill:#f96,stroke:#333
style UpdateStatus fill:#f96,stroke:#333
```

**Diagram sources**
- [backup.rs](file://client-core/src/backup.rs#L346-L413)

**Section sources**
- [backup.rs](file://client-core/src/backup.rs#L346-L413)

## Common Issues and Troubleshooting

### Known Issues and Solutions
The following table outlines common issues encountered during rollback operations and their recommended solutions:

| Issue | Cause | Solution | Detection Method |
|-------|-------|----------|------------------|
| Corrupted Backup Archives | File system errors, incomplete backups, or storage issues | Verify backup integrity before restoration; implement checksum validation | Attempt to read archive header and validate entry count |
| Version Incompatibilities | Attempting to restore a backup created with a different software version | Ensure version compatibility between backup and current system; implement version validation | Compare service_version in backup record with current version |
| Running Container Conflicts | Services still running during restoration attempt | Ensure proper service shutdown before restoration; implement timeout and retry logic | Check service status via get_services_status() before proceeding |
| Permission Issues | Insufficient file system permissions for restoration | Ensure proper directory permissions; run with appropriate privileges | Catch Io errors during file operations |
| Disk Space Insufficiency | Insufficient space for extracted backup | Check available disk space before restoration; provide space estimation | Use estimate_backup_size() and compare with available space |

**Section sources**
- [backup.rs](file://client-core/src/backup.rs#L530-L568)
- [service.rs](file://client-core/src/container/service.rs#L450-L474)

## Verification and Monitoring

### Post-Rollback Validation
After completing the rollback process, the system performs comprehensive validation to ensure successful restoration and service operation.

```mermaid
flowchart TD
Start([Rollback Complete]) --> StartServices["Start Docker Services"]
StartServices --> WaitForStart["Wait for Service Startup"]
WaitForStart --> CheckInterval["Wait SERVICE_CHECK_INTERVAL seconds"]
CheckInterval --> GetStatus["Get Services Status"]
GetStatus --> AnalyzeStatus["Analyze Service States"]
AnalyzeStatus --> RunningServices{"All Services Running?"}
RunningServices --> |Yes| Success["Return Success"]
RunningServices --> |No| CheckAttempts{"Max Attempts Reached?"}
CheckAttempts --> |No| WaitForStart
CheckAttempts --> |Yes| FailedServices["Identify Failed Services"]
FailedServices --> ReturnFailure["Return Error with Details"]
Success --> LogSuccess["Log: '所有服务启动验证成功！'"]
ReturnFailure --> LogFailure["Log: '服务启动验证失败: {details}'"]
LogSuccess --> End([Verification Complete])
LogFailure --> End
```

**Diagram sources**
- [service.rs](file://client-core/src/container/service.rs#L450-L474)

**Section sources**
- [service.rs](file://client-core/src/container/service.rs#L450-L474)