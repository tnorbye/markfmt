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

# Builds with `cargo build <args>` and prints the path of the executable it produced, as Cargo
# reports it, so custom target directories and CARGO_BUILD_TARGET are followed:
#
#   scripts/cargo-executable.sh --release --bin markfmt
#
# Compiler messages still go to stderr. Fails if the build fails or produces no executable. Set
# CARGO to use another cargo.

set -eu

messages="$(${CARGO:-cargo} build --message-format=json-render-diagnostics "$@")"

# Decodes the JSON string after the last `"executable":"` (Cargo escapes `\` and `"` in paths,
# and control characters as \u00XX).
path="$(printf '%s\n' "$messages" | awk '
    function hex(s,    i, n, c) {
        n = 0
        for (i = 1; i <= length(s); i++) {
            c = index("0123456789abcdef", tolower(substr(s, i, 1)))
            if (c == 0) return -1
            n = n * 16 + c - 1
        }
        return n
    }
    {
        key = "\"executable\":\""
        start = index($0, key)
        if (start == 0) next
        rest = substr($0, start + length(key))
        out = ""
        while (length(rest) > 0) {
            c = substr(rest, 1, 1)
            if (c == "\"") break
            if (c == "\\") {
                e = substr(rest, 2, 1)
                if (e == "u") {
                    n = hex(substr(rest, 3, 4))
                    if (n < 1 || n > 127) { bad = 1; exit }
                    out = out sprintf("%c", n)
                    rest = substr(rest, 7)
                    continue
                }
                if (e == "n") e = "\n"
                else if (e == "t") e = "\t"
                else if (e == "r") e = "\r"
                else if (e == "b") e = "\b"
                else if (e == "f") e = "\f"
                out = out e
                rest = substr(rest, 3)
                continue
            }
            out = out c
            rest = substr(rest, 2)
        }
        last = out
    }
    END { if (!bad && last != "") print last }
')"

if [ -z "$path" ] || [ ! -f "$path" ]; then
    echo "error: cargo build $* produced no executable" >&2
    exit 1
fi
printf '%s\n' "$path"
