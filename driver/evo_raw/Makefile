obj-m := evo_raw.o

KDIR ?= /lib/modules/$(shell uname -r)/build

# Detect if the running kernel was built with Clang/LLVM
ifeq ($(shell grep -q "CONFIG_CC_IS_CLANG=y" $(KDIR)/.config 2>/dev/null && echo clang),clang)
  CC ?= clang
  LLVM ?= 1
  export CC LLVM
endif

all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules

clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) clean

install:
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules_install
	depmod -a

.PHONY: all clean install
