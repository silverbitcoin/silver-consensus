use silver_consensus::{
    ValidatorMonitor, ValidatorMetrics, MonitoringConfig, AlertSeverity, AlertType,
    Status,
};
use silver_core::ValidatorID;

#[test]
fn test_comprehensive_monitoring_workflow() {
    let config = MonitoringConfig::default();
    let mut monitor = ValidatorMonitor::new(config);

    // Register validators
    let v1 = ValidatorID::from("validator1");
    let v2 = ValidatorID::from("validator2");
    let v3 = ValidatorID::from("validator3");

    monitor.register_validator(v1.clone());
    monitor.register_validator(v2.clone());
    monitor.register_validator(v3.clone());

    assert_eq!(monitor.validator_count(), 3);

    // Simulate healthy validator
    for _ in 0..100 {
        monitor.record_snapshot(&v1, true, 50).unwrap();
    }

    // Simulate degraded validator
    for _ in 0..90 {
        monitor.record_snapshot(&v2, true, 100).unwrap();
    }
    for _ in 0..10 {
        monitor.record_snapshot(&v2, false, 0).unwrap();
    }

    // Simulate offline validator
    for _ in 0..50 {
        monitor.record_snapshot(&v3, true, 50).unwrap();
    }
    for _ in 0..50 {
        monitor.record_snapshot(&v3, false, 0).unwrap();
    }

    // Check metrics
    let m1 = monitor.get_metrics(&v1).unwrap();
    assert_eq!(m1.participation_rate, 1.0);
    assert_eq!(m1.uptime_percentage, 100.0);

    let m2 = monitor.get_metrics(&v2).unwrap();
    assert!((m2.participation_rate - 0.9).abs() < 0.01);

    let m3 = monitor.get_metrics(&v3).unwrap();
    assert!((m3.participation_rate - 0.5).abs() < 0.01);

    // Check health status
    let health = monitor.get_health_status();
    assert_eq!(health.total_validators, 3);
    assert!(health.average_participation > 0.0);
}

#[test]
fn test_alert_generation() {
    let config = MonitoringConfig {
        participation_warning_threshold: 0.95,
        participation_critical_threshold: 0.90,
        response_time_threshold_ms: 500,
        offline_timeout_secs: 300,
        consecutive_failures_threshold: 5,
    };

    let mut monitor = ValidatorMonitor::new(config);
    let validator_id = ValidatorID::from("validator1");

    monitor.register_validator(validator_id.clone());

    // Record 85% participation (below critical threshold)
    for _ in 0..85 {
        monitor.record_snapshot(&validator_id, true, 100).unwrap();
    }
    for _ in 0..15 {
        monitor.record_snapshot(&validator_id, false, 0).unwrap();
    }

    let alerts = monitor.get_active_alerts();
    assert!(!alerts.is_empty());

    let critical_alerts: Vec<_> = alerts
        .iter()
        .filter(|a| a.severity == AlertSeverity::Critical)
        .collect();
    assert!(!critical_alerts.is_empty());
}

#[test]
fn test_response_time_tracking() {
    let mut monitor = ValidatorMonitor::default();
    let validator_id = ValidatorID::from("validator1");

    monitor.register_validator(validator_id.clone());

    // Record snapshots with varying response times
    monitor.record_snapshot(&validator_id, true, 100).unwrap();
    monitor.record_snapshot(&validator_id, true, 200).unwrap();
    monitor.record_snapshot(&validator_id, true, 150).unwrap();

    let metrics = monitor.get_metrics(&validator_id).unwrap();
    assert_eq!(metrics.min_response_time_ms, 100);
    assert_eq!(metrics.max_response_time_ms, 200);
    assert_eq!(metrics.avg_response_time_ms, 150);
}

#[test]
fn test_consecutive_failures_tracking() {
    let mut monitor = ValidatorMonitor::default();
    let validator_id = ValidatorID::from("validator1");

    monitor.register_validator(validator_id.clone());

    // Record consecutive failures
    for _ in 0..5 {
        monitor.record_snapshot(&validator_id, false, 0).unwrap();
    }

    let metrics = monitor.get_metrics(&validator_id).unwrap();
    assert_eq!(metrics.consecutive_failures, 5);
    assert_eq!(metrics.total_failures, 5);

    // Record success - should reset consecutive failures
    monitor.record_snapshot(&validator_id, true, 100).unwrap();

    let metrics = monitor.get_metrics(&validator_id).unwrap();
    assert_eq!(metrics.consecutive_failures, 0);
    assert_eq!(metrics.total_failures, 5);
}

#[test]
fn test_alert_history() {
    let config = MonitoringConfig {
        participation_critical_threshold: 0.90,
        ..Default::default()
    };

    let mut monitor = ValidatorMonitor::new(config);
    let validator_id = ValidatorID::from("validator1");

    monitor.register_validator(validator_id.clone());

    // Generate alerts
    for _ in 0..85 {
        monitor.record_snapshot(&validator_id, true, 100).unwrap();
    }
    for _ in 0..15 {
        monitor.record_snapshot(&validator_id, false, 0).unwrap();
    }

    let history = monitor.get_validator_alert_history(&validator_id);
    assert!(!history.is_empty());
}

#[test]
fn test_critical_and_warning_validators() {
    let config = MonitoringConfig {
        participation_warning_threshold: 0.95,
        participation_critical_threshold: 0.90,
        ..Default::default()
    };

    let mut monitor = ValidatorMonitor::new(config);

    let v_critical = ValidatorID::from("validator_critical");
    let v_warning = ValidatorID::from("validator_warning");
    let v_healthy = ValidatorID::from("validator_healthy");

    monitor.register_validator(v_critical.clone());
    monitor.register_validator(v_warning.clone());
    monitor.register_validator(v_healthy.clone());

    // Critical: 85% participation
    for _ in 0..85 {
        monitor.record_snapshot(&v_critical, true, 100).unwrap();
    }
    for _ in 0..15 {
        monitor.record_snapshot(&v_critical, false, 0).unwrap();
    }

    // Warning: 93% participation
    for _ in 0..93 {
        monitor.record_snapshot(&v_warning, true, 100).unwrap();
    }
    for _ in 0..7 {
        monitor.record_snapshot(&v_warning, false, 0).unwrap();
    }

    // Healthy: 100% participation
    for _ in 0..100 {
        monitor.record_snapshot(&v_healthy, true, 100).unwrap();
    }

    let critical = monitor.get_critical_validators();
    let warnings = monitor.get_warning_validators();

    assert!(critical.contains(&v_critical));
    assert!(warnings.contains(&v_warning));
    assert!(!critical.contains(&v_healthy));
    assert!(!warnings.contains(&v_healthy));
}

#[test]
fn test_average_metrics() {
    let mut monitor = ValidatorMonitor::default();

    let v1 = ValidatorID::from("validator1");
    let v2 = ValidatorID::from("validator2");

    monitor.register_validator(v1.clone());
    monitor.register_validator(v2.clone());

    // v1: 100% participation
    for _ in 0..100 {
        monitor.record_snapshot(&v1, true, 100).unwrap();
    }

    // v2: 50% participation
    for _ in 0..50 {
        monitor.record_snapshot(&v2, true, 100).unwrap();
    }
    for _ in 0..50 {
        monitor.record_snapshot(&v2, false, 0).unwrap();
    }

    let avg_participation = monitor.get_average_participation();
    assert!((avg_participation - 0.75).abs() < 0.01);

    let avg_uptime = monitor.get_average_uptime();
    assert!((avg_uptime - 75.0).abs() < 0.1);
}

#[test]
fn test_health_status_transitions() {
    let config = MonitoringConfig {
        participation_critical_threshold: 0.90,
        ..Default::default()
    };

    let mut monitor = ValidatorMonitor::new(config);
    let validator_id = ValidatorID::from("validator1");

    monitor.register_validator(validator_id.clone());

    // Initially healthy
    for _ in 0..100 {
        monitor.record_snapshot(&validator_id, true, 100).unwrap();
    }

    let health = monitor.get_health_status();
    assert_eq!(health.status, Status::Healthy);

    // Clear and make it critical
    monitor.reset_period();
    for _ in 0..85 {
        monitor.record_snapshot(&validator_id, true, 100).unwrap();
    }
    for _ in 0..15 {
        monitor.record_snapshot(&validator_id, false, 0).unwrap();
    }

    let health = monitor.get_health_status();
    assert_eq!(health.status, Status::Critical);
}

#[test]
fn test_period_reset() {
    let mut monitor = ValidatorMonitor::default();
    let validator_id = ValidatorID::from("validator1");

    monitor.register_validator(validator_id.clone());

    // Record some snapshots
    for _ in 0..50 {
        monitor.record_snapshot(&validator_id, true, 100).unwrap();
    }

    let metrics_before = monitor.get_metrics(&validator_id).unwrap();
    assert_eq!(metrics_before.total_snapshots, 50);

    // Reset period
    monitor.reset_period();

    let metrics_after = monitor.get_metrics(&validator_id).unwrap();
    assert_eq!(metrics_after.total_snapshots, 0);
    assert_eq!(metrics_after.snapshots_participated, 0);
}

#[test]
fn test_config_update() {
    let mut monitor = ValidatorMonitor::default();

    let original_config = monitor.get_config().clone();
    assert_eq!(original_config.participation_warning_threshold, 0.95);

    let new_config = MonitoringConfig {
        participation_warning_threshold: 0.90,
        ..Default::default()
    };

    monitor.update_config(new_config);

    let updated_config = monitor.get_config();
    assert_eq!(updated_config.participation_warning_threshold, 0.90);
}

#[test]
fn test_multiple_validators_monitoring() {
    let mut monitor = ValidatorMonitor::default();

    // Register 10 validators
    let validators: Vec<_> = (0..10)
        .map(|i| ValidatorID::from(format!("validator{}", i)))
        .collect();

    for v in &validators {
        monitor.register_validator(v.clone());
    }

    assert_eq!(monitor.validator_count(), 10);

    // Record different participation rates
    for (i, v) in validators.iter().enumerate() {
        let participation = 100 - (i as u64 * 5);
        for _ in 0..participation {
            monitor.record_snapshot(v, true, 100).unwrap();
        }
        for _ in 0..(100 - participation) {
            monitor.record_snapshot(v, false, 0).unwrap();
        }
    }

    let all_metrics = monitor.get_all_metrics();
    assert_eq!(all_metrics.len(), 10);

    let avg_participation = monitor.get_average_participation();
    assert!(avg_participation > 0.0 && avg_participation < 1.0);
}

#[test]
fn test_alert_severity_levels() {
    let config = MonitoringConfig {
        participation_warning_threshold: 0.95,
        participation_critical_threshold: 0.90,
        response_time_threshold_ms: 500,
        offline_timeout_secs: 300,
        consecutive_failures_threshold: 5,
    };

    let mut monitor = ValidatorMonitor::new(config);

    let v_critical = ValidatorID::from("validator_critical");
    let v_warning = ValidatorID::from("validator_warning");

    monitor.register_validator(v_critical.clone());
    monitor.register_validator(v_warning.clone());

    // Critical: 85% participation
    for _ in 0..85 {
        monitor.record_snapshot(&v_critical, true, 100).unwrap();
    }
    for _ in 0..15 {
        monitor.record_snapshot(&v_critical, false, 0).unwrap();
    }

    // Warning: 93% participation
    for _ in 0..93 {
        monitor.record_snapshot(&v_warning, true, 100).unwrap();
    }
    for _ in 0..7 {
        monitor.record_snapshot(&v_warning, false, 0).unwrap();
    }

    let alerts = monitor.get_active_alerts();

    let critical_alerts: Vec<_> = alerts
        .iter()
        .filter(|a| a.severity == AlertSeverity::Critical)
        .collect();
    let warning_alerts: Vec<_> = alerts
        .iter()
        .filter(|a| a.severity == AlertSeverity::Warning)
        .collect();

    assert!(!critical_alerts.is_empty());
    assert!(!warning_alerts.is_empty());
}
