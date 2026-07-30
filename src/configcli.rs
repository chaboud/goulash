//! `goulash --config` — read and edit settings without hand-editing TOML.
//!
//! The sweep produced a dozen settings and none were reachable except by
//! opening the file. This is the smallest surface that fixes that while
//! keeping the zero-setup promise: everything still has a working default
//! and nobody is ever required to run it.

use crate::config::Config;
use std::path::PathBuf;

const USAGE: &str = "usage: goulash --config [print|path|set KEY VALUE|reset [KEY]]

  print          effective values, and whether each is a default or yours
  path           where config.toml lives
  set KEY VALUE  surgical write, preserving comments  (e.g. engine.thinking auto)
  reset [KEY]    remove KEY (or the whole file) so defaults apply again";

fn path() -> Option<PathBuf> {
    Config::dir().map(|d| d.join("config.toml"))
}

/// Values the user has actually set, as dotted keys.
fn set_keys(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return out;
    };
    for (section, item) in doc.iter() {
        match item.as_table() {
            Some(t) => {
                for (k, v) in t.iter() {
                    if v.is_table() {
                        if let Some(inner) = v.as_table() {
                            for (k2, _) in inner.iter() {
                                out.push(format!("{section}.{k}.{k2}"));
                            }
                        }
                    } else {
                        out.push(format!("{section}.{k}"));
                    }
                }
            }
            None => out.push(section.to_string()),
        }
    }
    out
}

fn print_effective() -> i32 {
    let p = match path() {
        Some(p) => p,
        None => {
            eprintln!("goulash: no home directory");
            return 1;
        }
    };
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let mine = set_keys(&text);
    let c = Config::load();
    let e = &c.engine;
    let mark = |k: &str| if mine.iter().any(|m| m == k) { "you" } else { "default" };

    println!("# {}\n", p.display());
    let rows: Vec<(String, String)> = vec![
        ("engine.provider".into(), e.provider.clone()),
        ("engine.host".into(), e.host.clone()),
        ("engine.model".into(), e.model.clone().unwrap_or_else(|| "(auto)".into())),
        ("engine.thinking".into(), e.thinking.clone()),
        ("engine.response_tokens".into(), e.response_tokens.to_string()),
        ("engine.reasoning_tokens".into(), e.reasoning_tokens.to_string()),
        ("engine.num_ctx_min".into(), e.num_ctx_min.to_string()),
        ("engine.num_ctx".into(), e.num_ctx.map(|v| v.to_string()).unwrap_or_else(|| "(unset)".into())),
        ("engine.prefer_resident".into(), e.prefer_resident.to_string()),
        ("engine.keep_alive".into(), e.keep_alive.clone()),
        ("engine.commentary".into(), e.commentary.to_string()),
        ("engine.divulge.platform".into(), e.divulge.platform.to_string()),
        ("engine.divulge.tools".into(), e.divulge.tools.to_string()),
        ("engine.divulge.full_path".into(), e.divulge.full_path.to_string()),
        ("status.band_rows".into(), c.status.band_rows.to_string()),
        ("record.enabled".into(), c.record.enabled.to_string()),
    ];
    let w = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(24);
    for (k, v) in &rows {
        println!("{k:<w$}  {v:<10}  [{}]", mark(k), w = w);
    }
    0
}

/// Set one dotted key. Values are parsed as TOML, so `true`, `512` and
/// `"auto"` all land with the right type; a bare word becomes a string.
fn set(key: &str, value: &str) -> i32 {
    let Some(p) = path() else {
        eprintln!("goulash: no home directory");
        return 1;
    };
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("goulash: config parse: {e}");
            return 1;
        }
    };
    let parts: Vec<&str> = key.split('.').collect();
    if parts.len() < 2 {
        eprintln!("goulash: key must be dotted, e.g. engine.thinking");
        return 2;
    }
    let parsed: toml_edit::Value = value
        .parse()
        .unwrap_or_else(|_| toml_edit::Value::from(value));

    // Walk/create intermediate tables.
    let mut node = doc.as_table_mut();
    for seg in &parts[..parts.len() - 1] {
        if node.get(seg).is_none() {
            node[seg] = toml_edit::table();
        }
        match node[seg].as_table_mut() {
            Some(t) => node = t,
            None => {
                eprintln!("goulash: {seg} is not a table");
                return 1;
            }
        }
    }
    node[parts[parts.len() - 1]] = toml_edit::value(parsed);

    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&p, doc.to_string()) {
        Ok(_) => {
            println!("{key} = {value}");
            0
        }
        Err(e) => {
            eprintln!("goulash: write {}: {e}", p.display());
            1
        }
    }
}

/// Reset by **deleting** the key, never by writing today's default.
///
/// Two things fall out of that: `config.toml` holds only deliberate
/// deviations, so `print` can honestly say default-vs-yours; and a user
/// who resets today inherits a *better* default in a later version rather
/// than being pinned to the value that happened to be current.
fn reset(key: Option<&str>) -> i32 {
    let Some(p) = path() else {
        eprintln!("goulash: no home directory");
        return 1;
    };
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(_) => {
            println!("nothing to reset (no config file)");
            return 0;
        }
    };
    match key {
        None => {
            // The one destructive path, so it keeps a copy.
            let bak = p.with_extension("toml.bak");
            let _ = std::fs::write(&bak, &text);
            match std::fs::remove_file(&p) {
                Ok(_) => {
                    println!("reset all settings to defaults (old file kept at {})", bak.display());
                    0
                }
                Err(e) => {
                    eprintln!("goulash: {e}");
                    1
                }
            }
        }
        Some(k) => {
            let mut doc: toml_edit::DocumentMut = match text.parse() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("goulash: config parse: {e}");
                    return 1;
                }
            };
            let parts: Vec<&str> = k.split('.').collect();
            let mut node = doc.as_table_mut();
            for seg in &parts[..parts.len().saturating_sub(1)] {
                match node.get_mut(seg).and_then(|i| i.as_table_mut()) {
                    Some(t) => node = t,
                    None => {
                        println!("{k} was not set");
                        return 0;
                    }
                }
            }
            let last = parts[parts.len() - 1];
            if node.remove(last).is_some() {
                let _ = std::fs::write(&p, doc.to_string());
                println!("{k} reset to default");
            } else {
                println!("{k} was not set");
            }
            0
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("print") => print_effective(),
        Some("path") => match path() {
            Some(p) => {
                println!("{}", p.display());
                0
            }
            None => 1,
        },
        Some("set") => match (args.get(1), args.get(2)) {
            (Some(k), Some(v)) => set(k, v),
            _ => {
                eprintln!("{USAGE}");
                2
            }
        },
        Some("reset") => reset(args.get(1).map(String::as_str)),
        Some(other) => {
            eprintln!("goulash: unknown --config command {other:?}\n{USAGE}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::set_keys;

    #[test]
    fn set_keys_finds_nested_and_flat() {
        let t = "[engine]\nthinking = \"auto\"\n\n[engine.divulge]\ntools = true\n";
        let k = set_keys(t);
        assert!(k.contains(&"engine.thinking".to_string()));
        assert!(k.contains(&"engine.divulge.tools".to_string()));
        assert!(!k.contains(&"engine.response_tokens".to_string()));
    }
}
