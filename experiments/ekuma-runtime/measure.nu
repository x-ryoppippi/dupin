#!/usr/bin/env nu

# Builds the Ekuma helper variants, records their sizes, and runs the bench through the helper.

def --wrapped checked [...args: string]: nothing -> string {
    let result = run-external ...$args | complete
    if $result.exit_code != 0 {
        error make {msg: $"($args.0) failed:\n($result.stderr)"}
    }
    $result.stdout | str trim
}

def bytes [file: path]: nothing -> int {
    ls --all $file | get 0.size | into int
}

def --wrapped go-build [here: path, env_vars: record, ...args: string]: nothing -> nothing {
    with-env $env_vars { checked go -C $here build -trimpath ...$args } | ignore
}

const VARIANTS = [
    {label: base, tags: ''}
    {label: tzdata, tags: 'timetzdata'}
    {label: intl, tags: 'intl,timetzdata'}
]

def helper-sizes [here: path, work: path]: nothing -> list<record> {
    $VARIANTS | each {|variant|
        let out = $work | path join $"helper-($variant.label)"
        print --stderr $"Building helper ($variant.label)…"
        go-build $here {CGO_ENABLED: '0'} -tags $variant.tags -ldflags '-s -w' -o $out ./helper
        let signed = $work | path join $"signed-($variant.label)"
        cp $out $signed
        checked /usr/bin/codesign --force --options runtime --sign - $signed | ignore
        let archive = $work | path join $"helper-($variant.label).zip"
        checked /usr/bin/ditto -c -k $signed $archive | ignore
        {
            label: $variant.label
            tags: $variant.tags
            stripped_bytes: (bytes $out)
            signed_bytes: (bytes $signed)
            zip_bytes: (bytes $archive)
        }
    }
}

# The cost of linking Ekuma into a C stand-in for `dupin` instead of shipping a second binary.
def carchive-sizes [here: path, work: path]: nothing -> list<record> {
    let host = $here | path join carchive host host.c
    let bare = $work | path join host-bare
    checked cc -Os -o $bare $host | ignore
    checked strip -x $bare | ignore
    [
        {label: base, tags: ''}
        {label: intl, tags: 'intl,timetzdata'}
    ] | each {|variant|
        print --stderr $"Building c-archive ($variant.label)…"
        let dir = $work | path join $"carchive-($variant.label)"
        mkdir $dir
        let archive = $dir | path join libekuma.a
        go-build $here {CGO_ENABLED: '1'} -tags $variant.tags -buildmode=c-archive -o $archive ./carchive
        let linked = $dir | path join host
        (checked cc -Os -DWITH_EKUMA -I $dir -o $linked $host $archive
            -framework CoreFoundation -framework Security -lresolv -Wl,-dead_strip)
        | ignore
        checked strip -x $linked | ignore
        let output = checked $linked
        {
            label: $variant.label
            archive_bytes: (bytes $archive)
            bare_host_bytes: (bytes $bare)
            linked_host_bytes: (bytes $linked)
            increase_bytes: ((bytes $linked) - (bytes $bare))
            output: $output
        }
    }
}

def reproducible [here: path, work: path]: nothing -> record {
    let first = $work | path join repro-1
    let second = $work | path join repro-2
    (go-build
        $here
        {CGO_ENABLED: '0'}
        -a
        -tags
        'intl,timetzdata'
        -ldflags
        '-s -w'
        -o
        $first
        ./helper
    )
    go-build $here {
        CGO_ENABLED: '0'
        GOCACHE: ($work | path join gocache)
    } -a -tags 'intl,timetzdata' -ldflags '-s -w' -o $second ./helper
    let a = open --raw $first | hash sha256
    let b = open --raw $second | hash sha256
    {
        sha256_first: $a
        sha256_second: $b
        identical: ($a == $b)
    }
}

def cross [here: path, work: path]: nothing -> list<record> {
    [[goos, goarch]; [darwin, amd64] [linux, arm64] [linux, amd64]] | each {|target|
        let out = $work | path join $"helper-($target.goos)-($target.goarch)"
        go-build $here {CGO_ENABLED: '0', GOOS: $target.goos, GOARCH: $target.goarch} -tags 'intl,timetzdata' -ldflags '-s -w' -o $out ./helper
        {goos: $target.goos, goarch: $target.goarch, stripped_bytes: (bytes $out)}
    }
}

def main [
    --samples: int = 2000 # Timed warm calls per workload
    --cold-runs: int = 100 # Fresh helpers per cold measurement
    --skip-size # Run only the bench
    --skip-bench # Measure only sizes
] {
    let here = $env.FILE_PWD
    let root = $here | path join .. .. | path expand
    let work = mktemp --directory --tmpdir dupin-ekuma.XXXXXX
    print --stderr $"Working in ($work)"
    let bench = $work | path join bench
    go-build $here {CGO_ENABLED: '0'} -o $bench ./bench
    let helper = $work | path join helper-intl
    if $skip_size {
        (go-build
            $here
            {CGO_ENABLED: '0'}
            -tags
            'intl,timetzdata'
            -ldflags
            '-s -w'
            -o
            $helper
            ./helper
        )
    }

    let sizes = if $skip_size { null } else {
        {
            helper: (helper-sizes $here $work)
            carchive: (carchive-sizes $here $work)
            reproducible: (reproducible $here $work)
            cross: (cross $here $work)
        }
    }
    let fixtures = $root | path join experiments js-runtime bench fixtures
    let run = {|label: string, gogc: string|
        print --stderr $"Bench ($label)…"
        (checked $bench -helper $helper -fixtures $fixtures -label $label -samples ($samples | into string)
            -cold-runs ($cold_runs | into string) -gogc $gogc all)
        | from json
    }
    let bench_results = if $skip_bench { null } else {
        {
            default_gc: (do $run 'GOGC=100' '')
            gogc_400: (do $run 'GOGC=400' '400')
            features: (
                (checked
                    $bench
                    -helper
                    $helper
                    -fixtures
                    $fixtures
                    features
                )
                | from json
            )
        }
    }
    let results = {
        ekuma: 'v0.1.0'
        go: (checked go version)
        host: {
            machine: (checked sysctl -n machdep.cpu.brand_string)
            macos: (checked sw_vers -productVersion)
            load_average: (checked sysctl -n vm.loadavg)
        }
        revision: (checked git -C $root rev-parse HEAD)
        sizes: $sizes
        bench: $bench_results
    }
    let out = $work | path join results.json
    $results | to json | save $out
    print $out
}
