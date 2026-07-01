#!/bin/sh
# Real-world role coverage analyzer for komandan-playbook (spec §11.3).
#
# Clones three popular Ansible roles, wraps each in a one-play playbook, and
# drives komandan's `playbook` subcommand against it:
#   1. `--syntax-check`           — does komandan parse the role cleanly?
#   2. `--list-tasks`             — enumerate every leaf task + its module.
# Module coverage is computed from the authoritative `[<module>]` tokens in
# the `--list-tasks` output, compared against the supported-module set baked
# into this komandan build.
#
# Robustness: a failed clone, a komandan crash, or a parse failure for one
# role is reported as FAILED/SKIPPED for that role and does not abort the run.
#
# Usage:
#   scripts/analyze-real-roles.sh
#   KOMANDAN_PLUGIN_DIR=/tmp/komandan-plugins scripts/analyze-real-roles.sh
#
# Environment:
#   KOMANDAN_PLUGIN_DIR  where the host discovers cdylib plugins
#                        (default: /tmp/komandan-plugins)

set -u

# ---- paths ---------------------------------------------------------------

# Repo root = parent of the directory holding this script.
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
BIN="$REPO_ROOT/target/debug/komandan"
PLUGIN_SRC="$REPO_ROOT/target/debug/libkomandan_playbook.so"

export KOMANDAN_PLUGIN_DIR="${KOMANDAN_PLUGIN_DIR:-/tmp/komandan-plugins}"

if [ ! -x "$BIN" ]; then
    echo "error: komandan binary not found at $BIN" >&2
    echo "       run 'cargo build -p komandan' first." >&2
    exit 1
fi

# Refresh the plugin in the discovery dir if a fresh build is available.
mkdir -p "$KOMANDAN_PLUGIN_DIR"
if [ -f "$PLUGIN_SRC" ]; then
    cp "$PLUGIN_SRC" "$KOMANDAN_PLUGIN_DIR/" 2>/dev/null || true
fi

# ---- scratch area --------------------------------------------------------

WORKDIR=$(mktemp -d 2>/dev/null || mktemp -d -t komandan-roles)
cleanup() {
    rm -rf "$WORKDIR"
}
trap cleanup EXIT INT TERM HUP

mkdir -p "$WORKDIR/roles"

# ---- supported module set (keep in sync with executors/register_all) -----
# Canonical names + aliases. Fully-qualified forms (ansible.builtin.*) are
# reduced to their last dotted segment before comparison.
SUPPORTED_MODULES="
command shell raw
apt dnf yum
file copy fetch get_url
lineinfile blockinfile replace template
user group
systemd service
script postgresql_user
debug ping set_fact assert fail meta pause
add_host group_by
hostname timezone git pip stat known_hosts
unarchive wait_for archive cron mount reboot uri
include_tasks import_tasks include_role import_role import_playbook
"

# Structural / directive keys that are not module invocations and must not be
# counted when scanning raw task YAML (used only by the fallback scanner).
RESERVED_KEYS="
name when loop with_items with_dict with_indexed_items with_list
with_sequence with_together with_subelements tags register
become become_user become_method vars block rescue always
notify listen changed_when failed_when ignore_errors
delegate_to local_action environment run_once no_log
until retries delay loop_control any_errors_fatal
module_defaults connection remote_user check_mode diff
sudo sudo_user action transport
"

# ---- helpers -------------------------------------------------------------

# Print 0 on failure, 1 on success.
#
# NOTE: callers may have narrowed IFS (e.g. to newline for line iteration);
# this function restores the default whitespace IFS internally so the
# space-separated SUPPORTED_MODULES blob word-splits correctly.
is_supported() {
    _is_key=$1
    _is_oldifs=$IFS
    # space, tab, newline
    IFS=' 	
'
    for _is_m in $SUPPORTED_MODULES; do
        if [ "$_is_m" = "$_is_key" ]; then
            IFS=$_is_oldifs
            return 0
        fi
    done
    IFS=$_is_oldifs
    return 1
}

# Reduce a possibly fully-qualified module name to its last segment.
#   ansible.builtin.command -> command ; apt -> apt
canonicalize() {
    echo "${1##*.}"
}

# Clone one role into $WORKDIR/roles/<galaxy_name>.
# Args: galaxy_name  git_url
# Returns 0 on success, 1 on failure (after printing a warning).
clone_role() {
    galaxy=$1
    url=$2
    target="$WORKDIR/roles/$galaxy"
    if git clone --quiet --depth 1 "$url" "$target" >/dev/null 2>&1; then
        echo "  cloned $galaxy"
        return 0
    fi
    echo "  WARNING: failed to clone $galaxy ($url) — skipping" >&2
    return 1
}

# Write a wrapper playbook that invokes a single role against localhost.
# Args: galaxy_name  out_path
write_wrapper() {
    galaxy=$1
    out=$2
    cat > "$out" <<EOF
---
- hosts: localhost
  gather_facts: false
  roles:
    - role: $galaxy
EOF
}

# Run komandan `playbook <wrapper> <flags>` capturing combined stdout+stderr.
# Args: wrapper_path  (extra flags passed through `$@` after the wrapper)
# Sets globals: RUN_RC, RUN_OUT.
run_komandan() {
    wrapper=$1; shift
    # shellcheck disable=SC2086
    RUN_OUT=$("$BIN" playbook "$wrapper" "$@" 2>&1)
    RUN_RC=$?
}

# ---- target roles --------------------------------------------------------

# galaxy_name | git_url
ROLES="\
geerlingguy.docker|https://github.com/geerlingguy/ansible-role-docker.git
geerlingguy.nginx|https://github.com/geerlingguy/ansible-role-nginx.git
dev-sec.ssh-hardening|https://github.com/dev-sec/ansible-ssh-hardening.git"

# ---- main loop -----------------------------------------------------------

printf '\n=== komandan real-world role analysis ===\n\n'
echo "workdir : $WORKDIR"
echo "binary  : $BIN"
echo "plugins : $KOMANDAN_PLUGIN_DIR"
printf '\n'

# Accumulated summary lines (one per role).
SUMMARY=""

IFS='
'
for entry in $ROLES; do
    IFS='|'
    # shellcheck disable=SC2086
    set -- $entry
    galaxy=$1
    url=$2
    IFS='
'

    echo "--- role: $galaxy ---"

    if ! clone_role "$galaxy" "$url"; then
        SUMMARY="$SUMMARY
$galaxy|SKIP|clone failed|-|-|-|-"
        continue
    fi

    wrapper="$WORKDIR/wrapper-$galaxy.yml"
    write_wrapper "$galaxy" "$wrapper"

    # 1) syntax check
    run_komandan "$wrapper" --syntax-check
    syn_rc=$RUN_RC
    if [ "$syn_rc" -ne 0 ]; then
        echo "  SYNTAX-CHECK: FAILED (rc=$syn_rc)"
        echo "  ---- stderr/stdout ----"
        printf '%s\n' "$RUN_OUT" | sed 's/^/    /' | head -n 40
        echo "  -----------------------"
        SUMMARY="$SUMMARY
$galaxy|FAIL|syntax-check rc=$syn_rc|-|-|-|-"
        continue
    fi
    echo "  SYNTAX-CHECK: ok"

    # 2) list-tasks (authoritative module source)
    run_komandan "$wrapper" --list-tasks --skip-unsupported
    list_rc=$RUN_RC
    if [ "$list_rc" -ne 0 ]; then
        echo "  LIST-TASKS   : FAILED (rc=$list_rc)"
        echo "  ---- stderr/stdout ----"
        printf '%s\n' "$RUN_OUT" | sed 's/^/    /' | head -n 40
        echo "  -----------------------"
        SUMMARY="$SUMMARY
$galaxy|FAIL|list-tasks rc=$list_rc|-|-|-|-"
        continue
    fi

    # Extract module names from `task #N  <name>  [<module>]` lines.
    # The bracketed token is the module as recorded by komandan's parser.
    modules=$(printf '%s\n' "$RUN_OUT" | sed -n 's/.*\[ *\([a-zA-Z0-9_.]*\) *\] *$/\1/p')
    total=0
    sup=0
    unsup_list=""
    unsup_count=0
    sup_list=""

    IFS='
'
    for mod in $modules; do
        IFS='
'
        total=$((total + 1))
        canon=$(canonicalize "$mod")
        if is_supported "$canon"; then
            sup=$((sup + 1))
        else
            unsup_count=$((unsup_count + 1))
            # De-dup the unsupported list.
            case " $unsup_list " in
                *" $canon "*) ;;
                *) unsup_list="$unsup_list $canon" ;;
            esac
        fi
    done

    if [ "$total" -gt 0 ]; then
        pct=$((sup * 100 / total))
    else
        pct=0
    fi
    echo "  LIST-TASKS   : ok ($total tasks)"
    echo "  modules      : $sup/$total supported (${pct}%)"
    if [ -n "$unsup_list" ]; then
        echo "  unsupported  :$unsup_list"
    else
        echo "  unsupported  : (none)"
    fi

    SUMMARY="$SUMMARY
$galaxy|OK|parsed|$total|$sup|$pct|$unsup_list"

    printf '\n'
done

# ---- summary table -------------------------------------------------------

printf '\n=== summary ===\n\n'
printf '%-26s %-8s %-22s %-10s %-10s %-8s %s\n' \
    "ROLE" "STATUS" "DETAIL" "TASKS" "SUPPORTED" "COV%" "UNSUPPORTED"
printf '%-26s %-8s %-22s %-10s %-10s %-8s %s\n' \
    "----" "------" "------" "-----" "---------" "----" "-----------"

# Strip the leading blank line we used to seed $SUMMARY.
SUMMARY=$(printf '%s\n' "$SUMMARY" | sed -n '2,$p')

IFS='
'
for line in $SUMMARY; do
    IFS='|'
    # shellcheck disable=SC2086
    set -- $line
    # Fields with no trailing value (e.g. an empty unsupported list leaves a
    # dangling `|`) are dropped by word-splitting; default them to "-".
    r_role=${1:-unknown}
    r_status=${2:-?}
    r_detail=${3:-?}
    r_tasks=${4:--}
    r_sup=${5:--}
    r_pct=${6:--}
    r_unsup=${7:-}
    [ "$r_unsup" = "" ] && r_unsup="(none)"
    IFS='
'
    printf '%-26s %-8s %-22s %-10s %-10s %-8s %s\n' \
        "$r_role" "$r_status" "$r_detail" "$r_tasks" "$r_sup" "$r_pct" "$r_unsup"
done

printf '\ndone.\n'
