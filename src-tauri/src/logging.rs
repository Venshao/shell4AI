use std::path::Path;

pub fn init(data_dir: &Path) {
    let log_dir = data_dir.join("logs");
    if std::fs::create_dir_all(&log_dir).is_err() {
        return;
    }
    let appender = tracing_appender::rolling::daily(log_dir, "ssh-mcp.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(true)
        .with_writer(writer)
        .try_init();
    std::mem::forget(guard);
}
