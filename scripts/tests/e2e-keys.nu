use std/assert
use ../e2e/keys.nu key-script

def main [] {
    assert equal (key-script cmd+shift+s) ([
        'tell application "System Events"'
        '    key code 1 using {command down, shift down}'
        'end tell'
    ] | str join (char nl))
    assert equal (key-script down return 'cmd+,' alt+space cmd+backspace) ([
        'tell application "System Events"'
        '    key code 125'
        '    key code 36'
        '    key code 43 using {command down}'
        '    key code 49 using {option down}'
        '    key code 51 using {command down}'
        'end tell'
    ] | str join (char nl))
    assert equal (key-script delete) ([
        'tell application "System Events"'
        '    key code 117'
        'end tell'
    ] | str join (char nl))

    for refused in [
        del
        S
        cmd+O
        hyper+s
        'cmd+'
        enter
    ] {
        assert error {|| key-script $refused }
    }
    assert error {|| key-script }
}
