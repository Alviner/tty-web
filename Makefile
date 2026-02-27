IMAGE ?= ghcr.io/alviner/tty-web
VERSION ?= $(shell cargo metadata --format-version=1 --no-deps | jq -r '.packages[0].version')

.PHONY: build run release clean fmt lint check
.PHONY: docker upload

build:
	cargo build

run:
	cargo run

release:
	cargo build --release

clean:
	cargo clean

fmt:
	cargo fmt

lint:
	cargo clippy -- -D warnings

check:
	cargo check

docker: release
	docker build -t $(IMAGE):$(VERSION) .
