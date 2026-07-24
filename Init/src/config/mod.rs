use std::fs;
use std::collections::HashMap;

pub struct Config {
    config_values: HashMap<String, String>
}

impl Config {
    pub fn new(path: &str) -> Result<Self, String>{

        //Reads File, creates Hashmap
        let filestring = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut config_values: HashMap<String, String> = HashMap::new();

        for text in filestring.lines() {

            //Whitespace and Comment removal
            let line = text.trim();

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            //Storing values
            if let Some((key, value)) = line.split_once('=') {
                config_values.insert(
                    key.trim().to_string(),
                    value.trim().to_string(),
                );
            }

        }

        Ok(Self {
            config_values
        })
    }

    pub fn insert(&mut self, key: String, value: String) {
        self.config_values.insert(key, value);
    }

    pub fn get(&self, key: &str) -> Result<&String, &str> {
        self.config_values.get(key).ok_or("Value Not Found")
    }

    pub fn get_bool(&self, key: &str) -> Result<bool, &str> {
        let value_option = self.config_values.get(key);
        match value_option {
            Some(val) => {
                match val.parse::<bool>() {
                    Ok(conv_val) => Ok(conv_val),
                    Err(_) => Err("Value could not be converted to type 'bool'")
                }
            }
            None => Err("Value Not Found")
        }
    }
    pub fn get_i32(&self, key: &str) -> Result<i32, &str> {
        let value_option = self.config_values.get(key);
        match value_option {
            Some(val) => {
                match val.parse::<i32>() {
                    Ok(conv_val) => Ok(conv_val),
                    Err(_) => Err("Value could not be converted to type 'i32'")
                }
            }
            None => Err("Value Not Found")
        }
    }


}