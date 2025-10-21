use prometheus::{self, Encoder};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::{sync::atomic::AtomicBool, thread};
use tracing::{debug, warn};

pub use prometheus::{
    GaugeVec, Histogram, HistogramVec, IntCounter as Counter, IntCounterVec as CounterVec,
    IntGauge as Gauge,
};

#[derive(Debug)]
pub enum MetricsError {
    ReadStatsError(String),
    ParseStatPartError(String),
    ReadFdError(String),
    SysconfError(String),
}

impl std::fmt::Display for MetricsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetricsError::ReadStatsError(msg) => write!(f, "Failed to read stats: {}", msg),
            MetricsError::ParseStatPartError(msg) => {
                write!(f, "Failed to parse stat part: {}", msg)
            }
            MetricsError::ReadFdError(msg) => write!(f, "Failed to read fd directory: {}", msg),
            MetricsError::SysconfError(msg) => write!(f, "Failed to get sysconf value: {}", msg),
        }
    }
}

impl std::error::Error for MetricsError {}

pub struct Metrics {
    reg: prometheus::Registry,
}

impl Metrics {
    pub fn new() -> Metrics {
        Metrics {
            reg: prometheus::Registry::new(),
        }
    }

    pub fn counter(&self, opts: prometheus::Opts) -> Counter {
        let c = Counter::with_opts(opts).unwrap();
        self.reg.register(Box::new(c.clone())).unwrap();
        c
    }

    pub fn counter_vec(&self, opts: prometheus::Opts, labels: &[&str]) -> CounterVec {
        let c = CounterVec::new(opts, labels).unwrap();
        self.reg.register(Box::new(c.clone())).unwrap();
        c
    }

    pub fn gauge(&self, opts: prometheus::Opts) -> Gauge {
        let g = Gauge::with_opts(opts).unwrap();
        self.reg.register(Box::new(g.clone())).unwrap();
        g
    }

    pub fn gauge_vec(&self, opts: prometheus::Opts, labels: &[&str]) -> GaugeVec {
        let g = GaugeVec::new(opts, labels).unwrap();
        self.reg.register(Box::new(g.clone())).unwrap();
        g
    }

    pub fn histogram(&self, opts: prometheus::HistogramOpts) -> Histogram {
        let h = Histogram::with_opts(opts).unwrap();
        self.reg.register(Box::new(h.clone())).unwrap();
        h
    }

    pub fn histogram_vec(&self, opts: prometheus::HistogramOpts, labels: &[&str]) -> HistogramVec {
        let h = HistogramVec::new(opts, labels).unwrap();
        self.reg.register(Box::new(h.clone())).unwrap();
        h
    }

    pub fn start(&self, shutdown_flag: Arc<AtomicBool>) -> thread::JoinHandle<()> {
        let registry = self.reg.clone();
        let handle = thread::spawn(move || loop {
            if shutdown_flag.load(Ordering::Relaxed) {
                break;
            }

            let mut buffer = Vec::new();
            let encoder = prometheus::TextEncoder::new();
            if let Err(e) = encoder.encode(&registry.gather(), &mut buffer) {
                warn!("failed to encode metrics: {}", e);
            } else {
                // Compute average latency per method
                let mut method_avg_latencies = Vec::new();
                for metric_family in registry.gather() {
                    if metric_family.get_name() == "indexer_latency" {
                        for metric in metric_family.get_metric() {
                            let hist = metric.get_histogram();
                            let count = hist.get_sample_count();
                            let sum = hist.get_sample_sum();
                            if count > 0 {
                                let avg_latency_ms = (sum / count as f64) * 1000.0;
                                if let Some(label_pair) = metric
                                    .get_label()
                                    .iter()
                                    .find(|lp| lp.get_name() == "method")
                                {
                                    method_avg_latencies
                                        .push((label_pair.get_value().to_string(), avg_latency_ms));
                                }
                            }
                        }
                    }
                }
                for (method, avg) in method_avg_latencies {
                    debug!("Average Latency for {}: {:.3} ms", method, avg);
                }
            }

            thread::sleep(Duration::from_secs(5));
        });

        handle
    }
}
