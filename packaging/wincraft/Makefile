# Build the wincraft package from the working tree on an Arch/Artix host:
#   make -C packaging/wincraft package
VERSION := $(shell sed -n 's/^pkgver=//p' PKGBUILD)
ROOT := $(abspath ../..)

source:
	tar -C $(ROOT) --exclude=./target --exclude=./.git --exclude=./iso/out \
	    --exclude=./iso/iso --exclude=./packaging/wincraft/src --exclude=./packaging/wincraft/pkg \
	    --exclude='*.pkg.tar.*' --exclude='./packaging/wincraft/*.tar.gz' \
	    --transform 's,^\.,wincraft-$(VERSION),' -czf wincraft-$(VERSION).tar.gz .

package: source
	makepkg -f --noconfirm

clean:
	rm -rf src pkg wincraft-*.tar.gz *.pkg.tar.*

.PHONY: source package clean
