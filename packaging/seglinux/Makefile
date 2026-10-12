# Build the seglinux package from the working tree on an Arch/Artix host:
#   make -C packaging/seglinux package
VERSION := $(shell sed -n 's/^pkgver=//p' PKGBUILD)
ROOT := $(abspath ../..)

source:
	tar -C $(ROOT) --exclude=./target --exclude=./.git --exclude=./iso/out \
	    --exclude=./iso/iso --exclude=./packaging/seglinux/src --exclude=./packaging/seglinux/pkg \
	    --exclude='*.pkg.tar.*' --exclude='./packaging/seglinux/*.tar.gz' \
	    --transform 's,^\.,seglinux-$(VERSION),' -czf seglinux-$(VERSION).tar.gz .

package: source
	makepkg -f --noconfirm

clean:
	rm -rf src pkg seglinux-*.tar.gz *.pkg.tar.*

.PHONY: source package clean
