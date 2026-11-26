//go:build !darwin

package main

import "errors"

func limitResources() error { return nil }

// Dupin has no sandbox for other platforms yet; this build exists to show cross-compilation.
func enterSandbox() error { return errors.New("no sandbox for this platform") }
