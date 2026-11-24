# Hardware key codes, as Dupin's Carbon hotkeys match them: `keystroke` would map a character
# through the runner's keyboard layout. Names follow rune's `Key`, so `backspace` is the key
# macOS labels delete and `delete` deletes forwards.
const KEY_CODES = {
    a: 0
    s: 1
    d: 2
    f: 3
    h: 4
    g: 5
    z: 6
    x: 7
    c: 8
    v: 9
    b: 11
    q: 12
    w: 13
    e: 14
    r: 15
    y: 16
    t: 17
    '1': 18
    '2': 19
    '3': 20
    '4': 21
    '6': 22
    '5': 23
    '=': 24
    '9': 25
    '7': 26
    '-': 27
    '8': 28
    '0': 29
    ']': 30
    o: 31
    u: 32
    '[': 33
    i: 34
    p: 35
    l: 37
    j: 38
    "'": 39
    k: 40
    ';': 41
    '\': 42
    ',': 43
    '/': 44
    n: 45
    m: 46
    '.': 47
    '`': 50
    return: 36
    tab: 48
    space: 49
    backspace: 51
    escape: 53
    home: 115
    pageup: 116
    delete: 117
    end: 119
    pagedown: 121
    left: 123
    right: 124
    down: 125
    up: 126
}

const MODIFIERS = {
    cmd: 'command down'
    shift: 'shift down'
    alt: 'option down'
    ctrl: 'control down'
}

# The System Events script that presses each chord (`cmd+shift+s`, `alt+space`, `down`) in turn.
export def key-script [...chords: string]: nothing -> string {
    let strokes = $chords | each {|chord|
        let parts = $chord | split row '+'
        let key = $parts | last
        let code = $KEY_CODES | get --optional $key
        if $code == null {
            error make {msg: $'Unknown key `($key)` in `($chord)`'}
        }
        let modifiers = $parts | drop 1 | each {|modifier|
            let flag = $MODIFIERS | get --optional $modifier
            if $flag == null {
                error make {msg: $'Unknown modifier `($modifier)` in `($chord)`'}
            }
            $flag
        }
        if ($modifiers | is-empty) {
            $'    key code ($code)'
        } else {
            $'    key code ($code) using {($modifiers | str join ", ")}'
        }
    }
    if ($strokes | is-empty) {
        error make {msg: 'No key to press'}
    }
    ['tell application "System Events"' ...$strokes 'end tell'] | str join (char nl)
}
