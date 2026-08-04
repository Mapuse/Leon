//! `lbt config`: read and write the boot config.

use anyhow::{Result, bail};

use crate::boot_config::boot_config;
use crate::cli::ConfigCommand;

pub fn run(cmd: ConfigCommand) -> Result<()> {
    match cmd {
        ConfigCommand::Get { key } => {
            let cfg = boot_config()?;
            match key {
                Some(k) => match cfg.field(&k) {
                    Some(v) => println!("{v}"),
                    None => bail!("no such boot config key: {k}"),
                },
                None => println!("{}", toml::to_string(&cfg).unwrap_or_default().trim_end()),
            }
        }
        ConfigCommand::Set { key, value } => {
            let mut cfg = boot_config()?;
            cfg.set_field(&key, &value)?;
            cfg.save()?;
            println!("{key} = {value}");
        }
    }
    Ok(())
}
