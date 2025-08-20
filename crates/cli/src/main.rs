use std::{path::PathBuf, time::Duration};

use alloy::signers::local::PrivateKeySigner;
use clap::{ArgAction, Parser};
use color_eyre::eyre::{bail, Context, Result};
use opentelemetry::{trace::TracerProvider, KeyValue};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{MetricExporter, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    logs::SdkLoggerProvider,
    metrics::{PeriodicReader, SdkMeterProvider},
    trace::{Sampler, SdkTracerProvider},
    Resource,
};
use opentelemetry_semantic_conventions::{resource::SERVICE_VERSION, SCHEMA_URL};
use tracing::{info, trace};
use tracing_error::ErrorLayer;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{
    layer::SubscriberExt, registry, util::SubscriberInitExt, EnvFilter, Layer,
};
use workspace_filter::workspace_filter;

mod commands;
mod config;

#[derive(Parser)]
#[command(author, version, about)]
pub(crate) struct Cli {
    /// Path to config file
    #[arg(
        short,
        long,
        global = true,
        display_order(0),
        default_value = "~/.config/nomad/config.toml"
    )]
    pub config: PathBuf,

    /// Ethereum private keys to use
    #[arg(long, global = true, action(ArgAction::Append), display_order(0))]
    pub pk: Option<Vec<String>>,

    /// Increases the level of verbosity. Max value is -vvvv.
    ///
    /// * Default: All crates at info level
    /// * -v     : Nomad crates at debug level, all others at info
    /// * -vv    : Nomad crates at trace level, all others at info
    /// * -vvv   : Nomad crates at trace level, all others at debug
    /// * -vvvv  : All crates at trace level
    #[arg(short, global = true, action = ArgAction::Count, display_order(99))]
    #[clap(verbatim_doc_comment)]
    pub verbose: u8,

    #[command(subcommand)]
    pub cmd: commands::Command,
}

impl Cli {
    /// Run the app
    async fn execute(self) -> Result<()> {
        let config = config::Config::load(&self.config)?;
        self.setup_logging(&config);
        let signers = self.build_signers()?;
        self.cmd.execute(config, signers).await
    }

    /// Build list of signers from the cli arguments
    fn build_signers(&self) -> Result<Vec<PrivateKeySigner>> {
        let Some(accounts) = &self.pk else {
            return Ok(vec![]);
        };
        if accounts.len() < 2 {
            bail!("At least 2 ethereum keys are required");
        }
        accounts
            .iter()
            .map(|s| {
                s.parse::<PrivateKeySigner>()
                    .inspect(|v| {
                        info!("Using Ethereum Account: {}", v.address());
                    })
                    .with_context(|| format!("failed to parse key: {s}"))
            })
            .collect()
    }

    // Setup logging filters and subscriber
    pub fn setup_logging(&self, config: &config::Config) {
        // Setup console logging
        let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| {
            // Default which is directed by the verbosity flag
            match self.verbose {
                0 => "info".into(),
                1 => workspace_filter!("debug", "info,nomad={level}"),
                2 => workspace_filter!("trace", "info,nomad={level}"),
                3 => workspace_filter!("trace", "debug,nomad={level}"),
                _ => "trace".into(),
            }
        });
        let filter = EnvFilter::builder().parse_lossy(filter);
        let env_filter = filter.to_string();
        let console = tracing_subscriber::fmt::layer()
            .with_target(self.verbose > 2)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .compact()
            .with_filter(filter);

        let mut logger = None;
        let mut tracer = None;
        if let Some(url) = &config.otlp.url {
            // Create a Resource that captures information about the entity for which telemetry is recorded.
            let resource = Resource::builder()
                .with_service_name(env!("CARGO_PKG_NAME"))
                .with_schema_url(
                    [KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION"))],
                    SCHEMA_URL,
                )
                .with_attribute(KeyValue::new(
                    "hostname",
                    hostname::get()
                        .unwrap_or("unknown".into())
                        .display()
                        .to_string(),
                ))
                .build();

            if config.otlp.logs {
                let exporter = opentelemetry_otlp::LogExporter::builder()
                    .with_http()
                    .with_headers(config.otlp.headers.clone())
                    .with_endpoint(url.join("v1/logs").unwrap().as_str())
                    .build()
                    .unwrap();
                let provider = SdkLoggerProvider::builder()
                    .with_simple_exporter(exporter)
                    .with_resource(resource.clone())
                    .build();
                logger = Some(
                    OpenTelemetryTracingBridge::new(&provider).with_filter(
                        EnvFilter::builder()
                            .parse_lossy(workspace_filter!("trace", "info,nomad={level}")),
                    ),
                );
            }

            if config.otlp.traces {
                // Setup opentelemetry tracing
                let exporter = SpanExporter::builder()
                    .with_http()
                    .with_headers(config.otlp.headers.clone())
                    .with_endpoint(url.join("v1/traces").unwrap().as_str())
                    .build()
                    .unwrap();
                let provider = SdkTracerProvider::builder()
                    .with_simple_exporter(exporter)
                    .with_sampler(Sampler::AlwaysOn)
                    .with_resource(resource.clone())
                    .build();
                tracer = Some(
                    OpenTelemetryLayer::new(provider.tracer("nomad"))
                        .with_threads(false)
                        .with_location(false)
                        .with_tracked_inactivity(false)
                        .with_error_records_to_exceptions(true)
                        .with_filter(
                            EnvFilter::builder()
                                .parse_lossy(workspace_filter!("trace", "debug,nomad={level}")),
                        ),
                );
            }

            if config.otlp.metrics {
                let exporter = MetricExporter::builder()
                    .with_http()
                    .with_headers(config.otlp.headers.clone())
                    .with_endpoint(url.join("v1/metrics").unwrap().as_str())
                    .build()
                    .unwrap();
                let provider = SdkMeterProvider::builder()
                    .with_reader(
                        PeriodicReader::builder(exporter)
                            .with_interval(Duration::from_secs(10))
                            .build(),
                    )
                    .with_resource(resource)
                    .build();
                opentelemetry::global::set_meter_provider(provider.clone());
            }
        }

        registry()
            .with(ErrorLayer::default())
            .with(console)
            .with(logger)
            .with(tracer)
            .init();

        trace!(env_filter);
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let builder = color_eyre::config::HookBuilder::new()
        .display_env_section(false)
        .display_location_section(false);
    if cli.verbose < 2 {
        builder
            .issue_url(concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new"))
            .add_issue_metadata("version", env!("CARGO_PKG_VERSION"))
            .install()?;
    } else {
        builder.install()?;
    }
    tokio::runtime::Runtime::new()?.block_on(cli.execute())
}
