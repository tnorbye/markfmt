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
PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
CARGO_ABOUT ?= cargo about
LICENSES_FILE := target/THIRD-PARTY-LICENSES.txt

.PHONY: check fmt format fmt-check lint test doc build release licenses dist install uninstall clean

check: fmt-check lint test doc

fmt:
	$(CARGO) fmt --all
	$(CARGO) run $(CARGO_FLAGS) -- .

format: fmt

fmt-check:
	$(CARGO) fmt --all -- --check
	$(CARGO) run $(CARGO_FLAGS) -- --check .

lint:
	$(CARGO) clippy $(CARGO_FLAGS) --all-targets -- -D warnings

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

dist: licenses
	@set -e; \
	VERSION="$$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"; \
	if [ -n "$(TARGET)" ]; then \
		TRIPLE="$(TARGET)"; \
		$(CARGO) build --release --target "$$TRIPLE" $(CARGO_FLAGS); \
		BIN_DIR="target/$$TRIPLE/release"; \
	else \
		TRIPLE="$$($(RUSTC) -vV | sed -n 's/^host: //p')"; \
		$(CARGO) build --release $(CARGO_FLAGS); \
		BIN_DIR="target/release"; \
	fi; \
	BIN_NAME="markfmt"; \
	if [ -f "$$BIN_DIR/markfmt.exe" ]; then BIN_NAME="markfmt.exe"; fi; \
	STAGE="markfmt-v$${VERSION}-$${TRIPLE}"; \
	rm -rf "target/dist/$$STAGE" "target/dist/$${STAGE}.tar.gz" "target/dist/$${STAGE}.tar.gz.sha256"; \
	mkdir -p "target/dist/$$STAGE"; \
	cp "$$BIN_DIR/$$BIN_NAME" README.md LICENSE $(LICENSES_FILE) "target/dist/$$STAGE/"; \
	tar -C target/dist -czf "target/dist/$${STAGE}.tar.gz" "$$STAGE"; \
	rm -rf "target/dist/$$STAGE"; \
	if command -v sha256sum >/dev/null 2>&1; then \
		(cd target/dist && sha256sum "$${STAGE}.tar.gz" > "$${STAGE}.tar.gz.sha256"); \
	elif command -v shasum >/dev/null 2>&1; then \
		(cd target/dist && shasum -a 256 "$${STAGE}.tar.gz" > "$${STAGE}.tar.gz.sha256"); \
	fi; \
	echo "Created distribution: target/dist/$${STAGE}.tar.gz"

install: release
	install -d "$(DESTDIR)$(BINDIR)"
	rm -f "$(DESTDIR)$(BINDIR)/markfmt"
	install -m 755 target/release/markfmt "$(DESTDIR)$(BINDIR)/markfmt"
	@echo "Installed $(DESTDIR)$(BINDIR)/markfmt"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/markfmt"

clean:
	$(CARGO) clean
