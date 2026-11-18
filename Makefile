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

CARGO ?= cargo
RUSTC ?= rustc
CARGO_FLAGS ?=
TARGET ?=
# Extra flags for the host build that generates the man page and completions in `make dist`
# (CARGO_FLAGS go to the release build of TARGET, and may select another target).
HOST_CARGO_FLAGS ?=
PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
MANDIR ?= $(PREFIX)/share/man
CARGO_ABOUT ?= cargo about
CARGO_DENY ?= cargo deny
CARGO_DENY_FLAGS ?=
TYPOS ?= typos
LICENSES_FILE := target/THIRD-PARTY-LICENSES.txt

.PHONY: check fmt format fmt-check lint deny spell semver test doc build release licenses dist install uninstall clean

check: fmt-check lint spell deny test doc

fmt:
	$(CARGO) fmt --all
	$(CARGO) run $(CARGO_FLAGS) -- .

format: fmt

fmt-check:
	$(CARGO) fmt --all -- --check
	$(CARGO) run $(CARGO_FLAGS) -- --check .

lint:
	$(CARGO) clippy $(CARGO_FLAGS) --all-targets -- -D warnings

# Spelling mistakes in source, tests, and docs (deliberate misspellings are listed in
# _typos.toml). Skipped with a warning when typos isn't installed; CI always runs it. Install with:
#   cargo install --locked typos-cli
spell:
	@if $(TYPOS) --version >/dev/null 2>&1; then \
		$(TYPOS); \
	else \
		echo "warning: typos is not installed; skipping spell check" >&2; \
		echo "         (cargo install --locked typos-cli)" >&2; \
	fi

# Breaking public API changes since the latest crates.io release. The release workflow runs it
# before publishing. Requires cargo-semver-checks:
#   cargo install --locked cargo-semver-checks
semver:
	$(CARGO) semver-checks

# Known vulnerabilities, unmaintained or yanked crates, and unexpected sources in Cargo.lock
# (configured in deny.toml). Fetches the RustSec advisory database into
# $CARGO_HOME/advisory-dbs (~/.cargo by default), so it needs the network and a writable cache
# there (set CARGO_HOME to move it); a fetch or cache failure fails the check. CARGO_DENY_FLAGS
# adds global flags, such as `make deny CARGO_DENY_FLAGS=--offline` to use a fetched database.
# Skipped with a warning when cargo-deny isn't installed; CI always runs it. Install with:
#   cargo install --locked cargo-deny
deny:
	@if $(CARGO_DENY) --version >/dev/null 2>&1; then \
		$(CARGO_DENY) --locked --all-features $(CARGO_DENY_FLAGS) check advisories bans sources; \
	else \
		echo "warning: cargo-deny is not installed; skipping dependency checks" >&2; \
		echo "         (cargo install --locked cargo-deny)" >&2; \
	fi

test:
	$(CARGO) test $(CARGO_FLAGS)

doc:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --no-deps $(CARGO_FLAGS)

build:
	$(CARGO) build $(CARGO_FLAGS)

release:
	$(CARGO) build --release $(CARGO_FLAGS)

# Third-party license notices for binary distributions, generated from Cargo.lock. Fails if a
# dependency's license is not accepted in about.toml. Requires cargo-about:
#   cargo install --locked cargo-about --features cli
licenses:
	@mkdir -p target
	$(CARGO_ABOUT) generate --locked --fail --config about.toml about.hbs -o $(LICENSES_FILE)

# Builds and packages TARGET (default: CARGO_BUILD_TARGET, else the host), always passing it to
# Cargo as --target so the archive name matches the binary inside it.
dist: licenses
	@set -e; \
	VERSION="$$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"; \
	HOST="$$($(RUSTC) -vV | sed -n 's/^host: //p')"; \
	set -- $(CARGO_FLAGS); \
	for arg do \
		case "$$arg" in --target | --target=*) \
			echo "error: make dist names the archive after its target; pass TARGET=<triple> instead of --target in CARGO_FLAGS" >&2; \
			exit 1;; \
		esac; \
	done; \
	TRIPLE="$(TARGET)"; \
	if [ -z "$$TRIPLE" ]; then TRIPLE="$${CARGO_BUILD_TARGET:-$$HOST}"; fi; \
	BIN="$$(CARGO="$(CARGO)" sh scripts/cargo-executable.sh --release --target "$$TRIPLE" \
		--bin markfmt "$$@")"; \
	STAGE="markfmt-v$${VERSION}-$${TRIPLE}"; \
	rm -rf "target/dist/$$STAGE" "target/dist/$${STAGE}.tar.gz" "target/dist/$${STAGE}.tar.gz.sha256"; \
	mkdir -p "target/dist/$$STAGE"; \
	cp "$$BIN" README.md LICENSE $(LICENSES_FILE) "target/dist/$$STAGE/"; \
	EXTRAS_BIN="$$(CARGO="$(CARGO)" sh scripts/cargo-executable.sh --target "$$HOST" \
		--bin markfmt $(HOST_CARGO_FLAGS))"; \
	sh scripts/generate-extras.sh "$$EXTRAS_BIN" "target/dist/$$STAGE"; \
	tar -C target/dist -czf "target/dist/$${STAGE}.tar.gz" "$$STAGE"; \
	rm -rf "target/dist/$$STAGE"; \
	if command -v sha256sum >/dev/null 2>&1; then \
		(cd target/dist && sha256sum "$${STAGE}.tar.gz" > "$${STAGE}.tar.gz.sha256"); \
	elif command -v shasum >/dev/null 2>&1; then \
		(cd target/dist && shasum -a 256 "$${STAGE}.tar.gz" > "$${STAGE}.tar.gz.sha256"); \
	fi; \
	echo "Created distribution: target/dist/$${STAGE}.tar.gz"

# The binary's path comes from Cargo, so custom target directories and CARGO_BUILD_TARGET work.
# The man page comes from that binary, or from a host build (HOST_CARGO_FLAGS) when it cannot run
# here, such as for a cross-compilation target. It is generated before anything is installed.
install:
	@set -e; \
	BIN="$$(CARGO="$(CARGO)" sh scripts/cargo-executable.sh --release --bin markfmt $(CARGO_FLAGS))"; \
	MAN="$$(mktemp)"; \
	trap 'rm -f "$$MAN"' EXIT; \
	if ! "$$BIN" --man > "$$MAN" 2>/dev/null; then \
		HOST="$$($(RUSTC) -vV | sed -n 's/^host: //p')"; \
		MAN_BIN="$$(CARGO="$(CARGO)" sh scripts/cargo-executable.sh --target "$$HOST" \
			--bin markfmt $(HOST_CARGO_FLAGS))"; \
		"$$MAN_BIN" --man > "$$MAN"; \
	fi; \
	install -d "$(DESTDIR)$(BINDIR)" "$(DESTDIR)$(MANDIR)/man1"; \
	rm -f "$(DESTDIR)$(BINDIR)/markfmt"; \
	install -m 755 "$$BIN" "$(DESTDIR)$(BINDIR)/markfmt"; \
	install -m 644 "$$MAN" "$(DESTDIR)$(MANDIR)/man1/markfmt.1"; \
	echo "Installed $(DESTDIR)$(BINDIR)/markfmt and $(DESTDIR)$(MANDIR)/man1/markfmt.1"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/markfmt" "$(DESTDIR)$(MANDIR)/man1/markfmt.1"

clean:
	$(CARGO) clean
