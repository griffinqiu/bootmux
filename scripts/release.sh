#!/usr/bin/env bash

set -Eeuo pipefail

action="${1:-}"
track="${2:-}"
requested_version="${3:-}"
channel="${4:-rc}"

project_name="bootmux"
github_repo="${BOOTMUX_GITHUB_REPO:-griffinqiu/bootmux}"
tap_repo="${BOOTMUX_HOMEBREW_TAP_REPO:-griffinqiu/homebrew-tap}"
tap_dir=""
registry="crates-io"

release_files=(
  Cargo.toml
  Cargo.lock
)
stable_release_files=(
  CHANGELOG.md
  README.md
  README.zh-CN.md
  docs/getting-started.md
  docs/manual.md
  docs/manual.zh-CN.md
)
stable_install_docs=(
  README.md
  README.zh-CN.md
  docs/getting-started.md
  docs/manual.md
  docs/manual.zh-CN.md
)

restore_release_files=false
temporary_tap_dir=""
temporary_style_dir=""
temporary_formula_dir=""
temporary_check_parent=""
temporary_check_dir=""
release_lock_dir=""
release_lock_acquired=false
local_formula_restore_needed=false
prepared_formula=""
prepared_formula_sha=""
repo_root=""

log() {
  printf '==> %s\n' "$*"
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'USAGE'
Usage:
  scripts/release.sh version stable|prerelease [VERSION] [CHANNEL]
  scripts/release.sh check stable|prerelease [VERSION] [CHANNEL]
  scripts/release.sh publish stable|prerelease [VERSION] [CHANNEL]

Use the Makefile targets instead of invoking this script directly:
  make release PUBLISH=1
  make release VERSION=0.2.0 PUBLISH=1
  make prerelease PUBLISH=1
  make prerelease CHANNEL=beta PUBLISH=1

For a reviewable, reproducible release, capture the version once:
  VERSION="$(make --no-print-directory release-version)"
  make release-check VERSION="$VERSION"
  make release VERSION="$VERSION" PUBLISH=1
USAGE
}

cleanup() {
  status=$?
  trap - EXIT

  if [[ "$restore_release_files" == "true" ]]; then
    if ! git restore --staged --worktree -- \
      "${release_files[@]}" "${stable_release_files[@]}"; then
      printf 'error: failed to restore temporary release edits\n' >&2
      status=1
    elif [[ -n "$(git status --porcelain)" ]]; then
      printf 'error: release check left unexpected working-tree changes\n' >&2
      git status --short >&2
      status=1
    fi
  fi

  if [[ "$local_formula_restore_needed" == "true" ]]; then
    git restore --staged --worktree -- packaging/homebrew/tap/Formula/bootmux.rb >/dev/null 2>&1 || {
      printf 'error: failed to restore the local Homebrew Formula\n' >&2
      status=1
    }
  fi

  if [[ -n "$temporary_tap_dir" && -d "$temporary_tap_dir" ]]; then
    rm -rf "$temporary_tap_dir"
  fi

  if [[ -n "$temporary_style_dir" && -d "$temporary_style_dir" ]]; then
    rm -rf "$temporary_style_dir"
  fi

  if [[ -n "$temporary_formula_dir" && -d "$temporary_formula_dir" ]]; then
    rm -rf "$temporary_formula_dir"
  fi

  if [[ -n "$temporary_check_dir" && -n "$repo_root" ]]; then
    git -C "$repo_root" worktree remove --force "$temporary_check_dir" >/dev/null 2>&1 || {
      printf 'error: failed to remove the temporary release-check worktree\n' >&2
      status=1
    }
  fi

  if [[ -n "$temporary_check_parent" && -d "$temporary_check_parent" ]]; then
    rm -rf "$temporary_check_parent"
  fi

  if [[ "$release_lock_acquired" == "true" && -d "$release_lock_dir" ]]; then
    rm -rf "$release_lock_dir"
  fi

  exit "$status"
}
trap cleanup EXIT

need() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

manifest_version_from() {
  awk '
    /^\[package\]$/ {
      in_package = 1
      next
    }
    in_package && /^\[/ {
      exit
    }
    in_package && /^version = "[^"]+"$/ {
      value = $0
      sub(/^version = "/, "", value)
      sub(/"$/, "", value)
      print value
      exit
    }
  ' "$1"
}

lockfile_version() {
  awk -v project="$project_name" '
    /^\[\[package\]\]$/ {
      in_package = 1
      name = ""
      version = ""
      next
    }
    in_package && /^name = "[^"]+"$/ {
      name = $0
      sub(/^name = "/, "", name)
      sub(/"$/, "", name)
      next
    }
    in_package && /^version = "[^"]+"$/ {
      version = $0
      sub(/^version = "/, "", version)
      sub(/"$/, "", version)
      if (name == project) {
        print version
        exit
      }
    }
  ' Cargo.lock
}

validate_version() {
  local value="$1"
  local prerelease=""
  local identifier=""
  local -a prerelease_parts
  if [[ ! "$value" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]]; then
    die "invalid semantic version: $value"
  fi

  prerelease="$(version_prerelease "$value")"
  if [[ -n "$prerelease" ]]; then
    IFS=. read -r -a prerelease_parts <<< "$prerelease"
    for identifier in "${prerelease_parts[@]}"; do
      if [[ "$identifier" =~ ^[0-9]+$ && "$identifier" != "0" && "$identifier" == 0* ]]; then
        die "numeric prerelease identifiers cannot have leading zeroes: $value"
      fi
    done
  fi
}

version_core() {
  local value="${1%%+*}"
  printf '%s\n' "${value%%-*}"
}

version_prerelease() {
  local value="${1%%+*}"
  if [[ "$value" == *-* ]]; then
    printf '%s\n' "${value#*-}"
  fi
}

compare_number() {
  local left="$1"
  local right="$2"
  if (( 10#$left > 10#$right )); then
    return 0
  fi
  return 1
}

semver_is_greater() {
  local left_version="$1"
  local right_version="$2"
  local left_major left_minor left_patch
  local right_major right_minor right_patch
  local component left_component right_component
  local left_pre right_pre
  local index left_part right_part
  local -a left_parts right_parts

  IFS=. read -r left_major left_minor left_patch <<< "$(version_core "$left_version")"
  IFS=. read -r right_major right_minor right_patch <<< "$(version_core "$right_version")"

  for component in major minor patch; do
    eval "left_component=\$left_${component}"
    eval "right_component=\$right_${component}"
    if compare_number "$left_component" "$right_component"; then
      return 0
    fi
    if compare_number "$right_component" "$left_component"; then
      return 1
    fi
  done

  left_pre="$(version_prerelease "$left_version")"
  right_pre="$(version_prerelease "$right_version")"

  [[ -z "$left_pre" && -n "$right_pre" ]] && return 0
  [[ -n "$left_pre" && -z "$right_pre" ]] && return 1
  [[ -z "$left_pre" && -z "$right_pre" ]] && return 1

  IFS=. read -r -a left_parts <<< "$left_pre"
  IFS=. read -r -a right_parts <<< "$right_pre"
  index=0
  while (( index < ${#left_parts[@]} || index < ${#right_parts[@]} )); do
    if (( index >= ${#left_parts[@]} )); then
      return 1
    fi
    if (( index >= ${#right_parts[@]} )); then
      return 0
    fi

    left_part="${left_parts[$index]}"
    right_part="${right_parts[$index]}"
    if [[ "$left_part" == "$right_part" ]]; then
      index=$((index + 1))
      continue
    fi

    if [[ "$left_part" =~ ^[0-9]+$ && "$right_part" =~ ^[0-9]+$ ]]; then
      compare_number "$left_part" "$right_part"
      return $?
    fi
    if [[ "$left_part" =~ ^[0-9]+$ ]]; then
      return 1
    fi
    if [[ "$right_part" =~ ^[0-9]+$ ]]; then
      return 0
    fi
    [[ "$left_part" > "$right_part" ]]
    return $?
  done

  return 1
}

resolve_version() {
  local current="$1"
  local target=""
  local current_core=""
  local current_pre=""
  local target_pre=""
  local current_major current_minor current_patch

  if [[ -n "$requested_version" ]]; then
    target="$requested_version"
  else
    IFS=. read -r current_major current_minor current_patch <<< "$(version_core "$current")"
    current_core="$(version_core "$current")"
    current_pre="$(version_prerelease "$current")"

    if [[ "$track" == "stable" ]]; then
      if [[ -n "$current_pre" ]]; then
        target="$current_core"
      else
        target="${current_major}.${current_minor}.$((10#$current_patch + 1))"
      fi
    else
      [[ "$channel" =~ ^[0-9A-Za-z-]+$ ]] ||
        die "CHANNEL must contain only letters, digits, and hyphens"

      if [[ -z "$current_pre" ]]; then
        target="${current_major}.${current_minor}.$((10#$current_patch + 1))-${channel}.1"
      elif [[ "$current_pre" =~ ^${channel}\.([0-9]+)$ ]]; then
        target="${current_core}-${channel}.$((10#${BASH_REMATCH[1]} + 1))"
      else
        target="${current_core}-${channel}.1"
      fi
    fi
  fi

  validate_version "$target"
  target_pre="$(version_prerelease "$target")"
  if [[ "$track" == "stable" && -n "$target_pre" ]]; then
    die "stable releases cannot use a prerelease version: $target"
  fi
  if [[ "$track" == "prerelease" && -z "$target_pre" ]]; then
    die "prereleases must include a suffix such as -rc.1: $target"
  fi
  if [[ "$target" != "$current" ]] && ! semver_is_greater "$target" "$current"; then
    die "target version $target must be newer than $current"
  fi

  printf '%s\n' "$target"
}

update_manifest_version() {
  local target="$1"
  local output
  output="$(mktemp "${TMPDIR:-/tmp}/bootmux-cargo.XXXXXX")"

  awk -v target="$target" '
    /^\[package\]$/ {
      in_package = 1
      print
      next
    }
    in_package && /^\[/ {
      in_package = 0
    }
    in_package && /^version = "[^"]+"$/ && !updated {
      print "version = \"" target "\""
      updated = 1
      next
    }
    {
      print
    }
    END {
      if (!updated) {
        exit 42
      }
    }
  ' Cargo.toml > "$output" || {
    rm "$output"
    die "could not update [package].version in Cargo.toml"
  }

  chmod 0644 "$output"
  mv "$output" Cargo.toml
}

update_stable_doc_versions() {
  local target="$1"
  BOOTMUX_TARGET_VERSION="$target" perl -pi -e \
    's/cargo:bootmux\@\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?/cargo:bootmux\@$ENV{BOOTMUX_TARGET_VERSION}/g' \
    "${stable_install_docs[@]}"
}

run_formula_style() {
  local formula="$1"
  temporary_style_dir="$(mktemp -d "${TMPDIR:-/tmp}/bootmux-brew-style.XXXXXX")"
  HOMEBREW_CACHE="$temporary_style_dir" HOMEBREW_NO_AUTO_UPDATE=1 \
    brew style "$formula"
  rm -rf "$temporary_style_dir"
  temporary_style_dir=""
}

roll_changelog() {
  local target="$1"
  local output
  local release_date
  if changelog_has_version "$target"; then
    return
  fi
  grep -Fqx '## [Unreleased]' CHANGELOG.md ||
    die "CHANGELOG.md must contain an exact '## [Unreleased]' heading"
  awk '
    $0 == "## [Unreleased]" {
      in_unreleased = 1
      next
    }
    in_unreleased && /^## / {
      exit
    }
    in_unreleased && $0 !~ /^[[:space:]]*$/ && $0 !~ /^[[:space:]]*<!--/ {
      found = 1
      exit
    }
    END {
      exit(found ? 0 : 1)
    }
  ' CHANGELOG.md ||
    die "CHANGELOG.md [Unreleased] section is empty"

  output="$(mktemp "${TMPDIR:-/tmp}/bootmux-changelog.XXXXXX")"
  release_date="$(date +%F)"
  awk -v target="$target" -v release_date="$release_date" '
    $0 == "## [Unreleased]" {
      print
      print ""
      print "## [" target "] - " release_date
      next
    }
    {
      print
    }
  ' CHANGELOG.md > "$output"
  chmod 0644 "$output"
  mv "$output" CHANGELOG.md
}

changelog_has_version() {
  local target="$1"
  local escaped_target="${target//./\\.}"
  grep -Eq "^## \\[${escaped_target}\\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" CHANGELOG.md ||
    grep -Eq "^## ${escaped_target} - [0-9]{4}-[0-9]{2}-[0-9]{2}$" CHANGELOG.md
}

validate_release_files() {
  local target="$1"
  local expected_track="$2"
  local lock_version
  local document
  lock_version="$(lockfile_version)"
  [[ "$lock_version" == "$target" ]] ||
    die "Cargo.lock uses $lock_version instead of $target"

  if [[ "$expected_track" == "stable" ]]; then
    changelog_has_version "$target" ||
      die "CHANGELOG.md has no dated section for $target"
    for document in "${stable_install_docs[@]}"; do
      grep -Fq "cargo:bootmux@$target" "$document" ||
        die "$document does not pin the stable mise version $target"
    done
  fi
}

run_quality_gates() {
  log "Running Rust and documentation quality gates"
  cargo fmt --all -- --check
  cargo clippy --all-targets --locked -- -D warnings
  cargo test --all-targets --locked
  bash -n completion/bootmux.bash
  zsh -n completion/bootmux.zsh
  if command -v fish >/dev/null 2>&1; then
    fish -n completion/bootmux.fish
  fi
  ruby -c packaging/homebrew/tap/Formula/bootmux.rb >/dev/null
  if [[ "$track" == "stable" ]]; then
    run_formula_style packaging/homebrew/tap/Formula/bootmux.rb
  fi
  git diff --check

  log "Checking the crates.io package without publishing"
  cargo publish --dry-run --allow-dirty --locked --registry "$registry"
}

crate_exists() {
  cargo info "${project_name}@$1" --registry "$registry" >/dev/null 2>&1
}

release_exists() {
  gh release view "v$1" --repo "$github_repo" >/dev/null 2>&1
}

remote_tag_exists() {
  git ls-remote --exit-code --tags origin "refs/tags/v$1" >/dev/null 2>&1
}

local_tag_exists() {
  git show-ref --verify --quiet "refs/tags/v$1"
}

release_preparation_marker_exists() {
  local target="$1"
  local version_line
  local version_commit
  local subject

  version_line="$(
    awk '
      /^\[package\]$/ {
        in_package = 1
        next
      }
      in_package && /^\[/ {
        exit
      }
      in_package && /^version = "[^"]+"$/ {
        print NR
        exit
      }
    ' Cargo.toml
  )"
  [[ -n "$version_line" ]] || return 1

  version_commit="$(
    git blame --porcelain -L "${version_line},${version_line}" -- Cargo.toml |
      awk 'NR == 1 { print $1 }'
  )"
  [[ -n "$version_commit" ]] || return 1
  subject="$(git show -s --format=%s "$version_commit")"
  [[ "$subject" == "chore(release): prepare bootmux $target" ]]
}

ensure_pending_track_matches() {
  local target="$1"
  local pending_track="stable"
  local resume_target="release"

  if [[ -n "$(version_prerelease "$target")" ]]; then
    pending_track="prerelease"
    resume_target="prerelease"
  fi

  [[ "$track" == "$pending_track" ]] ||
    die "bootmux $target is an unfinished $pending_track release; resume it with: make $resume_target VERSION=$target PUBLISH=1"
}

current_version_is_pending_release() {
  local target="$1"
  local expected_prerelease=false
  local draft_state

  if [[ -n "$(version_prerelease "$target")" ]]; then
    expected_prerelease=true
  fi

  if release_exists "$target"; then
    validate_existing_release "$target" "$expected_prerelease"
    draft_state="$(release_is_draft "$target")"
    case "$draft_state" in
      false) return 1 ;;
      true) return 0 ;;
      *) die "GitHub returned an invalid draft state for v$target: $draft_state" ;;
    esac
  fi

  local_tag_exists "$target" ||
    remote_tag_exists "$target" ||
    crate_exists "$target" ||
    release_preparation_marker_exists "$target"
}

validate_existing_release() {
  local target="$1"
  local expected_prerelease="$2"
  local actual
  local expected
  actual="$(
    gh release view "v$target" \
      --repo "$github_repo" \
      --json tagName,isPrerelease \
      --jq '[.tagName, (.isPrerelease | tostring)] | @tsv'
  )"
  expected="v${target}"$'\t'"${expected_prerelease}"
  [[ "$actual" == "$expected" ]] ||
    die "existing GitHub release v$target has unexpected metadata"
}

release_is_draft() {
  gh release view "v$1" \
    --repo "$github_repo" \
    --json isDraft \
    --jq '.isDraft'
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

published_crate_commit() {
  local target="$1"
  local archive
  local source_url
  local attempt=1
  local vcs_json
  local published_commit
  archive="$(mktemp "${TMPDIR:-/tmp}/bootmux-crate.XXXXXX")"
  source_url="https://static.crates.io/crates/${project_name}/${project_name}-${target}.crate"

  while ! curl --fail --location --silent --show-error "$source_url" -o "$archive"; do
    if (( attempt >= 20 )); then
      rm "$archive"
      die "crates.io did not make $project_name $target downloadable"
    fi
    attempt=$((attempt + 1))
    sleep 3
  done

  vcs_json="$(
    tar -xOf "$archive" \
      "${project_name}-${target}/.cargo_vcs_info.json" 2>/dev/null
  )" || {
    rm "$archive"
    die "published crate $project_name $target has no VCS metadata"
  }
  published_commit="$(
    printf '%s\n' "$vcs_json" |
      ruby -rjson -e '
        metadata = JSON.parse(STDIN.read)
        abort "published crate has dirty VCS metadata" if metadata.dig("git", "dirty")
        commit = metadata.dig("git", "sha1")
        abort "published crate has no VCS commit" unless commit
        print commit
      '
  )" || {
    rm "$archive"
    die "published crate $project_name $target has invalid VCS metadata"
  }
  rm "$archive"

  printf '%s\n' "$published_commit"
}

verify_published_crate() {
  local target="$1"
  local expected_commit="$2"
  local published_commit
  published_commit="$(published_crate_commit "$target")"
  [[ "$published_commit" == "$expected_commit" ]] ||
    die "published crate $project_name $target came from $published_commit, expected $expected_commit"
}

formula_tag_version() {
  sed -n \
    's#^  url "https://github.com/.*/archive/refs/tags/v\([^"]*\)\.tar\.gz"$#\1#p' \
    "$1" |
    awk 'NR == 1 { print; exit }'
}

formula_sha256() {
  sed -n 's/^  sha256 "\([^"]*\)"$/\1/p' "$1" |
    awk 'NR == 1 { print; exit }'
}

validate_formula_update() {
  local formula="$1"
  local target="$2"
  local expected_sha="$3"
  local current_formula_version
  local current_formula_sha
  current_formula_version="$(formula_tag_version "$formula")"
  current_formula_sha="$(formula_sha256 "$formula")"

  [[ -n "$current_formula_version" && -n "$current_formula_sha" ]] ||
    die "could not read the current version and SHA-256 from $formula"
  validate_version "$current_formula_version"

  if [[ "$current_formula_version" == "$target" ]]; then
    [[ "$current_formula_sha" == "$expected_sha" ]] ||
      die "$formula already uses $target with a different SHA-256"
    return
  fi

  semver_is_greater "$target" "$current_formula_version" ||
    die "refusing to downgrade $formula from $current_formula_version to $target"
}

update_formula() {
  local formula="$1"
  local archive_url="$2"
  local archive_sha="$3"
  local output
  output="$(mktemp "${TMPDIR:-/tmp}/bootmux-formula.XXXXXX")"

  awk -v url="$archive_url" -v sha="$archive_sha" '
    /^  url "/ {
      print "  url \"" url "\""
      found_url = 1
      next
    }
    /^  sha256 "/ {
      print "  sha256 \"" sha "\""
      found_sha = 1
      next
    }
    {
      print
    }
    END {
      if (!found_url || !found_sha) {
        exit 42
      }
    }
  ' "$formula" > "$output" || {
    rm "$output"
    die "could not update Homebrew formula: $formula"
  }

  chmod 0644 "$output"
  mv "$output" "$formula"
}

prepare_homebrew_formula() {
  local target="$1"
  local tag="v$target"
  local archive_url="https://github.com/${github_repo}/archive/refs/tags/${tag}.tar.gz"
  local archive
  local archive_sha
  local tap_origin
  local local_formula="packaging/homebrew/tap/Formula/bootmux.rb"
  local tap_formula
  local tap_readme

  archive="$(mktemp "${TMPDIR:-/tmp}/bootmux-archive.XXXXXX")"

  log "Downloading the versioned GitHub tag archive"
  curl --fail --location --retry 3 --silent --show-error "$archive_url" -o "$archive"
  archive_sha="$(sha256_file "$archive")"
  rm "$archive"

  temporary_tap_dir="$(mktemp -d "${TMPDIR:-/tmp}/bootmux-tap.XXXXXX")"
  tap_dir="$temporary_tap_dir/repository"
  gh repo clone "$tap_repo" "$tap_dir" -- --quiet

  [[ "$(git -C "$tap_dir" branch --show-current)" == "main" ]] ||
    die "Homebrew tap checkout must be on main: $tap_dir"
  tap_origin="$(git -C "$tap_dir" remote get-url origin)"
  tap_origin="${tap_origin%.git}"
  if [[ "$tap_origin" != "https://github.com/${tap_repo}" &&
    "$tap_origin" != "git@github.com:${tap_repo}" &&
    "$tap_origin" != "ssh://git@github.com/${tap_repo}" ]]; then
    die "Homebrew tap origin does not match $tap_repo: $tap_origin"
  fi
  [[ -z "$(git -C "$tap_dir" status --porcelain)" ]] ||
    die "Homebrew tap checkout is not clean: $tap_dir"
  git -C "$tap_dir" fetch --quiet origin main
  [[ "$(git -C "$tap_dir" rev-parse HEAD)" == "$(git -C "$tap_dir" rev-parse origin/main)" ]] ||
    die "Homebrew tap checkout does not exactly match origin/main"
  git -C "$tap_dir" push --dry-run origin HEAD:main >/dev/null

  tap_formula="$tap_dir/Formula/bootmux.rb"
  tap_readme="$tap_dir/README.md"
  [[ -f "$tap_formula" ]] || die "Homebrew formula not found: $tap_formula"
  [[ -f "$tap_readme" ]] || die "Homebrew tap README not found: $tap_readme"
  validate_formula_update "$local_formula" "$target" "$archive_sha"
  validate_formula_update "$tap_formula" "$target" "$archive_sha"

  temporary_formula_dir="$(mktemp -d "${TMPDIR:-/tmp}/bootmux-formula-stage.XXXXXX")"
  mkdir "$temporary_formula_dir/Formula"
  prepared_formula="$temporary_formula_dir/Formula/bootmux.rb"
  prepared_formula_sha="$archive_sha"
  cp "$local_formula" "$prepared_formula"
  update_formula "$prepared_formula" "$archive_url" "$archive_sha"
  ruby -c "$prepared_formula" >/dev/null
  run_formula_style "$prepared_formula"
}

publish_homebrew_formula() {
  local target="$1"
  local local_formula="packaging/homebrew/tap/Formula/bootmux.rb"
  local local_tap_readme="packaging/homebrew/tap/README.md"
  local tap_formula="$tap_dir/Formula/bootmux.rb"
  local tap_readme="$tap_dir/README.md"

  [[ -n "$prepared_formula" && -f "$prepared_formula" ]] ||
    die "the Homebrew Formula was not prepared before publication"
  [[ -n "$prepared_formula_sha" ]] ||
    die "the prepared Homebrew Formula has no archive SHA-256"
  [[ -d "$tap_dir/.git" ]] || die "the prepared Homebrew tap checkout is missing"

  git -C "$tap_dir" fetch --quiet origin main
  [[ "$(git -C "$tap_dir" rev-parse HEAD)" == "$(git -C "$tap_dir" rev-parse origin/main)" ]] ||
    die "Homebrew tap changed after Formula preparation; rerun the release"
  [[ -z "$(git -C "$tap_dir" status --porcelain)" ]] ||
    die "Homebrew tap checkout changed after Formula preparation"
  validate_formula_update "$local_formula" "$target" "$prepared_formula_sha"
  validate_formula_update "$tap_formula" "$target" "$prepared_formula_sha"

  local_formula_restore_needed=true
  cp "$prepared_formula" "$local_formula"
  cp "$prepared_formula" "$tap_formula"
  cp "$local_tap_readme" "$tap_readme"

  if ! git -C "$tap_dir" diff --quiet -- Formula/bootmux.rb README.md; then
    git -C "$tap_dir" add Formula/bootmux.rb README.md
    git -C "$tap_dir" commit -m "bootmux $target"
  fi
  git -C "$tap_dir" push origin HEAD:main

  if ! git diff --quiet -- "$local_formula"; then
    git add "$local_formula"
    git commit -m "chore(homebrew): update bootmux to $target"
    local_formula_restore_needed=false
    git push origin HEAD:main
  else
    local_formula_restore_needed=false
  fi

  rm -rf "$temporary_formula_dir"
  temporary_formula_dir=""
  prepared_formula=""
  prepared_formula_sha=""
}

if [[ "${BOOTMUX_RELEASE_TEST_MODE:-0}" == "1" ]]; then
  trap - EXIT
  return 0 2>/dev/null || exit 0
fi

[[ "$action" == "version" || "$action" == "check" || "$action" == "publish" ]] || {
  usage >&2
  exit 2
}
[[ "$track" == "stable" || "$track" == "prerelease" ]] || {
  usage >&2
  exit 2
}

need awk
need cargo
need git

repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" ||
  die "run this command from a bootmux Git checkout"
cd "$repo_root"

if [[ "$action" == "check" && "${BOOTMUX_INTERNAL_RELEASE_CHECK:-0}" != "1" ]]; then
  [[ -z "$(git status --porcelain)" ]] ||
    die "the working tree must be clean; commit the current code before running a release check"

  temporary_check_parent="$(mktemp -d "${TMPDIR:-/tmp}/bootmux-release-check.XXXXXX")"
  temporary_check_dir="$temporary_check_parent/repository"
  git worktree add --detach --quiet "$temporary_check_dir" HEAD

  log "Running the release check in an isolated temporary worktree"
  (
    cd "$temporary_check_dir"
    BOOTMUX_INTERNAL_RELEASE_CHECK=1 \
      bash scripts/release.sh "$action" "$track" "$requested_version" "$channel"
  )
  exit 0
fi

current_version="$(manifest_version_from Cargo.toml)"
[[ -n "$current_version" ]] || die "could not read the package version from Cargo.toml"
validate_version "$current_version"
target_version="$(resolve_version "$current_version")"

if [[ "$action" == "version" ]]; then
  printf '%s\n' "$target_version"
  exit 0
fi

if [[ "$action" == "publish" && "${BOOTMUX_PUBLISH:-0}" != "1" ]]; then
  die "publishing is irreversible; rerun with PUBLISH=1 after a successful release check"
fi

need gh
need perl
need ruby
need curl
need sed
need tar
need fish
need zsh
if [[ "$track" == "stable" ]]; then
  need brew
fi
if ! command -v sha256sum >/dev/null 2>&1; then
  need shasum
fi

umask 077
release_lock_dir="${TMPDIR:-/tmp}/bootmux-release.lock"
if ! mkdir "$release_lock_dir" 2>/dev/null; then
  die "another release command is active, or a stale lock exists: $release_lock_dir"
fi
release_lock_acquired=true

[[ -z "$(git status --porcelain)" ]] ||
  die "the working tree must be clean; commit the current code before releasing"

branch="$(git branch --show-current)"
if [[ "$action" == "publish" && "$branch" != "main" ]]; then
  die "publish releases from the main branch, not $branch"
fi
origin_url="$(git remote get-url origin)"
origin_url="${origin_url%.git}"
if [[ "$origin_url" != "https://github.com/${github_repo}" &&
  "$origin_url" != "git@github.com:${github_repo}" &&
  "$origin_url" != "ssh://git@github.com/${github_repo}" ]]; then
  die "origin does not match $github_repo: $origin_url"
fi

git fetch --quiet origin main --tags
git merge-base --is-ancestor origin/main HEAD ||
  die "local history has diverged from origin/main"
if [[ "$action" == "publish" ]]; then
  git push --dry-run origin HEAD:main >/dev/null
fi

if [[ "$action" == "publish" && -z "$requested_version" ]] &&
  current_version_is_pending_release "$current_version"; then
  ensure_pending_track_matches "$current_version"
  target_version="$current_version"
  log "Resuming unfinished $track release $target_version"
fi

log "Release plan: $current_version -> $target_version ($track)"

if [[ "$target_version" != "$current_version" ]]; then
  if crate_exists "$target_version" || remote_tag_exists "$target_version" || release_exists "$target_version"; then
    die "version $target_version already exists in at least one release source"
  fi

  restore_release_files=true
  update_manifest_version "$target_version"
  if [[ "$track" == "stable" ]]; then
    update_stable_doc_versions "$target_version"
    roll_changelog "$target_version"
  fi
  cargo check
fi

validate_release_files "$target_version" "$track"
run_quality_gates

if [[ "$action" == "check" ]]; then
  log "Release check passed; no tags, packages, releases, or tap changes were published"
  exit 0
fi

if [[ "$target_version" != "$current_version" ]]; then
  git add "${release_files[@]}"
  if [[ "$track" == "stable" ]]; then
    git add "${stable_release_files[@]}"
  fi
  git commit -m "chore(release): prepare bootmux $target_version"
  restore_release_files=false
fi

log "Verifying the clean release package"
cargo publish --dry-run --locked --registry "$registry"

tag="v$target_version"
crate_already_published=false
crate_verified=false
if crate_exists "$target_version"; then
  crate_already_published=true
  release_commit="$(published_crate_commit "$target_version")"
  git cat-file -e "${release_commit}^{commit}" 2>/dev/null ||
    die "published crate commit $release_commit is not present in this checkout"
else
  release_commit="$(git rev-parse HEAD)"
fi

commit_version="$(
  git show "${release_commit}:Cargo.toml" |
    manifest_version_from /dev/stdin
)"
[[ "$commit_version" == "$target_version" ]] ||
  die "release commit $release_commit contains bootmux $commit_version, expected $target_version"

if git show-ref --verify --quiet "refs/tags/$tag"; then
  tagged_version="$(git show "$tag:Cargo.toml" | manifest_version_from /dev/stdin)"
  [[ "$tagged_version" == "$target_version" ]] ||
    die "local tag $tag does not contain bootmux $target_version"
  local_tag_commit="$(git rev-list -n 1 "$tag")"
  [[ "$local_tag_commit" == "$release_commit" ]] ||
    die "local tag $tag points to $local_tag_commit, expected $release_commit"
else
  git tag -a "$tag" "$release_commit" -m "bootmux $target_version"
fi

remote_tag_was_present=false
if remote_tag_exists "$target_version"; then
  remote_tag_was_present=true
  remote_commit="$(
    git ls-remote --tags origin "refs/tags/${tag}^{}" |
      awk 'NR == 1 { print $1 }'
  )"
  if [[ -z "$remote_commit" ]]; then
    remote_commit="$(
      git ls-remote --tags origin "refs/tags/${tag}" |
        awk 'NR == 1 { print $1 }'
    )"
  fi
  [[ "$remote_commit" == "$release_commit" ]] ||
    die "remote tag $tag points to $remote_commit, expected $release_commit"
fi

git push origin HEAD:main
if [[ "$remote_tag_was_present" != "true" ]]; then
  git push origin "$tag"
fi

if [[ "$track" == "stable" ]]; then
  log "Preparing and validating the Homebrew Formula before publishing the crate"
  prepare_homebrew_formula "$target_version"
fi

expected_prerelease=false
if [[ "$track" == "prerelease" ]]; then
  expected_prerelease=true
fi

if release_exists "$target_version"; then
  validate_existing_release "$target_version" "$expected_prerelease"
  log "GitHub release $tag already exists; preserving its current draft state"
else
  log "Creating draft GitHub release $tag"
  if [[ "$track" == "prerelease" ]]; then
    gh release create "$tag" \
      --repo "$github_repo" \
      --verify-tag \
      --title "bootmux $target_version" \
      --generate-notes \
      --draft \
      --prerelease
  else
    gh release create "$tag" \
      --repo "$github_repo" \
      --verify-tag \
      --title "bootmux $target_version" \
      --generate-notes \
      --draft
  fi
fi

if [[ "$crate_already_published" == "true" ]]; then
  log "crates.io already contains $project_name $target_version; skipping cargo publish"
else
  log "Publishing $project_name $target_version to crates.io"
  if ! cargo publish --locked --registry "$registry"; then
    log "cargo publish returned an error; waiting for the uploaded crate before deciding whether publication failed"
    verify_published_crate "$target_version" "$release_commit"
    crate_verified=true
  fi
fi

if [[ "$crate_verified" != "true" ]]; then
  verify_published_crate "$target_version" "$release_commit"
fi

if [[ "$track" == "stable" ]]; then
  publish_homebrew_formula "$target_version"
else
  log "Keeping the Homebrew formula on the latest stable release"
fi

if ! draft_state="$(release_is_draft "$target_version")"; then
  die "could not verify the draft state of GitHub release $tag"
fi
case "$draft_state" in
  true)
    log "Publishing GitHub release $tag"
    gh release edit "$tag" --repo "$github_repo" --draft=false
    ;;
  false) ;;
  *) die "GitHub returned an invalid draft state for $tag: $draft_state" ;;
esac

if ! final_draft_state="$(release_is_draft "$target_version")"; then
  die "could not verify the final state of GitHub release $tag"
fi
[[ "$final_draft_state" == "false" ]] ||
  die "GitHub release $tag is still a draft"

log "Release $target_version is complete"
printf '\nInstall this exact release:\n'
printf '  cargo install %s --version %s --locked\n' "$project_name" "$target_version"
printf '  mise use -g cargo:%s@%s\n' "$project_name" "$target_version"
if [[ "$track" == "stable" ]]; then
  printf '  brew install griffinqiu/tap/%s\n' "$project_name"
fi
