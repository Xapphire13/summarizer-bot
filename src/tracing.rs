use tracing_subscriber::{fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init() {
    match tracing_journald::layer() {
        Ok(journald_layer) => tracing_subscriber::registry().with(journald_layer).init(),
        Err(_) => tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::NEW | FmtSpan::CLOSE))
            .init(),
    };
}
