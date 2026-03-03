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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let config = Config::parse_from(["tty-web"]);
        assert_eq!(config.address, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(config.port, 9090);
        assert_eq!(config.shell, "/bin/bash");
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn test_custom_values() {
        let config = Config::parse_from([
            "tty-web",
            "--port", "8080",
            "--shell", "/bin/sh",
            "--address", "0.0.0.0",
            "--log-level", "debug",
        ]);
        assert_eq!(config.address, "0.0.0.0".parse::<IpAddr>().unwrap());
        assert_eq!(config.port, 8080);
        assert_eq!(config.shell, "/bin/sh");
        assert_eq!(config.log_level, "debug");
    }
}
