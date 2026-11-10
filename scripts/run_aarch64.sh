#!/bin/zsh

BINARY_PATH="$1"
BINARY_BIN="${BINARY_PATH}.bin"

rust-objcopy -O binary "$BINARY_PATH" "$BINARY_BIN"

qemu-system-aarch64 \
  -machine virt \
  -cpu cortex-a72 \
  -device ramfb \
  -serial stdio \
  -monitor none \
  -d in_asm,int \
  -D dynamix_asm.log \
  -kernel "$BINARY_BIN"