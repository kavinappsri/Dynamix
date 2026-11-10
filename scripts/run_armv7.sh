#!/bin/zsh

BINARY_PATH="$1"
BINARY_BIN="${BINARY_PATH}.bin"

rust-objcopy -O binary "$BINARY_PATH" "$BINARY_BIN"

qemu-system-arm \
  -machine virt \
  -cpu cortex-a7 \
  -device ramfb \
  -serial stdio \
  -monitor none \
  -d "${QEMU_DEBUG:-int,guest_errors}" \
  -D dynamix_asm_v7.log \
  -kernel "$BINARY_BIN"