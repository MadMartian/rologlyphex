PREFIX ?= /usr/local
BINDIR = $(PREFIX)/bin
SERVICEDIR = $(HOME)/.config/systemd/user

.PHONY: build install uninstall

build:
	cargo build --release

install: build
	sudo install -Dm755 target/release/rologlyphex $(BINDIR)/rologlyphex
	install -Dm644 rologlyphex.service $(SERVICEDIR)/rologlyphex.service

uninstall:
	sudo rm -f $(BINDIR)/rologlyphex
	rm -f $(SERVICEDIR)/rologlyphex.service
