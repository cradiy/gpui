use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::Limits;

/// Nonfatal source metadata reported without logging or changing import policy.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportDiagnostic {
    /// An unsupported optional extension is ignored in favor of base glTF data.
    IgnoredOptionalExtension {
        extension: String,
        /// JSON Pointer to the payload, or its declaration when no payload is scanned.
        path: String,
    },
}

impl fmt::Display for ImportDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IgnoredOptionalExtension { extension, path } => {
                write!(f, "ignored optional extension {extension:?} at {path:?}")
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct Metadata {
    pub source: Arc<()>,
    pub unsupported_morphs: HashMap<(usize, usize), String>,
    pub diagnostics: Vec<ImportDiagnostic>,
}

pub(crate) fn metadata(bytes: &[u8], limits: Limits) -> Result<Metadata> {
    let json = if bytes.starts_with(b"glTF") {
        gltf::binary::Glb::from_slice(bytes)?.json
    } else {
        std::borrow::Cow::Borrowed(bytes)
    };
    let raw: Value = serde_json::from_slice(&json)?;
    let mut collector = Collector {
        limits,
        text_bytes: 0,
        seen: HashSet::new(),
        diagnostics: Vec::new(),
    };
    collector.visit(&raw, &mut String::new())?;
    for (index, name) in raw["extensionsUsed"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let Some(name) = name.as_str()
            && !crate::validation::supports_extension(name)
            && !collector.seen.contains(name)
        {
            collector.record(name, &format!("/extensionsUsed/{index}"))?;
        }
    }
    collector.diagnostics.sort_by(|a, b| match (a, b) {
        (
            ImportDiagnostic::IgnoredOptionalExtension { path: a, .. },
            ImportDiagnostic::IgnoredOptionalExtension { path: b, .. },
        ) => a.cmp(b),
    });
    Ok(Metadata {
        source: Arc::new(()),
        unsupported_morphs: crate::morph::unsupported_attributes(&raw),
        diagnostics: collector.diagnostics,
    })
}

struct Collector<'a> {
    limits: Limits,
    text_bytes: usize,
    seen: HashSet<&'a str>,
    diagnostics: Vec<ImportDiagnostic>,
}

impl<'a> Collector<'a> {
    fn record(&mut self, extension: &'a str, path: &str) -> Result<()> {
        ensure!(
            self.diagnostics.len() < self.limits.diagnostics,
            "diagnostic count limit exceeded"
        );
        self.text_bytes = self
            .text_bytes
            .checked_add(extension.len())
            .and_then(|bytes| bytes.checked_add(path.len()))
            .context("diagnostic text size overflow")?;
        ensure!(
            self.text_bytes <= self.limits.diagnostic_bytes,
            "diagnostic text byte limit exceeded"
        );
        self.diagnostics
            .push(ImportDiagnostic::IgnoredOptionalExtension {
                extension: extension.to_owned(),
                path: path.to_owned(),
            });
        self.seen.insert(extension);
        Ok(())
    }

    fn visit(&mut self, value: &'a Value, path: &mut String) -> Result<()> {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    if key == "extras" {
                        continue;
                    }
                    let length = path.len();
                    push_segment(path, key);
                    if key == "extensions" {
                        for (name, payload) in value.as_object().into_iter().flatten() {
                            let length = path.len();
                            push_segment(path, name);
                            if crate::validation::supports_extension(name) {
                                self.visit(payload, path)?;
                            } else {
                                self.record(name, path)?;
                            }
                            path.truncate(length);
                        }
                    } else {
                        self.visit(value, path)?;
                    }
                    path.truncate(length);
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    let length = path.len();
                    push_segment(path, &index.to_string());
                    self.visit(value, path)?;
                    path.truncate(length);
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn push_segment(path: &mut String, segment: &str) {
    path.push('/');
    for c in segment.chars() {
        match c {
            '~' => path.push_str("~0"),
            '/' => path.push_str("~1"),
            _ => path.push(c),
        }
    }
}
