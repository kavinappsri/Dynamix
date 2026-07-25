#[derive(Debug, Clone)]
pub enum DeviceType {
    Display,
    Storage,
    Network,
    Input,
    Audio,
    Battery,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub device_type: DeviceType,
    pub initialized: bool,
}

impl Device {
    pub fn new(name: impl Into<String>, device_type: DeviceType) -> Self {
        Self {
            name: name.into(),
            device_type,
            initialized: false,
        }
    }

    pub fn set_initialized(&mut self, value: bool) {
        self.initialized = value;
    }
}

pub struct DeviceManager {
    devices: Vec<Device>,
}

impl DeviceManager {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
        }
    }

    pub fn register(&mut self, device: Device) {
        self.devices.push(device);
    }

    pub fn count(&self) -> usize {
        self.devices.len()
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn clear(&mut self) {
        self.devices.clear();
    }
}