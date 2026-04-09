use crate::observability::Observability;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn info(component: &str, message: impl AsRef<str>) {
    Observability::record_log("INFO", component, message.as_ref());
    println!(
        "[{}] [INFO] [{}] {}",
        now_epoch_ms(),
        component,
        message.as_ref()
    );
}

pub fn warn(component: &str, message: impl AsRef<str>) {
    Observability::record_log("WARN", component, message.as_ref());
    eprintln!(
        "[{}] [WARN] [{}] {}",
        now_epoch_ms(),
        component,
        message.as_ref()
    );
}

pub fn error(component: &str, message: impl AsRef<str>) {
    Observability::record_log("ERROR", component, message.as_ref());
    eprintln!(
        "[{}] [ERROR] [{}] {}",
        now_epoch_ms(),
        component,
        message.as_ref()
    );
}
