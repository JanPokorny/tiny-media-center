# SRPM generator for Fedora Copr ("make srpm" build method). Copr runs
# `make -f .copr/Makefile srpm outdir=... spec=...` in a mock chroot that,
# unlike the RPM build itself, has network access -- so this is where the
# Rust dependencies get vendored into a tarball for the offline build.
# Runs locally too: `make -f .copr/Makefile srpm` (needs cargo + rpm-build).

NAME    := tiny-media-center
VERSION := $(shell sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)
COMMITS := $(shell git rev-list --count HEAD 2>/dev/null || echo 0)
SHA     := $(shell git rev-parse --short HEAD 2>/dev/null || echo local)

outdir  ?= $(CURDIR)/srpm-build
spec    ?= $(NAME).spec

.PHONY: srpm
srpm:
	command -v cargo >/dev/null && command -v rpmbuild >/dev/null || \
	    dnf -y install cargo git-core rpm-build xz
	mkdir -p "$(outdir)"
	git archive --format=tar --prefix=$(NAME)-$(VERSION)/ HEAD \
	    | gzip > "$(outdir)/$(NAME)-$(VERSION).tar.gz"
	cargo vendor
	tar -cJf "$(outdir)/$(NAME)-$(VERSION)-vendor.tar.xz" vendor
	rpmbuild -bs "$(spec)" \
	    --define "_sourcedir $(outdir)" \
	    --define "_srcrpmdir $(outdir)" \
	    --define "commits $(COMMITS)" \
	    --define "shortcommit $(SHA)"
