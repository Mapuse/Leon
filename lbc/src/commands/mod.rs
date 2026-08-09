//! One module per `lbc` subcommand family, plus the dispatch table from the
//! tree's [`Handler`] ids to the real implementations.
//!
//! Shared host-side helpers (discovery, geometry, utilities) are re-exported
//! from the `lbt` library rather than duplicated here.

use anyhow::{Result, bail};

use crate::cli::args::Parsed;
use crate::cli::tree::{Handler, Node};

pub mod alias;
pub mod boot;
pub mod config;
pub mod misc;

pub use lbt::commands::util;

/// Runs the handler for a leaf node with its parsed arguments.
pub fn dispatch(node: &Node, parsed: Parsed) -> Result<()> {
    let Some(handler) = node.handler() else {
        bail!("`{}` is a namespace, not a command", parsed.command_line());
    };
    match handler {
        Handler::Info => boot::info(),
        Handler::Stage => boot::stage(parsed.value("dest"), parsed.value("arch")),
        Handler::BootOnce => boot::once(parsed.value("name").unwrap_or_default()),
        Handler::EspSync => boot::esp_sync(parsed.value("vol").unwrap_or_default()),
        Handler::ConfigGet => config::get(parsed.value("key")),
        Handler::ConfigSet => config::set(
            parsed.value("key").unwrap_or_default(),
            parsed.value("value").unwrap_or_default(),
        ),
        Handler::ConfigList => config::list(),
        Handler::ConfigReset => config::reset(),
        Handler::ConfigPath => config::path(),
        Handler::DefaultGet => config::default_get(),
        Handler::DefaultSet => config::default_set(parsed.value("name").unwrap_or_default()),
        Handler::EntriesList => misc::entries_list(),
        Handler::EntriesGet => misc::entries_get(parsed.value("name").unwrap_or_default()),
        Handler::EntriesCount => misc::entries_count(),
        Handler::FindEsps => misc::esps(),
        Handler::FindEntries => misc::entries_get(parsed.value("name").unwrap_or_default()),
        Handler::GeometryShow => misc::geometry_show(),
        Handler::GeometryBgr => misc::geometry_bgr(),
        Handler::JsonConfig => misc::json_config(),
        Handler::JsonDiscover => misc::json_discover(),
        Handler::JsonInfo => misc::json_info(),
        Handler::JsonAll => misc::json_all(),
        Handler::NamesList => misc::names_list(),
        Handler::NamesGet => misc::names_get(parsed.value("name").unwrap_or_default()),
        Handler::NamesCount => misc::names_count(),
        Handler::QueryInfo => misc::query_info(),
        Handler::QueryEsps => misc::query_esps(),
        Handler::ProfileSave => config::profile_save(parsed.value("profile").unwrap_or_default()),
        Handler::ProfileList => config::profile_list(),
        Handler::ProfileLoad => config::profile_load(parsed.value("profile").unwrap_or_default()),
        Handler::ProfileDelete => config::profile_delete(parsed.value("profile").unwrap_or_default()),
        Handler::KeymapShow => misc::keymap_show(),
        Handler::Version => misc::version(),
        Handler::HelpAll => misc::help_all(),
        Handler::HelpTree => misc::help_tree(),
        Handler::HelpCommand => misc::help_command(parsed.value("name").unwrap_or_default()),
        Handler::AliasList => alias::list(),
        Handler::Status => misc::status(),
    }
}
