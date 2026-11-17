#!/bin/sh
# Copyright 2026 Tor Norbye
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Writes the man page and shell completions that release archives ship:
#
#   scripts/generate-extras.sh <markfmt binary> <output directory>
#
# The output is the same on every platform, so any host binary can generate it for every target.

set -eu

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 <markfmt binary> <output directory>" >&2
    exit 2
fi
BIN="$1"
OUT="$2"

mkdir -p "$OUT/man/man1" "$OUT/completions"
"$BIN" --man > "$OUT/man/man1/markfmt.1"
"$BIN" --completions bash > "$OUT/completions/markfmt.bash"
"$BIN" --completions zsh > "$OUT/completions/_markfmt"
"$BIN" --completions fish > "$OUT/completions/markfmt.fish"
"$BIN" --completions powershell > "$OUT/completions/_markfmt.ps1"
"$BIN" --completions elvish > "$OUT/completions/markfmt.elv"
