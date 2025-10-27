# Docker Service Command

<cite>
**Referenced Files in This Document**   
- [docker_service.rs](file://nuwax-cli/src/commands/docker_service.rs)
- [manager.rs](file://nuwax-cli/src/docker_service/manager.rs)
- [health_check.rs](file://nuwax-cli/src/docker_service/health_check.rs)
- [image_loader.rs](file://nuwax-cli/src/docker_service/image_loader.rs)
- [mod.rs](file://nuwax-cli/src/docker_service/mod.rs)
- [cli.rs](file://nuwax-cli/src/cli.rs)
- [command.rs](file://client-core/src/container/command.rs)
- [service.rs](file://client-core/src/container/service.rs)
- [config.rs](file://client-core/src/container/config.rs)
- [types.rs](file://client-core/src/container/types.rs)
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
The Docker Service Command is a comprehensive toolset within the Nuwax CLI that provides low-level control over Dockerized services. This documentation details its capabilities for service inspection, lifecycle management (start/stop/restart), and configuration introspection. The command integrates with Bollard for Docker API communication, enabling container listing, log retrieval, and health status checks. It addresses security implications of Docker socket access and permission requirements, while also covering common failure modes such as container crashes, network conflicts, and image pull failures with diagnostic procedures and resolution steps.

## Project Structure
The project structure reveals a well-organized codebase with clear separation of concerns. The Docker service functionality is primarily located in the nuwax-cli component, with supporting infrastructure in client-core. The architecture follows a modular design pattern with distinct components for command handling, service management, health checking, and image loading.

```mermaid
graph TD
A[nuwax-cli] --> B[commands/docker_service.rs]
A --> C[docker_service/manager.rs]
A --> D[docker_service/health_check.rs]
A --> E[docker_service/image_loader.rs]
F[client-core] --> G[container/command.rs]
F --> H[container/service.rs]
F --> I[container/config.rs]
F --> J[container/types.rs]
B --> C
C --> D
C --> E
C --> G
C --> H
C --> I
C --> J
```

**Diagram sources**
- [docker_service.rs](file://nuwax-cli/src/commands/docker_service.rs)
- [manager.rs](file://nuwax-cli/src/docker_service/manager.rs)
- [command.rs](file://client-core/src/container/command.rs)
- [service.rs](file://client-core/src/container/service.rs)

**Section sources**
- [docker_service.rs](file://nuwax-cli/src/commands/docker_service.rs)
- [manager.rs](file://nuwax-cli/src/docker_service/manager.rs)
- [command.rs](file://client-core/src/container/command.rs)

## Core Components
The core components of the Docker Service Command system include the DockerServiceManager, HealthChecker, ImageLoader, and DockerManager. These components work together to provide comprehensive control over Dockerized services. The DockerServiceManager serves as the primary interface, coordinating operations between the various subsystems. The HealthChecker component provides detailed status information about running containers, while the ImageLoader handles the loading and tagging of Docker images. The DockerManager from client-core provides the foundational interface to Docker operations.

**Section sources**
- [manager.rs](file://nuwax-cli/src/docker_service/manager.rs)
- [health_check.rs](file://nuwax-cli/src/docker_service/health_check.rs)
- [image_loader.rs](file://nuwax-cli/src/docker_service/image_loader.rs)
- [types.rs](file://client-core/src/container/types.rs)

## Architecture Overview
The architecture of the Docker Service Command follows a layered approach with clear separation between command handling, service management, and Docker interaction. The system uses a modular design where each component has a specific responsibility, promoting maintainability and testability.

```mermaid
graph TD
A[CLI Interface] --> B[DockerServiceCommand]
B --> C[DockerServiceManager]
C --> D[HealthChecker]
C --> E[ImageLoader]
C --> F[PortManager]
C --> G[ScriptPermissionManager]
C --> H[DirectoryPermissionManager]
C --> I[DockerManager]
I --> J[Docker Daemon]
D --> I
E --> I
F --> I
G --> I
H --> I
```

**Diagram sources**
- [docker_service.rs](file://nuwax-cli/src/commands/docker_service.rs)
- [manager.rs](file://nuwax-cli/src/docker_service/manager.rs)
- [mod.rs](file://nuwax-cli/src/docker_service/mod.rs)
- [types.rs](file://client-core/src/container/types.rs)

## Detailed Component Analysis
This section provides an in-depth analysis of the key components that make up the Docker Service Command system, detailing their functionality, interactions, and implementation patterns.

### Docker Service Manager Analysis
The DockerServiceManager is the central component that orchestrates Docker service operations. It coordinates between various subsystems to provide a unified interface for service management.

#### Class Diagram
```mermaid
classDiagram
class DockerServiceManager {
+config : Arc<AppConfig>
+docker_manager : Arc<DockerManager>
+work_dir : PathBuf
+architecture : Architecture
+image_loader : ImageLoader
+health_checker : HealthChecker
+port_manager : PortManager
+script_permission_manager : ScriptPermissionManager
+directory_permission_manager : DirectoryPermissionManager
+new(config, docker_manager, work_dir) DockerServiceManager
+get_architecture() Architecture
+get_work_dir() &PathBuf
+deploy_services() DockerServiceResult~()~
+check_environment() DockerServiceResult~()~
+ensure_compose_mount_directories() DockerServiceResult~()~
+load_images() DockerServiceResult~LoadResult~
+setup_image_tags_with_mappings(image_mappings) DockerServiceResult~TagResult~
+setup_image_tags_with_ducker_validation(image_mappings) DockerServiceResult~TagResult~
+list_docker_images_with_ducker() DockerServiceResult~Vec<String>~
+start_services() DockerServiceResult~()~
+stop_services() DockerServiceResult~()~
+restart_services() DockerServiceResult~()~
+restart_container(container_name) DockerServiceResult~()~
+health_check() DockerServiceResult~HealthReport~
+get_status_summary() DockerServiceResult~String~
}
class DockerManager {
+compose_file : PathBuf
+env_file : PathBuf
+compose_config : Option<Compose>
+check_docker_status() Result~()~
+check_prerequisites() Result~()~
+run_compose_command(args) Result~Output~
+run_docker_command(args) Result~Output~
+start_services() Result~()~
+stop_services() Result~()~
+restart_services() Result~()~
+restart_service(service_name) Result~()~
+get_services_status() Result~Vec<ServiceInfo>~
+get_all_containers_status() Result~Vec<ServiceInfo>~
+ensure_host_volumes_exist() Result~()~
+get_compose_file() &Path
+get_working_directory() Option~&Path~
+load_compose_config() Result~Compose~
+is_oneshot_service(service_name) Result~bool~
+parse_service_config(service_name) Result~ServiceConfig~
+get_compose_service_names() Result~HashSet<String>~
+get_compose_project_name() String
+generate_compose_container_patterns(service_name) Vec~String~
}
class ImageLoader {
+docker_manager : Arc<DockerManager>
+work_dir : PathBuf
+architecture : Architecture
+images_dir : PathBuf
+new(docker_manager, work_dir) DockerServiceResult~Self~
+scan_architecture_images() DockerServiceResult~Vec<ImageInfo>~
+load_all_images() DockerServiceResult~LoadResult~
+setup_image_tags_with_mappings(image_mappings) DockerServiceResult~TagResult~
+setup_image_tags_with_validation(image_mappings) DockerServiceResult~TagResult~
+list_images_with_ducker() DockerServiceResult~Vec<String>~
}
class HealthChecker {
+docker_manager : Arc<DockerManager>
+new(docker_manager) Self
+health_check() DockerServiceResult~HealthReport~
+wait_for_services_ready(check_interval) DockerServiceResult~HealthReport~
+get_status_summary() DockerServiceResult~String~
}
DockerServiceManager --> DockerManager : "uses"
DockerServiceManager --> ImageLoader : "uses"
DockerServiceManager --> HealthChecker : "uses"
DockerServiceManager --> PortManager : "uses"
DockerServiceManager --> ScriptPermissionManager : "uses"
DockerServiceManager --> DirectoryPermissionManager : "uses"
ImageLoader --> DockerManager : "uses"
HealthChecker --> DockerManager : "uses"
```

**Diagram sources**
- [manager.rs](file://nuwax-cli/src/docker_service/manager.rs)
- [image_loader.rs](file://nuwax-cli/src/docker_service/image_loader.rs)
- [health_check.rs](file://nuwax-cli/src/docker_service/health_check.rs)
- [types.rs](file://client-core/src/container/types.rs)

**Section sources**
- [manager.rs](file://nuwax-cli/src/docker_service/manager.rs)
- [image_loader.rs](file://nuwax-cli/src/docker_service/image_loader.rs)
- [health_check.rs](file://nuwax-cli/src/docker_service/health_check.rs)

### Health Check System Analysis
The health check system provides comprehensive monitoring of Docker service status, offering detailed insights into container states and overall system health.

#### Sequence Diagram
```mermaid
sequenceDiagram
participant CLI as "CLI Command"
participant Manager as "DockerServiceManager"
participant Checker as "HealthChecker"
participant Docker as "DockerManager"
participant Daemon as "Docker Daemon"
CLI->>Manager : health_check()
Manager->>Checker : health_check()
Checker->>Docker : get_services_status()
Docker->>Daemon : docker ps --format JSON
Daemon-->>Docker : Container Status Data
Docker->>Docker : parse_container_status()
Docker-->>Checker : ServiceInfo List
Checker->>Checker : create_health_report()
Checker->>Checker : determine_overall_status()
Checker-->>Manager : HealthReport
Manager-->>CLI : HealthReport
```

**Diagram sources**
- [health_check.rs](file://nuwax-cli/src/docker_service/health_check.rs)
- [service.rs](file://client-core/src/container/service.rs)
- [types.rs](file://client-core/src/container/types.rs)

**Section sources**
- [health_check.rs](file://nuwax-cli/src/docker_service/health_check.rs)
- [service.rs](file://client-core/src/container/service.rs)

### Image Loading System Analysis
The image loading system handles the loading and tagging of Docker images, with special consideration for architecture-specific images.

#### Flowchart
```mermaid
flowchart TD
Start([Start Load Images]) --> Scan["Scan Images Directory"]
Scan --> Exists{"Images Directory Exists?"}
Exists --> |No| Error["Return Error"]
Exists --> |Yes| Filter["Filter by Architecture"]
Filter --> Found{"Images Found?"}
Found --> |No| Error
Found --> |Yes| Load["Load Each Image"]
Load --> Success{"Load Successful?"}
Success --> |Yes| RecordSuccess["Record Success"]
Success --> |No| RecordFailure["Record Failure"]
RecordSuccess --> Next["Next Image"]
RecordFailure --> Next
Next --> More{"More Images?"}
More --> |Yes| Load
More --> |No| Complete["Complete Load Process"]
Complete --> Return["Return LoadResult"]
Error --> Return
```

**Diagram sources**
- [image_loader.rs](file://nuwax-cli/src/docker_service/image_loader.rs)
- [command.rs](file://client-core/src/container/command.rs)

**Section sources**
- [image_loader.rs](file://nuwax-cli/src/docker_service/image_loader.rs)

## Dependency Analysis
The Docker Service Command system has a well-defined dependency structure with clear relationships between components. The system relies on external libraries like Bollard for Docker API communication and ducker for container management.

```mermaid
graph TD
A[DockerServiceCommand] --> B[DockerServiceManager]
B --> C[HealthChecker]
B --> D[ImageLoader]
B --> E[PortManager]
B --> F[ScriptPermissionManager]
B --> G[DirectoryPermissionManager]
B --> H[DockerManager]
C --> H
D --> H
E --> H
F --> H
G --> H
H --> I[Bollard]
H --> J[ducker]
I --> K[Docker Daemon]
J --> K
```

**Diagram sources**
- [mod.rs](file://nuwax-cli/src/docker_service/mod.rs)
- [manager.rs](file://nuwax-cli/src/docker_service/manager.rs)
- [types.rs](file://client-core/src/container/types.rs)

**Section sources**
- [mod.rs](file://nuwax-cli/src/docker_service/mod.rs)
- [manager.rs](file://nuwax-cli/src/docker_service/manager.rs)

## Performance Considerations
The Docker Service Command system is designed with performance in mind, implementing caching mechanisms and efficient resource management. The HealthChecker component uses a 30-second TTL cache for docker-compose configuration to avoid repeated parsing. The system also implements progressive permission management to minimize filesystem operations. When loading images, the system processes them sequentially to avoid overwhelming system resources. The health check system uses efficient container status queries rather than full container inspections for performance.

## Troubleshooting Guide
This section addresses common issues encountered when using the Docker Service Command and provides diagnostic procedures and resolution steps.

**Section sources**
- [docker_service.rs](file://nuwax-cli/src/commands/docker_service.rs)
- [manager.rs](file://nuwax-cli/src/docker_service/manager.rs)
- [health_check.rs](file://nuwax-cli/src/docker_service/health_check.rs)
- [command.rs](file://client-core/src/container/command.rs)

### Common Failure Modes and Solutions
#### Container Crashes
When containers crash, the system provides detailed error information through the health check command. Common causes include:
- **Missing dependencies**: Ensure all required services are running
- **Configuration errors**: Verify environment variables and configuration files
- **Resource constraints**: Check system resources (memory, CPU, disk space)

Diagnostic command:
```bash
nuwax-cli docker-service status
```

#### Network Conflicts
Network conflicts typically occur when ports are already in use. The system includes a port manager to detect and report these conflicts.

Resolution steps:
1. Check for port conflicts:
```bash
nuwax-cli docker-service status
```
2. Identify the conflicting process:
```bash
lsof -i :<port_number>
```
3. Either stop the conflicting process or configure the service to use a different port

#### Image Pull Failures
Image pull failures can occur due to network issues, authentication problems, or incorrect image names.

Diagnostic and resolution steps:
1. Check image availability:
```bash
nuwax-cli docker-service list-images
```
2. Verify network connectivity to the registry
3. Check authentication credentials if using a private registry
4. Ensure the image name and tag are correct

### Security Implications
The Docker Service Command requires access to the Docker socket, which grants significant system privileges. This access is necessary for managing containers but presents security considerations:

- **Permission Requirements**: The command must run with sufficient privileges to access the Docker daemon
- **Socket Security**: The Docker socket should be protected from unauthorized access
- **Container Isolation**: Ensure containers are properly isolated from the host system
- **Image Verification**: Only use trusted images from verified sources

Best practices:
- Run the command with the minimum required privileges
- Regularly update Docker and container images
- Implement network policies to restrict container communication
- Monitor container activity for suspicious behavior

## Conclusion
The Docker Service Command provides a comprehensive and robust interface for managing Dockerized services. Its modular architecture, clear separation of concerns, and comprehensive feature set make it a powerful tool for both development and production environments. The system's integration with Bollard and ducker provides reliable Docker API communication, while its detailed health checking and diagnostic capabilities facilitate effective troubleshooting. By understanding the system's architecture and components, users can effectively leverage its capabilities for service management, monitoring, and maintenance.