#!/bin/sh
set -eu

failures=0
warn() { printf 'WARN  %s\n' "$1"; }
pass() { printf 'PASS  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; failures=$((failures + 1)); }

printf 'PulseScope appliance preflight v1\n'
if [ -d /dev/bus/usb ]; then pass 'USB bus is mounted'; else fail '/dev/bus/usb is not mounted'; fi
if [ -r /dev/bus/usb ]; then pass 'USB bus is readable'; else fail 'USB bus is not readable'; fi
if [ -d /dev/shm ] && [ -w /dev/shm ]; then pass 'shared memory is writable'; else fail '/dev/shm is not writable'; fi

available_kb=$(df -Pk /var/lib/pulsescope 2>/dev/null | awk 'NR==2 {print $4}')
if [ -n "${available_kb:-}" ] && [ "$available_kb" -ge 1048576 ]; then pass 'at least 1 GiB data storage is free'; else warn 'less than 1 GiB data storage appears free'; fi

receive_buffer=$(cat /host/rmem_max 2>/dev/null || cat /proc/sys/net/core/rmem_max 2>/dev/null || printf 0)
if [ "$receive_buffer" -ge 4194304 ]; then pass "socket receive buffer max is $receive_buffer"; else warn "net.core.rmem_max=$receive_buffer; 4194304 or higher is recommended"; fi

if command -v SoapySDRUtil >/dev/null 2>&1; then
  pass 'SoapySDRUtil is installed'
  printf '\nDetected SDRs:\n'
  SoapySDRUtil --find 2>&1 || warn 'Soapy discovery returned no usable receiver'
  printf '\nInstalled modules:\n'
  SoapySDRUtil --info 2>&1 || true
else
  fail 'SoapySDRUtil is missing'
fi

if [ -d /opt/pulsescope/drivers ]; then pass 'persistent driver volume is mounted'; else warn 'persistent proprietary-driver volume is not mounted'; fi

if [ "$failures" -gt 0 ]; then
  printf '\nPreflight failed with %s blocking issue(s).\n' "$failures"
  exit 1
fi
printf '\nPreflight passed. Warnings should be reviewed before a soak test.\n'
