use nuwax_cli::health_check::{ContainerStatus, RestartPolicy};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restart_policy_parsing() {
        assert_eq!(RestartPolicy::parse("no"), Some(RestartPolicy::No));
        assert_eq!(RestartPolicy::parse("always"), Some(RestartPolicy::Always));
        assert_eq!(
            RestartPolicy::parse("unless-stopped"),
            Some(RestartPolicy::UnlessStopped)
        );
        assert_eq!(
            RestartPolicy::parse("on-failure"),
            Some(RestartPolicy::OnFailure)
        );
        assert_eq!(
            RestartPolicy::parse("on-failure:3"),
            Some(RestartPolicy::OnFailureWithRetries(3))
        );
        assert_eq!(RestartPolicy::parse("invalid"), None);
    }

    #[test]
    fn test_restart_policy_oneshot_detection() {
        assert!(RestartPolicy::No.is_oneshot());
        assert!(!RestartPolicy::Always.is_oneshot());
        assert!(!RestartPolicy::UnlessStopped.is_oneshot());
        assert!(!RestartPolicy::OnFailure.is_oneshot());
        assert!(!RestartPolicy::OnFailureWithRetries(3).is_oneshot());
    }

    #[test]
    fn test_container_status_health() {
        assert!(ContainerStatus::Running.is_healthy());
        assert!(ContainerStatus::Completed.is_healthy());
        assert!(!ContainerStatus::Stopped.is_healthy());
        assert!(!ContainerStatus::Unknown.is_healthy());
        assert!(!ContainerStatus::Starting.is_healthy());
    }

    #[test]
    fn test_restart_policy_to_string() {
        assert_eq!(RestartPolicy::No.to_string(), "no");
        assert_eq!(RestartPolicy::Always.to_string(), "always");
        assert_eq!(RestartPolicy::UnlessStopped.to_string(), "unless-stopped");
        assert_eq!(RestartPolicy::OnFailure.to_string(), "on-failure");
        assert_eq!(
            RestartPolicy::OnFailureWithRetries(3).to_string(),
            "on-failure:3"
        );
    }
}
