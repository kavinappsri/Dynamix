#!/bin/zsh

mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-uefi/release/dynamix_kernel.efi esp/EFI/BOOT/BOOTX64.EFI

# -no-reboot: a triple fault ends QEMU (with the reason in the -d log) instead
# of rebooting in a loop. Do NOT add -no-shutdown: it keeps QEMU alive, paused,
# after the guest powers off, so the `shutdown` command looks like a hang.
qemu-system-x86_64 \
  -machine q35,smm=off \
  -cpu qemu64 \
  -m 256M \
  -drive if=pflash,format=raw,readonly=on,file=ovmf/OVMF_CODE.fd \
  -drive if=pflash,format=raw,file=ovmf/OVMF_VARS.fd \
  -drive format=raw,file=fat:rw:esp \
  -serial stdio \
  -monitor none \
  -net none \
  -d "${QEMU_DEBUG:-int,cpu_reset,guest_errors}" \
  -no-reboot \
  -D dynamix64.log