#!/usr/bin/env bash
# Shared helpers for AoC / check scripts (dotenv + provider API key).
# shellcheck shell=bash

_AOC_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/aoc-lib.py"

import_dotenv() {
  local path="$1"
  local line name value

  if [[ ! -f "$path" ]]; then
    return 0
  fi

  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    if [[ -z "$line" || "$line" == \#* ]]; then
      continue
    fi
    if [[ "$line" != *=* ]]; then
      continue
    fi
    name="${line%%=*}"
    value="${line#*=}"
    name="${name%"${name##*[![:space:]]}"}"
    name="${name#"${name%%[![:space:]]*}"}"
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value%"${value##*[![:space:]]}"}"
    if [[ ${#value} -ge 2 ]]; then
      if [[ "$value" == \"*\" ]]; then
        value="${value:1:${#value}-2}"
      elif [[ "$value" == \'*\' ]]; then
        value="${value:1:${#value}-2}"
      fi
    fi
    if [[ -z "${!name+x}" || -z "${!name}" ]]; then
      export "$name=$value"
    fi
  done <"$path"
}

# Usage: yaml_scalar <key> [default] <<< "$yaml_text"
yaml_scalar() {
  local key="$1"
  local default="${2:-}"
  python3 "$_AOC_LIB" yaml-scalar "$key" "$default"
}

# Usage: yaml_provider_api_key <<< "$yaml_text"
yaml_provider_api_key() {
  python3 "$_AOC_LIB" yaml-api-key
}

provider_api_key_available() {
  local text="$1"
  python3 "$_AOC_LIB" api-key-available <<<"$text"
}
