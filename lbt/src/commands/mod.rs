//! One module per `lbt` subcommand family, plus the dispatch table from the
//! tree's [`Handler`] ids to the real implementations. Boot configuration and
//! boot control (config, staging, the boot-manager TUI) live in the sibling
//! `lbc` binary.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::cli::args::Parsed;
use crate::cli::tree::{Handler, Node};

pub mod alias;
pub mod build;
#[cfg(feature = "cps")]
pub mod cps;
pub mod discover;
pub mod fs;
pub mod image;
pub mod info;
pub mod log;
pub mod misc;
pub mod sys;

pub mod util;

mod keys;

/// Runs the handler for a leaf node with its parsed arguments.
pub fn dispatch(node: &Node, parsed: Parsed) -> Result<()> {
    let Some(handler) = node.handler() else {
        bail!("`{}` is a namespace, not a command", parsed.command_line());
    };
    match handler {
        Handler::Info => info::run(parsed.pos(0).map(PathBuf::from)),
        Handler::Discover => discover::run(parsed.flag("json")),
        Handler::ImageIso => image::iso(
            parsed.value("rootfs"),
            parsed.value("out"),
            parsed.value("label"),
        ),
        Handler::ImageImg => image::img(
            parsed.value("rootfs"),
            parsed.value("out"),
            parsed.value("size"),
        ),
        Handler::ImageEsp => image::esp(
            parsed.value("rootfs"),
            parsed.value("out"),
            parsed.value("size"),
            parsed.value("arch"),
        ),
        Handler::LogShow => log::show(parsed.value("path")),
        Handler::LogTail => log::tail(parsed.value("path")),
        Handler::LogClear => log::clear(parsed.value("path")),
        Handler::LogFind => log::find(parsed.value("pattern"), parsed.value("path")),
        Handler::EnvShow => sys::env_show(),
        Handler::EnvCheck => sys::env_check(),
        Handler::EnvTool => sys::env_tool(parsed.value("name").unwrap_or_default()),
        Handler::FsLs => fs::ls(parsed.value("path")),
        Handler::FsInfo => fs::info(parsed.value("path")),
        Handler::FsTree => fs::tree(parsed.value("path"), parsed.value("depth")),
        Handler::RootfsShow => fs::rootfs_show(parsed.value("path")),
        Handler::RootfsCheck => fs::rootfs_check(parsed.value("path")),
        Handler::RootfsTree => fs::rootfs_tree(parsed.value("path"), parsed.value("depth")),
        Handler::Version => misc::version(),
        Handler::HelpAll => misc::help_all(),
        Handler::HelpTree => misc::help_tree(),
        Handler::HelpCommand => misc::help_command(parsed.value("name").unwrap_or_default()),
        Handler::AliasList => alias::list(),
        Handler::JsonInfo => misc::json_info(),
        Handler::JsonDiscover => misc::json_discover(),
        Handler::JsonAll => misc::json_all(),
        Handler::NamesList => misc::names_list(),
        Handler::NamesGet => misc::names_get(parsed.value("name").unwrap_or_default()),
        Handler::NamesCount => misc::names_count(),
        Handler::OsRelease => sys::os_release(),
        Handler::OsKernel => sys::os_kernel(),
        Handler::OsId => sys::os_id(),
        Handler::UsbList => sys::usb_list(),
        Handler::UsbFlash => sys::usb_flash(
            parsed.value("device").unwrap_or_default(),
            parsed.value("path").unwrap_or_default(),
        ),
        Handler::UsbDetect => sys::usb_detect(),
        Handler::FirmwareInfo => sys::firmware_info(),
        Handler::FirmwareAcpi => sys::firmware_acpi(),
        Handler::FirmwareSb => sys::firmware_sb(),
        Handler::BuildAll => build::all(parsed.value("arch")),
        Handler::BuildBoot => build::boot(parsed.value("arch")),
        Handler::BuildKernel => build::kernel(parsed.value("arch")),
        Handler::BuildLbt => build::lbt(),
        Handler::KeysSetup => keys::setup(parsed.value("keydir")),
        Handler::KeysSign => keys::sign(
            parsed.value("keydir"),
            parsed.value("path"),
            parsed.value("out"),
        ),
        Handler::KeysEsl => keys::esl(parsed.value("cert").unwrap_or_default()),
        Handler::KeysVerify => keys::verify(
            parsed.value("cert").unwrap_or_default(),
            parsed.value("path").unwrap_or_default(),
        ),
        Handler::XferEsp => build::xfer_esp(parsed.value("source"), parsed.value("dest")),
        Handler::XferStage => build::xfer_stage(parsed.value("source"), parsed.value("dest")),
        Handler::QueryInfo => misc::query_info(),
        Handler::QueryEsps => misc::query_esps(),
        Handler::GeometryShow => misc::geometry_show(),
        Handler::GeometryBgr => misc::geometry_bgr(),
        Handler::NetCheck => sys::net_check(),
        #[cfg(feature = "cps")]
        Handler::CpsThemeList => cps::theme_list(),
        #[cfg(feature = "cps")]
        Handler::CpsThemeApply => cps::theme_apply(parsed.value("name").unwrap_or_default()),
        #[cfg(feature = "cps")]
        Handler::CpsThemeRegister => cps::theme_register(
            parsed.value("name").unwrap_or_default(),
            parsed.value("path"),
            parsed.value("display"),
            parsed.value("desc"),
        ),
        #[cfg(feature = "cps")]
        Handler::CpsThemeUnregister => cps::theme_unregister(parsed.value("name").unwrap_or_default()),
        #[cfg(feature = "cps")]
        Handler::CpsThemeInfo => cps::theme_info(parsed.value("name").unwrap_or_default()),
        #[cfg(feature = "cps")]
        Handler::CpsPluginList => cps::plugin_list(),
        #[cfg(feature = "cps")]
        Handler::CpsPluginRun => cps::plugin_run(
            parsed.value("name").unwrap_or_default(),
            parsed.value("func").unwrap_or_default(),
            &parsed.positional,
        ),
        #[cfg(feature = "cps")]
        Handler::CpsPluginRegister => cps::plugin_register(
            parsed.value("name").unwrap_or_default(),
            parsed.value("path"),
            parsed.value("aliases"),
        ),
        #[cfg(feature = "cps")]
        Handler::CpsPluginUnregister => cps::plugin_unregister(parsed.value("name").unwrap_or_default()),
        #[cfg(feature = "cps")]
        Handler::CpsPluginInfo => cps::plugin_info(parsed.value("name").unwrap_or_default()),
        #[cfg(feature = "cps")]
        Handler::CpsTuiList => cps::tui_list(),
        #[cfg(feature = "cps")]
        Handler::CpsTuiApply => cps::tui_apply(parsed.value("name").unwrap_or_default()),
        #[cfg(feature = "cps")]
        Handler::CpsTuiRegister => cps::tui_register(
            parsed.value("name").unwrap_or_default(),
            parsed.value("path"),
            parsed.value("display"),
            parsed.value("desc"),
        ),
        #[cfg(feature = "cps")]
        Handler::CpsTuiUnregister => cps::tui_unregister(parsed.value("name").unwrap_or_default()),
        #[cfg(feature = "cps")]
        Handler::CpsTuiInfo => cps::tui_info(parsed.value("name").unwrap_or_default()),
        #[cfg(feature = "cps")]
        Handler::CpsEngineBoot => cps::engine_boot(parsed.value("config")),
        #[cfg(feature = "cps")]
        Handler::CpsConfigGet => cps::config_get(parsed.value("key").unwrap_or_default()),
        #[cfg(feature = "cps")]
        Handler::CpsConfigSet => cps::config_set(
            parsed.value("key").unwrap_or_default(),
            parsed.value("value").unwrap_or_default(),
        ),
        #[cfg(feature = "cps")]
        Handler::CpsConfigPath => cps::config_path(),
        #[cfg(feature = "cps")]
        Handler::CpsConfigLoad => cps::config_load(parsed.value("path")),
        #[cfg(feature = "cps")]
        Handler::CpsStatus => cps::status(),
    }
}
