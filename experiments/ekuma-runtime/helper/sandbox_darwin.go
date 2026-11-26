package main

import (
	"errors"
	"syscall"
	"unsafe"

	"github.com/ebitengine/purego"
)

// The same limits and Seatbelt profile as crates/dupin-extension-host/src/sandbox.rs. purego
// reaches libSystem without cgo, so the helper stays a pure Go build.
func limitResources() error {
	for _, limit := range []struct {
		resource int
		value    uint64
	}{
		{4 /* RLIMIT_CORE */, 0},
		{1 /* RLIMIT_FSIZE */, 0},
		{7 /* RLIMIT_NPROC */, 0},
		{8 /* RLIMIT_NOFILE */, 8},
	} {
		if err := syscall.Setrlimit(limit.resource, &syscall.Rlimit{Cur: limit.value, Max: limit.value}); err != nil {
			return err
		}
	}
	return nil
}

func enterSandbox() error {
	library, err := purego.Dlopen("/usr/lib/libSystem.B.dylib", purego.RTLD_NOW|purego.RTLD_GLOBAL)
	if err != nil {
		return err
	}
	symbol, err := purego.Dlsym(library, "sandbox_init")
	if err != nil {
		return err
	}
	profile := []byte("(version 1)\n(deny default)\n\x00")
	var reason *byte
	status, _, _ := purego.SyscallN(symbol, uintptr(unsafe.Pointer(&profile[0])), 0, uintptr(unsafe.Pointer(&reason)))
	if int32(status) != 0 {
		return errors.New("sandbox_init refused the profile")
	}
	return nil
}
