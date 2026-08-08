//! Command-line argument parsing for the `lbt` command tree.
//!
//! `lbt` uses a custom, table-driven dispatcher instead of a derive CLI so
//! that every command can mix short flags, long flags, aliases and positional
//! arguments freely at any depth (the tree supports up to 25 levels). Flags
//! may appear before, between, or after subcommand tokens:
//!
//! ```text
//! lbt -j d                 # short flag + alias for `discover`
//! lbt c theme apply -n cps # alias `c` + subcommands + long flag
//! lbt image iso -r ./sys -o boot.iso -l "Leon"
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use anyhow::{Result, bail};

use super::tree::{Flag, Kind, Node, ROOT};

/// What a successfully parsed invocation wants to do.
#[derive(Debug)]
pub enum Action {
    /// Print help for the node reached (namespace with no leaf, or -h).
    Help(&'static Node),
    /// Invoke the leaf's handler with the parsed arguments.
    Run(&'static Node, Parsed),
}

/// The parsed command: the canonical path walked, flags, and positionals.
#[derive(Debug, Default)]
pub struct Parsed {
    /// Canonical names of the nodes walked from the root.
    pub path: Vec<&'static str>,
    /// Long flags that were given a value (`--name=val`, `-n val`).
    pub values: HashMap<String, String>,
    /// Long flags set as booleans (`--json`, `-j`).
    pub bools: HashSet<String>,
    /// Bare positional tokens.
    pub positional: Vec<String>,
}

impl Parsed {
    /// Whether a flag (long name) was given, with or without a value.
    pub fn flag(&self, long: &str) -> bool {
        self.bools.contains(long) || self.values.contains_key(long)
    }

    /// The value of a flag, if given.
    pub fn value(&self, long: &str) -> Option<&str> {
        self.values.get(long).map(String::as_str)
    }

    /// The i-th positional argument, if any.
    pub fn pos(&self, i: usize) -> Option<&str> {
        self.positional.get(i).map(String::as_str)
    }

    /// The full canonical command line (`info`, `boot config get`, ...).
    pub fn command_line(&self) -> String {
        self.path.join(" ")
    }
}

/// Long names and short characters that take a value, across the whole tree.
/// Because every short character maps to exactly one long name and every long
/// name is consistently a value or a boolean, this global vocabulary is safe
/// to consult while scanning tokens before the path is resolved.
fn value_flags() -> (&'static HashSet<&'static str>, &'static HashSet<char>) {
    static SETS: OnceLock<(HashSet<&'static str>, HashSet<char>)> = OnceLock::new();
    let sets = SETS.get_or_init(|| {
        let mut longs = HashSet::new();
        let mut shorts = HashSet::new();
        for f in super::tree::VALUE_FLAGS {
            longs.insert(f.long);
            shorts.insert(f.short);
        }
        (longs, shorts)
    });
    (&sets.0, &sets.1)
}

/// Resolves a long or short flag name to a `Flag` declared on the walked path.
fn flag_on_path(path: &[&'static str], long: &str) -> Option<&'static Flag> {
    ROOT.find_long(long).or_else(|| {
        // The path is stored canonically; walk from the root again to collect
        // every node on the path and check their declared flags.
        let mut node = &ROOT;
        for name in path {
            node = node.children().iter().find(|c| c.name == *name)?;
            if let Some(f) = node.flags.iter().find(|f| f.long == long) {
                return Some(f);
            }
        }
        None
    })
}

fn short_to_long(path: &[&'static str], c: char) -> Option<&'static str> {
    let mut node = &ROOT;
    if let Some(f) = ROOT.flags.iter().find(|f| f.short == c) {
        return Some(f.long);
    }
    for name in path {
        node = node.children().iter().find(|n| n.name == *name)?;
        if let Some(f) = node.flags.iter().find(|f| f.short == c) {
            return Some(f.long);
        }
    }
    None
}

/// Parses the process arguments (skipping argv[0]).
pub fn parse() -> Result<Action> {
    parse_from(std::env::args().skip(1).collect())
}

/// Parses a token slice (used by tests).
pub fn parse_from(tokens: Vec<String>) -> Result<Action> {
    let (value_longs, value_shorts) = value_flags();
    let mut parsed = Parsed::default();
    let mut node = &ROOT;
    let mut i = 0usize;
    let mut help = false;
    let mut after_dd = false;

    while i < tokens.len() {
        let tok = &tokens[i];

        if after_dd {
            parsed.positional.push(tok.clone());
            i += 1;
            continue;
        }

        match tok.as_str() {
            "--" => {
                after_dd = true;
                i += 1;
                continue;
            }
            "-h" | "--help" => {
                help = true;
                i += 1;
                continue;
            }
            _ => {}
        }

        if let Some(body) = tok.strip_prefix("--") {
            // Long flag: --name or --name=value.
            if let Some((name, val)) = body.split_once('=') {
                parsed.values.insert(name.to_string(), val.to_string());
            } else if value_longs.contains(body) {
                let val = tokens.get(i + 1).cloned().ok_or_else(|| {
                    anyhow::anyhow!("flag `--{body}` requires a value")
                })?;
                parsed.values.insert(body.to_string(), val);
                i += 1;
            } else {
                parsed.bools.insert(body.to_string());
            }
            i += 1;
            continue;
        }

        if tok.len() >= 2 && tok.starts_with('-') && tok != "-" {
            // Short flag(s): -abc, -f, -fVAL.
            let chars: Vec<char> = tok[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                let c = chars[j];
                if value_shorts.contains(&c) {
                    let rest: String = chars[j + 1..].iter().collect();
                    let has_rest = !rest.is_empty();
                    let val = if has_rest {
                        rest
                    } else {
                        tokens
                            .get(i + 1)
                            .cloned()
                            .ok_or_else(|| anyhow::anyhow!("flag `-{c}` requires a value"))?
                    };
                    i += if has_rest { 0 } else { 1 }; // consumed the next token
                    if let Some(long) = short_to_long(&parsed.path, c) {
                        parsed.values.insert(long.to_string(), val);
                    } else {
                        parsed.values.insert(format!("-{c}"), val);
                    }
                    break;
                } else {
                    // boolean short: resolve later once the path is known.
                    parsed.bools.insert(format!("-{c}"));
                    j += 1;
                }
            }
            i += 1;
            continue;
        }

        // Subcommand token?
        let child = node
            .children()
            .iter()
            .find(|n| n.matches(tok));
        if let Some(child) = child {
            parsed.path.push(child.name);
            node = child;
            i += 1;
            continue;
        }

        // Not a subcommand: positional argument.
        parsed.positional.push(tok.clone());
        i += 1;
    }

    // Resolve short booleans to their long names using the resolved path.
    let shorts: Vec<String> = parsed
        .bools
        .iter()
        .filter(|b| b.starts_with('-') && b.len() == 2)
        .cloned()
        .collect();
    for s in shorts {
        let c = s.chars().nth(1).unwrap();
        if let Some(long) = short_to_long(&parsed.path, c) {
            parsed.bools.insert(long.to_string());
        }
    }

    // Validate every long flag is known on the path.
    let known = |long: &str| {
        if long.starts_with('-') {
            return short_to_long(&parsed.path, long.chars().nth(1).unwrap()).is_some();
        }
        flag_on_path(&parsed.path, long).is_some()
    };
    for long in parsed.bools.iter() {
        if !known(long) {
            bail!("unknown flag `--{long}` for `{}`", parsed.command_line());
        }
    }
    for long in parsed.values.keys() {
        if !known(long) {
            bail!("unknown flag `--{long}` for `{}`", parsed.command_line());
        }
    }

    if help {
        return Ok(Action::Help(node));
    }
    match &node.kind {
        Kind::Ns(_) => Ok(Action::Help(node)),
        Kind::Leaf(h) => {
            if h.is_cps() && !cfg!(feature = "cps") {
                bail!(
                    "`{}` needs the `cps` feature: rebuild with `cargo build -p lbt --features cps`",
                    parsed.command_line()
                );
            }
            Ok(Action::Run(node, parsed))
        }
    }
}

/// True when no arguments at all were given.
pub fn empty() -> bool {
    std::env::args().len() <= 1
}
