use crate::config::Config;

pub struct BootContext {
    pub config: Config,
}

impl BootContext {
    pub fn new(config: Config) -> Self {
        Self {
            config,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
}