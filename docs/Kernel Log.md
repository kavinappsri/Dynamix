# The kernel log

Usually, subsystems need a place to log stuff that isn't the serial port. This system was designed for the kernel to have
a logging system for that (Or more so of a place for ACPICA to dump its ginormous amount of logs). it can work from very 
early in boot, so even if serial is down, once it comes back up, we can view these logs

As always, This file may be and probably is outdated. Your first resource should always be the source code

## What it is

This system works by having a ring of text records in a 64 kb array. When this ring is full, the oldest records get 
overwritten for new ones. 


Every record has a small header (length, source, level, a version byte, a sequence number and a timestamp from `now_ns`), then
the text. The layout is written at the top of the file.

## Writing

```rust
pub fn my_random_fn_so_that_i_dont_get_bombed_by_syntax_errors() {
    klog::log(Source::Ec, Level::Warn, format_args!("register {reg:#04x} access failed: {err}"));
}
```

ACPICA prints one line in several pieces, so there's also `write_fragment`. Every source gets its own line buffer, and a
record is made when the line ends. The lock is IRQ-safe, so the SCI handler can log. Lines longer than 256 bytes are split
into several records.

Sources can also echo to serial as they log (`klog::set_echo`). It's off for everyone by default.

## Reading

A reader doesn't point into the ring. It holds a sequence number and asks for it:

```rust
pub fn my_random_fn_so_that_i_dont_get_bombed_by_syntax_errors() {
    let read = klog::read_from(seq, &mut text);   // copies one record out
    // read.record, read.next_seq, read.lost
}
```

If a record we wanted was already overwritten (womp womp), it jumps to the oldest currently present and gives `lost`, which says
how many we missed. 


## Where you see it

* `dmesg` in the shell prints everything still stored, like `[    2.113503] acpi: ACPI: RSDP ...`
* `stats klog` shows the ring's bookkeeping
* a panic or fault prints the last few records after its report. That one never waits for the lock, in case the fault
  happened while the log was being written

## What isn't done yet

The kernel's own `writeln!` calls in `main.rs` still go straight to serial, there's no backend for the `log` crate, and
there are no per-CPU rings (it's one ring and one lock for now).

Source Code:

[The log](/hal/src/services/klog.rs)

[dmesg and stats klog](/kernel/src/shell.rs)

[The fault report that prints the tail](/hal/src/services/fault.rs)

[ACPICA's output, forwarded to the log](/hal/src/services/acpica_osl.rs)
