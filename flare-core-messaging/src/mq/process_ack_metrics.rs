use std::sync::OnceLock;
use std::time::Duration;

use prometheus::{HistogramOpts, HistogramVec, IntCounterVec, Opts};

static PROCESS_ACK_METRICS: OnceLock<ProcessAckMetrics> = OnceLock::new();

#[derive(Clone)]
pub(crate) struct ProcessAckMetrics {
    total: IntCounterVec,
    duration_seconds: HistogramVec,
}

impl ProcessAckMetrics {
    fn new() -> Self {
        let total = IntCounterVec::new(
            Opts::new(
                "mq_process_ack_total",
                "Total MQ process ack/nack/term operations by backend, operation, and outcome.",
            ),
            &["backend", "operation", "outcome"],
        )
        .expect("process ack total metric is valid");

        let duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "mq_process_ack_duration_seconds",
                "MQ process ack/nack/term latency by backend, operation, and outcome.",
            )
            .buckets(vec![
                0.0001, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0,
            ]),
            &["backend", "operation", "outcome"],
        )
        .expect("process ack duration metric is valid");

        Self {
            total,
            duration_seconds,
        }
    }

    fn register_default(&self) {
        let registry = prometheus::default_registry();
        let _ = registry.register(Box::new(self.total.clone()));
        let _ = registry.register(Box::new(self.duration_seconds.clone()));
    }

    pub(crate) fn record(
        &self,
        backend: &'static str,
        operation: &'static str,
        outcome: &'static str,
        duration: Duration,
    ) {
        let labels = &[backend, operation, outcome];
        self.total.with_label_values(labels).inc();
        self.duration_seconds
            .with_label_values(labels)
            .observe(duration.as_secs_f64());
    }
}

fn metrics() -> &'static ProcessAckMetrics {
    PROCESS_ACK_METRICS.get_or_init(|| {
        let metrics = ProcessAckMetrics::new();
        metrics.register_default();
        metrics
    })
}

pub(crate) fn record_process_ack(
    backend: &'static str,
    operation: &'static str,
    outcome: &'static str,
    duration: Duration,
) {
    metrics().record(backend, operation, outcome, duration);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_ack_metrics_record_counter_and_duration() {
        let metrics = ProcessAckMetrics::new();

        metrics.record("kafka", "ack", "success", Duration::from_millis(3));
        metrics.record("kafka", "ack", "success", Duration::from_millis(7));

        let labels = &["kafka", "ack", "success"];
        assert_eq!(metrics.total.with_label_values(labels).get(), 2);
        assert_eq!(
            metrics
                .duration_seconds
                .with_label_values(labels)
                .get_sample_count(),
            2
        );
        assert!(
            metrics
                .duration_seconds
                .with_label_values(labels)
                .get_sample_sum()
                >= 0.01
        );
    }
}
