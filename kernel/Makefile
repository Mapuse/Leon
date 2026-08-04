TARGET  ?= x86_64-unknown-uefi
PROFILE ?= release

.PHONY: build clippy clean

build:
	cargo build --locked --target $(TARGET) --profile $(PROFILE)

clippy:
	cargo clippy --all-targets --target $(TARGET) -- -D warnings

clean:
	cargo clean
