# OSS Storage Integration

<cite>
**Referenced Files in This Document**   
- [downloader.rs](file://client-core/src/downloader.rs)
- [update.rs](file://nuwax-cli/src/commands/update.rs)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs)
- [api_types.rs](file://client-core/src/api_types.rs)
- [config_manager.rs](file://client-core/src/config_manager.rs)
- [docker-compose.yml](file://client-core/fixtures/docker-compose.yml)
</cite>

## Table of Contents
1. [Introduction](#introduction)
2. [Project Structure](#project-structure)
3. [Core Components](#core-components)
4. [Architecture Overview](#architecture-overview)
5. [Detailed Component Analysis](#detailed-component-analysis)
6. [Dependency Analysis](#dependency-analysis)
7. [Performance Considerations](#performance-considerations)
8. [Troubleshooting Guide](#troubleshooting-guide)
9. [Conclusion](#conclusion)

## Introduction
This document provides comprehensive documentation for the Object Storage Service (OSS) integration within the duck_client application. The system is designed to handle large update packages and patch files through robust download mechanisms that support resumable transfers, integrity verification, and performance optimization. The implementation focuses on reliability for large file transfers, with features including signed URL construction, chunked downloads, hash-based integrity checks, and digital signature validation. This documentation details the architecture, functionality, and configuration of the downloader module and its integration into upgrade workflows.

## Project Structure
The project follows a modular architecture with distinct components for different responsibilities. The core functionality for OSS integration resides in the client-core module, specifically within the downloader component. The CLI interface (nuwax-cli) orchestrates upgrade workflows, while the UI layer (cli-ui) provides user interaction. Configuration management is centralized through the config_manager, and patch execution is handled by dedicated modules.

```mermaid
graph TB
subgraph "User Interface"
CLI[nuwax-cli]
UI[cli-ui]
end
subgraph "Core Functionality"
Downloader[client-core/downloader]
ConfigManager[client-core/config_manager]
PatchExecutor[client-core/patch_executor]
API[client-core/api]
end
subgraph "Infrastructure"
OSS[(OSS Providers)]
MinIO[MinIO]
AWS[AWS S3]
end
CLI --> Downloader
UI --> CLI
Downloader --> OSS
Downloader --> ConfigManager
PatchExecutor --> Downloader
API --> Downloader
style Downloader fill:#4CAF50,stroke:#388E3C
style OSS fill:#2196F3,stroke:#1976D2
```

**Diagram sources**
- [downloader.rs](file://client-core/src/downloader.rs)
- [update.rs](file://nuwax-cli/src/commands/update.rs)

**Section sources**
- [downloader.rs](file://client-core/src/downloader.rs)
- [update.rs](file://nuwax-cli/src/commands/update.rs)

## Core Components
The OSS integration system comprises several key components that work together to provide reliable file downloads for update packages. The FileDownloader class is the central component responsible for managing downloads, supporting both standard HTTP and extended timeout connections for object storage services. It implements resumable downloads through HTTP Range requests and maintains download state through metadata files. The system integrates with upgrade workflows through the update command, which coordinates the download of service packages. Integrity verification is performed using SHA-256 hashes and digital signatures, with the patch processor handling validation of downloaded packages.

**Section sources**
- [downloader.rs](file://client-core/src/downloader.rs#L1-L100)
- [update.rs](file://nuwax-cli/src/commands/update.rs#L1-L50)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L1-L30)

## Architecture Overview
The OSS integration architecture follows a layered approach with clear separation of concerns. At the foundation is the HTTP client layer, which handles network communication with OSS providers. Above this, the downloader module provides abstraction for different storage providers and implements advanced download features. The configuration manager stores settings for download behavior and credentials. The upgrade workflow orchestrator coordinates the download process, while the patch executor handles post-download validation and processing.

```mermaid
graph TD
A[User Request] --> B[Upgrade Workflow]
B --> C[Download Manager]
C --> D[OSS Provider]
D --> E[(AWS S3)]
D --> F[(MinIO)]
D --> G[(Aliyun OSS)]
D --> H[(Other S3-compatible)]
C --> I[Configuration Manager]
I --> J[(Database)]
C --> K[Metadata Storage]
K --> L[.download files]
C --> M[Integrity Verification]
M --> N[SHA-256 Hash Check]
M --> O[Digital Signature]
B --> P[Patch Processor]
P --> Q[Extract Package]
P --> R[Apply Updates]
style C fill:#4CAF50,stroke:#388E3C
style D fill:#2196F3,stroke:#1976D2
style M fill:#FF9800,stroke:#F57C00
```

**Diagram sources**
- [downloader.rs](file://client-core/src/downloader.rs#L1-L50)
- [update.rs](file://nuwax-cli/src/commands/update.rs#L1-L30)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L1-L20)

## Detailed Component Analysis

### File Downloader Analysis
The FileDownloader implementation provides comprehensive functionality for downloading files from OSS providers with support for resumable transfers and integrity verification. It automatically detects the type of storage provider based on the URL and configures appropriate timeouts and headers.

#### Class Diagram for Downloader Components
```mermaid
classDiagram
class FileDownloader {
+config : DownloaderConfig
+client : Client
+custom_client : Option~Client~
+new(config : DownloaderConfig) FileDownloader
+new_with_custom_client(config : DownloaderConfig, client : Client) FileDownloader
+download_file(url : &str, path : &Path, callback : Option~F~) Result~()~
+download_file_with_options(url : &str, path : &Path, callback : Option~F~, hash : Option~&str~, version : Option~&str~) Result~()~
+is_object_storage_or_cdn_url(url : &str) bool
+get_downloader_type(url : &str) DownloaderType
+check_range_support(url : &str) Result~(bool, u64)~
+calculate_file_hash(path : &Path) Result~String~
+verify_file_integrity(path : &Path, expected_hash : &str) Result~bool~
}
class DownloaderConfig {
+timeout_seconds : u64
+chunk_size : usize
+retry_count : u32
+enable_progress_logging : bool
+enable_resume : bool
+resume_threshold : u64
+progress_interval_seconds : u64
+progress_bytes_interval : u64
+enable_metadata : bool
}
class DownloadMetadata {
+url : String
+expected_size : u64
+expected_hash : Option~String~
+downloaded_bytes : u64
+start_time : String
+last_update : String
+version : String
+new(url : String, size : u64, hash : Option~String~, version : String) DownloadMetadata
+update_progress(bytes : u64) void
+is_same_task(url : &str, size : u64, version : &str) bool
}
class DownloadProgress {
+task_id : String
+file_name : String
+downloaded_bytes : u64
+total_bytes : u64
+download_speed : f64
+eta_seconds : u64
+percentage : f64
+status : DownloadStatus
}
class DownloadStatus {
<<enumeration>>
Starting
Downloading
Resuming
Paused
Completed
Failed
}
class DownloaderType {
<<enumeration>>
Http
HttpExtendedTimeout
}
FileDownloader --> DownloaderConfig : "uses"
FileDownloader --> DownloadMetadata : "creates/manages"
FileDownloader --> DownloadProgress : "emits"
FileDownloader --> DownloadStatus : "uses"
FileDownloader --> DownloaderType : "returns"
```

**Diagram sources**
- [downloader.rs](file://client-core/src/downloader.rs#L50-L200)

**Section sources**
- [downloader.rs](file://client-core/src/downloader.rs#L1-L300)

### Upgrade Workflow Integration
The downloader is integrated into the upgrade workflow through the update command, which orchestrates the download of service packages from OSS providers. The workflow handles both full upgrades and patch-based updates, with appropriate download strategies for each.

#### Sequence Diagram for Upgrade Process
```mermaid
sequenceDiagram
participant CLI as "nuwax-cli"
participant Upgrade as "Upgrade Manager"
participant Downloader as "FileDownloader"
participant OSS as "OSS Provider"
participant Patch as "Patch Processor"
CLI->>Upgrade : run_upgrade(args)
Upgrade->>Upgrade : check_for_updates(force)
Upgrade-->>CLI : UpgradeStrategy
alt Full Upgrade
CLI->>Downloader : download_service_update_optimized()
Downloader->>OSS : HEAD request
OSS-->>Downloader : Content-Length, Accept-Ranges
Downloader->>Downloader : check_resume_feasibility()
alt Resume Possible
Downloader->>OSS : GET with Range header
OSS-->>Downloader : Partial Content (206)
Downloader->>Downloader : download_with_resume_internal()
Downloader->>CLI : progress_callback()
else New Download
Downloader->>OSS : GET
OSS-->>Downloader : OK (200)
Downloader->>Downloader : download_stream_with_resume()
Downloader->>CLI : progress_callback()
end
Downloader-->>CLI : download_result
CLI->>CLI : handle_service_download completion
else Patch Upgrade
CLI->>Downloader : download_service_update_optimized()
Downloader->>OSS : Download patch file
OSS-->>Downloader : File data
Downloader-->>CLI : download_result
CLI->>Patch : verify_patch_integrity()
Patch->>Patch : verify_hash()
Patch->>Patch : verify_signature()
Patch-->>CLI : verification_result
end
CLI-->>User : Update completion status
```

**Diagram sources**
- [update.rs](file://nuwax-cli/src/commands/update.rs#L1-L160)
- [downloader.rs](file://client-core/src/downloader.rs#L500-L1000)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L100-L200)

**Section sources**
- [update.rs](file://nuwax-cli/src/commands/update.rs#L1-L160)

### Integrity Verification Analysis
The system implements a multi-layered approach to file integrity verification, combining cryptographic hash checks with digital signature validation to ensure downloaded packages have not been tampered with.

#### Flowchart for Integrity Verification
```mermaid
flowchart TD
Start([Start Verification]) --> CheckHash{"Hash Provided?"}
CheckHash --> |Yes| CalculateHash["Calculate SHA-256 hash of file"]
CalculateHash --> CompareHash["Compare with expected hash"]
CompareHash --> HashMatch{"Hashes match?"}
HashMatch --> |No| FailHash["Fail: Hash mismatch"]
FailHash --> End([Verification Failed])
CheckHash --> |No| CheckSignature{"Signature Provided?"}
CheckSignature --> |Yes| ValidateSignature["Validate digital signature"]
ValidateSignature --> SigValid{"Signature valid?"}
SigValid --> |No| FailSig["Fail: Invalid signature"]
FailSig --> End
HashMatch --> |Yes| CheckSignature
SigValid --> |Yes| Success["Success: Integrity verified"]
Success --> End([Verification Passed])
CheckSignature --> |No| Success
```

**Diagram sources**
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L130-L180)
- [api_types.rs](file://client-core/src/api_types.rs#L800-L830)

**Section sources**
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L130-L180)

## Dependency Analysis
The OSS integration system has well-defined dependencies between components, with clear interfaces and minimal coupling. The downloader module depends on external crates for HTTP functionality, cryptographic operations, and asynchronous I/O, while maintaining independence from the UI and business logic layers.

```mermaid
graph LR
A[FileDownloader] --> B[reqwest]
A --> C[sha2]
A --> D[tokio]
A --> E[futures]
A --> F[serde]
A --> G[tracing]
H[ConfigManager] --> I[duckdb]
H --> J[serde_json]
K[PatchProcessor] --> L[flate2]
K --> M[tar]
K --> N[tempfile]
A --> H
K --> A
L[update.rs] --> A
L --> K
style A fill:#4CAF50,stroke:#388E3C
style H fill:#2196F3,stroke:#1976D2
style K fill:#FF9800,stroke:#F57C00
```

**Diagram sources**
- [downloader.rs](file://client-core/src/downloader.rs#L1-L20)
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L20)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L1-L20)

**Section sources**
- [downloader.rs](file://client-core/src/downloader.rs#L1-L50)
- [config_manager.rs](file://client-core/src/config_manager.rs#L1-L50)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L1-L50)

## Performance Considerations
The downloader implementation includes several performance optimizations for handling large update packages. The default configuration sets a 60-minute timeout for downloads, with an 8KB chunk size for streaming data. Progress updates are throttled to reduce logging overhead, with metadata saved only every 500MB or 5 minutes to minimize disk I/O. The system automatically detects object storage providers and applies extended timeouts for these services. For files larger than 1MB, the resumable download feature is enabled by default, allowing interrupted downloads to be resumed from the point of failure. Bandwidth usage is not explicitly limited, but the chunked download approach naturally limits memory usage during transfers.

**Section sources**
- [downloader.rs](file://client-core/src/downloader.rs#L130-L170)

## Troubleshooting Guide
When encountering issues with OSS downloads, several common problems and their solutions should be considered. For failed downloads, check network connectivity and ensure the OSS URL is accessible. If resumable downloads are not working, verify that the server supports HTTP Range requests by checking for the Accept-Ranges header in responses. For integrity verification failures, confirm that the expected hash matches the actual file content, and ensure the digital signature is in valid base64 format. If metadata files (.download) become corrupted, they can be safely deleted to force a fresh download. For authentication issues with private OSS buckets, ensure credentials are properly configured in environment variables or configuration files. Monitoring logs can provide insight into the download process, with detailed information about HTTP requests, progress updates, and error conditions.

**Section sources**
- [downloader.rs](file://client-core/src/downloader.rs#L300-L400)
- [patch_processor.rs](file://client-core/src/patch_executor/patch_processor.rs#L150-L180)

## Conclusion
The OSS integration in duck_client provides a robust solution for downloading large update packages and patch files. The system's architecture supports reliable transfers through resumable downloads, ensures file integrity through cryptographic verification, and integrates seamlessly with upgrade workflows. By automatically detecting object storage providers and applying appropriate configurations, the downloader simplifies the process of working with various OSS services. The implementation demonstrates best practices in error handling, progress tracking, and performance optimization for large file transfers. Future enhancements could include bandwidth throttling, parallel downloads for segmented files, and more comprehensive digital signature verification with certificate chain validation.