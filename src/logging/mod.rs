use std::{fs::OpenOptions, io::Write, path::Path};

const DEFAULT_CONFIG_FILE: &str = "music-server-1.log4rs.yaml";
const DEFAULT_CONFIG: &[u8] = include_bytes!("default.log4rs.yaml");

pub fn init() {
    let config_file_path = Path::new(DEFAULT_CONFIG_FILE);
    if !config_file_path.exists() {
        let mut write_cfg_file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(config_file_path)
            .unwrap();
        write_cfg_file.write_all(DEFAULT_CONFIG).unwrap();
    }

    log4rs::init_file(config_file_path, Default::default()).unwrap();
}
