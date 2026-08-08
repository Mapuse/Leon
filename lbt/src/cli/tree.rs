//! The `lbt` command tree.
//!
//! A single static table drives the whole CLI: every node carries a canonical
//! name, up to five aliases, a short help string, its flags, and either child
//! nodes (a namespace, up to 25 levels deep) or a [`Handler`]. The tree is
//! intentionally flat-ish per letter but deep in the real subtrees (`cps`,
//! `image`), and the alphabet `a..z` / `A..Z` is fully populated.
//!
//! Boot configuration and boot control (config, staging, the boot-manager
//! TUI) live in the sibling `lbc` binary.

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
    Discover,
    ImageIso,
    ImageImg,
    ImageEsp,
    LogShow,
    LogTail,
    LogClear,
    LogFind,
    EnvShow,
    EnvCheck,
    EnvTool,
    FsLs,
    FsInfo,
    FsTree,
    RootfsShow,
    RootfsCheck,
    RootfsTree,
    Version,
    HelpAll,
    HelpTree,
    HelpCommand,
    AliasList,
    JsonInfo,
    JsonDiscover,
    JsonAll,
    NamesList,
    NamesGet,
    NamesCount,
    OsRelease,
    OsKernel,
    OsId,
    UsbList,
    UsbFlash,
    UsbDetect,
    FirmwareInfo,
    FirmwareAcpi,
    FirmwareSb,
    BuildAll,
    BuildBoot,
    BuildKernel,
    BuildLbt,
    KeysSetup,
    KeysSign,
    KeysEsl,
    KeysVerify,
    XferEsp,
    XferStage,
    QueryInfo,
    QueryEsps,
    GeometryShow,
    GeometryBgr,
    NetCheck,
    #[cfg(feature = "cps")]
    CpsThemeList,
    #[cfg(feature = "cps")]
    CpsThemeApply,
    #[cfg(feature = "cps")]
    CpsThemeRegister,
    #[cfg(feature = "cps")]
    CpsThemeUnregister,
    #[cfg(feature = "cps")]
    CpsThemeInfo,
    #[cfg(feature = "cps")]
    CpsPluginList,
    #[cfg(feature = "cps")]
    CpsPluginRun,
    #[cfg(feature = "cps")]
    CpsPluginRegister,
    #[cfg(feature = "cps")]
    CpsPluginUnregister,
    #[cfg(feature = "cps")]
    CpsPluginInfo,
    #[cfg(feature = "cps")]
    CpsTuiList,
    #[cfg(feature = "cps")]
    CpsTuiApply,
    #[cfg(feature = "cps")]
    CpsTuiRegister,
    #[cfg(feature = "cps")]
    CpsTuiUnregister,
    #[cfg(feature = "cps")]
    CpsTuiInfo,
    #[cfg(feature = "cps")]
    CpsEngineBoot,
    #[cfg(feature = "cps")]
    CpsConfigGet,
    #[cfg(feature = "cps")]
    CpsConfigSet,
    #[cfg(feature = "cps")]
    CpsConfigPath,
    #[cfg(feature = "cps")]
    CpsConfigLoad,
    #[cfg(feature = "cps")]
    CpsStatus,
}

impl Handler {
    /// Whether this handler belongs to the feature-gated `cps` subtree.
    pub fn is_cps(&self) -> bool {
        match self {
            #[cfg(feature = "cps")]
            Self::CpsThemeList
            | Self::CpsThemeApply
            | Self::CpsThemeRegister
            | Self::CpsThemeUnregister
            | Self::CpsThemeInfo
            | Self::CpsPluginList
            | Self::CpsPluginRun
            | Self::CpsPluginRegister
            | Self::CpsPluginUnregister
            | Self::CpsPluginInfo
            | Self::CpsTuiList
            | Self::CpsTuiApply
            | Self::CpsTuiRegister
            | Self::CpsTuiUnregister
            | Self::CpsTuiInfo
            | Self::CpsEngineBoot
            | Self::CpsConfigGet
            | Self::CpsConfigSet
            | Self::CpsConfigPath
            | Self::CpsConfigLoad
            | Self::CpsStatus => true,
            _ => false,
        }
    }
}

// ── Global flag vocabulary ──────────────────────────────────────────────────
pub const F_JSON: Flag = Flag { short: 'j', long: "json", value: false, doc: "Emit machine-readable JSON" };
pub const F_PATH: Flag = Flag { short: 'p', long: "path", value: true, doc: "File or directory path" };
pub const F_OUT: Flag = Flag { short: 'o', long: "out", value: true, doc: "Output file" };
pub const F_ROOTFS: Flag = Flag { short: 'r', long: "rootfs", value: true, doc: "Root filesystem directory" };
pub const F_NAME: Flag = Flag { short: 'n', long: "name", value: true, doc: "Entry name" };
pub const F_DEST: Flag = Flag { short: 'd', long: "dest", value: true, doc: "Destination directory" };
pub const F_ARCH: Flag = Flag { short: 'a', long: "arch", value: true, doc: "Architecture: amd64 | arm64" };
pub const F_CONFIG: Flag = Flag { short: 'c', long: "config", value: true, doc: "Config file path" };
pub const F_KEYDIR: Flag = Flag { short: 'k', long: "keydir", value: true, doc: "Secure Boot key directory" };
pub const F_LABEL: Flag = Flag { short: 'l', long: "label", value: true, doc: "Volume label" };
pub const F_SIZE: Flag = Flag { short: 's', long: "size", value: true, doc: "Size in MiB" };
pub const F_DEPTH: Flag = Flag { short: 'D', long: "depth", value: true, doc: "Traversal depth" };
pub const F_PATTERN: Flag = Flag { short: 'f', long: "pattern", value: true, doc: "Search pattern" };
pub const F_DEVICE: Flag = Flag { short: 'e', long: "device", value: true, doc: "Block device path" };
pub const F_SOURCE: Flag = Flag { short: 'g', long: "source", value: true, doc: "Source path" };
pub const F_DISPLAY: Flag = Flag { short: 'x', long: "display", value: true, doc: "Display name" };
pub const F_DESC: Flag = Flag { short: 'y', long: "desc", value: true, doc: "Description" };
pub const F_FUNC: Flag = Flag { short: 'F', long: "func", value: true, doc: "Plugin hook function" };
pub const F_ALIASES: Flag = Flag { short: 'A', long: "aliases", value: true, doc: "Comma-separated aliases" };
pub const F_KEY: Flag = Flag { short: 'K', long: "key", value: true, doc: "Config key" };
pub const F_VALUE: Flag = Flag { short: 'V', long: "value", value: true, doc: "Config value" };
pub const F_CERT: Flag = Flag { short: 'C', long: "cert", value: true, doc: "Certificate PEM path" };

/// Flags that consume a value; consulted by the tokenizer before path
/// resolution. Every short character is unique, so this union is unambiguous.
pub const VALUE_FLAGS: &[Flag] = &[
    F_PATH, F_OUT, F_ROOTFS, F_NAME, F_DEST, F_ARCH, F_CONFIG, F_KEYDIR, F_LABEL, F_SIZE, F_DEPTH,
    F_PATTERN, F_DEVICE, F_SOURCE, F_DISPLAY, F_DESC, F_FUNC, F_ALIASES, F_KEY, F_VALUE, F_CERT,
];

const NO_FLAGS: &[Flag] = &[];

// ── cps subtree (feature-gated) ─────────────────────────────────────────────
#[cfg(feature = "cps")]
mod cps_tree {
    use super::*;

    pub const THEME: &[Node] = &[
        Node { name: "list", doc: "List registered themes", aliases: &["l", "ls", "enumerate", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::CpsThemeList) },
        Node { name: "apply", doc: "Apply a theme", aliases: &["a", "use", "set", "enable"], flags: &[F_NAME], kind: Kind::Leaf(Handler::CpsThemeApply) },
        Node { name: "register", doc: "Register a theme", aliases: &["r", "reg", "add", "install"], flags: &[F_NAME, F_PATH, F_DISPLAY, F_DESC], kind: Kind::Leaf(Handler::CpsThemeRegister) },
        Node { name: "unregister", doc: "Unregister a theme", aliases: &["u", "del", "remove", "drop"], flags: &[F_NAME], kind: Kind::Leaf(Handler::CpsThemeUnregister) },
        Node { name: "info", doc: "Describe a theme", aliases: &["i", "show", "describe", "get"], flags: &[F_NAME], kind: Kind::Leaf(Handler::CpsThemeInfo) },
    ];

    pub const PLUGIN: &[Node] = &[
        Node { name: "list", doc: "List registered plugins", aliases: &["l", "ls", "enumerate", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::CpsPluginList) },
        Node { name: "run", doc: "Run a plugin alias or hook", aliases: &["r", "exec", "fire", "invoke"], flags: &[F_NAME, F_FUNC], kind: Kind::Leaf(Handler::CpsPluginRun) },
        Node { name: "register", doc: "Register a plugin", aliases: &["r", "reg", "add", "install"], flags: &[F_NAME, F_PATH, F_ALIASES], kind: Kind::Leaf(Handler::CpsPluginRegister) },
        Node { name: "unregister", doc: "Unregister a plugin", aliases: &["u", "del", "remove", "drop"], flags: &[F_NAME], kind: Kind::Leaf(Handler::CpsPluginUnregister) },
        Node { name: "info", doc: "Describe a plugin", aliases: &["i", "show", "describe", "get"], flags: &[F_NAME], kind: Kind::Leaf(Handler::CpsPluginInfo) },
    ];

    pub const TUI: &[Node] = &[
        Node { name: "list", doc: "List registered TUIs", aliases: &["l", "ls", "enumerate", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::CpsTuiList) },
        Node { name: "apply", doc: "Apply a TUI", aliases: &["a", "use", "set", "enable"], flags: &[F_NAME], kind: Kind::Leaf(Handler::CpsTuiApply) },
        Node { name: "register", doc: "Register a TUI", aliases: &["r", "reg", "add", "install"], flags: &[F_NAME, F_PATH, F_DISPLAY, F_DESC], kind: Kind::Leaf(Handler::CpsTuiRegister) },
        Node { name: "unregister", doc: "Unregister a TUI", aliases: &["u", "del", "remove", "drop"], flags: &[F_NAME], kind: Kind::Leaf(Handler::CpsTuiUnregister) },
        Node { name: "info", doc: "Describe a TUI", aliases: &["i", "show", "describe", "get"], flags: &[F_NAME], kind: Kind::Leaf(Handler::CpsTuiInfo) },
    ];

    pub const ENGINE: &[Node] = &[
        Node { name: "boot", doc: "Boot the cps engine against a config", aliases: &["b", "start", "init", "run"], flags: &[F_CONFIG], kind: Kind::Leaf(Handler::CpsEngineBoot) },
    ];

    pub const CFG: &[Node] = &[
        Node { name: "get", doc: "Read a cps config key", aliases: &["g", "gt", "fetch", "read"], flags: &[F_KEY], kind: Kind::Leaf(Handler::CpsConfigGet) },
        Node { name: "set", doc: "Write a cps config key", aliases: &["s", "st", "write", "put"], flags: &[F_KEY, F_VALUE], kind: Kind::Leaf(Handler::CpsConfigSet) },
        Node { name: "path", doc: "Print the resolved cps config path", aliases: &["p", "loc", "where", "find"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::CpsConfigPath) },
        Node { name: "load", doc: "Parse a cps config file", aliases: &["l", "open", "read", "parse"], flags: &[F_PATH], kind: Kind::Leaf(Handler::CpsConfigLoad) },
    ];

    pub const CPS: &[Node] = &[
        Node { name: "theme", doc: "Manage cps themes", aliases: &["t", "th", "thm"], flags: NO_FLAGS, kind: Kind::Ns(THEME) },
        Node { name: "plugin", doc: "Manage cps plugins", aliases: &["p", "pl", "plug"], flags: NO_FLAGS, kind: Kind::Ns(PLUGIN) },
        Node { name: "tui", doc: "Manage cps TUIs", aliases: &["u", "ui", "tu"], flags: NO_FLAGS, kind: Kind::Ns(TUI) },
        Node { name: "engine", doc: "Boot the cps engine", aliases: &["e", "eng", "runtime"], flags: NO_FLAGS, kind: Kind::Ns(ENGINE) },
        Node { name: "config", doc: "Inspect cps configuration", aliases: &["c", "cfg", "conf"], flags: NO_FLAGS, kind: Kind::Ns(CFG) },
        Node { name: "status", doc: "Report cps engine status", aliases: &["s", "st", "state", "check"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::CpsStatus) },
    ];
}

// ── image subtree ───────────────────────────────────────────────────────────
mod image_tree {
    use super::*;

    pub const IMAGE: &[Node] = &[
        Node { name: "iso", doc: "Build a bootable ISO from a rootfs", aliases: &["i", "cd", "optical", "disc"], flags: &[F_ROOTFS, F_OUT, F_LABEL], kind: Kind::Leaf(Handler::ImageIso) },
        Node { name: "img", doc: "Build a GPT disk image from a rootfs", aliases: &["g", "disk", "raw", "hdd"], flags: &[F_ROOTFS, F_OUT, F_SIZE], kind: Kind::Leaf(Handler::ImageImg) },
        Node { name: "esp", doc: "Build an ESP image from a rootfs", aliases: &["e", "efi", "fat", "part"], flags: &[F_ROOTFS, F_OUT, F_SIZE], kind: Kind::Leaf(Handler::ImageEsp) },
    ];
}

// ── fs / rootfs subtrees ────────────────────────────────────────────────────
mod fs_tree {
    use super::*;

    pub const FS: &[Node] = &[
        Node { name: "ls", doc: "List a directory", aliases: &["l", "list", "browse", "walk"], flags: &[F_PATH], kind: Kind::Leaf(Handler::FsLs) },
        Node { name: "info", doc: "Stat a file or directory", aliases: &["i", "stat", "describe", "detail"], flags: &[F_PATH], kind: Kind::Leaf(Handler::FsInfo) },
        Node { name: "tree", doc: "Print a directory tree", aliases: &["t", "depth", "dump", "hierarchy"], flags: &[F_PATH, F_DEPTH], kind: Kind::Leaf(Handler::FsTree) },
    ];

    pub const ROOTFS: &[Node] = &[
        Node { name: "show", doc: "Describe a rootfs", aliases: &["s", "display", "describe", "dump"], flags: &[F_PATH], kind: Kind::Leaf(Handler::RootfsShow) },
        Node { name: "check", doc: "Validate a rootfs layout", aliases: &["c", "verify", "validate", "test"], flags: &[F_PATH], kind: Kind::Leaf(Handler::RootfsCheck) },
        Node { name: "tree", doc: "Inventory a rootfs", aliases: &["t", "list", "walk", "inventory"], flags: &[F_PATH, F_DEPTH], kind: Kind::Leaf(Handler::RootfsTree) },
    ];
}

// ── log subtree ─────────────────────────────────────────────────────────────
mod log_tree {
    use super::*;

    pub const LOG: &[Node] = &[
        Node { name: "show", doc: "Print the Leon boot log", aliases: &["s", "view", "cat", "print"], flags: &[F_PATH], kind: Kind::Leaf(Handler::LogShow) },
        Node { name: "tail", doc: "Tail the boot log (polling)", aliases: &["t", "follow", "watch", "stream"], flags: &[F_PATH], kind: Kind::Leaf(Handler::LogTail) },
        Node { name: "clear", doc: "Truncate the boot log", aliases: &["c", "wipe", "truncate", "empty"], flags: &[F_PATH], kind: Kind::Leaf(Handler::LogClear) },
        Node { name: "find", doc: "Search the boot log", aliases: &["f", "grep", "search", "scan"], flags: &[F_PATTERN, F_PATH], kind: Kind::Leaf(Handler::LogFind) },
    ];
}

// ── keys subtree ────────────────────────────────────────────────────────────
mod keys_tree {
    use super::*;

    pub const KEYS: &[Node] = &[
        Node { name: "setup", doc: "Generate a Secure Boot key set", aliases: &["s", "gen", "create", "init"], flags: &[F_KEYDIR], kind: Kind::Leaf(Handler::KeysSetup) },
        Node { name: "sign", doc: "Sign EFI images with the db key", aliases: &["g", "sbsign", "certify", "seal"], flags: &[F_KEYDIR, F_PATH, F_OUT], kind: Kind::Leaf(Handler::KeysSign) },
        Node { name: "esl", doc: "Emit an .esl for a certificate", aliases: &["e", "enroll", "cert", "emit"], flags: &[F_CERT], kind: Kind::Leaf(Handler::KeysEsl) },
        Node { name: "verify", doc: "Verify an EFI image signature", aliases: &["v", "check", "sbverify", "validate"], flags: &[F_CERT, F_PATH], kind: Kind::Leaf(Handler::KeysVerify) },
    ];
}

// ── build subtree ───────────────────────────────────────────────────────────
mod build_tree {
    use super::*;

    pub const BUILD: &[Node] = &[
        Node { name: "all", doc: "Build bootloader + kernel + lbt", aliases: &["a", "full", "everything", "default"], flags: &[F_ARCH], kind: Kind::Leaf(Handler::BuildAll) },
        Node { name: "boot", doc: "Build the bootloader", aliases: &["b", "loader", "lbl", "bl"], flags: &[F_ARCH], kind: Kind::Leaf(Handler::BuildBoot) },
        Node { name: "kernel", doc: "Build the EFI-stub kernel", aliases: &["k", "kbl", "stub", "kern"], flags: &[F_ARCH], kind: Kind::Leaf(Handler::BuildKernel) },
        Node { name: "lbt", doc: "Build the lbt host tool", aliases: &["l", "tool", "host", "builder"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::BuildLbt) },
    ];

    pub const XFER: &[Node] = &[
        Node { name: "esp", doc: "Copy the staged ESP to a volume", aliases: &["e", "install", "deploy", "sync"], flags: &[F_SOURCE, F_DEST], kind: Kind::Leaf(Handler::XferEsp) },
        Node { name: "stage", doc: "Copy a staged tree to a directory", aliases: &["s", "copy", "prepare", "sync"], flags: &[F_SOURCE, F_DEST], kind: Kind::Leaf(Handler::XferStage) },
    ];
}

// ── root tree: alphabet + real subtrees ─────────────────────────────────────
pub static ROOT: Node = Node {
    name: "lbt",
    doc: "Leon Build Tool — discover boot entries, build ESP/ISO/IMG images, query geometry, control the cps subsystem",
    aliases: &[],
    flags: &[],
    kind: Kind::Ns(&[
        Node { name: "a", doc: "Alias reports", aliases: &["alias", "aliases", "abbr", "all"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "list", doc: "List every command alias", aliases: &["l", "ls", "all", "dump"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::AliasList) },
        ]) },
        #[cfg(feature = "cps")]
        Node { name: "c", doc: "Control the cps subsystem (themes, TUIs, plugins)", aliases: &["cps", "py", "subsystem"], flags: NO_FLAGS, kind: Kind::Ns(cps_tree::CPS) },
        Node { name: "d", doc: "Discover ESPs and boot entries", aliases: &["discover", "disc", "scan", "detect"], flags: &[F_JSON], kind: Kind::Leaf(Handler::Discover) },
        Node { name: "e", doc: "Environment and diagnostics", aliases: &["env", "environment", "diag"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "show", doc: "Print the environment", aliases: &["s", "print", "dump", "all"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::EnvShow) },
            Node { name: "check", doc: "Verify host prerequisites", aliases: &["c", "verify", "test", "validate"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::EnvCheck) },
            Node { name: "tool", doc: "Locate a host tool", aliases: &["t", "which", "path", "find"], flags: &[F_NAME], kind: Kind::Leaf(Handler::EnvTool) },
        ]) },
        Node { name: "f", doc: "Filesystem helpers", aliases: &["fs", "files", "filesystem"], flags: NO_FLAGS, kind: Kind::Ns(fs_tree::FS) },
        Node { name: "g", doc: "Framebuffer geometry", aliases: &["geometry", "geom", "geo", "fb"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "show", doc: "Show framebuffer + BGRT geometry", aliases: &["s", "print", "dump", "info"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::GeometryShow) },
            Node { name: "bgr", doc: "Show BGRT logo geometry", aliases: &["b", "logo", "bgrt", "splash"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::GeometryBgr) },
        ]) },
        Node { name: "h", doc: "Help", aliases: &["help", "?", "man", "usage"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "all", doc: "Print the full command reference", aliases: &["a", "full", "everything", "overview"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::HelpAll) },
            Node { name: "tree", doc: "Print the command tree", aliases: &["t", "graph", "structure", "outline"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::HelpTree) },
            Node { name: "command", doc: "Describe one command", aliases: &["c", "cmd", "about", "explain"], flags: &[F_NAME], kind: Kind::Leaf(Handler::HelpCommand) },
        ]) },
        Node { name: "i", doc: "Print framebuffer + BGRT geometry", aliases: &["info", "inspect", "show", "geom"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::Info) },
        Node { name: "j", doc: "JSON output helpers", aliases: &["json", "jq", "machine"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "discover", doc: "Discovery as JSON", aliases: &["d", "disc", "scan", "detect"], flags: &[F_JSON], kind: Kind::Leaf(Handler::JsonDiscover) },
            Node { name: "info", doc: "Geometry as JSON", aliases: &["i", "geom", "geometry", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::JsonInfo) },
            Node { name: "all", doc: "Everything as JSON", aliases: &["a", "full", "everything", "complete"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::JsonAll) },
        ]) },
        Node { name: "l", doc: "Boot log", aliases: &["log", "logs", "journal"], flags: NO_FLAGS, kind: Kind::Ns(log_tree::LOG) },
        Node { name: "n", doc: "Boot entry names", aliases: &["names", "entries", "nodes", "paths"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "list", doc: "List boot entries", aliases: &["l", "ls", "enumerate", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::NamesList) },
            Node { name: "get", doc: "Resolve a boot entry by label", aliases: &["g", "find", "lookup", "resolve"], flags: &[F_NAME], kind: Kind::Leaf(Handler::NamesGet) },
            Node { name: "count", doc: "Count boot entries", aliases: &["c", "n", "tally", "total"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::NamesCount) },
        ]) },
        Node { name: "o", doc: "Host OS information", aliases: &["os", "osinfo", "release", "uname"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "release", doc: "Print OS release info", aliases: &["r", "rel", "info", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::OsRelease) },
            Node { name: "kernel", doc: "Print the running kernel", aliases: &["k", "kver", "uname", "version"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::OsKernel) },
            Node { name: "id", doc: "Print the OS pretty name", aliases: &["i", "name", "pretty", "label"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::OsId) },
        ]) },
        #[cfg(feature = "cps")]
        Node { name: "p", doc: "cps plugin control", aliases: &["plugin", "plg", "pl", "pyplug"], flags: NO_FLAGS, kind: Kind::Ns(cps_tree::PLUGIN) },
        Node { name: "q", doc: "Queries", aliases: &["query", "qry", "ask", "inspect2"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "info", doc: "Query geometry", aliases: &["i", "geom", "geometry", "fb"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::QueryInfo) },
            Node { name: "esps", doc: "Query ESP volumes", aliases: &["e", "discover", "scan", "volumes"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::QueryEsps) },
        ]) },
        Node { name: "r", doc: "Rootfs helpers", aliases: &["rootfs", "fsroot", "rootsys", "sysroot"], flags: NO_FLAGS, kind: Kind::Ns(fs_tree::ROOTFS) },
        Node { name: "u", doc: "USB / ESP flash helpers", aliases: &["usb", "stick", "flash", "burn"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "list", doc: "List USB / block devices", aliases: &["l", "ls", "enumerate", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::UsbList) },
            Node { name: "flash", doc: "Flash an image to a device", aliases: &["f", "write", "burn", "install"], flags: &[F_DEVICE, F_PATH], kind: Kind::Leaf(Handler::UsbFlash) },
            Node { name: "detect", doc: "Detect removable devices", aliases: &["d", "find", "scan", "probe"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::UsbDetect) },
        ]) },
        Node { name: "v", doc: "Version", aliases: &["version", "ver", "rel", "ver2"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::Version) },
        Node { name: "w", doc: "Write disk images", aliases: &["write", "build", "emit", "gen"], flags: NO_FLAGS, kind: Kind::Ns(image_tree::IMAGE) },
        Node { name: "x", doc: "Build ISOs", aliases: &["iso", "cd", "optical"], flags: &[F_ROOTFS, F_OUT, F_LABEL], kind: Kind::Leaf(Handler::ImageIso) },
        #[cfg(feature = "cps")]
        Node { name: "y", doc: "cps theme control", aliases: &["theme", "thm", "style", "skin"], flags: NO_FLAGS, kind: Kind::Ns(cps_tree::THEME) },
        Node { name: "z", doc: "Firmware information", aliases: &["zone", "fw", "firmware", "bios"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "info", doc: "Describe the firmware", aliases: &["i", "show", "describe", "detail"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::FirmwareInfo) },
            Node { name: "secureboot", doc: "Report Secure Boot state", aliases: &["s", "sb", "sec", "verify"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::FirmwareSb) },
            Node { name: "acpi", doc: "Report ACPI tables", aliases: &["a", "tables", "rsdp", "dt"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::FirmwareAcpi) },
        ]) },
        Node { name: "A", doc: "Alias report (uppercase)", aliases: &["Aliases", "AL", "Abbr"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "list", doc: "List every alias", aliases: &["l", "ls", "all", "dump"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::AliasList) },
        ]) },
        Node { name: "B", doc: "Build front-ends", aliases: &["Build", "bld", "make"], flags: NO_FLAGS, kind: Kind::Ns(build_tree::BUILD) },
        Node { name: "D", doc: "Devices", aliases: &["Devices", "dev", "hw"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "list", doc: "List block devices", aliases: &["l", "ls", "enumerate", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::UsbList) },
            Node { name: "esps", doc: "List ESP partitions", aliases: &["e", "efi", "partitions", "esp"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::QueryEsps) },
        ]) },
        Node { name: "E", doc: "ESP helpers", aliases: &["Esp", "efi", "fat32"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "make", doc: "Build an ESP image", aliases: &["m", "build", "create", "gen"], flags: &[F_OUT, F_ARCH], kind: Kind::Leaf(Handler::ImageEsp) },
            Node { name: "check", doc: "Check an ESP volume", aliases: &["c", "verify", "validate", "fsck"], flags: &[F_PATH], kind: Kind::Leaf(Handler::FsInfo) },
            Node { name: "find", doc: "Find ESP volumes", aliases: &["f", "locate", "search", "discover"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::QueryEsps) },
        ]) },
        Node { name: "F", doc: "Firmware (uppercase)", aliases: &["Firmware", "fw", "uefi"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "info", doc: "Describe the firmware", aliases: &["i", "show", "describe", "detail"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::FirmwareInfo) },
            Node { name: "sb", doc: "Secure Boot state", aliases: &["s", "secureboot", "verify", "status"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::FirmwareSb) },
        ]) },
        Node { name: "G", doc: "Geometry (uppercase)", aliases: &["Geom", "geo2"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "show", doc: "Show geometry", aliases: &["s", "print", "dump", "info"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::GeometryShow) },
        ]) },
        Node { name: "H", doc: "Help (uppercase)", aliases: &["Help2", "man2"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "all", doc: "Full reference", aliases: &["a", "full", "everything", "overview"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::HelpAll) },
            Node { name: "tree", doc: "Command tree", aliases: &["t", "graph", "structure", "outline"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::HelpTree) },
        ]) },
        Node { name: "I", doc: "Image building", aliases: &["Image", "img", "diskimg"], flags: NO_FLAGS, kind: Kind::Ns(image_tree::IMAGE) },
        Node { name: "J", doc: "JSON (uppercase)", aliases: &["Json2", "jq2"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "discover", doc: "Discovery as JSON", aliases: &["d", "disc", "scan", "detect"], flags: &[F_JSON], kind: Kind::Leaf(Handler::JsonDiscover) },
            Node { name: "info", doc: "Geometry as JSON", aliases: &["i", "geom", "geometry", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::JsonInfo) },
        ]) },
        Node { name: "K", doc: "Secure Boot keys", aliases: &["Keys", "key", "sign"], flags: NO_FLAGS, kind: Kind::Ns(keys_tree::KEYS) },
        Node { name: "L", doc: "Logs (uppercase)", aliases: &["Logs2", "journal"], flags: NO_FLAGS, kind: Kind::Ns(log_tree::LOG) },
        Node { name: "M", doc: "Make front-end", aliases: &["Make", "mk", "front"], flags: NO_FLAGS, kind: Kind::Ns(build_tree::BUILD) },
        Node { name: "N", doc: "Network", aliases: &["Net", "network", "web"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "check", doc: "Check network reachability", aliases: &["c", "ping", "reach", "online"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::NetCheck) },
        ]) },
        Node { name: "O", doc: "OS info (uppercase)", aliases: &["Os2", "osinfo2"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "release", doc: "OS release", aliases: &["r", "rel", "info", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::OsRelease) },
            Node { name: "id", doc: "Pretty name", aliases: &["i", "name", "pretty", "label"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::OsId) },
        ]) },
        #[cfg(feature = "cps")]
        Node { name: "P", doc: "Plugins (uppercase)", aliases: &["Plugins", "Plug"], flags: NO_FLAGS, kind: Kind::Ns(cps_tree::PLUGIN) },
        Node { name: "Q", doc: "Queries (uppercase)", aliases: &["Query", "qry2"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "info", doc: "Query geometry", aliases: &["i", "geom", "geometry", "fb"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::QueryInfo) },
            Node { name: "esps", doc: "Query ESP volumes", aliases: &["e", "discover", "scan", "volumes"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::QueryEsps) },
        ]) },
        Node { name: "R", doc: "Rootfs (uppercase)", aliases: &["Rootfs2", "rootsys2"], flags: NO_FLAGS, kind: Kind::Ns(fs_tree::ROOTFS) },
        Node { name: "S", doc: "Secure Boot signing", aliases: &["Sign", "sb2", "keys2"], flags: NO_FLAGS, kind: Kind::Ns(keys_tree::KEYS) },
        Node { name: "U", doc: "USB (uppercase)", aliases: &["Usb2", "flash2"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "list", doc: "List devices", aliases: &["l", "ls", "enumerate", "show"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::UsbList) },
            Node { name: "flash", doc: "Flash an image", aliases: &["f", "write", "burn", "install"], flags: &[F_DEVICE, F_PATH], kind: Kind::Leaf(Handler::UsbFlash) },
        ]) },
        Node { name: "V", doc: "Version (uppercase)", aliases: &["Ver2", "v2"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::Version) },
        Node { name: "W", doc: "Write images (uppercase)", aliases: &["Write2", "gen2"], flags: NO_FLAGS, kind: Kind::Ns(image_tree::IMAGE) },
        Node { name: "X", doc: "Transfer files", aliases: &["Xfer", "transfer", "copy"], flags: NO_FLAGS, kind: Kind::Ns(build_tree::XFER) },
        #[cfg(feature = "cps")]
        Node { name: "Y", doc: "Theme control (uppercase)", aliases: &["Theme2", "thm2"], flags: NO_FLAGS, kind: Kind::Ns(cps_tree::THEME) },
        Node { name: "Z", doc: "Zones / firmware (uppercase)", aliases: &["Zones", "fw2"], flags: NO_FLAGS, kind: Kind::Ns(&[
            Node { name: "info", doc: "Firmware info", aliases: &["i", "show", "describe", "detail"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::FirmwareInfo) },
            Node { name: "secureboot", doc: "Secure Boot state", aliases: &["s", "sb", "sec", "verify"], flags: NO_FLAGS, kind: Kind::Leaf(Handler::FirmwareSb) },
        ]) },
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
