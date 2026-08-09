//! Text / JSON output helpers for `config` management commands.

use std::io::{self, Write};

use serde::Serialize;
use serde_json::json;

use crate::cli::ConfigFormat;

/// Emit a successful management response.
///
/// # Errors
///
/// Returns I/O errors when writing to stdout fails.
pub fn emit_ok<T: Serialize>(
    format: ConfigFormat,
    resource: &str,
    action: &str,
    data: &T,
) -> io::Result<()> {
    match format {
        ConfigFormat::Json => {
            let envelope = json!({
                "ok": true,
                "resource": resource,
                "action": action,
                "data": data,
            });
            let mut out = io::stdout().lock();
            serde_json::to_writer_pretty(&mut out, &envelope)?;
            writeln!(out)?;
            Ok(())
        }
        ConfigFormat::Text => emit_text(resource, action, data),
    }
}

/// Emit a JSON error envelope (text mode prints nothing; caller prints to stderr).
///
/// # Errors
///
/// Returns I/O errors when writing to stdout fails.
pub fn emit_error(format: ConfigFormat, error: &str) -> io::Result<()> {
    if format != ConfigFormat::Json {
        return Ok(());
    }
    let envelope = json!({
        "ok": false,
        "error": error,
    });
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &envelope)?;
    writeln!(out)?;
    Ok(())
}

fn emit_text<T: Serialize>(resource: &str, action: &str, data: &T) -> io::Result<()> {
    let mut out = io::stdout().lock();
    writeln!(out, "{resource} {action}")?;
    let value = serde_json::to_value(data).unwrap_or(serde_json::Value::Null);
    write_text_value(&mut out, &value, 0)?;
    Ok(())
}

fn write_text_value(
    out: &mut impl Write,
    value: &serde_json::Value,
    indent: usize,
) -> io::Result<()> {
    let pad = "  ".repeat(indent);
    match value {
        serde_json::Value::Null => writeln!(out, "{pad}(null)"),
        serde_json::Value::Bool(b) => writeln!(out, "{pad}{b}"),
        serde_json::Value::Number(n) => writeln!(out, "{pad}{n}"),
        serde_json::Value::String(s) => {
            if s.contains('\n') {
                writeln!(out, "{pad}---")?;
                for line in s.lines() {
                    writeln!(out, "{pad}{line}")?;
                }
                writeln!(out, "{pad}---")
            } else {
                writeln!(out, "{pad}{s}")
            }
        }
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                writeln!(out, "{pad}[]")
            } else {
                for (i, item) in items.iter().enumerate() {
                    if matches!(
                        item,
                        serde_json::Value::Object(_) | serde_json::Value::Array(_)
                    ) {
                        writeln!(out, "{pad}- [{i}]")?;
                        write_text_value(out, item, indent + 1)?;
                    } else {
                        write!(out, "{pad}- ")?;
                        write_text_value_inline(out, item)?;
                        writeln!(out)?;
                    }
                }
                Ok(())
            }
        }
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                writeln!(out, "{pad}{{}}")
            } else {
                for (key, val) in map {
                    match val {
                        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                            writeln!(out, "{pad}{key}:")?;
                            write_text_value(out, val, indent + 1)?;
                        }
                        _ => {
                            write!(out, "{pad}{key}: ")?;
                            write_text_value_inline(out, val)?;
                            writeln!(out)?;
                        }
                    }
                }
                Ok(())
            }
        }
    }
}

fn write_text_value_inline(out: &mut impl Write, value: &serde_json::Value) -> io::Result<()> {
    match value {
        serde_json::Value::Null => write!(out, "(null)"),
        serde_json::Value::Bool(b) => write!(out, "{b}"),
        serde_json::Value::Number(n) => write!(out, "{n}"),
        serde_json::Value::String(s) => write!(out, "{s}"),
        other => write!(out, "{other}"),
    }
}
