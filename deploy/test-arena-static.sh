#!/usr/bin/env bash
# Tests what `arena.sh static` ASKS rsync and ssh to do, with both mocked.
#
# Why mocked rather than a real sync: the dangerous part of this subcommand is
# the argument list, not whether rsync can copy a file. Two of those arguments
# are load-bearing against production and both fail silently if they regress:
#
#   * a stray --delete would erase the 1.1 GB bundles.blades.bgs.services/ asset
#     mirror the box maintains in place, plus every generated file that is not
#     in git. Nothing would report it until a client asked for an asset.
#   * a missing restart makes the whole command a no-op, because the server
#     reads deploy/static exactly once, at startup. That is the bug this
#     subcommand exists to fix, so it is the one worth a regression test.
#
# Both are pure argv, so they can be tested exactly, offline, in a second.
#
#   deploy/test-arena-static.sh

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
PASS=0; FAIL=0

# Mock rsync, ssh and sleep. Each logs its argv to its own file and succeeds.
for tool in rsync ssh sleep; do
  cat > "$WORK/$tool" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$WORK/$tool.log"
exit 0
EOF
  chmod +x "$WORK/$tool"
done

# Deliberately not the real box: if a mock is ever bypassed, the test must fail
# to connect rather than reach production.
run_static() {
  : > "$WORK/rsync.log"; : > "$WORK/ssh.log"; : > "$WORK/sleep.log"
  PATH="$WORK:$PATH" \
  ARENA_BOX="testuser@test.invalid" \
  ARENA_BOX_DIR="/srv/testbox" \
  ARENA_SSH_KEY="/dev/null" \
    bash deploy/arena.sh "$@" > "$WORK/out" 2>&1
  echo $? > "$WORK/rc"
}

ok()   { echo "  ok    $1"; PASS=$((PASS+1)); }
bad()  { echo "  FAIL  $1"; FAIL=$((FAIL+1)); }
want() { # want <name> <file> <substring>
  grep -qF -- "$3" "$2" && ok "$1" || bad "$1 — no '$3' in $(basename "$2")"
}
wantnot() {
  grep -qF -- "$3" "$2" && bad "$1 — found '$3' in $(basename "$2")" || ok "$1"
}

echo "1. static (real run)"
run_static static
want    "syncs the whole directory, not a file" "$WORK/rsync.log" "/deploy/static/ testuser@test.invalid:/srv/testbox/deploy/static/"
wantnot "no --delete"                           "$WORK/rsync.log" "--delete"
want    "skips the on-box asset mirror"         "$WORK/rsync.log" "--exclude=bundles.blades.bgs.services/"
want    "keeps a backup of overwrites"          "$WORK/rsync.log" "--backup-dir=/srv/testbox/deploy/static-backup/"
want    "itemises what changed"                 "$WORK/rsync.log" "--itemize-changes"
want    "restarts arena-server"                 "$WORK/ssh.log"   "docker restart arena-server"
want    "reports [static] complaints after"     "$WORK/ssh.log"   "docker logs"
[ "$(cat "$WORK/rc")" = 0 ] && ok "exits 0" || bad "exits $(cat "$WORK/rc")"

# The backup dir must not land inside the directory the server reads, or the
# server would try to parse the backups as game data.
if grep -qF -- "--backup-dir=/srv/testbox/deploy/static/" "$WORK/rsync.log"; then
  bad "backup dir is outside the mounted static dir"
else
  ok "backup dir is outside the mounted static dir"
fi

echo
echo "2. static --dry-run (must touch nothing)"
run_static static --dry-run
want    "passes --dry-run to rsync"   "$WORK/rsync.log" "--dry-run"
wantnot "no --delete"                 "$WORK/rsync.log" "--delete"
[ ! -s "$WORK/ssh.log" ] && ok "no ssh at all — no restart, no box contact" \
                         || bad "ssh ran during a dry run: $(cat "$WORK/ssh.log")"
want    "says it changed nothing"     "$WORK/out" "the box was not touched"

echo
echo "3. the dry run must describe the real run"
run_static static;           sed -E 's/ --backup --backup-dir=[^ ]+//' "$WORK/rsync.log" > "$WORK/real.args"
run_static static --dry-run; sed -E 's/ --dry-run//'                   "$WORK/rsync.log" > "$WORK/dry.args"
if diff -q "$WORK/real.args" "$WORK/dry.args" >/dev/null; then
  ok "same rsync invocation either way"
else
  bad "dry run and real run disagree:"; diff "$WORK/real.args" "$WORK/dry.args" | sed 's/^/        /'
fi

echo
echo "4. argument handling"
run_static static --delete
[ "$(cat "$WORK/rc")" != 0 ] && ok "rejects an unknown argument" || bad "accepted 'static --delete'"
[ ! -s "$WORK/rsync.log" ] && ok "and runs nothing when it does" || bad "ran rsync anyway"

echo
echo "5. it is discoverable"
run_static --help
want "listed in usage" "$WORK/out" "static"
wantnot "usage prints no code" "$WORK/out" "set -euo pipefail"

echo
if [ $FAIL -gt 0 ]; then echo "FAILED — $FAIL failure(s), $PASS passed"; exit 1; fi
echo "passed — $PASS checks"
