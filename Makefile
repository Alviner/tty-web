IMAGE ?= ghcr.io/alviner/tty-web
VERSION ?= $(shell cargo metadata --format-version=1 --no-deps | jq -r '.packages[0].version')
TARGET ?= $(shell rustc -vV | awk '/host/{print $$2}' | sed 's/gnu/musl/')

.PHONY: build run release clean fmt lint check
.PHONY: docker upload

build:
	cargo build

run:
	cargo run

release:
	cargo build --release --target $(TARGET)

clean:
	cargo clean

fmt:
	cargo fmt

lint:
	cargo clippy -- -D warnings

check:
	cargo check

docker: release
	docker build --build-arg BINARY=target/$(TARGET)/release/tty-web -t $(IMAGE):$(VERSION) .
