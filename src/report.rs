use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use clap::Parser;

use crate::args::Args;

static REPORT: OnceLock<Mutex<Vec<ReportRecord>>> = OnceLock::new();

fn get_report() -> &'static Mutex<Vec<ReportRecord>> {
    REPORT.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn insert_record(task: String, host: String, status: TaskStatus) {
    let record = ReportRecord { task, host, status };
    let report = get_report();
    report
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(record);
}

pub fn generate_report() {
    let report = get_report()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if report.is_empty() {
        return;
    }
    let width = 80;
    let col2_width = 8;
    let col1_width = width - col2_width - 2;
    println!();
    println!("{:=^width$}", " Komando Report ");
    if Args::parse().flags.dry_run {
        println!("{:-^width$}", " Dry-run mode: no changes were made ");
    }
    println!("{:<col1_width$}{:>col2_width$}", "Task on Host", "Status");
    println!("{:-<width$}", "");
    let mut counters = HashMap::new();
    counters.insert(TaskStatus::OK, 0);
    counters.insert(TaskStatus::Changed, 0);
    counters.insert(TaskStatus::Failed, 0);
    let mut last_task = String::new();
    for record in &*report {
        if last_task != record.task {
            println!(
                "{}",
                format!("* {}", record.task)
                    .chars()
                    .take(width)
                    .collect::<String>()
            );
        }
        let col1_width = col1_width - 3;
        println!("  - {:<col1_width$} {}", record.host, record.status);
        last_task.clone_from(&record.task);
        if let Some(counter) = counters.get_mut(&record.status) {
            *counter += 1;
        }
    }
    println!("{:-<width$}", "");
    println!(
        "OK: {}, Changed: {}, Failed: {}",
        counters[&TaskStatus::OK],
        counters[&TaskStatus::Changed],
        counters[&TaskStatus::Failed]
    );
}

#[derive(Debug, Clone)]
struct ReportRecord {
    task: String,
    host: String,
    status: TaskStatus,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum TaskStatus {
    OK,
    Changed,
    Failed,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OK => write!(f, "OK"),
            Self::Changed => write!(f, "Changed"),
            Self::Failed => write!(f, "Failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_record() {
        insert_record("task1".to_string(), "host1".to_string(), TaskStatus::OK);
        insert_record(
            "task1".to_string(),
            "host2".to_string(),
            TaskStatus::Changed,
        );
        insert_record("task2".to_string(), "host1".to_string(), TaskStatus::Failed);

        let report = {
            let guard = get_report()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.clone()
        };
        assert_eq!(report.len(), 3);
        assert_eq!(report[0].task, "task1");
        assert_eq!(report[0].host, "host1");
        assert_eq!(report[0].status, TaskStatus::OK);
        assert_eq!(report[1].task, "task1");
        assert_eq!(report[1].host, "host2");
        assert_eq!(report[1].status, TaskStatus::Changed);
        assert_eq!(report[2].task, "task2");
        assert_eq!(report[2].host, "host1");
        assert_eq!(report[2].status, TaskStatus::Failed);
    }
}
