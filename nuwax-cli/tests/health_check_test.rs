use nuwax_cli::health_check::{ContainerStatus, RestartPolicy};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restart_policy_parsing() {
        assert_eq!(RestartPolicy::from_str("no"), Some(RestartPolicy::No));
        assert_eq!(
            RestartPolicy::from_str("always"),
            Some(RestartPolicy::Always)
        );
        assert_eq!(
            RestartPolicy::from_str("unless-stopped"),
            Some(RestartPolicy::UnlessStopped)
        );
        assert_eq!(
            RestartPolicy::from_str("on-failure"),
            Some(RestartPolicy::OnFailure)
        );
        assert_eq!(
            RestartPolicy::from_str("on-failure:3"),
            Some(RestartPolicy::OnFailureWithRetries(3))
        );
        assert_eq!(RestartPolicy::from_str("invalid"), None);
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

    #[test]
    fn test_container_status_display() {
        assert_eq!(format!("{:?}", ContainerStatus::Running), "运行中");
        assert_eq!(format!("{:?}", ContainerStatus::Stopped), "已停止");
        assert_eq!(format!("{:?}", ContainerStatus::Completed), "已完成");
        assert_eq!(format!("{:?}", ContainerStatus::Unknown), "未知");
        assert_eq!(format!("{:?}", ContainerStatus::Starting), "启动中");
    }

    #[test]
    fn test_restart_policy_display() {
        assert_eq!(format!("{:?}", RestartPolicy::No), "不重启");
        assert_eq!(format!("{:?}", RestartPolicy::Always), "总是重启");
        assert_eq!(format!("{:?}", RestartPolicy::UnlessStopped), "除非手动停止");
        assert_eq!(format!("{:?}", RestartPolicy::OnFailure), "失败时重启");
        assert_eq!(
            format!("{:?}", RestartPolicy::OnFailureWithRetries(3)),
            "失败时重启(最多3次)"
        );
    }
}
