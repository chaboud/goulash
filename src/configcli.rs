//! `goulash --config` — read and edit settings without hand-editing TOML.
//!
//! The sweep produced a dozen settings and none were reachable except by
//! opening the file. This is the smallest surface that fixes that while
//! keeping the zero-setup promise: everything still has a working default
//! and nobody is ever required to run it.

use crate::config::Config;
use std::path::PathBuf;

const USAGE: &str = "usage: goulash --config [print|path|set KEY VALUE|reset [KEY]]

Reads and edits ~/.goulash/config.toml without opening it. Runs before
the tty check, so it works over ssh, in a script, and in a Dockerfile.

  print            every effective setting, marked (default) or (yours).
                   With no subcommand at all, this is what you get.
  path             absolute path to config.toml, whether or not it
                   exists \u{2014} for `$EDITOR $(goulash --config path)`.
  set KEY VALUE    write one dotted key, preserving the rest of the file
                   and its comments. Sections are created as needed.
  reset [KEY]      remove KEY so its default applies again. With NO key,
                   removes the whole file \u{2014} every setting goes back
                   to its default. That one keeps a copy at
                   config.toml.bak.

Keys are dotted paths into the file, exactly as `print` shows them:

  goulash --config set engine.model gemma4:e4b
  goulash --config set engine.slow query
  goulash --config set engine.slow_lane.thinking high
  goulash --config set debug.working_bar off
  goulash --config reset engine.model      # back to auto-pick
  goulash --config reset                   # back to a clean install

Removing a key is not the same as writing today's default into it: a
reset key follows the default as it improves, a written one does not.
Everything here is optional \u{2014} goulash runs with no config file at
all, and the same settings are live-tunable from `#/settings` in a
session.";

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

/// Every settable key and its effective value.
///
/// One list, used by `print` to show them and by `set` to know a key
/// exists at all: serde ignores fields it does not recognise, so
/// without this a typo (`engine.mdoel`) wrote happily to the file and
/// then did nothing forever.
fn rows(c: &Config) -> Vec<(String, String)> {
    let e = &c.engine;
    vec![
        ("engine.provider".into(), e.provider.clone()),
        ("engine.host".into(), e.host.clone()),
        ("engine.openai_host".into(), e.openai_host.clone()),
        ("engine.api_key_env".into(), e.api_key_env.clone()),
        ("engine.trusted".into(), e.trusted.clone()),
        (
            "engine.model".into(),
            e.model.clone().unwrap_or_else(|| "(auto)".into()),
        ),
        ("engine.thinking".into(), e.thinking.clone()),
        ("engine.max_tokens".into(), e.max_tokens.to_string()),
        ("engine.command_first".into(), e.command_first.to_string()),
        ("engine.slow".into(), e.slow.clone()),
        ("engine.slow_max_steps".into(), e.slow_max_steps.to_string()),
        ("engine.slow_max_secs".into(), e.slow_max_secs.to_string()),
        (
            "engine.num_ctx".into(),
            if e.num_ctx == 0 {
                "(the service's own)".into()
            } else {
                e.num_ctx.to_string()
            },
        ),
        ("engine.num_keep".into(), e.num_keep.to_string()),
        ("engine.keep_alive".into(), e.keep_alive.clone()),
        ("engine.favorites".into(), e.favorites.join(", ")),
        ("engine.commentary".into(), e.commentary.to_string()),
        ("engine.prewarm".into(), e.prewarm.to_string()),
        (
            "engine.divulge.platform".into(),
            e.divulge.platform.to_string(),
        ),
        ("engine.divulge.tools".into(), e.divulge.tools.to_string()),
        (
            "engine.divulge.full_path".into(),
            e.divulge.full_path.to_string(),
        ),
        (
            "engine.slow_lane.model".into(),
            e.slow_lane
                .model
                .clone()
                .unwrap_or_else(|| "(same as fast)".into()),
        ),
        (
            "engine.slow_lane.host".into(),
            e.slow_lane
                .host
                .clone()
                .unwrap_or_else(|| "(same as fast)".into()),
        ),
        ("status.band_rows".into(), c.status.band_rows.to_string()),
        ("status.stats".into(), c.status.stats.to_string()),
        ("debug.idle_repaint".into(), c.debug.idle_repaint.to_string()),
        ("debug.cursor_save".into(), c.debug.cursor_save.clone()),
        ("record.enabled".into(), c.record.enabled.to_string()),
        ("engine.stream".into(), e.stream.to_string()),
        ("engine.seed".into(), e.seed.to_string()),
        ("engine.backfill_abandoned".into(), e.backfill_abandoned.to_string()),
        ("engine.context_max_chars".into(), e.context_max_chars.to_string()),
        ("engine.tail_chars".into(), e.tail_chars.to_string()),
        (
            "engine.context_files_max_chars".into(),
            e.context_files_max_chars.to_string(),
        ),
        (
            "engine.context_tree_max_files".into(),
            e.context_tree_max_files.to_string(),
        ),
        (
            "engine.context_tree_max_depth".into(),
            e.context_tree_max_depth.to_string(),
        ),
        (
            "engine.slow_lane.provider".into(),
            e.slow_lane
                .provider
                .clone()
                .unwrap_or_else(|| "(same as fast)".into()),
        ),
        (
            "engine.slow_lane.openai_host".into(),
            e.slow_lane
                .openai_host
                .clone()
                .unwrap_or_else(|| "(same as fast)".into()),
        ),
        (
            "engine.slow_lane.api_key_env".into(),
            e.slow_lane
                .api_key_env
                .clone()
                .unwrap_or_else(|| "(same as fast)".into()),
        ),
        (
            "engine.slow_lane.trusted".into(),
            e.slow_lane
                .trusted
                .clone()
                .unwrap_or_else(|| "(same as fast)".into()),
        ),
        (
            "engine.slow_lane.thinking".into(),
            e.slow_lane
                .thinking
                .clone()
                .unwrap_or_else(|| "(same as fast)".into()),
        ),
        (
            "engine.slow_lane.max_tokens".into(),
            e.slow_lane
                .max_tokens
                .map(|n| n.to_string())
                .unwrap_or_else(|| "(same as fast)".into()),
        ),
        ("status.enabled".into(), c.status.enabled.to_string()),
        ("status.rows".into(), c.status.rows.to_string()),
        ("status.band".into(), c.status.band.to_string()),
        ("status.menu_rows".into(), c.status.menu_rows.to_string()),
        ("shell.auto_integrate".into(), c.shell.auto_integrate.to_string()),
        ("record.output".into(), c.record.output.to_string()),
        ("debug.show_advanced".into(), c.debug.show_advanced.to_string()),
        ("debug.wrap_guard".into(), c.debug.wrap_guard.to_string()),
        ("debug.working_bar".into(), c.debug.working_bar.to_string()),
        (
            "debug.working_bar_on_watch".into(),
            c.debug.working_bar_on_watch.to_string(),
        ),
        (
            "debug.working_bar_step_ms".into(),
            c.debug.working_bar_step_ms.to_string(),
        ),
        (
            "debug.working_bar_grow_ms".into(),
            c.debug.working_bar_grow_ms.to_string(),
        ),
        ("debug.slow_via_fast".into(), c.debug.slow_via_fast.to_string()),
        (
            "debug.quote_fast_to_slow".into(),
            c.debug.quote_fast_to_slow.to_string(),
        ),
    ]
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
    let mark = |k: &str| if mine.iter().any(|m| m == k) { "you" } else { "default" };

    println!("# {}\n", p.display());
    let rows = rows(&c);

    let w = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(24);
    for (k, v) in &rows {
        println!("{k:<w$}  {v:<10}  [{}]", mark(k), w = w);
    }
    0
}

/// Set one dotted key, and REFUSE to write a file that will not load.
///
/// The old version parsed the value as TOML and, failing that, wrote it
/// as a string — so `set debug.working_bar off` printed
/// `debug.working_bar = off`, exited 0, and wrote `working_bar = "off"`
/// into a bool field. serde then rejected the whole document at the
/// next launch and fell back to defaults, silently, taking every other
/// setting in the file with it. A control that does not take must say
/// so: every candidate is now round-tripped through the real `Config`
/// before anything reaches the disk.
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
    // A key serde has never heard of writes fine and does nothing, so
    // the typo is caught here rather than at the next launch.
    if !rows(&Config::load()).iter().any(|(k, _)| k == key) {
        eprintln!("goulash: no such setting {key:?}");
        let near: Vec<String> = rows(&Config::load())
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| {
                k.split('.').next_back() == key.split('.').next_back()
                    || k.starts_with(parts[0])
            })
            .take(6)
            .collect();
        if !near.is_empty() {
            eprintln!("  did you mean: {}", near.join(", "));
        }
        eprintln!("  `goulash --config print` lists every key");
        return 2;
    }
    // Candidates, most specific first. `on`/`off` are what the menus
    // say and what a person types; TOML has never heard of them.
    let mut cands: Vec<toml_edit::Value> = Vec::new();
    if let Ok(v) = value.parse::<toml_edit::Value>() {
        cands.push(v);
    }
    match value.to_ascii_lowercase().as_str() {
        "on" | "yes" => cands.push(true.into()),
        "off" | "no" => cands.push(false.into()),
        _ => {}
    }
    cands.push(toml_edit::Value::from(value));

    let write_at = |doc: &mut toml_edit::DocumentMut, v: toml_edit::Value| -> Result<String, String> {
        let mut node = doc.as_table_mut();
        for seg in &parts[..parts.len() - 1] {
            if node.get(seg).is_none() {
                node[seg] = toml_edit::table();
            }
            node = node[seg]
                .as_table_mut()
                .ok_or_else(|| format!("{seg} is not a table"))?;
        }
        node[parts[parts.len() - 1]] = toml_edit::value(v);
        Ok(doc.to_string())
    };

    let mut last_err = String::new();
    let mut good: Option<String> = None;
    for c in cands {
        let mut trial = doc.clone();
        match write_at(&mut trial, c) {
            Err(e) => {
                eprintln!("goulash: {e}");
                return 1;
            }
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(_) => {
                    good = Some(text);
                    break;
                }
                Err(e) => last_err = e.to_string(),
            },
        }
    }
    let Some(out) = good else {
        eprintln!("goulash: {key} does not accept {value:?}");
        eprintln!("  {last_err}");
        eprintln!("  nothing was written");
        return 1;
    };
    doc = out.parse().unwrap_or(doc);

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
        // `--config --help` is reached before main's own --help arm,
        // since --config is handled first so it works without a tty.
        // Asking for help is not an error and does not exit 2.
        Some("--help") | Some("-h") | Some("help") => {
            println!("{USAGE}");
            0
        }
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
