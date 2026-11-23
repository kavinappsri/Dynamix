//! Providers

use alloc::boxed::Box;
use core::any::{Any, TypeId};

use crate::driver_traits::driver::ProbeError;
use crate::services::device::{Device, State};
use crate::services::registry::AppendList;

struct Entry {
    device: &'static Device,
    type_id: TypeId,
    instance: &'static (dyn Any + Send + Sync),
}

static PROVIDERS: AppendList<Entry> = AppendList::new();

/// Called by a driver's probe to offer `instance` to other drivers, as
/// belonging to `device`. Returns the instance's permanent home.
pub fn register<T: Any + Send + Sync>(device: &'static Device, instance: T) -> &'static T {
    let instance: &'static T = Box::leak(Box::new(instance));
    PROVIDERS.push(Entry { device, type_id: TypeId::of::<T>(), instance });
    instance
}

/// The `T` that `device` offers.
pub fn get<T: Any + Send + Sync>(device: &Device) -> Result<&'static T, ProbeError> {
    let wanted = TypeId::of::<T>();
    let found = PROVIDERS
        .iter()
        .find(|entry| entry.device.id() == device.id() && entry.type_id == wanted)
        .and_then(|entry| entry.instance.downcast_ref::<T>());
    if let Some(found) = found {
        return Ok(found);
    }

    match device.state() {
        State::Pending | State::Deferred(_) => Err(ProbeError::Defer),
        State::NoDriver | State::Failed(..) | State::Bound(_) => Err(ProbeError::NotPresent),
    }
}

/// The first `T` any device offered
pub fn first<T: Any + Send + Sync>() -> Option<&'static T> {
    let wanted = TypeId::of::<T>();
    PROVIDERS
        .iter()
        .filter(|entry| entry.type_id == wanted)
        .find_map(|entry| entry.instance.downcast_ref::<T>())
}
