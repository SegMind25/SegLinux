# Build the segctl package from the working tree on an Arch/Artix host:
#   make -C pkg/segctl package
VERSION := $(shell sed -n 's/^pkgver=//p' PKGBUILD)
ROOT := $(abspath ../..)

source:
	tar -C $(ROOT) --exclude=./tools/seg/target \
	    --transform 's,^\./tools/seg,segctl-$(VERSION)/tools/seg,' -czf segctl-$(VERSION).tar.gz ./tools/seg

package: source
	makepkg -f --noconfirm

clean:
	rm -rf src pkg segctl-*.tar.gz *.pkg.tar.*

.PHONY: source package clean
