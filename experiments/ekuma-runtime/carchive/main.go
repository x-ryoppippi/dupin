// Package main is Ekuma built with -buildmode=c-archive, to measure what linking it into the
// `dupin` executable would cost instead of shipping a second binary.
package main

import "C"

import (
	"context"
	"time"

	"github.com/hajimehoshi/ekuma"
)

//export dupin_ekuma_eval
func dupin_ekuma_eval(source *C.char) *C.char {
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	var vm ekuma.VM
	script, err := vm.ParseScript("main.js", C.GoString(source))
	if err != nil {
		return C.CString(err.Error())
	}
	value, err := script.Evaluate(ctx)
	if err != nil {
		return C.CString(err.Error())
	}
	return C.CString(value.String())
}

func main() {}
