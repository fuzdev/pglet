#!/usr/bin/env bash
#
# power_loss_test.sh — TRUE power-loss durability test for pglet.
#
# WHY THIS EXISTS
# ---------------
# pglet's durability is already tested against *process* death: the in-process
# fault-injection tests (crates/pglet/tests/crash.rs), the real-process kill -9
# soak (crates/pglet_tests/src/bin/test_crash.rs, `deno task test:crash`), a
# randomized corruption fuzz, and a cargo-fuzz reopen target. All of those share
# the one invariant: after any interruption, a reopen recovers to a valid
# committed *prefix* or fails loud — never a torn read, and every *acked* commit
# survives.
#
# But a `kill -9` CANNOT reproduce true power loss. SIGKILL kills the process; the
# kernel page cache outlives it, so fsynced-but-still-cached data is NOT lost. A
# pulled power cord loses exactly that volatile state. To reproduce it without
# actually cutting power we need a fault-injecting block device that can *drop*
# writes the filesystem thinks it issued — `dm-flakey`. That needs root + the
# device-mapper subsystem, so this is the PRIVILEGED half of the power-loss test:
# run it on a THROWAWAY VM as root. The Rust driver it calls
# (`test_power_loss`, in pglet_tests) is the unprivileged half.
#
# WHAT IT DOES (the established sketch)
# -------------------------------------
#   1. Preconditions: root + dm_flakey + the device tools, else SKIP (exit
#      0) — a skip is fine; a fake pass is not.
#   2. Back a 256M file with a loop device.
#   3. Wrap it in a dm-flakey device (created in always-up / pass-through mode).
#   4. mkfs.ext4 + mount it.
#   5. Around the fault:
#        commit : drive a batch of ACKED commits onto the healthy device.
#        fault  : reload dm-flakey into drop_writes, umount (its writeback — the
#                 dirty pages not yet on the platter — is silently dropped, exactly
#                 as a power cut loses volatile/cached state), drop_caches, then
#                 reset to the clean pass-through table.
#        verify : remount and assert acked ⇒ durable + prefix-or-loud.
#   6. Teardown via trap: umount, dmsetup remove, losetup -d, rm tempfiles — so a
#      failure never leaks devices.
#
# WHY dm-flakey (not dm-dust)
# ---------------------------
# dm-flakey's `drop_writes` feature silently discards writes during its "down"
# window while still acking them — precisely the lie a volatile disk cache tells
# across a power cut (the data was "written" but never reached the platter).
# dm-dust models per-sector bad blocks (read failures), which is corruption, not
# power loss; the differential/fuzz corruption tests already cover that axis. So
# flakey's drop_writes is the right tool for *this* gap.
#
# USAGE
# -----
#   sudo tools/durability/power_loss_test.sh
#   (also: `deno task test:power-loss`, which just runs this script)
#
# Knobs: PGLET_POWER_LOSS_N (acked commit count, default 200).

set -euo pipefail

# ---- configuration ---------------------------------------------------------

DEVICE_NAME="pglet_flakey"
MAPPED="/dev/mapper/${DEVICE_NAME}"
BACKING_SIZE="256M"
ROWS="${PGLET_POWER_LOSS_N:-200}"

# Resolve the repo root from this script's own location (tools/durability/../..),
# so the Rust driver runs with cwd = workspace root (it spawns the relative
# `target/debug/pglet_server`).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# State filled in as we go; the teardown trap cleans up whatever exists.
BACKING=""
LOOP=""
MNT=""
RECORD=""
DM_CREATED=""
SECTORS=""

log() { printf '[power-loss] %s\n' "$*"; }

# Refusal: explain why, then exit 0. A SKIP is not a failure and is NEVER a
# fake pass — it leaves the device-backed assertions un-run, by design.
skip() {
  printf '[power-loss] SKIP: %s\n' "$*"
  exit 0
}

# ---- 1. preconditions / refusal (FIRST, before touching anything) ----

require_root_and_tools() {
  if [ "$(id -u)" -ne 0 ]; then
    skip "power-loss test needs root: it creates a loopback + dm-flakey device and mounts a filesystem; run on a throwaway VM as root, e.g. \`sudo tools/durability/power_loss_test.sh\`"
  fi

  local missing=()
  local tool
  for tool in losetup dmsetup mkfs.ext4 mount umount modprobe blockdev truncate cargo; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
  done
  if [ "${#missing[@]}" -ne 0 ]; then
    skip "missing required tools: ${missing[*]} — install them on the test VM and retry"
  fi

  # dm-flakey is a device-mapper target shipped with the kernel; load it (no-op if
  # already loaded / built in). A failure here means the kernel can't fault-inject.
  if ! modprobe dm_flakey 2>/dev/null; then
    skip "dm_flakey kernel module unavailable (modprobe dm_flakey failed) — load it on the test VM"
  fi
}

# ---- teardown: unwind whatever we created, in reverse order ----------------

cleanup() {
  # Best-effort: never let a teardown error mask the real exit status.
  set +e
  [ -n "$MNT" ] && umount -l "$MNT" 2>/dev/null
  [ -n "$DM_CREATED" ] && dmsetup remove "$DEVICE_NAME" 2>/dev/null
  [ -n "$LOOP" ] && losetup -d "$LOOP" 2>/dev/null
  [ -n "$BACKING" ] && rm -f "$BACKING"
  [ -n "$RECORD" ] && rm -f "$RECORD"
  [ -n "$MNT" ] && rmdir "$MNT" 2>/dev/null
  return 0
}

# (Re)load the dm-flakey table and resume. `--noflush` on suspend so in-flight
# dirty pages are NOT forced to the platter at the suspend boundary — we want the
# drop_writes window to actually drop them.
load_table() {
  dmsetup suspend --noflush "$DEVICE_NAME"
  dmsetup reload "$DEVICE_NAME" --table "$1"
  dmsetup resume "$DEVICE_NAME"
}

# Always-up table: up_interval huge, down_interval 0 → a plain pass-through. The
# healthy device for the commit + verify phases.
allow_table() { echo "0 $SECTORS flakey $LOOP 0 86400 0"; }

# Drop-writes table: up_interval 0, down_interval long → always "down" with the
# drop_writes feature, so every write is silently discarded (acked but never
# persisted) while reads still work. This is the power-loss window.
drop_table() { echo "0 $SECTORS flakey $LOOP 0 0 600 1 drop_writes"; }

# ---- main ------------------------------------------------------------------

main() {
  require_root_and_tools
  trap cleanup EXIT
  cd "$REPO_ROOT"

  # Build first (fail-fast BEFORE creating any device, so a build error can't leak
  # devices). Debug build — the driver/harness spawn `target/debug/pglet_server`.
  log "building pglet_server + test_power_loss driver..."
  cargo build -p pglet_server -p pglet_tests --bin test_power_loss
  local driver="${REPO_ROOT}/target/debug/test_power_loss"

  # The record file lives on a NON-faulty filesystem (/tmp) — it must survive the
  # device fault so the verify phase knows exactly what was acked.
  RECORD="$(mktemp /tmp/pglet_powerloss_record.XXXXXX)"

  # 2. backing file + loop device
  BACKING="$(mktemp /tmp/pglet_powerloss_backing.XXXXXX)"
  truncate -s "$BACKING_SIZE" "$BACKING"
  LOOP="$(losetup --find --show "$BACKING")"
  SECTORS="$(blockdev --getsz "$LOOP")"
  log "loop device $LOOP ($SECTORS sectors) backing $BACKING"

  # 3. dm-flakey device, created healthy (always-up / pass-through)
  # A previously hard-killed run can leave the fixed-name device (and a mount on
  # it) behind; `DM_CREATED` was never set then, so our own cleanup didn't remove
  # it. Clear a stale instance best-effort so the leak doesn't block `dmsetup
  # create` on every subsequent run.
  if dmsetup info "$DEVICE_NAME" >/dev/null 2>&1; then
    log "removing stale $DEVICE_NAME left by a prior run"
    umount -l "$MAPPED" 2>/dev/null || true
    dmsetup remove "$DEVICE_NAME" 2>/dev/null || true
  fi
  dmsetup create "$DEVICE_NAME" --table "$(allow_table)"
  DM_CREATED=1
  log "dm-flakey device $MAPPED created (pass-through)"

  # 4. filesystem + mount
  mkfs.ext4 -q -F "$MAPPED"
  MNT="$(mktemp -d /tmp/pglet_powerloss_mnt.XXXXXX)"
  mount "$MAPPED" "$MNT"
  mkdir -p "$MNT/data"
  log "mounted ext4 at $MNT (data dir $MNT/data)"

  # 5a. COMMIT — acked commits on the healthy device.
  log "phase commit: driving $ROWS acked commits..."
  "$driver" commit --data-dir "$MNT/data" --record "$RECORD" --n "$ROWS"

  # 5b. FAULT — simulate power loss AFTER the last ack:
  #   - switch the device to drop_writes (writes are silently discarded)
  #   - umount: the umount writeback (dirty fs pages not yet on the platter) is
  #     dropped — exactly the volatile state a power cut loses
  #   - drop_caches: evict the page cache so the remount reads the *platter*, not RAM
  #   - reset to the clean pass-through table so reads reflect what truly persisted
  log "tripping power-loss fault: drop_writes + umount + drop_caches..."
  # NB: deliberately NO `sync` here. A pre-fault `sync` would flush every dirty
  # page to the platter through the still-healthy device, so the drop window would
  # discard only the umount metadata — and the test could no longer catch a commit
  # that acked WITHOUT a real fsync (the exact regression this harness exists to
  # detect, and the gap `kill -9` can't reach). Leave the volatile state volatile.
  load_table "$(drop_table)"
  umount "$MNT"
  echo 3 >/proc/sys/vm/drop_caches
  load_table "$(allow_table)"

  # 5c. VERIFY — remount the post-fault image and assert recover-or-loud.
  mount "$MAPPED" "$MNT"
  log "phase verify: asserting acked => durable + prefix-or-loud..."
  if "$driver" verify --data-dir "$MNT/data" --record "$RECORD"; then
    log "PASS: every acked commit survived the simulated power loss (clean prefix recovery)"
  else
    log "FAIL: durability violation under simulated power loss (see driver output above)"
    exit 1
  fi
}

main "$@"
