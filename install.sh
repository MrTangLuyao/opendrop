#!/usr/bin/env bash
# opendrop installer (Docker). Safe to re-run: it upgrades and reconfigures, data is kept.
#
#   curl -fsSL https://raw.githubusercontent.com/MrTangLuyao/opendrop/main/install.sh | sudo bash
#   curl -fsSL https://raw.githubusercontent.com/MrTangLuyao/opendrop/main/install.sh | sudo bash -s -- --domain drop.example.com
#   sudo bash install.sh [options]
set -euo pipefail

REPO=MrTangLuyao/opendrop
RAW=${OPENDROP_RAW:-https://raw.githubusercontent.com/$REPO/main}
GIT=${OPENDROP_GIT:-https://github.com/$REPO.git#main}
DEFAULT_IMAGE=ghcr.io/mrtangluyao/opendrop:latest
CLI=/usr/local/bin/opendrop
NGINX_SITE=/etc/nginx/sites-enabled/opendrop
# First line of the .env written here; it marks a dir as an opendrop install.
ENV_HEADER='# opendrop settings, rewritten by install.sh (extra lines are kept)'

DOMAIN=
PORT=
PORT_GIVEN=0
DIR=/opt/opendrop
BUILD=0
YES=0
SRC=
V1=0
BIND=
TRUST=
IPV6=false
ADDR=
IMAGE=
TMPD=
FRESH=0
ADMIN_USER=
ADMIN_PASS=

if [ -t 1 ]; then
  BOLD=$'\033[1m' GREEN=$'\033[32m' YELLOW=$'\033[33m' RED=$'\033[31m' OFF=$'\033[0m'
else
  BOLD='' GREEN='' YELLOW='' RED='' OFF=''
fi

say() { printf '%s\n' "$*"; }
step() { printf '%s==>%s %s\n' "$GREEN" "$OFF" "$*"; }
warn() { printf '%swarning:%s %s\n' "$YELLOW" "$OFF" "$*" >&2; }
die() {
  printf '%serror:%s %s\n' "$RED" "$OFF" "$*" >&2
  if [ -n "$ADMIN_PASS" ]; then
    printf 'the admin account was already created, keep this login: %s / %s\n' "${ADMIN_USER:-admin}" "$ADMIN_PASS" >&2
  fi
  exit 1
}

usage() {
  cat <<'EOF'
usage: install.sh [options]

  --domain D   serve HTTPS on domain D (Caddy, automatic certificates)
  --port P     host port, default 80; with --domain the loopback port that
               Caddy forwards to, default 8080 (a free one when it is taken)
  --dir PATH   install dir, default /opt/opendrop; it must be missing, empty
               or an existing opendrop install
  --build      build the image from source instead of pulling it
  -y, --yes    no prompts, accept defaults
  -h, --help   show this help

Running it again keeps the current settings unless --domain or --port is given.
EOF
}

bad_usage() {
  printf 'install.sh: %s\n\n' "$1" >&2
  usage >&2
  exit 2
}

parse_args() {
  while [ $# -gt 0 ]; do
    case $1 in
      --domain | --port | --dir)
        [ $# -ge 2 ] || bad_usage "$1 needs a value"
        set_opt "$1" "$2"
        shift 2
        ;;
      --domain=* | --port=* | --dir=*)
        set_opt "${1%%=*}" "${1#*=}"
        shift
        ;;
      --build)
        BUILD=1
        shift
        ;;
      -y | --yes)
        YES=1
        shift
        ;;
      -h | --help)
        usage
        exit 0
        ;;
      *) bad_usage "unknown option: $1" ;;
    esac
  done
}

set_opt() {
  case $1 in
    --domain)
      DOMAIN=$(norm_domain "$2")
      valid_domain "$DOMAIN" || bad_usage "invalid domain: $2"
      ;;
    --port)
      valid_port "$2" || bad_usage "invalid port: $2"
      PORT=$((10#$2))
      PORT_GIVEN=1
      ;;
    --dir)
      DIR=$2
      while [ "${DIR%/}" != "$DIR" ]; do DIR=${DIR%/}; done
      [[ $DIR =~ ^/[A-Za-z0-9._/-]+$ ]] || bad_usage "--dir must be an absolute path: $2"
      case $DIR/ in */../* | */./* | *//*) bad_usage "invalid dir: $2" ;; esac
      if system_dir "$DIR"; then bad_usage "--dir must not be a system directory: $2"; fi
      ;;
  esac
}

# Directories that must never be the install dir: top level ones (/opt, /srv,
# /home, ...), the OS trees, home directories and the standard /var dirs.
system_dir() {
  case $1 in
    /bin/* | /boot/* | /dev/* | /etc/* | /lib/* | /lib32/* | /lib64/* | /libx32/* | /proc/* | /run/* | /sbin/* | /sys/* | /usr/*) return 0 ;;
    /home/*/* | /var/*/*) return 1 ;;
    /home/* | /var/backups | /var/cache | /var/lib | /var/local | /var/lock | /var/log | /var/mail | /var/opt | /var/run | /var/spool | /var/tmp | /var/www) return 0 ;;
    /*/*) return 1 ;;
  esac
  return 0
}

norm_domain() {
  local d
  d=$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')
  d=${d#http://}
  d=${d#https://}
  printf '%s' "${d%%/*}"
}

valid_domain() {
  [[ $1 =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$ ]] && [ ${#1} -le 253 ]
}

valid_port() {
  [[ $1 =~ ^[0-9]{1,5}$ ]] && [ $((10#$1)) -ge 1 ] && [ $((10#$1)) -le 65535 ]
}

# Value of KEY in the existing .env (empty when absent; a v1 dir has none of ours).
env_get() {
  if [ "$V1" = 0 ] && [ -f "$DIR/.env" ]; then
    sed -n "s/^$1=//p" "$DIR/.env" | tail -n 1
  fi
}

dc() { (cd "$DIR" && docker compose "$@"); }

check_platform() {
  local os
  [ "$(uname -s)" = Linux ] || die "Linux only"
  case $(uname -m) in
    x86_64 | amd64) ARCH=amd64 ;;
    aarch64 | arm64) ARCH=arm64 ;;
    *) die "unsupported CPU $(uname -m), only amd64 and arm64 are supported" ;;
  esac
  # shellcheck source=/dev/null
  os=$(. /etc/os-release 2>/dev/null && printf '%s' "${PRETTY_NAME:-}") || os=
  step "${os:-Linux} ($ARCH)"
}

need_curl() {
  command -v curl >/dev/null 2>&1 && return 0
  command -v apt-get >/dev/null 2>&1 || die "curl is required"
  step "installing curl"
  apt-get update -qq
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq curl ca-certificates >/dev/null
}

ensure_docker() {
  local n=0
  if ! command -v docker >/dev/null 2>&1; then
    step "installing Docker (get.docker.com)"
    need_curl
    curl -fsSL https://get.docker.com | sh
  fi
  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl enable --now docker >/dev/null 2>&1 || true
  elif ! docker info >/dev/null 2>&1 && command -v service >/dev/null 2>&1; then
    service docker start >/dev/null 2>&1 || true
  fi
  until docker info >/dev/null 2>&1; do
    n=$((n + 1))
    [ "$n" -le 15 ] || die "the Docker daemon is not running"
    sleep 1
  done
  docker compose version >/dev/null 2>&1 ||
    die "Docker Compose v2 is missing: install the docker-compose-plugin package (docker-compose-v2 on Ubuntu)"
}

pm2_runs_opendrop() {
  command -v pm2 >/dev/null 2>&1 && pm2 jlist 2>/dev/null | grep -F '"name":"opendrop"' >/dev/null
}

# opendrop v1 (Node + pm2) lived in the install dir. The v2 tree still has a
# server.js, so a dir with Cargo.toml only counts as v1 while pm2 runs it
# (a v1 clone that pulled v2).
detect_v1() {
  [ -f "$DIR/server.js" ] || return 0
  if [ ! -f "$DIR/Cargo.toml" ] || pm2_runs_opendrop; then
    V1=1
    step "found opendrop v1 in $DIR (it keeps running until all checks pass)"
  fi
}

# The install dir must be missing, empty, an opendrop install or opendrop v1;
# uninstall --purge relies on this.
check_dir() {
  [ -e "$DIR" ] || return 0
  [ -d "$DIR" ] || die "$DIR exists and is not a directory"
  [ "$V1" = 0 ] || return 0
  if [ -f "$DIR/Cargo.toml" ]; then
    die "$DIR is a source checkout; install into another dir (--dir PATH), the checkout can stay where it is"
  fi
  if [ -z "$(ls -A "$DIR")" ] || [ -e "$DIR/data/opendrop.db" ] ||
    [ "$(head -n 1 "$DIR/.env" 2>/dev/null)" = "$ENV_HEADER" ]; then
    return 0
  fi
  die "$DIR is not empty and is not an opendrop install; pick an empty or new dir with --dir PATH"
}

ask_config() {
  local d p
  while :; do
    read -r -p "Domain for HTTPS (empty for plain HTTP): " d || d=
    d=$(norm_domain "$d")
    if [ -z "$d" ] || valid_domain "$d"; then break; fi
    say "invalid domain"
  done
  DOMAIN=$d
  [ -z "$DOMAIN" ] || return 0
  while :; do
    read -r -p "Port [80]: " p || p=
    p=${p:-80}
    if valid_port "$p"; then break; fi
    say "invalid port"
  done
  PORT=$((10#$p))
  PORT_GIVEN=1
}

resolve_config() {
  if [ -n "$DOMAIN" ] || [ -n "$PORT" ]; then
    :
  elif [ "$V1" = 0 ] && [ -f "$DIR/.env" ]; then
    DOMAIN=$(env_get DOMAIN)
    PORT=$(env_get OPENDROP_PORT)
    BIND=$(env_get OPENDROP_BIND)
    TRUST=$(env_get OPENDROP_TRUST_PROXY)
    valid_port "${PORT:-x}" || PORT=
  elif [ -e "$NGINX_SITE" ]; then
    # v1 nginx site proxies to 127.0.0.1:3000; keep it and its certificates.
    step "keeping nginx ($NGINX_SITE), opendrop listens on 127.0.0.1:3000"
    BIND=127.0.0.1 PORT=3000 TRUST=true
  elif [ -t 0 ] && [ "$YES" = 0 ]; then
    ask_config
  fi
}

# IPv6 for the container network: with it Docker forwards IPv6 visitors with
# their own address (without it they would all share the gateway address).
probe_ipv6() {
  local net=opendrop-ipv6-check
  if [ -e /proc/net/if_inet6 ]; then ADDR='[::]:8080'; else ADDR=0.0.0.0:8080; fi
  IPV6=false
  [ "$ADDR" != 0.0.0.0:8080 ] || return 0
  docker network rm "$net" >/dev/null 2>&1 || true
  if docker network create --ipv6 "$net" >/dev/null 2>&1; then
    IPV6=true
    docker network rm "$net" >/dev/null 2>&1 || true
  fi
}

finish_config() {
  if [ -n "$DOMAIN" ]; then
    BIND=127.0.0.1 TRUST=true
  else
    # All addresses: IPv4 and IPv6 when the network has IPv6, else IPv4 only.
    case $BIND in '' | 0.0.0.0) if [ "$IPV6" = true ]; then BIND=; else BIND=0.0.0.0; fi ;; esac
    PORT=${PORT:-80} TRUST=${TRUST:-false}
  fi
}

# compose.yml goes to a private temp dir first: nothing in $DIR changes until
# every check has passed.
fetch_compose() {
  TMPD=$(mktemp -d)
  if [ -n "$SRC" ]; then
    cp "$SRC/compose.yml" "$TMPD/compose.yml"
  else
    need_curl
    curl -fsSL "$RAW/compose.yml" -o "$TMPD/compose.yml" || die "cannot download $RAW/compose.yml"
  fi
  grep -q '^services:' "$TMPD/compose.yml" || die "$RAW/compose.yml is not a compose file"
}

have_ss() { command -v ss >/dev/null 2>&1; }
tcp_busy() { have_ss && [ -n "$(ss -Hltn "sport = :$1" 2>/dev/null)" ]; }
udp_busy() { have_ss && [ -n "$(ss -Hlun "sport = :$1" 2>/dev/null)" ]; }
port_users() {
  local u
  have_ss || return 0
  u=$(ss -Hltunp "sport = :$1" 2>/dev/null | sed -n 's/.*users:(("\([^"]*\)".*/\1/p' | sort -u | tr '\n' ' ') || u=
  printf '%s' "${u% }"
}

# Free, already published by this install, or held by the v1 server that is about to stop.
port_ok() {
  if ! tcp_busy "$1"; then return 0; fi
  if docker port opendrop 8080/tcp 2>/dev/null | grep -E ":$1\$" >/dev/null; then return 0; fi
  [ "$V1" = 1 ] && [ "$1" = 3000 ]
}

pick_loopback_port() {
  local p
  PORT=${PORT:-8080}
  if port_ok "$PORT"; then return 0; fi
  [ "$PORT_GIVEN" = 0 ] || die "port $PORT is in use ($(port_users "$PORT")); pick another one with --port"
  for ((p = 8081; p <= 8199; p++)); do
    if port_ok "$p"; then
      say "    port $PORT is in use, opendrop listens on 127.0.0.1:$p for Caddy"
      PORT=$p
      return 0
    fi
  done
  die "no free port between 8080 and 8199 for opendrop; pick one with --port"
}

check_ports() {
  local p
  if [ -n "$DOMAIN" ]; then
    # Caddy shares the host network; its own running container already holds 80 and 443.
    if [ -z "$(docker ps -q --filter label=com.docker.compose.project=opendrop --filter label=com.docker.compose.service=caddy 2>/dev/null)" ]; then
      for p in 80 443; do
        if tcp_busy "$p"; then
          die "port $p is in use ($(port_users "$p")); Caddy needs ports 80 and 443, stop that service first"
        fi
      done
      if udp_busy 443; then die "UDP port 443 is in use ($(port_users 443)); Caddy needs it for HTTP/3"; fi
    fi
    pick_loopback_port
  elif ! port_ok "$PORT"; then
    die "port $PORT is in use ($(port_users "$PORT")); pick another one with --port"
  fi
}

build_image() {
  local ctx=${SRC:-$GIT}
  step "building $IMAGE from $ctx (takes a few minutes)"
  docker build --label opendrop.build=local -t "$IMAGE" "$ctx"
}

# Images are in place before v1 is stopped. Only the default image falls back
# to a source build: a pinned OPENDROP_IMAGE never gets main under its tag.
get_images() {
  local caddy
  IMAGE=$(env_get OPENDROP_IMAGE)
  IMAGE=${IMAGE:-$DEFAULT_IMAGE}
  if [ "$BUILD" = 1 ]; then
    build_image
  else
    step "pulling $IMAGE"
    if docker pull -q "$IMAGE" >/dev/null; then
      :
    elif [ "$IMAGE" = "$DEFAULT_IMAGE" ]; then
      warn "could not pull $IMAGE, building it from source"
      build_image
    elif docker image inspect "$IMAGE" >/dev/null 2>&1; then
      warn "could not pull $IMAGE, using the local copy"
    else
      die "cannot pull $IMAGE (OPENDROP_IMAGE in $DIR/.env); fix the tag, remove that line, or build it from source with --build"
    fi
  fi
  if [ -n "$DOMAIN" ]; then
    # the image line of the caddy service (a plain name, no variables)
    caddy=$(sed -n '/^  caddy:/,/^  [^ ]/s/^    image: *//p' "$TMPD/compose.yml")
    [ -n "$caddy" ] || die "compose.yml has no caddy service"
    if docker pull -q "$caddy" >/dev/null; then
      :
    elif docker image inspect "$caddy" >/dev/null 2>&1; then
      warn "could not pull $caddy, using the local copy"
    else
      die "cannot pull $caddy"
    fi
  fi
}

# Only now, with every check passed, is v1 stopped and moved aside.
migrate_v1() {
  local backup
  [ "$V1" = 1 ] || return 0
  step "stopping opendrop v1 in $DIR"
  if command -v pm2 >/dev/null 2>&1; then
    pm2 delete opendrop >/dev/null 2>&1 || true
    pm2 save --force >/dev/null 2>&1 || true
  fi
  backup=$DIR.v1-backup-$(date +%Y%m%d-%H%M%S)
  mv "$DIR" "$backup"
  # install.sh may run from inside the old dir (a v1 clone that pulled v2).
  case $SRC in "$DIR" | "$DIR"/*) SRC=$backup${SRC#"$DIR"} ;; esac
  say "    v1 moved to $backup"
  say "    old parcels are not migrated (they were temporary anyway)"
  if [ -f "$CLI" ] && grep -q pm2 "$CLI"; then
    rm -f "$CLI"
  fi
}

write_files() {
  local tmp
  mkdir -p "$DIR"

  tmp=$(mktemp "$DIR/.env.XXXXXX")
  {
    say "$ENV_HEADER"
    say "OPENDROP_BIND=$BIND"
    say "OPENDROP_PORT=$PORT"
    say "OPENDROP_TRUST_PROXY=$TRUST"
    say "OPENDROP_IPV6=$IPV6"
    say "OPENDROP_ADDR=$ADDR"
    say "DOMAIN=$DOMAIN"
    if [ -n "$DOMAIN" ]; then say "COMPOSE_PROFILES=https"; fi
    if [ -f "$DIR/.env" ]; then
      grep -v -E '^(#|[[:space:]]*$|OPENDROP_BIND=|OPENDROP_PORT=|OPENDROP_TRUST_PROXY=|OPENDROP_IPV6=|OPENDROP_ADDR=|DOMAIN=|COMPOSE_PROFILES=)' "$DIR/.env" || true
    fi
  } >"$tmp"
  chmod 600 "$tmp"
  mv -f "$tmp" "$DIR/.env"

  cp "$TMPD/compose.yml" "$DIR/compose.yml.tmp"
  chmod 644 "$DIR/compose.yml.tmp"
  mv -f "$DIR/compose.yml.tmp" "$DIR/compose.yml"

  if [ -n "$DOMAIN" ]; then
    # Caddy runs on the host network now; a file from before that cannot reach opendrop:8080.
    if [ -f "$DIR/Caddyfile" ] && grep -q 'reverse_proxy opendrop:8080' "$DIR/Caddyfile"; then
      warn "rewriting $DIR/Caddyfile for Caddy on the host network"
      rm -f "$DIR/Caddyfile"
    fi
    if [ ! -f "$DIR/Caddyfile" ]; then
      cat >"$DIR/Caddyfile" <<'EOF'
{
	admin off
}

{$DOMAIN} {
	encode zstd gzip
	reverse_proxy 127.0.0.1:{$OPENDROP_PORT} {
		flush_interval -1
	}
}
EOF
    fi
    mkdir -p "$DIR/caddy/data" "$DIR/caddy/config"
  fi

  mkdir -p "$DIR/data"
  # The service runs as 10001; a backup restored by root (cp -r, tar) would
  # leave files it cannot open. Symlinks are changed, never followed.
  find -H "$DIR/data" \( ! -user 10001 -o ! -group 10001 \) -exec chown -h 10001:10001 {} +
  chmod 700 "$DIR/data"
  if [ -e "$DIR/data/opendrop.db" ]; then FRESH=0; else FRESH=1; fi
}

json_str() {
  local re="\"$1\"[[:space:]]*:[[:space:]]*\"([^\"]*)\""
  if [[ $2 =~ $re ]]; then printf '%s' "${BASH_REMATCH[1]}"; fi
}

# Fresh install: the admin exists before the first start, so the server never
# logs a password of its own and the summary below is the only place it shows.
create_admin() {
  local out
  out=$(docker run --rm --network none --read-only --cap-drop ALL --security-opt no-new-privileges:true \
    -v "$DIR/data:/data" "$IMAGE" admin-reset --json) || die "could not create the admin account"
  ADMIN_USER=$(json_str username "$out")
  ADMIN_PASS=$(json_str password "$out")
  [ -n "$ADMIN_PASS" ] || die "unexpected admin-reset output: $out"
}

start_stack() {
  if [ -z "$DOMAIN" ]; then
    COMPOSE_PROFILES=https dc rm -f -s caddy >/dev/null 2>&1 || true
  fi
  step "starting"
  dc up -d --remove-orphans || die "docker compose up failed"
}

wait_healthy() {
  local n=0 s=
  while [ "$n" -lt 30 ]; do
    s=$(docker inspect -f '{{.State.Health.Status}}' opendrop 2>/dev/null) || s=
    [ "$s" != healthy ] || break
    n=$((n + 1))
    sleep 2
  done
  if [ "$s" != healthy ]; then
    docker logs --tail 20 opendrop >&2 || true
    die "opendrop is not healthy after 60 s (${s:-no status}); see: opendrop logs"
  fi
}

# Caddy on the host network cannot fail docker compose up over a busy port; it
# restarts instead. Running twice in a row with the same restart count is fine.
check_caddy() {
  local id first now
  [ -n "$DOMAIN" ] || return 0
  id=$(dc ps -q caddy) || id=
  [ -n "$id" ] || die "the Caddy container did not start; see: opendrop logs"
  first=$(docker inspect -f '{{.State.Status}} {{.RestartCount}}' "$id") || first=
  sleep 3
  now=$(docker inspect -f '{{.State.Status}} {{.RestartCount}}' "$id") || now=
  if [ "${first%% *}" != running ] || [ "$now" != "$first" ]; then
    docker logs --tail 20 "$id" >&2 || true
    die "Caddy keeps restarting (ports 80 and 443 taken?); see: opendrop logs"
  fi
}

install_cli() {
  local tmp
  mkdir -p "${CLI%/*}"
  tmp=$(mktemp "$CLI.XXXXXX")
  {
    say '#!/usr/bin/env bash'
    say '# opendrop management command, written by install.sh'
    printf 'DIR=%q\n' "$DIR"
    printf 'GIT=%q\n' "$GIT"
    cat <<'CLI_EOF'
set -euo pipefail
CLI_PATH=/usr/local/bin/opendrop
DEFAULT_IMAGE=ghcr.io/mrtangluyao/opendrop:latest

usage() {
  cat <<EOF
usage: opendrop <command>

  status [--json]      container, health and storage
  logs [-f]            last 200 log lines, -f to follow
  start | stop | restart
  update               pull the latest image and restart
  password [P]         reset the admin password (random when P is omitted)
  uninstall [--purge]  remove containers, images and this command; --purge
                       also deletes the data
  help                 show this help

install dir: $DIR
EOF
}

die() {
  printf 'opendrop: %s\n' "$*" >&2
  exit 1
}

bad_usage() {
  usage >&2
  exit 2
}

dc() { (cd "$DIR" && docker compose "$@"); }

state() { docker inspect -f '{{.State.Status}}' opendrop 2>/dev/null || printf 'missing'; }

env_get() {
  if [ -f "$DIR/.env" ]; then
    sed -n "s/^$1=//p" "$DIR/.env" | tail -n 1
  fi
}

cmd_status() {
  local st health
  case $# in
    0) ;;
    1) [ "$1" = --json ] || bad_usage ;;
    *) bad_usage ;;
  esac
  st=$(state)
  if [ $# -eq 1 ]; then
    if [ "$st" != running ]; then
      printf '{"error":"not_running","container":"%s"}\n' "$st"
      exit 1
    fi
    exec docker exec opendrop /opendrop status --json
  fi
  health=$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{end}}' opendrop 2>/dev/null) || health=
  printf 'container: %s%s\n' "$st" "${health:+ ($health)}"
  [ "$st" = running ] || exit 1
  docker exec opendrop /opendrop status
}

cmd_logs() {
  case $# in
    0) dc logs --tail 200 ;;
    1)
      [ "$1" = -f ] || [ "$1" = --follow ] || bad_usage
      dc logs --tail 200 -f
      ;;
    *) bad_usage ;;
  esac
}

# Only the default image is rebuilt from source when the pull fails, and only
# when it was built locally before; a pinned OPENDROP_IMAGE must be pulled.
cmd_update() {
  local image services
  [ $# -eq 0 ] || bad_usage
  image=$(env_get OPENDROP_IMAGE)
  image=${image:-$DEFAULT_IMAGE}
  if dc pull -q opendrop; then
    :
  elif [ "$image" != "$DEFAULT_IMAGE" ]; then
    die "could not pull $image (pinned by OPENDROP_IMAGE in $DIR/.env)"
  elif [ "$(docker image inspect -f '{{index .Config.Labels "opendrop.build"}}' "$image" 2>/dev/null)" = local ]; then
    printf 'pull failed, rebuilding %s from %s\n' "$image" "$GIT"
    docker build --label opendrop.build=local -t "$image" "$GIT"
  else
    die "could not pull $image"
  fi
  services=$(dc config --services)
  if grep -qx caddy <<<"$services"; then
    dc pull -q caddy || printf 'opendrop: could not pull caddy\n' >&2
  fi
  dc up -d --remove-orphans
  docker image prune -f >/dev/null
  printf 'updated\n'
}

cmd_password() {
  [ $# -le 1 ] || bad_usage
  [ "$(state)" = running ] || die "opendrop is not running (opendrop start)"
  if [ $# -eq 1 ]; then
    [ -n "$1" ] || bad_usage
    docker exec opendrop /opendrop admin-reset --password "$1"
  else
    docker exec opendrop /opendrop admin-reset
  fi
}

# Removes only what install.sh created; anything else in the dir stays.
cmd_uninstall() {
  local purge=0 ans
  case $# in
    0) ;;
    1)
      [ "$1" = --purge ] || bad_usage
      purge=1
      ;;
    *) bad_usage ;;
  esac
  case $DIR in /*/*) ;; *) die "bad install dir: $DIR" ;; esac
  if [ "$purge" = 0 ] && [ -t 0 ]; then
    printf 'Also delete %s/data (all parcels, files and accounts)? [y/N] ' "$DIR"
    read -r ans || ans=
    case $ans in y | Y | yes | YES) purge=1 ;; esac
  fi
  if [ -f "$DIR/compose.yml" ]; then
    COMPOSE_PROFILES=https dc down --rmi all --remove-orphans
  else
    docker rm -f opendrop >/dev/null 2>&1 || true
  fi
  rm -f "$CLI_PATH"
  rm -rf -- "$DIR/compose.yml" "$DIR/.env" "$DIR/Caddyfile" "$DIR/caddy"
  if [ "$purge" = 1 ]; then
    rm -rf -- "$DIR/data"
    rmdir -- "$DIR" 2>/dev/null || true
    printf 'removed opendrop and its data (%s/data)\n' "$DIR"
    if [ -d "$DIR" ]; then printf 'other files in %s were left in place\n' "$DIR"; fi
  else
    printf 'removed opendrop, kept %s/data\n' "$DIR"
    printf 'delete it with: rm -rf %s/data\n' "$DIR"
  fi
}

cmd=${1:-help}
[ $# -eq 0 ] || shift
case $cmd in
  help | -h | --help)
    usage
    exit 0
    ;;
  status | logs | start | stop | restart | update | password | uninstall) ;;
  *)
    printf 'opendrop: unknown command: %s\n\n' "$cmd" >&2
    bad_usage
    ;;
esac

if [ "$(id -u)" -ne 0 ]; then
  command -v sudo >/dev/null 2>&1 || die "run as root"
  exec sudo -- "$0" "$cmd" "$@"
fi
if [ "$cmd" != uninstall ] && [ ! -f "$DIR/compose.yml" ]; then
  die "$DIR/compose.yml is missing, run install.sh again"
fi

case $cmd in
  status) cmd_status "$@" ;;
  logs) cmd_logs "$@" ;;
  start)
    [ $# -eq 0 ] || bad_usage
    dc up -d
    ;;
  stop)
    [ $# -eq 0 ] || bad_usage
    dc stop
    ;;
  restart)
    [ $# -eq 0 ] || bad_usage
    dc restart
    ;;
  update) cmd_update "$@" ;;
  password) cmd_password "$@" ;;
  uninstall) cmd_uninstall "$@" ;;
esac
CLI_EOF
  } >"$tmp"
  chmod 755 "$tmp"
  mv -f "$tmp" "$CLI"
}

site_url() {
  local ip name scheme=http
  if [ -n "$DOMAIN" ]; then
    printf 'https://%s' "$DOMAIN"
  elif [ "$BIND" = 127.0.0.1 ] && [ -f "$NGINX_SITE" ]; then
    name=$(awk '$1 == "server_name" { for (i = 2; i <= NF; i++) { sub(/;$/, "", $i); if ($i != "" && $i != "_") { print $i; exit } } }' "$NGINX_SITE") || name=
    if grep -qE 'listen[^;]*443' "$NGINX_SITE"; then scheme=https; fi
    printf '%s://%s' "$scheme" "${name:-SERVER_IP}"
  else
    if [ -z "$BIND" ] || [ "$BIND" = 0.0.0.0 ]; then
      ip=$(hostname -I 2>/dev/null | awk '{ print $1 }') || ip=
    else
      ip=$BIND
    fi
    ip=${ip:-SERVER_IP}
    if [ "$PORT" = 80 ]; then printf 'http://%s' "$ip"; else printf 'http://%s:%s' "$ip" "$PORT"; fi
  fi
}

summary() {
  local url
  url=$(site_url)
  say ""
  say "${BOLD}opendrop is running${OFF}"
  say "  site   $url"
  say "  admin  $url/admin"
  if [ "$BIND" = 127.0.0.1 ] && [ -z "$DOMAIN" ] && [ -f "$NGINX_SITE" ]; then
    say "  nginx limits the upload size: set client_max_body_size 0; in $NGINX_SITE"
  fi
  if [ -n "$ADMIN_PASS" ]; then
    say ""
    say "${BOLD}admin login, shown only once (not in the logs either):${OFF}"
    say "  username  ${ADMIN_USER:-admin}"
    say "  password  ${BOLD}$ADMIN_PASS${OFF}"
    say "  lost it? opendrop password sets a new one"
  else
    say "  data   $DIR/data (kept)"
  fi
  say ""
  say "commands:"
  say "  opendrop status      state and storage"
  say "  opendrop logs -f     follow the logs"
  say "  opendrop update      update to the latest version"
  say "  opendrop password    reset the admin password"
}

main() {
  local here
  parse_args "$@"
  [ "$(id -u)" -eq 0 ] || die "run as root: sudo bash install.sh"
  trap '[ -z "$TMPD" ] || rm -rf -- "$TMPD"' EXIT
  here=${BASH_SOURCE[0]:-}
  if [ -n "$here" ] && [ -f "$here" ]; then
    here=$(cd "$(dirname "$here")" && pwd)
    if [ -f "$here/compose.yml" ] && [ -f "$here/Dockerfile" ] && [ -f "$here/Cargo.toml" ]; then SRC=$here; fi
  fi
  check_platform
  ensure_docker
  # Checks first: nothing is stopped or written until they all pass.
  detect_v1
  check_dir
  resolve_config
  probe_ipv6
  finish_config
  fetch_compose
  check_ports
  get_images
  # Changes from here on.
  migrate_v1
  write_files
  if [ "$FRESH" = 1 ]; then create_admin; fi
  start_stack
  install_cli
  wait_healthy
  check_caddy
  summary
}

main "$@"
