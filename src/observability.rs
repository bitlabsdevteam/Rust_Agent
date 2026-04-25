use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use opentelemetry::trace::{self, Span, Tracer, TracerProvider};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::trace::{SdkTracer, SdkTracerProvider};
use opentelemetry_sdk::Resource;
use std::borrow::Cow;
use std::collections::HashMap;
use std::env;
use std::time::Duration;

const DEFAULT_SERVICE_NAME: &str = "agent_in_rust";
const DEFAULT_LANGSMITH_ENDPOINT: &str = "https://api.smith.langchain.com";
const DEFAULT_LANGFUSE_BASE_URL: &str = "https://cloud.langfuse.com";
const LANGFUSE_INGESTION_VERSION: &str = "4";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservabilityTarget {
    label: &'static str,
    endpoint: String,
    headers: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservabilityConfig {
    service_name: String,
    targets: Vec<ObservabilityTarget>,
    warnings: Vec<String>,
}

pub struct Observability {
    tracer: Option<SdkTracer>,
    provider: Option<SdkTracerProvider>,
    enabled_targets: Vec<&'static str>,
    warnings: Vec<String>,
}

impl Observability {
    pub fn from_env() -> Self {
        let config = ObservabilityConfig::from_lookup(|key| env::var(key).ok());
        match build_provider(&config) {
            Ok((provider, tracer)) => Self {
                tracer: Some(tracer),
                provider: Some(provider),
                enabled_targets: config.targets.iter().map(|target| target.label).collect(),
                warnings: config.warnings,
            },
            Err(error) => {
                let mut warnings = config.warnings;
                warnings.push(format!("observability exporter setup failed: {error}"));
                Self {
                    tracer: None,
                    provider: None,
                    enabled_targets: Vec::new(),
                    warnings,
                }
            }
        }
    }

    pub fn enabled_targets(&self) -> &[&'static str] {
        &self.enabled_targets
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn is_enabled(&self) -> bool {
        self.tracer.is_some()
    }

    pub fn with_span<T, F>(
        &self,
        name: impl Into<Cow<'static, str>>,
        attributes: Vec<KeyValue>,
        f: F,
    ) -> T
    where
        F: FnOnce() -> T,
    {
        let Some(tracer) = &self.tracer else {
            return f();
        };

        let mut span = tracer.start(name);
        span.set_attributes(attributes);
        let _guard = trace::mark_span_as_active(span);
        f()
    }

    pub fn record_event(name: impl Into<Cow<'static, str>>, attributes: Vec<KeyValue>) {
        trace::get_active_span(|span| span.add_event(name.into(), attributes));
    }

    pub fn record_log(level: &str, component: &str, message: &str) {
        Self::record_event(
            "runtime.log",
            vec![
                KeyValue::new("log.level", level.to_string()),
                KeyValue::new("log.component", component.to_string()),
                KeyValue::new("log.message", compact_text(message, 2_000)),
            ],
        );
    }
}

impl Drop for Observability {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            let _ = provider.shutdown();
        }
    }
}

impl ObservabilityConfig {
    fn from_lookup<F>(mut lookup: F) -> Self
    where
        F: FnMut(&str) -> Option<String>,
    {
        let service_name = lookup("OTEL_SERVICE_NAME")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_string());
        let mut targets = Vec::new();
        let mut warnings = Vec::new();

        if langsmith_enabled(&mut lookup) {
            match build_langsmith_target(&mut lookup) {
                Ok(Some(target)) => targets.push(target),
                Ok(None) => warnings.push(
                    "LANGSMITH_OTEL_ENABLED was set but LANGSMITH_API_KEY was missing.".to_string(),
                ),
                Err(error) => warnings.push(error),
            }
        }

        if langfuse_enabled(&mut lookup) {
            match build_langfuse_target(&mut lookup) {
                Ok(Some(target)) => targets.push(target),
                Ok(None) => warnings.push(
                    "LANGFUSE_TRACING_ENABLED was set but LANGFUSE_PUBLIC_KEY or LANGFUSE_SECRET_KEY was missing.".to_string(),
                ),
                Err(error) => warnings.push(error),
            }
        }

        Self {
            service_name,
            targets,
            warnings,
        }
    }
}

fn build_provider(config: &ObservabilityConfig) -> Result<(SdkTracerProvider, SdkTracer), String> {
    if config.targets.is_empty() {
        return Err("no LangSmith or Langfuse exporters were configured".to_string());
    }

    let resource = Resource::builder_empty()
        .with_attributes([
            KeyValue::new("service.name", config.service_name.clone()),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION").to_string()),
            KeyValue::new("telemetry.sdk.language", "rust"),
            KeyValue::new(
                "telemetry.exporters",
                config
                    .targets
                    .iter()
                    .map(|target| target.label)
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        ])
        .build();

    let mut builder = SdkTracerProvider::builder().with_resource(resource);
    for target in &config.targets {
        let exporter = SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_timeout(Duration::from_secs(10))
            .with_endpoint(target.endpoint.clone())
            .with_headers(target.headers.clone())
            .build()
            .map_err(|error| format!("{} exporter build failed: {error}", target.label))?;
        builder = builder.with_simple_exporter(exporter);
    }

    let provider = builder.build();
    let tracer = provider.tracer(config.service_name.clone());
    Ok((provider, tracer))
}

fn langsmith_enabled<F>(lookup: &mut F) -> bool
where
    F: FnMut(&str) -> Option<String>,
{
    env_flag(lookup, "LANGSMITH_OTEL_ENABLED")
        || (env_flag(lookup, "LANGSMITH_TRACING") && lookup("LANGSMITH_API_KEY").is_some())
}

fn langfuse_enabled<F>(lookup: &mut F) -> bool
where
    F: FnMut(&str) -> Option<String>,
{
    env_flag(lookup, "LANGFUSE_TRACING_ENABLED")
        || env_flag(lookup, "LANGFUSE_OTEL_ENABLED")
        || lookup("LANGFUSE_PUBLIC_KEY").is_some()
        || lookup("LANGFUSE_SECRET_KEY").is_some()
}

fn build_langsmith_target<F>(lookup: &mut F) -> Result<Option<ObservabilityTarget>, String>
where
    F: FnMut(&str) -> Option<String>,
{
    let Some(api_key) = lookup("LANGSMITH_API_KEY")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    let base = lookup("LANGSMITH_ENDPOINT")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LANGSMITH_ENDPOINT.to_string());
    let endpoint = format!("{}/otel/v1/traces", base.trim_end_matches('/'));
    let mut headers = HashMap::from([("x-api-key".to_string(), api_key)]);
    if let Some(project) = lookup("LANGSMITH_PROJECT")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        headers.insert("Langsmith-Project".to_string(), project);
    }

    Ok(Some(ObservabilityTarget {
        label: "langsmith",
        endpoint,
        headers,
    }))
}

fn build_langfuse_target<F>(lookup: &mut F) -> Result<Option<ObservabilityTarget>, String>
where
    F: FnMut(&str) -> Option<String>,
{
    let public_key = lookup("LANGFUSE_PUBLIC_KEY")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let secret_key = lookup("LANGFUSE_SECRET_KEY")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let (Some(public_key), Some(secret_key)) = (public_key, secret_key) else {
        return Ok(None);
    };

    let base = lookup("LANGFUSE_BASE_URL")
        .or_else(|| lookup("LANGFUSE_HOST"))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_LANGFUSE_BASE_URL.to_string());
    let endpoint = format!("{}/api/public/otel/v1/traces", base.trim_end_matches('/'));
    let auth = BASE64_STANDARD.encode(format!("{public_key}:{secret_key}"));
    let headers = HashMap::from([
        ("Authorization".to_string(), format!("Basic {auth}")),
        (
            "x-langfuse-ingestion-version".to_string(),
            LANGFUSE_INGESTION_VERSION.to_string(),
        ),
    ]);

    Ok(Some(ObservabilityTarget {
        label: "langfuse",
        endpoint,
        headers,
    }))
}

fn env_flag<F>(lookup: &mut F, key: &str) -> bool
where
    F: FnMut(&str) -> Option<String>,
{
    lookup(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn compact_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let compact = trimmed.chars().take(max_chars).collect::<String>();
        format!("{compact}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(entries: &[(&str, &str)], key: &str) -> Option<String> {
        entries
            .iter()
            .find_map(|(entry_key, value)| (*entry_key == key).then_some((*value).to_string()))
    }

    #[test]
    fn builds_langsmith_target_with_project_header() {
        let config = ObservabilityConfig::from_lookup(|key| {
            lookup(
                &[
                    ("LANGSMITH_OTEL_ENABLED", "true"),
                    ("LANGSMITH_API_KEY", "ls-key"),
                    ("LANGSMITH_PROJECT", "starter"),
                ],
                key,
            )
        });

        assert_eq!(config.targets.len(), 1);
        assert_eq!(config.targets[0].label, "langsmith");
        assert_eq!(
            config.targets[0].endpoint,
            "https://api.smith.langchain.com/otel/v1/traces"
        );
        assert_eq!(
            config.targets[0].headers.get("Langsmith-Project"),
            Some(&"starter".to_string())
        );
    }

    #[test]
    fn builds_langfuse_target_with_basic_auth_header() {
        let config = ObservabilityConfig::from_lookup(|key| {
            lookup(
                &[
                    ("LANGFUSE_TRACING_ENABLED", "true"),
                    ("LANGFUSE_PUBLIC_KEY", "pk"),
                    ("LANGFUSE_SECRET_KEY", "sk"),
                    ("LANGFUSE_BASE_URL", "https://example.langfuse.com"),
                ],
                key,
            )
        });

        assert_eq!(config.targets.len(), 1);
        assert_eq!(config.targets[0].label, "langfuse");
        assert_eq!(
            config.targets[0].endpoint,
            "https://example.langfuse.com/api/public/otel/v1/traces"
        );
        assert_eq!(
            config.targets[0]
                .headers
                .get("x-langfuse-ingestion-version"),
            Some(&LANGFUSE_INGESTION_VERSION.to_string())
        );
        assert_eq!(
            config.targets[0].headers.get("Authorization"),
            Some(&format!("Basic {}", BASE64_STANDARD.encode("pk:sk")))
        );
    }

    #[test]
    fn warns_when_langsmith_is_enabled_without_api_key() {
        let config = ObservabilityConfig::from_lookup(|key| {
            lookup(&[("LANGSMITH_OTEL_ENABLED", "true")], key)
        });

        assert!(config.targets.is_empty());
        assert_eq!(config.warnings.len(), 1);
        assert!(config.warnings[0].contains("LANGSMITH_OTEL_ENABLED"));
    }

    #[test]
    fn compact_text_truncates_long_values() {
        let compact = compact_text("abcdef", 3);
        assert_eq!(compact, "abc...");
    }
}
