# SRPM generator for Fedora Copr ("make srpm" build method). Copr runs
# `make -f .copr/Makefile srpm outdir=... spec=...` in a mock chroot that,
# unlike the RPM build itself, has network access -- so this is where the
# Rust dependencies get vendored into a tarball for the offline build.
# Runs locally too: `make -f .copr/Makefile srpm` (needs cargo + rpm-build).
#
# Copr's spec= argument is the checkout directory, not a file, so it is
# ignored; the spec file is found by name at the repo root. SPECFILE must
# not be named "spec", or Copr's command-line argument would override it.

NAME     := tiny-media-center
VERSION  := $(shell sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)
SPECFILE := $(NAME).spec

outdir ?= $(CURDIR)/srpm-build

.PHONY: srpm
srpm:
	command -v cargo >/dev/null && command -v rpmbuild >/dev/null && command -v git >/dev/null || \
	    dnf -y install cargo git-core rpm-build xz
	mkdir -p "$(outdir)"
	git archive --format=tar --prefix=$(NAME)-$(VERSION)/ HEAD \
	    | gzip > "$(outdir)/$(NAME)-$(VERSION).tar.gz"
	cargo vendor
	tar -cJf "$(outdir)/$(NAME)-$(VERSION)-vendor.tar.xz" vendor
	# The snapshot info runs inside the recipe: git is only installed by the
	# first recipe line, so $(shell git ...) at parse time would come up
	# empty in the Copr chroot. The committer timestamp (rather than a
	# commit count) keeps the Release monotonic even in Copr's shallow
	# (--depth 500) clones.
	snapdate=$$(git log -1 --format=%cd --date=format:%Y%m%d%H%M) && \
	sha=$$(git rev-parse --short HEAD) && \
	rpmbuild -bs "$(SPECFILE)" \
	    --define "_sourcedir $(outdir)" \
	    --define "_srcrpmdir $(outdir)" \
	    --define "snapdate $$snapdate" \
	    --define "shortcommit $$sha"
