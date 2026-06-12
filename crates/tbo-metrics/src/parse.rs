//! A lenient parser for the Prometheus text exposition format.
//!
//! The booth's debug server exposes metrics in the standard Prometheus text
//! format produced by `metrics-exporter-prometheus`: optional `# HELP` /
//! `# TYPE` comment lines followed by one sample per line of the form
//! `name{label="value",…} value [timestamp]`. This module turns that text into
//! a [`MetricSet`] of [`Sample`]s plus the declared [`MetricType`] of each
//! metric family.
//!
//! Parsing is deliberately lenient: blank lines, `# HELP` comments, and any
//! line that cannot be parsed as a sample are skipped rather than producing an
//! error, so a single malformed line never discards an otherwise usable scrape.

use std::collections::HashMap;

/// The metric kind declared by a `# TYPE` comment line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricType {
    /// A monotonically increasing counter (e.g. `booth_network_*_bytes_total`).
    Counter,
    /// An instantaneous gauge (e.g. `booth_cpu_usage_ratio`).
    Gauge,
    /// A histogram, exposed as `_bucket`/`_sum`/`_count` samples.
    Histogram,
    /// A summary, exposed as quantile/`_sum`/`_count` samples.
    Summary,
    /// A metric whose type was not declared or was unrecognised.
    Untyped,
}

/// A single parsed metric sample: a metric name, its label set, and a value.
///
/// Labels are preserved in the order they were written. Histogram and summary
/// component series keep their conventional suffixes (`_bucket`, `_sum`,
/// `_count`) as part of [`Sample::name`].
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// The metric name, including any `_bucket`/`_sum`/`_count` suffix.
    pub name: String,
    /// The label set, in the order it appeared on the wire.
    pub labels: Vec<(String, String)>,
    /// The sample value. May be infinite or `NaN`.
    pub value: f64,
}

impl Sample {
    /// Returns the value of the label named `key`, if present.
    pub fn label(&self, key: &str) -> Option<&str> {
        self.labels
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// A parsed Prometheus exposition: every [`Sample`] plus declared metric types.
#[derive(Debug, Clone, Default)]
pub struct MetricSet {
    samples: Vec<Sample>,
    types: HashMap<String, MetricType>,
}

impl MetricSet {
    /// Parses a Prometheus text exposition into a [`MetricSet`].
    ///
    /// Unparseable lines are skipped. `# TYPE` declarations are recorded;
    /// `# HELP` and other comments are ignored.
    pub fn parse(text: &str) -> Self {
        let mut samples = Vec::new();
        let mut types = HashMap::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(comment) = line.strip_prefix('#') {
                if let Some((name, kind)) = parse_type(comment) {
                    types.insert(name, kind);
                }
                continue;
            }
            if let Some(sample) = parse_sample(line) {
                samples.push(sample);
            }
        }
        Self { samples, types }
    }

    /// All parsed samples, in source order.
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// The declared [`MetricType`] for `name`, if a `# TYPE` line was seen.
    pub fn metric_type(&self, name: &str) -> Option<MetricType> {
        self.types.get(name).copied()
    }

    /// Iterates every sample whose metric name equals `name`.
    pub fn series<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Sample> + 'a {
        self.samples.iter().filter(move |s| s.name == name)
    }

    /// The value of the first sample named `name`, ignoring labels.
    ///
    /// Convenient for unique gauges such as `booth_cpu_usage_ratio`.
    pub fn value(&self, name: &str) -> Option<f64> {
        self.series(name).next().map(|s| s.value)
    }

    /// The value of the first sample named `name` whose labels include every
    /// `(key, value)` pair in `match_labels`.
    pub fn value_where(&self, name: &str, match_labels: &[(&str, &str)]) -> Option<f64> {
        self.samples
            .iter()
            .find(|s| s.name == name && match_labels.iter().all(|(k, v)| s.label(k) == Some(*v)))
            .map(|s| s.value)
    }
}

/// Parses the body of a `# TYPE name kind` comment (the text after `#`).
fn parse_type(comment: &str) -> Option<(String, MetricType)> {
    let mut tokens = comment.split_whitespace();
    if tokens.next()? != "TYPE" {
        return None;
    }
    let name = tokens.next()?.to_owned();
    let kind = match tokens.next()? {
        "counter" => MetricType::Counter,
        "gauge" => MetricType::Gauge,
        "histogram" => MetricType::Histogram,
        "summary" => MetricType::Summary,
        _ => MetricType::Untyped,
    };
    Some((name, kind))
}

/// Parses a single sample line, returning `None` if it is malformed.
fn parse_sample(line: &str) -> Option<Sample> {
    let name_end = line.find(|c: char| !is_name_char(c)).unwrap_or(line.len());
    if name_end == 0 {
        return None;
    }
    let name = line[..name_end].to_owned();
    let after_name = &line[name_end..];

    let (labels, after_labels) = if let Some(inner) = after_name.strip_prefix('{') {
        scan_labels(inner)?
    } else {
        (Vec::new(), after_name)
    };

    let value = parse_value(after_labels)?;
    Some(Sample {
        name,
        labels,
        value,
    })
}

/// Returns `true` if `c` is valid within a Prometheus metric or label name.
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == ':'
}

/// Scans a label set starting just after the opening `{`.
///
/// Returns the parsed labels and the remainder of the line after the closing
/// `}`, or `None` if the label set is malformed.
fn scan_labels(input: &str) -> Option<(Vec<(String, String)>, &str)> {
    let mut labels = Vec::new();
    let mut rest = input.trim_start();
    if let Some(after) = rest.strip_prefix('}') {
        return Some((labels, after));
    }
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let key = rest[..eq].trim().to_owned();
        if key.is_empty() {
            return None;
        }
        rest = rest[eq + 1..].trim_start();
        let quoted = rest.strip_prefix('"')?;
        let (value, after_quote) = scan_quoted(quoted)?;
        labels.push((key, value));
        rest = after_quote.trim_start();
        if let Some(after) = rest.strip_prefix(',') {
            rest = after.trim_start();
            if let Some(after_brace) = rest.strip_prefix('}') {
                return Some((labels, after_brace));
            }
            continue;
        }
        if let Some(after_brace) = rest.strip_prefix('}') {
            return Some((labels, after_brace));
        }
        return None;
    }
}

/// Reads a double-quoted, backslash-escaped label value.
///
/// `input` must start immediately after the opening quote. Returns the
/// unescaped value and the remainder of the line after the closing quote.
fn scan_quoted(input: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = input.char_indices();
    while let Some((idx, c)) = chars.next() {
        match c {
            '\\' => {
                let (_, escaped) = chars.next()?;
                out.push(match escaped {
                    'n' => '\n',
                    't' => '\t',
                    other => other,
                });
            }
            '"' => {
                let rest = &input[idx + c.len_utf8()..];
                return Some((out, rest));
            }
            other => out.push(other),
        }
    }
    None
}

/// Parses the value (and optional ignored timestamp) following a sample's name
/// and labels.
fn parse_value(rest: &str) -> Option<f64> {
    let token = rest.split_whitespace().next()?;
    match token {
        "+Inf" | "Inf" => Some(f64::INFINITY),
        "-Inf" => Some(f64::NEG_INFINITY),
        "NaN" | "nan" | "NAN" => Some(f64::NAN),
        other => other.parse::<f64>().ok(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;

    const SAMPLE: &str = r#"
# HELP booth_cpu_usage_ratio Fraction of CPU in use.
# TYPE booth_cpu_usage_ratio gauge
booth_cpu_usage_ratio{booth_id="test-booth"} 0.25
# TYPE booth_load_average gauge
booth_load_average{booth_id="test-booth",window="1m"} 0.5
booth_load_average{booth_id="test-booth",window="5m"} 0.75
booth_load_average{booth_id="test-booth",window="15m"} 1
# TYPE booth_network_receive_bytes_total counter
booth_network_receive_bytes_total{booth_id="test-booth",iface="eth0"} 1024 1700000000000
# TYPE booth_uptime_seconds gauge
booth_uptime_seconds{booth_id="test-booth"} +Inf
empty_labels{} 7
"#;

    #[test]
    fn parses_gauge_value_and_type() {
        let set = MetricSet::parse(SAMPLE);
        assert_eq!(set.value("booth_cpu_usage_ratio"), Some(0.25));
        assert_eq!(
            set.metric_type("booth_cpu_usage_ratio"),
            Some(MetricType::Gauge)
        );
        assert_eq!(
            set.metric_type("booth_network_receive_bytes_total"),
            Some(MetricType::Counter)
        );
    }

    #[test]
    fn selects_sample_by_label() {
        let set = MetricSet::parse(SAMPLE);
        assert_eq!(
            set.value_where("booth_load_average", &[("window", "5m")]),
            Some(0.75)
        );
        assert_eq!(
            set.value_where("booth_load_average", &[("window", "15m")]),
            Some(1.0)
        );
        assert_eq!(
            set.value_where("booth_load_average", &[("window", "30m")]),
            None
        );
    }

    #[test]
    fn label_lookup_and_ordering() {
        let set = MetricSet::parse(SAMPLE);
        let net = set
            .series("booth_network_receive_bytes_total")
            .next()
            .unwrap();
        assert_eq!(net.label("iface"), Some("eth0"));
        assert_eq!(net.label("booth_id"), Some("test-booth"));
        assert_eq!(net.label("missing"), None);
        assert_eq!(net.value, 1024.0);
        assert_eq!(
            net.labels.first().map(|(k, _)| k.as_str()),
            Some("booth_id")
        );
    }

    #[test]
    fn ignores_trailing_timestamp() {
        let set = MetricSet::parse("metric_with_ts 42 1700000000000");
        assert_eq!(set.value("metric_with_ts"), Some(42.0));
    }

    #[test]
    fn handles_empty_label_set() {
        let set = MetricSet::parse(SAMPLE);
        assert_eq!(set.value("empty_labels"), Some(7.0));
    }

    #[test]
    fn handles_special_float_values() {
        let set = MetricSet::parse(SAMPLE);
        assert_eq!(set.value("booth_uptime_seconds"), Some(f64::INFINITY));

        let neg = MetricSet::parse("m -Inf");
        assert_eq!(neg.value("m"), Some(f64::NEG_INFINITY));

        let nan = MetricSet::parse("m NaN");
        assert!(nan.value("m").unwrap().is_nan());
    }

    #[test]
    fn unescapes_label_values() {
        let set = MetricSet::parse(r#"m{path="a\"b\\c\nd"} 1"#);
        let sample = set.series("m").next().unwrap();
        assert_eq!(sample.label("path"), Some("a\"b\\c\nd"));
    }

    #[test]
    fn skips_malformed_and_comment_lines() {
        let text = "\
# HELP only a comment
not a valid sample line {{{
good_metric 3
also bad =
";
        let set = MetricSet::parse(text);
        assert_eq!(set.value("good_metric"), Some(3.0));
        // Only the single well-formed sample should survive.
        assert_eq!(set.samples().len(), 1);
    }

    #[test]
    fn parses_multiple_label_values() {
        let set = MetricSet::parse(r#"m{a="1",b="2",c="3"} 9"#);
        let sample = set.series("m").next().unwrap();
        assert_eq!(sample.label("a"), Some("1"));
        assert_eq!(sample.label("b"), Some("2"));
        assert_eq!(sample.label("c"), Some("3"));
        assert_eq!(sample.labels.len(), 3);
    }
}
