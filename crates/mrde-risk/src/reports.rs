//! Bounded caller input: parsers do not follow XML entities, links, or filesystem paths.
use crate::config::Command;
use anyhow::{Context, Result, bail, ensure};
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};
fn attr(tag: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>> {
    tag.attributes()
        .find_map(|a| match a {
            Ok(a) if a.key.as_ref() == name => Some(
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map(|s| s.into_owned())
                    .map_err(Into::into),
            ),
            Err(e) => Some(Err(e.into())),
            _ => None,
        })
        .transpose()
}
fn path(value: &str, root: &Path) -> String {
    let value = Path::new(value);
    let value = value.strip_prefix(root).unwrap_or(value);
    value
        .to_string_lossy()
        .trim_start_matches("./")
        .replace('\\', "/")
}
pub fn coverage(text: &str, root: &Path) -> Result<BTreeMap<(String, u32), u64>> {
    let mut result = BTreeMap::new();
    if text.trim_start().starts_with('<') {
        let mut reader = Reader::from_str(text);
        let mut current = None;
        loop {
            match reader.read_event()? {
                Event::Start(t) if t.name().as_ref() == b"class" => {
                    current = attr(&t, b"filename")?.map(|p| path(&p, root))
                }
                Event::Empty(t) | Event::Start(t) if t.name().as_ref() == b"line" => {
                    if let Some(file) = &current {
                        let line = attr(&t, b"number")?
                            .context("Coverage line number missing")?
                            .parse()?;
                        let hits = attr(&t, b"hits")?
                            .context("Coverage hits missing")?
                            .parse()?;
                        result
                            .entry((file.clone(), line))
                            .and_modify(|v| *v = u64::max(*v, hits))
                            .or_insert(hits);
                    }
                }
                Event::End(t) if t.name().as_ref() == b"class" => current = None,
                Event::DocType(_) => bail!("Coverage reports with DTDs are not supported"),
                Event::Eof => break,
                _ => {}
            }
        }
    } else {
        let mut current = None;
        for line in text.lines() {
            if let Some(file) = line.strip_prefix("SF:") {
                current = Some(path(file, root));
            } else if let Some(data) = line.strip_prefix("DA:") {
                let mut values = data.split(',');
                let number = values.next().context("Missing LCOV line")?.parse()?;
                let hits = values.next().context("Missing LCOV count")?.parse()?;
                let file = current.as_ref().context("LCOV DA without SF")?;
                result
                    .entry((file.clone(), number))
                    .and_modify(|v| *v = u64::max(*v, hits))
                    .or_insert(hits);
            } else if line == "end_of_record" {
                current = None;
            }
        }
    }
    ensure!(!result.is_empty(), "Coverage report has no line records");
    Ok(result)
}
pub fn complexity(text: &str, root: &Path) -> Result<BTreeMap<String, f64>> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut result = BTreeMap::new();
    let mut measure = String::new();
    let mut name = None;
    let mut values = Vec::<f64>::new();
    let mut in_value = false;
    loop {
        match reader.read_event()? {
            Event::Start(t) if t.name().as_ref() == b"measure" => {
                measure = attr(&t, b"type")?.unwrap_or_default()
            }
            Event::Start(t) if t.name().as_ref() == b"item" => {
                name = attr(&t, b"name")?;
                values.clear();
            }
            Event::Start(t) if t.name().as_ref() == b"value" => in_value = true,
            Event::Text(t) if in_value => values.push(t.decode()?.parse()?),
            Event::End(t) if t.name().as_ref() == b"value" => in_value = false,
            Event::End(t) if t.name().as_ref() == b"item" => {
                if let Some(name) = name.take() {
                    if measure == "Function" {
                        let location = name
                            .rsplit_once(" at ")
                            .context("Invalid Lizard function location")?
                            .1;
                        let file = location.rsplit_once(':').context("Missing Lizard line")?.0;
                        let ccn = *values.get(2).context("Missing Lizard CCN")?;
                        ensure!(ccn.is_finite() && ccn >= 0., "Invalid CCN");
                        *result.entry(path(file, root)).or_insert(0.) += ccn;
                    } else if measure == "File" {
                        result.entry(path(&name, root)).or_insert(0.);
                    }
                }
            }
            Event::DocType(_) => bail!("Lizard reports with DTDs are not supported"),
            Event::Eof => break,
            _ => {}
        }
    }
    ensure!(
        !result.is_empty(),
        "Lizard reported no supported source files"
    );
    Ok(result)
}
#[derive(Debug, Deserialize)]
pub struct Diagnostic {
    pub path: String,
    #[serde(default)]
    pub line: u32,
    #[serde(default)]
    pub rule: String,
    pub message: String,
}
impl Diagnostic {
    pub fn key(&self) -> (String, String, String) {
        (self.path.clone(), self.rule.clone(), self.message.clone())
    }
}
pub fn diagnostics(command: &Command, text: &str, root: &Path) -> Result<Vec<Diagnostic>> {
    let mut out = Vec::new();
    match command.format.as_str() {
        "json" => {
            out = serde_json::from_str(text)
                .context("Expected a JSON array of {path, line, rule, message}")?
        }
        "eslint" => {
            let files: Vec<serde_json::Value> = serde_json::from_str(text)?;
            for file in files {
                let file_path = file["filePath"]
                    .as_str()
                    .context("ESLint filePath missing")?;
                for message in file["messages"]
                    .as_array()
                    .context("ESLint messages missing")?
                {
                    out.push(Diagnostic {
                        path: file_path.into(),
                        line: message["line"].as_u64().unwrap_or(0) as u32,
                        rule: message["ruleId"].as_str().unwrap_or("parse").into(),
                        message: message["message"]
                            .as_str()
                            .context("ESLint message missing")?
                            .into(),
                    });
                }
            }
        }
        "clippy" => {
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let value: serde_json::Value = serde_json::from_str(line)?;
                if value["reason"] != "compiler-message" {
                    continue;
                }
                let msg = &value["message"];
                ensure!(
                    msg["level"] != "error",
                    "Clippy failed to compile a snapshot"
                );
                if msg["level"] != "warning" {
                    continue;
                }
                let span = msg["spans"]
                    .as_array()
                    .and_then(|spans| spans.iter().find(|s| s["is_primary"] == true));
                if let Some(span) = span {
                    out.push(Diagnostic {
                        path: span["file_name"]
                            .as_str()
                            .context("Clippy path missing")?
                            .into(),
                        line: span["line_start"].as_u64().unwrap_or(0) as u32,
                        rule: msg["code"]["code"].as_str().unwrap_or("warning").into(),
                        message: msg["message"]
                            .as_str()
                            .context("Clippy message missing")?
                            .into(),
                    });
                }
            }
        }
        "regex" => {
            let pattern = regex::Regex::new(&command.warning_regex)?;
            for capture in pattern.captures_iter(text) {
                out.push(Diagnostic {
                    path: capture
                        .name("path")
                        .map(|m| m.as_str())
                        .unwrap_or("")
                        .into(),
                    line: capture
                        .name("line")
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(0),
                    rule: command.id.clone(),
                    message: capture
                        .name("message")
                        .map(|m| m.as_str())
                        .unwrap_or_else(|| capture.get(0).unwrap().as_str())
                        .into(),
                });
            }
        }
        _ => bail!("Unknown lint report format: {}", command.format),
    }
    for d in &mut out {
        d.path = path(&d.path, root);
    }
    Ok(out)
}
