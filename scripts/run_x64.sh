#!/bin/zsh

mkdir -p esp/EFI/BOOT
cp target/x86_64-unknown-uefi/release/dynamix_kernel.efi esp/EFI/BOOT/BOOTX64.EFI

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
  -d in_asm,int \
  -D dynamix64.log