// Command bench drives the Ekuma helper over its line protocol and measures what
// crates/dupin/tests/extension_host.rs measures for the QuickJS-ng helper, plus the containment
// fixtures of experiments/js-runtime run through the helper.
package main

import (
	"bufio"
	_ "embed"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"runtime/debug"
	"slices"
	"strings"
	"time"
	"unsafe"

	"github.com/ebitengine/purego"
)

const (
	searchQuery    = "pr"
	wordQuery      = "two words"
	searchRows     = 50
	budget         = 100 * time.Millisecond
	tolerance      = 10 * time.Millisecond
	outerLimit     = 2 * time.Second
	killAfter      = 10 * time.Second
	heapLimit      = 16 << 20
	heapSlack      = 8 << 20
	warmupCalls    = 50
	footprintCalls = 1000
)

var cases = []string{
	"busy-loop",
	"busy-module-body",
	"busy-async-job",
	"heap-object-spike",
	"heap-string-repeat",
	"heap-string-doubling",
	"heap-array-fill",
	"heap-typed-array",
	"stack-recursion",
	"stack-json-parse",
	"stack-json-stringify",
	"promise-chain-storm",
	"promise-fanout-storm",
	"regexp-backtracking",
	"native-string-search",
	"native-array-sort",
}

var (
	helperPath = flag.String("helper", "", "path to the helper binary")
	fixtures   = flag.String("fixtures", "experiments/js-runtime/bench/fixtures", "the js-runtime fixtures")
	label      = flag.String("label", "ekuma", "label for the results")
	samples    = flag.Int("samples", 2000, "timed warm calls")
	coldRuns   = flag.Int("cold-runs", 100, "fresh helpers per cold measurement")
	gogc       = flag.String("gogc", "", "GOGC for the helper; empty keeps Go's default of 100")
	locale     = flag.String("locale", "en-GB", "the locale passed to the helper")
)

func main() {
	flag.Parse()
	// The driver's own collections must not land in the helper's timings.
	debug.SetGCPercent(-1)
	args := flag.Args()
	if *helperPath == "" || len(args) == 0 {
		fmt.Fprintln(os.Stderr, "usage: bench -helper <path> [flags] all | contain <case>... | features")
		os.Exit(2)
	}
	var output any
	switch args[0] {
	case "all":
		output = all()
	case "contain":
		output = contain(args[1:])
	case "features":
		output = features()
	default:
		fmt.Fprintln(os.Stderr, "unknown mode", args[0])
		os.Exit(2)
	}
	data, _ := json.MarshalIndent(output, "", "  ")
	fmt.Println(string(data))
}

type helper struct {
	cmd    *exec.Cmd
	stdin  io.WriteCloser
	lines  chan []byte
	nextID uint64
	hello  map[string]any
}

// start launches a helper as Dupin does: a cleared environment, started in /.
func start() (*helper, error) {
	cmd := exec.Command(*helperPath, "-locale", *locale)
	cmd.Dir = "/"
	cmd.Env = []string{}
	if *gogc != "" {
		cmd.Env = append(cmd.Env, "GOGC="+*gogc)
	}
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return nil, err
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return nil, err
	}
	cmd.Stderr = io.Discard
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	h := &helper{cmd: cmd, stdin: stdin, lines: make(chan []byte, 1)}
	go func() {
		reader := bufio.NewReaderSize(stdout, 1<<20)
		for {
			line, err := reader.ReadBytes('\n')
			if err != nil {
				close(h.lines)
				return
			}
			h.lines <- line
		}
	}()
	line, ok := <-h.lines
	if !ok {
		h.stop()
		return nil, fmt.Errorf("helper exited before hello")
	}
	var hello struct {
		Hello   map[string]any `json:"hello"`
		Refused string         `json:"refused"`
	}
	if err := json.Unmarshal(line, &hello); err != nil || hello.Hello == nil {
		h.stop()
		return nil, fmt.Errorf("no hello: %s %s", hello.Refused, line)
	}
	h.hello = hello.Hello
	return h, nil
}

func (h *helper) stop() {
	h.stdin.Close()
	done := make(chan struct{})
	go func() { h.cmd.Wait(); close(done) }()
	select {
	case <-done:
	case <-time.After(time.Second):
		h.cmd.Process.Kill()
		<-done
	}
}

type reply struct {
	ID   uint64 `json:"id"`
	Rows []struct {
		Title string `json:"title"`
		Text  string `json:"text,omitempty"`
	} `json:"rows"`
	Failure *struct {
		Kind   string `json:"kind"`
		Detail string `json:"detail"`
	} `json:"failure"`
	Stages struct {
		Setup    float64 `json:"setup_ms"`
		Load     float64 `json:"load_ms"`
		Call     float64 `json:"call_ms"`
		Validate float64 `json:"validate_ms"`
		Total    float64 `json:"total_ms"`
	} `json:"stages"`
	GCCycles uint64 `json:"gc_cycles"`
	HeapPeak uint64 `json:"heap_peak_bytes"`
}

// call sends one request and waits for its answer, or kills the helper after wait.
func (h *helper) call(req map[string]any, wait time.Duration) (*reply, error) {
	h.nextID++
	req["id"] = h.nextID
	data, _ := json.Marshal(req)
	if _, err := h.stdin.Write(append(data, '\n')); err != nil {
		return nil, err
	}
	timer := time.NewTimer(wait)
	defer timer.Stop()
	select {
	case line, ok := <-h.lines:
		if !ok {
			return nil, fmt.Errorf("helper exited: %v", h.exitStatus())
		}
		var r reply
		if err := json.Unmarshal(line, &r); err != nil {
			return nil, fmt.Errorf("bad reply %q: %w", line, err)
		}
		return &r, nil
	case <-timer.C:
		h.cmd.Process.Kill()
		return nil, errKilled
	}
}

var errKilled = fmt.Errorf("killed after the outer limit")

func (h *helper) exitStatus() string {
	h.cmd.Wait()
	if h.cmd.ProcessState == nil {
		return "unknown"
	}
	return h.cmd.ProcessState.String()
}

func (h *helper) search(source string, pkg uint64, query string) (*reply, error) {
	req := map[string]any{"op": "run", "query": query, "deadline_ms": 2000}
	if source != "" {
		req["source"] = source
	}
	if pkg != 0 {
		req["package"] = pkg
	}
	return h.call(req, killAfter)
}

func expectRows(r *reply, err error, rows int) {
	if err != nil {
		panic(err)
	}
	if r.Failure != nil {
		panic(fmt.Sprintf("%s: %s", r.Failure.Kind, r.Failure.Detail))
	}
	if len(r.Rows) != rows {
		panic(fmt.Sprintf("expected %d rows, got %d", rows, len(r.Rows)))
	}
}

func searchSource() string {
	adjectives := []string{"amber", "brisk", "cobalt", "dusty", "eager", "frosty", "gentle", "hollow", "ivory", "jolly", "keen",
		"lunar", "mellow", "nimble", "olive", "proud", "quiet", "rapid", "silver", "tidal"}
	nouns := []string{"anchor", "beacon", "canyon", "delta", "ember", "falcon", "garden", "harbor", "island", "jungle",
		"kettle", "lantern", "meadow", "nebula", "orchard", "prairie", "quarry", "river", "summit", "thicket"}
	capitalize := func(s string) string { return strings.ToUpper(s[:1]) + s[1:] }
	type candidate struct {
		ID       string `json:"id"`
		Title    string `json:"title"`
		Subtitle string `json:"subtitle"`
		Path     string `json:"path"`
	}
	var data []candidate
	for _, adjective := range adjectives {
		for _, noun := range nouns {
			id := adjective + "-" + noun
			data = append(data, candidate{id, capitalize(adjective) + " " + capitalize(noun), "~/Projects/" + id, "/Users/example/Projects/" + id})
		}
	}
	encoded, _ := json.Marshal(data)
	return "const DATA = " + string(encoded) + ";\n" + read("search.js")
}

func read(name string) string {
	data, err := os.ReadFile(filepath.Join(*fixtures, name))
	if err != nil {
		panic(err)
	}
	return string(data)
}

func millis(d time.Duration) float64 { return float64(d) / float64(time.Millisecond) }

func summary(values []float64) map[string]any {
	if len(values) == 0 {
		return map[string]any{"count": 0}
	}
	sorted := slices.Clone(values)
	slices.Sort(sorted)
	rank := func(p float64) float64 {
		i := int(math.Ceil(p/100*float64(len(sorted)))) - 1
		return sorted[max(i, 0)]
	}
	var sum float64
	for _, v := range sorted {
		sum += v
	}
	return map[string]any{
		"count": len(sorted), "min_ms": sorted[0], "p50_ms": rank(50), "p95_ms": rank(95),
		"p99_ms": rank(99), "max_ms": sorted[len(sorted)-1], "mean_ms": sum / float64(len(sorted)),
	}
}

func all() map[string]any {
	search := searchSource()
	words := read("word-count.js")

	fmt.Fprintf(os.Stderr, "[%s] compile and cold…\n", *label)
	var compiles, colds, coldStarts []float64
	for range *coldRuns {
		began := time.Now()
		h, err := start()
		if err != nil {
			panic(err)
		}
		r, err := h.call(map[string]any{"op": "compile", "source": search}, killAfter)
		if err != nil || r.Failure != nil {
			panic(fmt.Sprint(err, r))
		}
		compiles = append(compiles, millis(time.Since(began)))
		h.stop()
	}
	for range *coldRuns {
		began := time.Now()
		h, err := start()
		if err != nil {
			panic(err)
		}
		coldStarts = append(coldStarts, millis(time.Since(began)))
		r, err := h.search(search, 0, searchQuery)
		expectRows(r, err, searchRows)
		colds = append(colds, millis(time.Since(began)))
		h.stop()
	}

	fmt.Fprintf(os.Stderr, "[%s] warm…\n", *label)
	h, err := start()
	if err != nil {
		panic(err)
	}
	idle := footprint(h)
	r, err := h.search(search, 1, searchQuery)
	expectRows(r, err, searchRows)
	afterFirst := footprint(h)
	for range warmupCalls {
		r, err := h.search("", 1, searchQuery)
		expectRows(r, err, searchRows)
	}
	var warm, withGC, withoutGC, setup, load, call, validate, inHelper []float64
	var gcCalls int
	for range *samples {
		began := time.Now()
		r, err := h.search("", 1, searchQuery)
		expectRows(r, err, searchRows)
		took := millis(time.Since(began))
		warm = append(warm, took)
		if r.GCCycles > 0 {
			gcCalls++
			withGC = append(withGC, took)
		} else {
			withoutGC = append(withoutGC, took)
		}
		setup = append(setup, r.Stages.Setup)
		load = append(load, r.Stages.Load)
		call = append(call, r.Stages.Call)
		validate = append(validate, r.Stages.Validate)
		inHelper = append(inHelper, r.Stages.Total)
	}
	afterWarm := footprint(h)
	r, err = h.search(words, 2, wordQuery)
	expectRows(r, err, 1)
	var wordWarm []float64
	for range *samples {
		began := time.Now()
		r, err := h.search("", 2, wordQuery)
		expectRows(r, err, 1)
		wordWarm = append(wordWarm, millis(time.Since(began)))
	}
	var pings []float64
	for range *samples {
		began := time.Now()
		if _, err := h.call(map[string]any{"op": "ping"}, killAfter); err != nil {
			panic(err)
		}
		pings = append(pings, millis(time.Since(began)))
	}
	for range footprintCalls {
		r, err := h.search("", 1, searchQuery)
		expectRows(r, err, searchRows)
	}
	afterCycles := footprint(h)
	hello := h.hello
	h.stop()

	fmt.Fprintf(os.Stderr, "[%s] containment…\n", *label)
	containment := contain(cases)
	var failed []string
	for _, c := range containment {
		if !c["pass"].(bool) {
			failed = append(failed, c["case"].(string))
		}
	}
	p := func(values []float64, q string) float64 { return summary(values)[q].(float64) }
	return map[string]any{
		"label":                 *label,
		"hello":                 hello,
		"gogc":                  *gogc,
		"host":                  map[string]any{"goos": runtime.GOOS, "goarch": runtime.GOARCH, "cpus": runtime.NumCPU()},
		"source":                map[string]any{"search_bytes": len(search)},
		"compile_in_own_helper": summary(compiles),
		"cold_first_result":     summary(colds),
		"cold_spawn_to_hello":   summary(coldStarts),
		"warm_search_400": map[string]any{
			"round_trip":            summary(warm),
			"calls_with_gc":         gcCalls,
			"round_trip_with_gc":    summary(withGC),
			"round_trip_without_gc": summary(withoutGC),
			"helper_setup":          summary(setup),
			"helper_load":           summary(load),
			"helper_call":           summary(call),
			"helper_validate":       summary(validate),
			"helper_total":          summary(inHelper),
		},
		"warm_word_count": summary(wordWarm),
		"ping_round_trip": summary(pings),
		"footprint": map[string]any{
			"idle":                   idle,
			"after_first_search":     afterFirst,
			"after_warm":             afterWarm,
			"after_more_create_drop": afterCycles,
			"create_drop_calls":      warmupCalls + *samples + 1 + footprintCalls,
		},
		"containment": containment,
		"gates": map[string]any{
			"warm_p95":    map[string]any{"limit_ms": 5.0, "measured_ms": p(warm, "p95_ms")},
			"warm_p99":    map[string]any{"limit_ms": 10.0, "measured_ms": p(warm, "p99_ms")},
			"cold_p95":    map[string]any{"limit_ms": 20.0, "measured_ms": p(colds, "p95_ms")},
			"ping_p95":    map[string]any{"limit_ms": 1.0, "measured_ms": p(pings, "p95_ms")},
			"containment": map[string]any{"failed_cases": failed},
		},
	}
}

func contain(names []string) []map[string]any {
	words := read("word-count.js")
	var results []map[string]any
	for _, name := range names {
		h, err := start()
		if err != nil {
			panic(err)
		}
		idle := footprint(h)["now_bytes"]
		began := time.Now()
		r, err := h.call(map[string]any{
			"op": "run", "source": read("containment/" + name + ".js"), "query": "query",
			"deadline_ms": budget.Milliseconds(),
		}, killAfter)
		wall := time.Since(began)
		result := map[string]any{"case": name, "process_wall_ms": millis(wall)}
		killed := err == errKilled
		var peak uint64
		switch {
		case killed:
			result["outcome"] = "killed"
			result["detail"] = fmt.Sprintf("no answer within %s", killAfter)
		case err != nil:
			result["outcome"] = "helper-died"
			result["detail"] = err.Error()
		default:
			usage := footprint(h)
			peak = usage["lifetime_max_bytes"] - idle
			result["peak_over_idle_bytes"] = peak
			result["heap_peak_bytes"] = r.HeapPeak
			result["elapsed_ms"] = r.Stages.Total
			if r.Failure != nil {
				result["outcome"] = r.Failure.Kind
				result["detail"] = r.Failure.Detail
			} else {
				result["outcome"] = "returned"
				if len(r.Rows) > 0 {
					result["detail"] = r.Rows[0].Title
				}
			}
			// The helper must still serve the next call without a restart.
			next, err := h.search(words, 0, wordQuery)
			result["helper_survived"] = err == nil && next.Failure == nil && len(next.Rows) == 1
		}
		h.stop()
		elapsed, _ := result["elapsed_ms"].(float64)
		inTime := !killed && err == nil && elapsed <= millis(budget+tolerance)
		heapHeld := err == nil && peak <= heapLimit+heapSlack
		result["stopped_within_budget"] = inTime
		result["heap_held"] = heapHeld
		result["terminated_within_outer_limit"] = !killed && wall <= outerLimit
		result["pass"] = inTime && heapHeld && result["helper_survived"] == true
		results = append(results, result)
		fmt.Fprintf(os.Stderr, "  %-22s %v\n", name, result["outcome"])
	}
	return results
}

// features runs probes of what Dupin's guests would use: Intl and Temporal.
func features() map[string]any {
	h, err := start()
	if err != nil {
		panic(err)
	}
	defer h.stop()
	r, err := h.call(map[string]any{"op": "run", "source": featuresProbe, "query": "", "deadline_ms": 2000}, killAfter)
	if err != nil {
		panic(err)
	}
	return map[string]any{"hello": h.hello, "reply": r}
}

//go:embed features.js
var featuresProbe string

var procPidRusage func(pid int32, flavor int32, buffer unsafe.Pointer) int32

func init() {
	library, err := purego.Dlopen("/usr/lib/libSystem.B.dylib", purego.RTLD_NOW|purego.RTLD_GLOBAL)
	if err != nil {
		panic(err)
	}
	purego.RegisterLibFunc(&procPidRusage, library, "proc_pid_rusage")
}

// footprint reads the helper's physical footprint from rusage_info_v4.
func footprint(h *helper) map[string]uint64 {
	var info [64]uint64
	if status := procPidRusage(int32(h.cmd.Process.Pid), 4, unsafe.Pointer(&info[0])); status != 0 {
		panic("proc_pid_rusage failed")
	}
	// After the 16-byte UUID: ri_phys_footprint is the 8th field and
	// ri_lifetime_max_phys_footprint the 29th.
	return map[string]uint64{"now_bytes": info[2+7], "lifetime_max_bytes": info[2+28]}
}
