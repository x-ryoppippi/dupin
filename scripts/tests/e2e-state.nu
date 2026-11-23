use std/assert
use ../e2e-state.nu is-focused-query

def main [] {
    let focused = {
        ok: true
        data: {
            match: {
                value: calculator
                states: [focused]
            }
        }
    }
    assert (is-focused-query $focused calculator)
    assert not (is-focused-query $focused other)
    assert not (is-focused-query {ok: false, data: $focused.data} calculator)
    assert not (
        is-focused-query {ok: true, data: {match: {value: calculator, states: []}}} calculator
    )
    assert not (is-focused-query {ok: true, data: {match: null}} calculator)
    assert not (is-focused-query {ok: true} calculator)
    print 'E2E query state tests passed'
}
