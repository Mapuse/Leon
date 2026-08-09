//! The `lbc` command tree.
//!
//! A single static table drives the whole CLI: every node carries a canonical
//! name, up to five aliases, a short help string, its flags, and either child
//! nodes (a namespace, up to 25 levels deep) or a [`Handler`]. The tree is
//! intentionally flat-ish per letter but deep in the real subtrees (`config`,
//! `boot`, `profile`), and the alphabet `a..z` / `A..Z` is fully populated.
//!
//! `lbc` owns boot configuration and boot control; the sibling `lbt` binary
//! owns discovery helpers, image building, and the cps subsystem.

/// One command-line flag. Short characters and long names are unique across
/// the whole tree, and each is consistently either a boolean or a value.
#[derive(Debug)]
pub struct Flag {
    pub short: char,
    pub long: &'static str,
    pub value: bool,
    pub doc: &'static str,
}

/// A command node in the tree.
#[derive(Debug)]
pub struct Node {
    pub name: &'static str,
    pub doc: &'static str,
    pub aliases: &'static [&'static str],
    pub flags: &'static [Flag],
    pub kind: Kind,
}

#[derive(Debug)]
pub enum Kind {
    /// A namespace with children.
    Ns(&'static [Node]),
    /// A runnable command.
    Leaf(Handler),
}

impl Node {
    /// Whether a token matches this node's canonical name or any alias.
    pub fn matches(&self, tok: &str) -> bool {
        self.name == tok || self.aliases.contains(&tok)
    }

    /// The leaf handler if this node is a leaf.
    pub fn handler(&self) -> Option<Handler> {
        match &self.kind {
            Kind::Leaf(h) => Some(*h),
            Kind::Ns(_) => None,
        }
    }

    pub fn children(&self) -> &'static [Node] {
        match &self.kind {
            Kind::Ns(c) => c,
            Kind::Leaf(_) => &[],
        }
    }

    /// Finds a flag declaration with the given long name on this node.
    pub fn find_long(&self, long: &str) -> Option<&'static Flag> {
        self.flags.iter().find(|f| f.long == long)
    }
}

/// A runnable handler id; the dispatch lives in [`crate::commands`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handler {
    Info,
    Stage,
    BootOnce,
    EspSync,
    ConfigGet,
    ConfigSet,
    ConfigList,
    ConfigReset,
    ConfigPath,
    DefaultGet,
    DefaultSet,
    EntriesList,
    EntriesGet,
    EntriesCount,
    FindEsps,
    FindEntries,
    GeometryShow,
    GeometryBgr,
    JsonConfig,
    JsonDiscover,
    JsonInfo,
    JsonAll,
    NamesList,
    NamesGet,
    NamesCount,
    QueryInfo,
    QueryEsps,
    ProfileSave,
    ProfileList,
    ProfileLoad,
    ProfileDelete,
    KeymapShow,
    Version,
    HelpAll,
    HelpTree,
    HelpCommand,
    AliasList,
    Status,
}

// ── Global flag vocabulary ──────────────────────────────────────────────────
pub const F_JSON: Flag = Flag { short: 'j', long: "json", value: false, doc: "Emit machine-readable JSON" };
pub const F_PATH: Flag = Flag { short: 'p', long: "path", value: true, doc: "File or directory path" };
pub const F_NAME: Flag = Flag { short: 'n', long: "name", value: true, doc: "Entry or command name" };
pub const F_DEST: Flag = Flag { short: 'd', long: "dest", value: true, doc: "Destination directory" };
pub const F_ARCH: Flag = Flag { short: 'a', long: "arch", value: true, doc: "Architecture: amd64 | arm64" };
pub const F_KEY: Flag = Flag { short: 'k', long: "key", value: true, doc: "Config key" };
pub const F_VALUE: Flag = Flag { short: 'v', long: "value", value: true, doc: "Config value" };
pub const F_VOL: Flag = Flag { short: 'U', long: "vol", value: true, doc: "ESP volume path" };
pub const F_PROFILE: Flag = Flag { short: 'P', long: "profile", value: true, doc: "Config profile name" };

/// Flags that consume a value; consulted by the tokenizer before path
/// resolution. Every short character is unique, so this union is unambiguous.
pub const VALUE_FLAGS: &[Flag] = &[
    F_PATH, F_NAME, F_DEST, F_ARCH, F_KEY, F_VALUE, F_VOL, F_PROFILE,
];

const NO_FLAGS: &[Flag] = &[];

// ── boot subtree ────────────────────────────────────────────────────────────
mod boot_tree {
    use super::*;

    pub const CONFIG: &[Node] = &[
        Node { name: "get", doc: "Print a boot config key", aliases: &["g", "gt", "fetch", "read"], flags: &[F_KEY], kind: Kind::Leaf(Handler::ConfigGet) },
        Node { name: "set", doc: "Set a boot config key", aliases: &["s", "st", "write", "put"], flags: &[F_KEY, F_VALUE], kind: Kind::Leaf(Handler::ConfigSet) },
        Node { name: "list", doc: "Print the whole boot config", aliases: &["l", "ls", "dump", "all"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ConfigList) },
        Node { name: "reset", doc: "Reset the boot config to defaults", aliases: &["r", "rm", "clear", "wipe"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ConfigReset) },
        Node { name: "path", doc: "Print the resolved boot config path", aliases: &["p", "loc", "where", "find"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ConfigPath) },
    ];

    pub const BOOT: &[Node] = &[
        Node { name: "config", doc: "Manage the boot config", aliases: &["cfg", "c", "conf"], flags: NO_FLAGS, kind: Kind::Ns(CONFIG) },
        Node { name: "stage", doc: "Stage the ESP tree under build/esp", aliases: &["stg", "s", "deploy", "prepare"], flags: &[F_DEST, F_ARCH], kind: Kind::Leaf(Handler::Stage) },
        Node { name: "info", doc: "Report the boot volume layout", aliases: &["i", "show", "describe"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::Info) },
        Node { name: "once", doc: "Boot one entry a single time", aliases: &["o", "one", "single", "next"], flags: &[F_NAME], kind: Kind::Leaf(Handler::BootOnce) },
        Node { name: "upload", doc: "Mirror the boot config onto an ESP", aliases: &["u", "sync", "up", "push"], flags: &[F_VOL], kind: Kind::Leaf(Handler::EspSync) },
    ];
}

// ── config subtree ──────────────────────────────────────────────────────────
mod config_tree {
    use super::*;

    pub const CONFIG: &[Node] = &[
        Node { name: "get", doc: "Print a boot config key", aliases: &["g", "gt", "fetch", "read"], flags: &[F_KEY], kind: Kind::Leaf(Handler::ConfigGet) },
        Node { name: "set", doc: "Set a boot config key", aliases: &["s", "st", "write", "put"], flags: &[F_KEY, F_VALUE], kind: Kind::Leaf(Handler::ConfigSet) },
        Node { name: "list", doc: "Print the whole boot config", aliases: &["l", "ls", "dump", "all"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ConfigList) },
        Node { name: "reset", doc: "Reset the boot config to defaults", aliases: &["r", "rm", "clear", "wipe"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ConfigReset) },
        Node { name: "path", doc: "Print the resolved boot config path", aliases: &["p", "loc", "where", "find"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ConfigPath) },
    ];
}

// ── default subtree ─────────────────────────────────────────────────────────
mod default_tree {
    use super::*;

    pub const DEFAULT: &[Node] = &[
        Node { name: "get", doc: "Print the default boot entry", aliases: &["g", "gt", "fetch", "read"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::DefaultGet) },
        Node { name: "set", doc: "Set the default boot entry", aliases: &["s", "st", "write", "put"], flags: &[F_NAME], kind: Kind::Leaf(Handler::DefaultSet) },
    ];
}

// ── entries subtree ─────────────────────────────────────────────────────────
mod entries_tree {
    use super::*;

    pub const ENTRIES: &[Node] = &[
        Node { name: "list", doc: "List discovered boot entries", aliases: &["l", "ls", "enumerate", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::EntriesList) },
        Node { name: "get", doc: "Resolve one boot entry by label", aliases: &["g", "find", "lookup", "resolve"], flags: &[F_NAME], kind: Kind::Leaf(Handler::EntriesGet) },
        Node { name: "count", doc: "Count discovered boot entries", aliases: &["c", "n", "tally", "total"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::EntriesCount) },
    ];
}

// ── find subtree ────────────────────────────────────────────────────────────
mod find_tree {
    use super::*;

    pub const FIND: &[Node] = &[
        Node { name: "esps", doc: "Find ESP volumes", aliases: &["e", "efi", "esp", "volumes"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::FindEsps) },
        Node { name: "entries", doc: "Find boot entries by label", aliases: &["b", "boot", "lookup", "search"], flags: &[F_NAME], kind: Kind::Leaf(Handler::FindEntries) },
    ];
}

// ── geometry subtree ────────────────────────────────────────────────────────
mod geometry_tree {
    use super::*;

    pub const GEOMETRY: &[Node] = &[
        Node { name: "show", doc: "Show framebuffer + BGRT geometry", aliases: &["s", "print", "dump", "info"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::GeometryShow) },
        Node { name: "bgr", doc: "Show BGRT logo geometry", aliases: &["b", "logo", "bgrt", "splash"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::GeometryBgr) },
    ];
}

// ── help subtree ────────────────────────────────────────────────────────────
mod help_tree {
    use super::*;

    pub const HELP: &[Node] = &[
        Node { name: "all", doc: "Print the full command reference", aliases: &["a", "full", "everything", "overview"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::HelpAll) },
        Node { name: "tree", doc: "Print the command tree", aliases: &["t", "graph", "structure", "outline"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::HelpTree) },
        Node { name: "command", doc: "Describe one command", aliases: &["c", "cmd", "about", "explain"], flags: &[F_NAME], kind: Kind::Leaf(Handler::HelpCommand) },
    ];
}

// ── json subtree ────────────────────────────────────────────────────────────
mod json_tree {
    use super::*;

    pub const JSON: &[Node] = &[
        Node { name: "config", doc: "Boot config as JSON", aliases: &["c", "cfg", "conf", "boot"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::JsonConfig) },
        Node { name: "discover", doc: "Discovery as JSON", aliases: &["d", "disc", "scan", "detect"], flags: &[F_JSON], kind: Kind::Leaf(Handler::JsonDiscover) },
        Node { name: "info", doc: "Geometry as JSON", aliases: &["i", "geom", "geometry", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::JsonInfo) },
        Node { name: "all", doc: "Everything as JSON", aliases: &["a", "full", "everything", "complete"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::JsonAll) },
    ];
}

// ── keymap subtree ──────────────────────────────────────────────────────────
mod keymap_tree {
    use super::*;

    pub const KEYMAP: &[Node] = &[
        Node { name: "show", doc: "Print the boot-manager keymap", aliases: &["s", "print", "list", "all"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::KeymapShow) },
    ];
}

// ── list subtree ────────────────────────────────────────────────────────────
mod list_tree {
    use super::*;

    pub const LIST: &[Node] = &[
        Node { name: "entries", doc: "List boot entries", aliases: &["e", "boot", "names", "all"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::EntriesList) },
        Node { name: "esps", doc: "List ESP volumes", aliases: &["s", "efi", "volumes", "partitions"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::FindEsps) },
    ];
}

// ── names subtree ───────────────────────────────────────────────────────────
mod names_tree {
    use super::*;

    pub const NAMES: &[Node] = &[
        Node { name: "list", doc: "List boot entry labels", aliases: &["l", "ls", "enumerate", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::NamesList) },
        Node { name: "get", doc: "Resolve a label to its on-ESP path", aliases: &["g", "find", "lookup", "resolve"], flags: &[F_NAME], kind: Kind::Leaf(Handler::NamesGet) },
        Node { name: "count", doc: "Count boot entries", aliases: &["c", "n", "tally", "total"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::NamesCount) },
    ];
}

// ── once subtree ────────────────────────────────────────────────────────────
mod once_tree {
    use super::*;

    pub const ONCE: &[Node] = &[
        Node { name: "boot", doc: "Boot one entry a single time", aliases: &["b", "run", "start", "single"], flags: &[F_NAME], kind: Kind::Leaf(Handler::BootOnce) },
    ];
}

// ── profile subtree ─────────────────────────────────────────────────────────
mod profile_tree {
    use super::*;

    pub const PROFILE: &[Node] = &[
        Node { name: "save", doc: "Save the current config as a profile", aliases: &["s", "store", "keep", "write"], flags: &[F_PROFILE], kind: Kind::Leaf(Handler::ProfileSave) },
        Node { name: "list", doc: "List saved config profiles", aliases: &["l", "ls", "enumerate", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ProfileList) },
        Node { name: "load", doc: "Restore a config from a profile", aliases: &["g", "restore", "use", "apply"], flags: &[F_PROFILE], kind: Kind::Leaf(Handler::ProfileLoad) },
        Node { name: "delete", doc: "Delete a saved config profile", aliases: &["d", "rm", "remove", "drop"], flags: &[F_PROFILE], kind: Kind::Leaf(Handler::ProfileDelete) },
    ];
}

// ── query subtree ───────────────────────────────────────────────────────────
mod query_tree {
    use super::*;

    pub const QUERY: &[Node] = &[
        Node { name: "info", doc: "Query framebuffer geometry", aliases: &["i", "geom", "geometry", "fb"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::QueryInfo) },
        Node { name: "esps", doc: "Query ESP volumes", aliases: &["e", "discover", "scan", "volumes"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::QueryEsps) },
    ];
}

// ── root tree: alphabet + real subtrees ─────────────────────────────────────
pub static ROOT: Node = Node {
    name: "lbc",
    doc: "Leon Boot Configuration — manage the boot config and boot control: boot-manager menu, staging, boot volume layout",
    aliases: &[],
    flags: &[],
    kind: Kind::Ns(&[
        Node { name: "a", doc: "Alias reports", aliases: &["alias", "aliases", "abbr", "all"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "list", doc: "List every command alias", aliases: &["l", "ls", "all", "dump"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::AliasList) },
        ]) },
        Node { name: "b", doc: "Boot control", aliases: &["boot", "bctl", "bootctl"], flags: NO_FLAGS, kind: Kind::Ns(boot_tree::BOOT) },
        Node { name: "c", doc: "Boot config control", aliases: &["config", "cfg", "conf", "bootcfg"], flags: NO_FLAGS, kind: Kind::Ns(config_tree::CONFIG) },
        Node { name: "d", doc: "Default boot entry", aliases: &["default", "def", "preferred"], flags: NO_FLAGS, kind: Kind::Ns(default_tree::DEFAULT) },
        Node { name: "e", doc: "Boot entries", aliases: &["entries", "entry", "list", "boots"], flags: NO_FLAGS, kind: Kind::Ns(entries_tree::ENTRIES) },
        Node { name: "f", doc: "Find ESPs and entries", aliases: &["find", "locate", "search", "detect"], flags: NO_FLAGS, kind: Kind::Ns(find_tree::FIND) },
        Node { name: "g", doc: "Framebuffer geometry", aliases: &["geometry", "geom", "geo", "fb"], flags: NO_FLAGS, kind: Kind::Ns(geometry_tree::GEOMETRY) },
        Node { name: "h", doc: "Help", aliases: &["help", "?", "man", "usage"], flags: NO_FLAGS, kind: Kind::Ns(help_tree::HELP) },
        Node { name: "i", doc: "Report the boot volume layout", aliases: &["info", "inspect", "layout", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::Info) },
        Node { name: "j", doc: "JSON output helpers", aliases: &["json", "jq", "machine"], flags: NO_FLAGS, kind: Kind::Ns(json_tree::JSON) },
        Node { name: "k", doc: "Boot-manager keymap", aliases: &["keymap", "keys", "kb", "shortcuts"], flags: NO_FLAGS, kind: Kind::Ns(keymap_tree::KEYMAP) },
        Node { name: "l", doc: "List helpers", aliases: &["list", "ls", "enumerate"], flags: NO_FLAGS, kind: Kind::Ns(list_tree::LIST) },
        Node { name: "n", doc: "Boot entry names", aliases: &["names", "labels", "titles"], flags: NO_FLAGS, kind: Kind::Ns(names_tree::NAMES) },
        Node { name: "o", doc: "One-shot boot", aliases: &["once", "one", "single"], flags: NO_FLAGS, kind: Kind::Ns(once_tree::ONCE) },
        Node { name: "p", doc: "Boot config profiles", aliases: &["profile", "prof", "cfgprof"], flags: NO_FLAGS, kind: Kind::Ns(profile_tree::PROFILE) },
        Node { name: "q", doc: "Queries", aliases: &["query", "qry", "ask", "inspect"], flags: NO_FLAGS, kind: Kind::Ns(query_tree::QUERY) },
        Node { name: "r", doc: "Reset the boot config to defaults", aliases: &["reset", "wipe", "clear", "revert"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ConfigReset) },
        Node { name: "s", doc: "Stage the ESP tree", aliases: &["stage", "stg", "deploy", "prep"], flags: &[F_DEST, F_ARCH], kind: Kind::Leaf(Handler::Stage) },
        Node { name: "u", doc: "Upload the boot config to an ESP", aliases: &["upload", "sync", "push", "up"], flags: &[F_VOL], kind: Kind::Leaf(Handler::EspSync) },
        Node { name: "v", doc: "Version", aliases: &["version", "ver", "rel", "ver2"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::Version) },
        Node { name: "w", doc: "Write a boot config key", aliases: &["write", "put", "set", "edit"], flags: &[F_KEY, F_VALUE], kind: Kind::Leaf(Handler::ConfigSet) },
        Node { name: "x", doc: "Export the config as a profile", aliases: &["export", "xport", "save", "snapshot"], flags: &[F_PROFILE], kind: Kind::Leaf(Handler::ProfileSave) },
        Node { name: "y", doc: "Boot configuration status", aliases: &["status", "why", "state", "report"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::Status) },
        Node { name: "z", doc: "Zap the boot config (reset)", aliases: &["zap", "nuke", "erase", "scrub"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ConfigReset) },
        Node { name: "A", doc: "Alias report (uppercase)", aliases: &["Aliases", "AL", "Abbr"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "list", doc: "List every alias", aliases: &["l", "ls", "all", "dump"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::AliasList) },
        ]) },
        Node { name: "B", doc: "Boot control (uppercase)", aliases: &["Boot2", "bc"], flags: NO_FLAGS, kind: Kind::Ns(boot_tree::BOOT) },
        Node { name: "C", doc: "Boot config (uppercase)", aliases: &["Config", "cfg2", "conf2"], flags: NO_FLAGS, kind: Kind::Ns(config_tree::CONFIG) },
        Node { name: "D", doc: "Default entry (uppercase)", aliases: &["Default", "def2"], flags: NO_FLAGS, kind: Kind::Ns(default_tree::DEFAULT) },
        Node { name: "E", doc: "Entries (uppercase)", aliases: &["Entries", "entry2"], flags: NO_FLAGS, kind: Kind::Ns(entries_tree::ENTRIES) },
        Node { name: "F", doc: "Find (uppercase)", aliases: &["Find2", "locate2"], flags: NO_FLAGS, kind: Kind::Ns(find_tree::FIND) },
        Node { name: "G", doc: "Geometry (uppercase)", aliases: &["Geom", "geo2"], flags: NO_FLAGS, kind: Kind::Ns(geometry_tree::GEOMETRY) },
        Node { name: "H", doc: "Help (uppercase)", aliases: &["Help2", "man2"], flags: NO_FLAGS, kind: Kind::Ns(help_tree::HELP) },
        Node { name: "I", doc: "Boot layout info (uppercase)", aliases: &["Info2", "layout2"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::Info) },
        Node { name: "J", doc: "JSON (uppercase)", aliases: &["Json2", "jq2"], flags: NO_FLAGS, kind: Kind::Ns(json_tree::JSON) },
        Node { name: "K", doc: "Keymap (uppercase)", aliases: &["Keys2", "kb2"], flags: NO_FLAGS, kind: Kind::Ns(keymap_tree::KEYMAP) },
        Node { name: "L", doc: "List (uppercase)", aliases: &["List2", "ls2"], flags: NO_FLAGS, kind: Kind::Ns(list_tree::LIST) },
        Node { name: "N", doc: "Names (uppercase)", aliases: &["Names2", "labels2"], flags: NO_FLAGS, kind: Kind::Ns(names_tree::NAMES) },
        Node { name: "O", doc: "One-shot boot (uppercase)", aliases: &["Once2", "single2"], flags: NO_FLAGS, kind: Kind::Ns(once_tree::ONCE) },
        Node { name: "P", doc: "Profiles (uppercase)", aliases: &["Profile", "prof2"], flags: NO_FLAGS, kind: Kind::Ns(profile_tree::PROFILE) },
        Node { name: "Q", doc: "Queries (uppercase)", aliases: &["Query", "qry2"], flags: NO_FLAGS, kind: Kind::Ns(query_tree::QUERY) },
        Node { name: "R", doc: "Reset (uppercase)", aliases: &["Reset2", "wipe2"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ConfigReset) },
        Node { name: "S", doc: "Stage (uppercase)", aliases: &["Stage2", "stg2"], flags: &[F_DEST, F_ARCH], kind: Kind::Leaf(Handler::Stage) },
        Node { name: "U", doc: "Upload (uppercase)", aliases: &["Upload2", "sync2"], flags: &[F_VOL], kind: Kind::Leaf(Handler::EspSync) },
        Node { name: "V", doc: "Version (uppercase)", aliases: &["Ver2", "v2"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::Version) },
        Node { name: "W", doc: "Write (uppercase)", aliases: &["Write2", "put2"], flags: &[F_KEY, F_VALUE], kind: Kind::Leaf(Handler::ConfigSet) },
        Node { name: "X", doc: "Export profile (uppercase)", aliases: &["Export2", "xport2"], flags: &[F_PROFILE], kind: Kind::Leaf(Handler::ProfileSave) },
        Node { name: "Y", doc: "Status (uppercase)", aliases: &["Status2", "state2"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::Status) },
        Node { name: "Z", doc: "Zap (uppercase)", aliases: &["Zap2", "nuke2"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::ConfigReset) },
    ]),
};

/// Walks the tree printing each canonical path, used by `help tree`.
pub fn walk(node: &Node, prefix: &mut Vec<&'static str>, out: &mut Vec<String>) {
    for child in node.children() {
        prefix.push(child.name);
        out.push(format!(
            "{}{} — {}",
            "  ".repeat(prefix.len() - 1),
            prefix.join(" "),
            child.doc
        ));
        walk(child, prefix, out);
        prefix.pop();
    }
}

/// Renders help for a node: name, doc, aliases, flags, children.
pub fn help_for(node: &Node) -> String {
    let mut s = String::new();
    s.push_str(&format!("{} — {}\n", node.name, node.doc));
    if !node.aliases.is_empty() {
        s.push_str(&format!("aliases: {}\n", node.aliases.join(", ")));
    }
    if !node.flags.is_empty() {
        s.push_str("flags:\n");
        for f in node.flags {
            s.push_str(&format!(
                "  -{}/--{}{}  {}\n",
                f.short,
                f.long,
                if f.value { " <value>" } else { "" },
                f.doc
            ));
        }
    }
    let children = node.children();
    if !children.is_empty() {
        s.push_str("\nsubcommands:\n");
        let mut prefix = Vec::new();
        prefix.push(node.name);
        let mut out = Vec::new();
        walk(node, &mut prefix, &mut out);
        for line in out {
            s.push_str(&line);
            s.push('\n');
        }
    }
    s
}
