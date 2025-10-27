pub mod auto_backup;
pub mod auto_upgrade_deploy;
pub mod backup;
pub mod cache;
pub mod check_update;
pub mod diff_sql;
pub mod docker_service;
pub mod ducker;
pub mod status;
pub mod update;

// Status commands
pub use status::{run_api_info, run_status, run_status_details, show_client_version};

// Backup commands
pub use backup::{run_backup, run_list_backups};

// Update commands
pub use update::run_upgrade;

// Docker service commands
pub use docker_service::run_docker_service_command;

// Ducker command
pub use ducker::run_ducker;

// Auto backup commands
pub use auto_backup::handle_auto_backup;

// Auto upgrade deploy commands
pub use auto_upgrade_deploy::handle_auto_upgrade_deploy_command;

// Cache commands
pub use cache::handle_cache_command;

// Check update commands
pub use check_update::handle_check_update_command;

// Diff SQL commands
pub use diff_sql::run_diff_sql;
