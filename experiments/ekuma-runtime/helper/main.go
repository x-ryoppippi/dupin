// Command helper is an Ekuma counterpart of Dupin's `--extension-host` helper. It reads one JSON
// request per line on standard input and answers each with one JSON line on standard output.
// Every run gets a fresh VM, as every QuickJS-ng run gets a fresh runtime and context.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"os"
	"runtime"
	"runtime/debug"
	"runtime/metrics"
	"strings"
	"time"

	"github.com/hajimehoshi/ekuma"
)

const (
	heapLimit  = 16 << 20
	rowLimit   = 200
	fieldLimit = 1024
	// Go grows goroutine stacks up to 1 GB by default; a native recursion in the engine would
	// take that much before failing.
	maxStack = 64 << 20
)

// intlEnabled is set by intl.go when the helper is built with the `intl` tag.
var intlEnabled bool

type request struct {
	ID         uint64 `json:"id"`
	Op         string `json:"op"`
	Package    uint64 `json:"package,omitempty"`
	Source     string `json:"source,omitempty"`
	Query      string `json:"query,omitempty"`
	DeadlineMS int64  `json:"deadline_ms,omitempty"`
}

type stages struct {
	Setup    float64 `json:"setup_ms"`
	Load     float64 `json:"load_ms"`
	Call     float64 `json:"call_ms"`
	Validate float64 `json:"validate_ms"`
	Total    float64 `json:"total_ms"`
}

type failure struct {
	Kind   string `json:"kind"`
	Detail string `json:"detail"`
}

type row struct {
	ID       string `json:"id,omitempty"`
	Title    string `json:"title"`
	Subtitle string `json:"subtitle,omitempty"`
	Text     string `json:"text,omitempty"`
}

type reply struct {
	ID       uint64   `json:"id"`
	Rows     []row    `json:"rows,omitempty"`
	Failure  *failure `json:"failure,omitempty"`
	Stages   *stages  `json:"stages,omitempty"`
	GCCycles uint64   `json:"gc_cycles,omitempty"`
	// The highest live heap the watchdog saw during the call, over the heap when it began.
	HeapPeak uint64 `json:"heap_peak_bytes,omitempty"`
}

var (
	errDeadline  = errors.New("deadline")
	errHeapLimit = errors.New("heap limit")
)

func main() {
	noSandbox := flag.Bool("no-sandbox", false, "skip sandbox_init, only to see what the sandbox changes")
	flag.StringVar(&hostLocale, "locale", "", "the locale Dupin would pass, such as en-GB")
	flag.Parse()
	debug.SetMaxStack(maxStack)
	// Load the local time zone and the system locale before the sandbox denies the files and
	// frameworks they come from, as the QuickJS helper calls tzset first.
	locale := warmUp()
	if err := limitResources(); err != nil {
		write(map[string]any{"refused": err.Error()})
		os.Exit(1)
	}
	if !*noSandbox {
		if err := enterSandbox(); err != nil {
			write(map[string]any{"refused": err.Error()})
			os.Exit(1)
		}
	}
	write(map[string]any{"hello": map[string]any{
		"engine":    "ekuma v0.1.0",
		"go":        runtime.Version(),
		"intl":      intlEnabled,
		"locale":    locale,
		"sandboxed": !*noSandbox,
	}})

	held := map[uint64]string{}
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 0, 1<<20), 16<<20)
	for scanner.Scan() {
		var req request
		if err := json.Unmarshal(scanner.Bytes(), &req); err != nil {
			write(reply{Failure: &failure{Kind: "protocol", Detail: err.Error()}})
			continue
		}
		switch req.Op {
		case "ping":
			write(reply{ID: req.ID})
		case "compile":
			write(compile(req))
		case "run":
			source, ok := held[req.Package]
			if req.Source != "" {
				source = req.Source
				if req.Package != 0 {
					held[req.Package] = source
				}
			} else if !ok {
				write(reply{ID: req.ID, Failure: &failure{Kind: "missing", Detail: "no package held under that name"}})
				continue
			}
			write(run(req, source))
		default:
			write(reply{ID: req.ID, Failure: &failure{Kind: "protocol", Detail: "unknown op " + req.Op}})
		}
	}
}

var out = bufio.NewWriter(os.Stdout)

func write(v any) {
	data, _ := json.Marshal(v)
	out.Write(data)
	out.WriteByte('\n')
	out.Flush()
}

// localZone is the Mac's named time zone. Each VM resolves the name of time.Local by reading the
// /etc/localtime link, which the sandbox denies, and would fall back to UTC.
var localZone *time.Location

// hostLocale is the locale Dupin passes. Ekuma asks CoreFoundation for the default locale, which
// inside the sandbox can't reach the user's preferences and answers en-US.
var hostLocale string

func warmUp() string {
	time.Now().Zone()
	if link, err := os.Readlink("/etc/localtime"); err == nil {
		if i := strings.LastIndex(link, "zoneinfo/"); i >= 0 {
			localZone, _ = time.LoadLocation(link[i+len("zoneinfo/"):])
		}
	}
	var vm ekuma.VM
	configure(&vm)
	script, err := vm.ParseScript("warm-up.js", `
		Temporal.Now.timeZoneId();
		new Date().toLocaleString();
		typeof Intl === "object" ? new Intl.DateTimeFormat().resolvedOptions().locale : ""`)
	if err != nil {
		return ""
	}
	value, err := script.Evaluate(context.Background())
	if err != nil {
		return ""
	}
	return value.String()
}

func configure(vm *ekuma.VM) {
	if localZone != nil {
		vm.SetTimeZone(localZone)
	}
	if hostLocale != "" && intlEnabled {
		vm.SetLocale(hostLocale)
	}
}

// compile parses the module, which is all Ekuma offers: it has no bytecode a host could keep,
// so the app would cache source and every run parses it again. Parsing takes no context, so
// only the host's watchdog bounds it.
func compile(req request) reply {
	var vm ekuma.VM
	started := time.Now()
	_, err := vm.ParseModule("main.js", req.Source)
	r := reply{ID: req.ID, Stages: &stages{Load: ms(time.Since(started)), Total: ms(time.Since(started))}}
	if err != nil {
		r.Failure = classify(err)
	}
	return r
}

func deadline(req request) time.Duration {
	if req.DeadlineMS <= 0 {
		return 100 * time.Millisecond
	}
	return time.Duration(req.DeadlineMS) * time.Millisecond
}

func run(req request, source string) (r reply) {
	r.ID = req.ID
	start := time.Now()
	st := &stages{}
	r.Stages = st
	gcBefore := gcCycles()
	ctx, cancel := context.WithTimeoutCause(context.Background(), deadline(req), errDeadline)
	defer cancel()
	ctx, stop := context.WithCancelCause(ctx)
	defer stop(nil)
	peak, done := watchHeap(stop)
	defer func() {
		close(done)
		r.HeapPeak = <-peak
		r.GCCycles = gcCycles() - gcBefore
		r.Stages.Total = ms(time.Since(start))
	}()
	// A runaway engine recursion or another Go panic must cost one call, not the helper.
	defer func() {
		if p := recover(); p != nil {
			r.Rows = nil
			r.Failure = &failure{Kind: "panic", Detail: truncate(fmt.Sprint(p))}
		}
	}()

	var vm ekuma.VM
	configure(&vm)
	encoder, err := vm.BindScript(encodeScript).Evaluate(ctx)
	if err != nil {
		r.Failure = classify(err)
		return
	}
	st.Setup = ms(time.Since(start))

	began := time.Now()
	module, err := vm.ParseModule("main.js", source)
	if err != nil {
		r.Failure = classify(err)
		return
	}
	evaluated, err := module.Evaluate(ctx)
	if err == nil {
		_, err = settled(evaluated)
	}
	if err != nil {
		r.Failure = classify(err)
		return
	}
	namespace, err := module.Namespace()
	if err != nil {
		r.Failure = classify(err)
		return
	}
	st.Load = ms(time.Since(began))

	began = time.Now()
	search, err := namespace.Get("search")
	if err != nil || search.Type() != ekuma.TypeFunction {
		r.Failure = &failure{Kind: "invalid", Detail: "missing search export"}
		return
	}
	result, err := search.Invoke(ctx, req.Query)
	if err == nil {
		result, err = settled(result)
	}
	if err != nil {
		r.Failure = classify(err)
		return
	}
	st.Call = ms(time.Since(began))

	// Reading the rows with Value.Get would run guest getters without a deadline, so the guest's
	// own JSON.stringify encodes them under the call's context and Go validates plain data.
	began = time.Now()
	encoded, err := encoder.Invoke(ctx, result)
	if err != nil {
		r.Failure = classify(err)
		return
	}
	rows, fail := validate(encoded)
	r.Rows, r.Failure = rows, fail
	st.Validate = ms(time.Since(began))
	st.Total = ms(time.Since(start))
	return
}

var encodeScript = func() *ekuma.UnboundScript {
	script, err := ekuma.ParseUnboundScript("encode.js", "(value) => JSON.stringify(value)")
	if err != nil {
		panic(err)
	}
	return script
}()

func settled(value ekuma.Value) (ekuma.Value, error) {
	if !value.IsPromise() {
		return value, nil
	}
	state, result := value.PromiseState()
	switch state {
	case ekuma.PromiseFulfilled:
		return result, nil
	case ekuma.PromiseRejected:
		return ekuma.Value{}, &ekuma.ScriptError{Value: result}
	default:
		return ekuma.Value{}, errNeverSettled
	}
}

var errNeverSettled = errors.New("job queue empty before the result settled")

func validate(encoded ekuma.Value) ([]row, *failure) {
	if encoded.Type() != ekuma.TypeString {
		return nil, &failure{Kind: "invalid", Detail: "search must return an array"}
	}
	var raw []map[string]any
	if err := json.Unmarshal([]byte(encoded.String()), &raw); err != nil {
		return nil, &failure{Kind: "invalid", Detail: "search must return an array of objects"}
	}
	if len(raw) > rowLimit {
		return nil, &failure{Kind: "invalid", Detail: fmt.Sprintf("%d rows exceed the %d-row limit", len(raw), rowLimit)}
	}
	rows := make([]row, 0, len(raw))
	for _, item := range raw {
		text := func(name string) (string, bool) {
			value, present := item[name]
			if !present {
				return "", true
			}
			s, ok := value.(string)
			return s, ok && len(s) <= fieldLimit
		}
		title, ok1 := text("title")
		id, ok2 := text("id")
		subtitle, ok3 := text("subtitle")
		body, ok4 := text("text")
		if _, present := item["title"]; !present || !(ok1 && ok2 && ok3 && ok4) {
			return nil, &failure{Kind: "invalid", Detail: "row fields must be strings of at most 1 KiB"}
		}
		rows = append(rows, row{ID: id, Title: title, Subtitle: subtitle, Text: body})
	}
	return rows, nil
}

func classify(err error) *failure {
	var syntax *ekuma.SyntaxError
	var script *ekuma.ScriptError
	var internal *ekuma.InternalError
	switch {
	case errors.Is(err, errDeadline):
		return &failure{Kind: "deadline", Detail: truncate(err.Error())}
	case errors.Is(err, errHeapLimit):
		return &failure{Kind: "heap-limit", Detail: truncate(err.Error())}
	case errors.Is(err, errNeverSettled):
		return &failure{Kind: "never-settled", Detail: err.Error()}
	case errors.As(err, &syntax):
		return &failure{Kind: "syntax", Detail: truncate(err.Error())}
	case errors.As(err, &script):
		return &failure{Kind: "thrown", Detail: truncate(err.Error())}
	case errors.As(err, &internal):
		return &failure{Kind: "internal", Detail: truncate(err.Error())}
	default:
		return &failure{Kind: "engine", Detail: truncate(err.Error())}
	}
}

// watchHeap stops the call when the live heap grows heapLimit past where it began. Ekuma has
// no memory limit of its own, so this is the only in-process cap a host can build.
func watchHeap(stop context.CancelCauseFunc) (<-chan uint64, chan struct{}) {
	done := make(chan struct{})
	peak := make(chan uint64, 1)
	sample := []metrics.Sample{{Name: "/memory/classes/heap/objects:bytes"}}
	read := func() uint64 {
		metrics.Read(sample)
		return sample[0].Value.Uint64()
	}
	base := read()
	go func() {
		var highest uint64
		ticker := time.NewTicker(time.Millisecond)
		defer ticker.Stop()
		for {
			if now := read(); now > base && now-base > highest {
				highest = now - base
				if highest > heapLimit {
					stop(errHeapLimit)
				}
			}
			select {
			case <-done:
				peak <- highest
				return
			case <-ticker.C:
			}
		}
	}()
	return peak, done
}

func gcCycles() uint64 {
	sample := []metrics.Sample{{Name: "/gc/cycles/total:gc-cycles"}}
	metrics.Read(sample)
	return sample[0].Value.Uint64()
}

func ms(d time.Duration) float64 { return float64(d) / float64(time.Millisecond) }

func truncate(s string) string {
	if len(s) > 300 {
		return s[:300]
	}
	return s
}
