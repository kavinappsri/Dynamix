//! Boot-time exercise of the timer service.

use hal::services::timer;

/// Generous: busy-waits are time based, but a VM can be descheduled mid-wait.
const SLOP_NS: u64 = 250_000_000;

fn check_delay(name: &'static str, wanted_ns: u64, wait: impl FnOnce()) -> Result<(), &'static str> {
    let start = timer::now_ns();
    wait();
    let elapsed = timer::now_ns().wrapping_sub(start);
    if elapsed < wanted_ns {
        return Err(name);
    }
    if elapsed > wanted_ns + SLOP_NS {
        return Err("a delay took far longer than asked");
    }
    Ok(())
}

pub fn run() -> Result<(), &'static str> {
    // Never goes backwards.
    let mut last = timer::now_ns();
    for _ in 0..1000 {
        let now = timer::now_ns();
        if now < last {
            return Err("now_ns went backwards");
        }
        last = now;
    }

    check_delay("delay_us returned early", 2_000_000, || timer::delay_us(2_000))?;
    check_delay("delay_ms returned early", 5_000_000, || timer::delay_ms(5))?;

    // A counter that never advances (frequency unknown) would pass the checks
    // above only by accident; make sure it really moved.
    if timer::now_ns() <= last {
        return Err("now_ns does not advance");
    }
    Ok(())
}
