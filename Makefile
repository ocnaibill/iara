PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share
SYSTEMD_USER_DIR ?= $(PREFIX)/lib/systemd/user

CARGO ?= cargo
TARGET = release
VERSION ?= 0.1.0

.PHONY: all build check test clean install install-bin uninstall dist deb packages

all: build

build:
	$(CARGO) build --release

check:
	$(CARGO) check --workspace --all-targets

test:
	$(CARGO) test --workspace --all-features

clean:
	$(CARGO) clean
	rm -rf dist

install: build install-bin

install-bin:
	install -d "$(DESTDIR)$(BINDIR)"
	install -m 755 target/release/iara-service "$(DESTDIR)$(BINDIR)/iara-service"
	install -m 755 target/release/iara-ui "$(DESTDIR)$(BINDIR)/iara-ui"
	install -d "$(DESTDIR)$(DATADIR)/applications"
	install -m 644 data/dev.iara.Mixer.desktop "$(DESTDIR)$(DATADIR)/applications/dev.iara.Mixer.desktop"
	install -d "$(DESTDIR)$(DATADIR)/metainfo"
	install -m 644 data/dev.iara.Mixer.metainfo.xml "$(DESTDIR)$(DATADIR)/metainfo/dev.iara.Mixer.metainfo.xml"
	install -d "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps"
	install -m 644 data/icons/hicolor/scalable/apps/dev.iara.Mixer.svg "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/dev.iara.Mixer.svg"
	install -d "$(DESTDIR)$(SYSTEMD_USER_DIR)"
	install -m 644 data/systemd/user/iara.service "$(DESTDIR)$(SYSTEMD_USER_DIR)/iara.service"

uninstall:
	rm -f "$(DESTDIR)$(BINDIR)/iara-service"
	rm -f "$(DESTDIR)$(BINDIR)/iara-ui"
	rm -f "$(DESTDIR)$(DATADIR)/applications/dev.iara.Mixer.desktop"
	rm -f "$(DESTDIR)$(DATADIR)/metainfo/dev.iara.Mixer.metainfo.xml"
	rm -f "$(DESTDIR)$(DATADIR)/icons/hicolor/scalable/apps/dev.iara.Mixer.svg"
	rm -f "$(DESTDIR)$(SYSTEMD_USER_DIR)/iara.service"

dist: build
	./packaging/make-dist.sh $(VERSION)

deb: build
	./packaging/make-deb.sh $(VERSION)

packages: dist deb
