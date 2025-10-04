use std::{net::IpAddr, path::PathBuf};

use alloy::{primitives::Bytes, signers::local::PrivateKeySigner};
use clap::{ArgAction, Parser};
use color_eyre::eyre::{bail, Context, Result};
use opentelemetry::KeyValue;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, MetricExporter, SpanExporter};
use opentelemetry_sdk::{
    logs::SdkLoggerProvider,
    metrics::SdkMeterProvider,
    trace::{Sampler, SdkTracerProvider},
    Resource,
};
use opentelemetry_semantic_conventions::{resource::SERVICE_VERSION, SCHEMA_URL};
use tracing::{info, trace};
use tracing_subscriber::{
    layer::SubscriberExt, registry, util::SubscriberInitExt, EnvFilter, Layer,
};
use workspace_filter::workspace_filter;

use nomad_node::config::Config;

mod commands;

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
    pub pk: Option<Vec<Bytes>>,

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
        let config = Config::load(&self.config)?;
        let (tracer, logger, meter) = self.setup_logging(&config).await?;

        let signers = self.build_signers(&config)?;
        self.cmd.execute(config, signers).await?;

        if let Some(provider) = tracer {
            provider.shutdown()?;
        }
        if let Some(provider) = logger {
            provider.shutdown()?;
        }
        if let Some(meter) = meter {
            meter.shutdown()?;
        }

        Ok(())
    }

    /// Build list of signers from the cli arguments and config
    fn build_signers(&self, config: &Config) -> Result<Vec<PrivateKeySigner>> {
        let keys = if let Some(cli_keys) = &self.pk {
            // If CLI keys are provided, use only those
            cli_keys.clone()
        } else {
            // Otherwise, use config keys
            config.enclave.debug_keys.clone()
        };

        if keys.is_empty() {
            return Ok(vec![]);
        }
        if keys.len() < 2 {
            bail!("At least 2 ethereum keys are required");
        }

        keys.iter()
            .map(|s| {
                s.to_string()
                    .parse::<PrivateKeySigner>()
                    .inspect(|v| {
                        info!("Using Ethereum Account: {}", v.address());
                    })
                    .with_context(|| format!("failed to parse key: {s}"))
            })
            .collect()
    }

    /// Get global ip address
    async fn global_ip(&self) -> Result<Option<IpAddr>> {
        if matches!(self.cmd, commands::Command::Run(_)) {
            if let Ok(res) = reqwest::get("https://ifconfig.me/ip").await {
                if let Ok(remote_ip) = res.text().await {
                    return Ok(Some(remote_ip.parse()?));
                }
            }
        }
        Ok(None)
    }

    // Setup logging filters and subscriber
    pub async fn setup_logging(
        &self,
        config: &Config,
    ) -> Result<(
        Option<SdkTracerProvider>,
        Option<SdkLoggerProvider>,
        Option<SdkMeterProvider>,
    )> {
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

        // fetch ip address if we're running the node or telemetry is enabled
        let ip = self.global_ip().await?;

        let mut log_layer = None;
        let mut logger = None;
        let mut tracer = None;
        let mut meter = None;

        // setup telemetry if enabled
        if config.otlp.logs || config.otlp.metrics || config.otlp.traces {
            // Create a Resource that captures information about the entity for which telemetry is recorded.
            let mut resource = Resource::builder()
                .with_service_name(env!("CARGO_BIN_NAME"))
                .with_schema_url(
                    [KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION"))],
                    SCHEMA_URL,
                )
                .with_attribute(KeyValue::new(
                    "host.name",
                    hostname::get()
                        .unwrap_or("unknown".into())
                        .display()
                        .to_string(),
                ));
            if let Some(ip) = ip {
                resource = resource.with_attribute(KeyValue::new("host.ip", ip.to_string()));
            }
            if let Ok(env) = std::env::var("ENV") {
                resource = resource.with_attribute(KeyValue::new("deployment.environment", env))
            }
            let resource = resource.build();

            if config.otlp.logs {
                let exporter = LogExporter::builder().with_http().build()?;
                let provider = SdkLoggerProvider::builder()
                    .with_batch_exporter(exporter)
                    .with_resource(resource.clone())
                    .build();
                log_layer = Some(
                    OpenTelemetryTracingBridge::new(&provider).with_filter(
                        EnvFilter::builder()
                            .parse_lossy(workspace_filter!("trace", "info,nomad={level}")),
                    ),
                );
                logger = Some(provider);
            }

            if config.otlp.traces {
                // Setup opentelemetry tracing
                let exporter = SpanExporter::builder().with_http().build()?;
                let provider = SdkTracerProvider::builder()
                    .with_batch_exporter(exporter)
                    .with_sampler(Sampler::AlwaysOn)
                    .with_resource(resource.clone())
                    .build();
                opentelemetry::global::set_tracer_provider(provider.clone());
                tracer = Some(provider);
            }

            if config.otlp.metrics {
                // Setup opentelemetry metrics
                let exporter = MetricExporter::builder().with_http().build()?;
                let provider = SdkMeterProvider::builder()
                    .with_periodic_exporter(exporter)
                    .with_resource(resource)
                    .build();
                opentelemetry::global::set_meter_provider(provider.clone());
                meter = Some(provider);
            }
        }

        registry().with(console).with(log_layer).init();
        trace!(env_filter);
        if let Some(ip) = ip {
            info!("Remote Address: {ip}");
        }
        Ok((tracer, logger, meter))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::config::HookBuilder::new()
        .display_env_section(false)
        .display_location_section(false)
        .install()?;
    Cli::parse().execute().await
}
