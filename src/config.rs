use std::net::IpAddr;

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "tty-web", about = "Web-based terminal emulator")]
pub struct Config {
    /// Address to bind to
    #[arg(long, default_value = "127.0.0.1", env = "TTY_WEB_ADDRESS")]
    pub address: IpAddr,

    /// Port to listen on
    #[arg(long, default_value_t = 9090, env = "TTY_WEB_PORT")]
    pub port: u16,

    /// Shell to execute
    #[arg(long, default_value = "/bin/bash", env = "TTY_WEB_SHELL")]
    pub shell: String,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info", env = "TTY_WEB_LOG_LEVEL")]
    pub log_level: String,
}
