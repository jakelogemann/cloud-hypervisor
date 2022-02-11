// Custom harness to run performance tests
#[macro_use]
extern crate lazy_static;
extern crate test_infra;

mod performance_tests;

use performance_tests::*;
use serde_derive::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env, fmt,
    hash::{Hash, Hasher},
    sync::mpsc::channel,
    thread,
    time::Duration,
};

#[derive(Debug)]
enum Error {
    TestTimeout,
    TestFailed,
}

#[derive(Deserialize, Serialize)]
pub struct PerformanceTestResult {
    name: String,
    mean: f64,
    std_dev: f64,
    max: f64,
    min: f64,
}

#[derive(Deserialize, Serialize)]
pub struct MetricsReport {
    pub git_human_readable: String,
    pub git_revision: String,
    pub date: String,
    pub results: Vec<PerformanceTestResult>,
}

pub struct PerformanceTestControl {
    test_time: u32,
    test_iterations: u32,
    queue_num: Option<u32>,
    queue_size: Option<u32>,
    net_rx: Option<bool>,
    fio_ops: Option<FioOps>,
}

impl fmt::Display for PerformanceTestControl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut output = format!(
            "test_time = {}s, test_iterations = {}",
            self.test_time, self.test_iterations
        );
        if let Some(o) = self.queue_num {
            output = format!("{}, queue_num = {}", output, o);
        }
        if let Some(o) = self.queue_size {
            output = format!("{}, queue_size = {}", output, o);
        }
        if let Some(o) = self.net_rx {
            output = format!("{}, net_rx = {}", output, o);
        }
        if let Some(o) = &self.fio_ops {
            output = format!("{}, fio_ops = {}", output, o);
        }

        write!(f, "{}", output)
    }
}

impl Default for PerformanceTestControl {
    fn default() -> Self {
        Self {
            test_time: 10,
            test_iterations: 30,
            queue_num: Default::default(),
            queue_size: Default::default(),
            net_rx: Default::default(),
            fio_ops: Default::default(),
        }
    }
}

/// A performance test should finish within the a certain time-out and
/// return a performance metrics number (including the average number and
/// standard deviation)
struct PerformanceTest {
    pub name: &'static str,
    pub func_ptr: fn(&PerformanceTestControl) -> f64,
    pub control: PerformanceTestControl,
}

impl Hash for PerformanceTest {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl PartialEq for PerformanceTest {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for PerformanceTest {}

impl PerformanceTest {
    pub fn run(&self) -> PerformanceTestResult {
        let mut metrics = Vec::new();
        for _ in 0..self.control.test_iterations {
            metrics.push((self.func_ptr)(&self.control));
        }

        let mean = mean(&metrics).unwrap();
        let std_dev = std_deviation(&metrics).unwrap();
        let max = metrics.clone().into_iter().reduce(f64::max).unwrap();
        let min = metrics.clone().into_iter().reduce(f64::min).unwrap();

        PerformanceTestResult {
            name: self.name.to_string(),
            mean,
            std_dev,
            max,
            min,
        }
    }

    // Calculate the timeout for each test
    // Note: To cover the setup/cleanup time, 20s is added for each iteration of the test
    pub fn calc_timeout(&self) -> u64 {
        ((self.control.test_time + 20) * self.control.test_iterations) as u64
    }
}

fn mean(data: &[f64]) -> Option<f64> {
    let count = data.len();

    if count > 0 {
        Some(data.iter().sum::<f64>() / count as f64)
    } else {
        None
    }
}

fn std_deviation(data: &[f64]) -> Option<f64> {
    let count = data.len();

    if count > 0 {
        let mean = mean(data).unwrap();
        let variance = data
            .iter()
            .map(|value| {
                let diff = mean - *value;
                diff * diff
            })
            .sum::<f64>()
            / count as f64;

        Some(variance.sqrt())
    } else {
        None
    }
}

lazy_static! {
    static ref TEST_LIST: HashSet<PerformanceTest> = {
        let mut m = HashSet::new();
        m.insert(PerformanceTest {
            name: "performance_boot_time",
            func_ptr: performance_boot_time,
            control: PerformanceTestControl {
                test_time: 2,
                test_iterations: 10,
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_boot_time_pmem",
            func_ptr: performance_boot_time_pmem,
            control: PerformanceTestControl {
                test_time: 2,
                test_iterations: 10,
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_virtio_net_latency",
            func_ptr: performance_net_latency,
            control: Default::default(),
        });
        m.insert(PerformanceTest {
            name: "performance_virtio_net_throughput_bps_single_queue_rx",
            func_ptr: performance_net_throughput,
            control: PerformanceTestControl {
                queue_num: Some(1), // used as 'queue_pairs'
                queue_size: Some(256),
                net_rx: Some(true),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_virtio_net_throughput_bps_single_queue_tx",
            func_ptr: performance_net_throughput,
            control: PerformanceTestControl {
                queue_num: Some(1), // used as 'queue_pairs'
                queue_size: Some(256),
                net_rx: Some(false),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_virtio_net_throughput_bps_multi_queue_rx",
            func_ptr: performance_net_throughput,
            control: PerformanceTestControl {
                queue_num: Some(2), // used as 'queue_pairs'
                queue_size: Some(1024),
                net_rx: Some(true),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_virtio_net_throughput_bps_multi_queue_tx",
            func_ptr: performance_net_throughput,
            control: PerformanceTestControl {
                queue_num: Some(2), // used as 'queue_pairs'
                queue_size: Some(1024),
                net_rx: Some(false),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_block_io_bps_read",
            func_ptr: performance_block_io,
            control: PerformanceTestControl {
                queue_num: Some(1),
                queue_size: Some(1024),
                fio_ops: Some(FioOps::Read),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_block_io_bps_write",
            func_ptr: performance_block_io,
            control: PerformanceTestControl {
                queue_num: Some(1),
                queue_size: Some(1024),
                fio_ops: Some(FioOps::Write),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_block_io_bps_random_read",
            func_ptr: performance_block_io,
            control: PerformanceTestControl {
                queue_num: Some(1),
                queue_size: Some(1024),
                fio_ops: Some(FioOps::RandomRead),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_block_io_bps_random_write",
            func_ptr: performance_block_io,
            control: PerformanceTestControl {
                queue_num: Some(1),
                queue_size: Some(1024),
                fio_ops: Some(FioOps::RandomWrite),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_block_io_bps_multi_queue_read",
            func_ptr: performance_block_io,
            control: PerformanceTestControl {
                queue_num: Some(2),
                queue_size: Some(1024),
                fio_ops: Some(FioOps::Read),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_block_io_bps_multi_queue_write",
            func_ptr: performance_block_io,
            control: PerformanceTestControl {
                queue_num: Some(2),
                queue_size: Some(1024),
                fio_ops: Some(FioOps::Write),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_block_io_bps_multi_queue_random_read",
            func_ptr: performance_block_io,
            control: PerformanceTestControl {
                queue_num: Some(2),
                queue_size: Some(1024),
                fio_ops: Some(FioOps::RandomRead),
                ..Default::default()
            }
        });
        m.insert(PerformanceTest {
            name: "performance_block_io_bps_multi_queue_random_write",
            func_ptr: performance_block_io,
            control: PerformanceTestControl {
                queue_num: Some(2),
                queue_size: Some(1024),
                fio_ops: Some(FioOps::RandomWrite),
                ..Default::default()
            }
        });
        m
    };
}

fn run_test_with_timetout(test: &'static PerformanceTest) -> Result<PerformanceTestResult, Error> {
    let (sender, receiver) = channel::<Result<PerformanceTestResult, Error>>();
    thread::spawn(move || {
        println!("Test '{}' running .. ({})", test.name, test.control);

        let output = match std::panic::catch_unwind(|| test.run()) {
            Ok(test_result) => {
                println!(
                    "Test '{}' .. ok: mean = {}, std_dev = {}",
                    test_result.name, test_result.mean, test_result.std_dev
                );
                Ok(test_result)
            }
            Err(_) => Err(Error::TestFailed),
        };

        let _ = sender.send(output);
    });

    // Todo: Need to cleanup/kill all hanging child processes
    let test_timeout = test.calc_timeout();
    receiver
        .recv_timeout(Duration::from_secs(test_timeout))
        .map_err(|_| {
            eprintln!(
                "[Error] Test '{}' time-out after {} seconds",
                test.name, test_timeout
            );
            Error::TestTimeout
        })?
}

fn date() -> String {
    let output = test_infra::exec_host_command_output("date");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn main() {
    let test_filter = env::var("TEST_FILTER").map_or("".to_string(), |o| o);

    // Run performance tests sequentially and report results (in both readable/json format)
    let mut metrics_report = MetricsReport {
        git_human_readable: env!("GIT_HUMAN_READABLE").to_string(),
        git_revision: env!("GIT_REVISION").to_string(),
        date: date(),
        results: Vec::new(),
    };

    init_tests();

    for test in TEST_LIST.iter() {
        if test.name.contains(&test_filter) {
            match run_test_with_timetout(test) {
                Ok(r) => {
                    metrics_report.results.push(r);
                }
                Err(e) => {
                    eprintln!("Aborting test due to error: '{:?}'", e);
                    break;
                }
            };
        }
    }

    cleanup_tests();

    // Todo: Report/upload to the metrics database
    println!(
        "\n\nTests result in json format: \n {}",
        serde_json::to_string_pretty(&metrics_report).unwrap()
    );
}