use std::fs;
use std::collections::HashMap;

pub struct Config {
    values: HashMap<String, String>
}

impl Config {
    pub fn new() -> Result<Self, String>{

        //Reads File, creates Hashmap
        let filestring = fs::read_to_string("../config/dynamix.conf").map_err(|e| e.to_string())?;
        let mut values: HashMap<String, String> = HashMap::new();

        for text in filestring.lines() {

            //Whitespace and Comment removal
            let line = text.trim();

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            //Storing values
            if let Some((key, value)) = line.split_once('=') {
                values.insert(
                    key.trim().to_string(),
                    value.trim().to_string(),
                );
            }

        }


        Ok(Self {
            values
        })
    }

    pub fn insert(&mut self, key: String, value: String) {
        self.values.insert(key, value);
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.values.get(key)
    }

    pub fn parse(&mut self) {

    }
}

pub fn load() -> Result<String, String> {

    fs::read_to_string("../Config/dynamix.conf")
        .map_err(|e| e.to_string())

}