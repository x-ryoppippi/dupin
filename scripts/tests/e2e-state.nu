use std/assert
use ../e2e-state.nu [has-match is-focused-match is-focused-query]

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
    let unfocused = {
        ok: true
        data: {
            match: {
                states: []
            }
        }
    }
    let missing = {
        ok: true
        data: {match: null}
    }
    let failed = {ok: false, data: $focused.data}
    assert (has-match $focused)
    assert (has-match $unfocused)
    assert not (has-match $missing)
    assert not (has-match $failed)
    assert (is-focused-match $focused)
    assert (is-focused-match {ok: true, data: {match: {states: [focused]}}})
    assert not (is-focused-match $unfocused)
    assert not (is-focused-match $missing)
    assert not (is-focused-match $failed)
    assert (is-focused-query $focused calculator)
    assert not (is-focused-query $focused other)
    assert not (is-focused-query {ok: false, data: $focused.data} calculator)
    assert not (
        is-focused-query {ok: true, data: {match: {value: calculator, states: []}}} calculator
    )
    assert not (is-focused-query {ok: true, data: {match: null}} calculator)
    assert not (is-focused-query {ok: true} calculator)
    for value in [null ''] {
        assert (
            is-focused-query {ok: true, data: {match: {value: $value, states: [focused]}}} ''
        )
    }
    assert not (is-focused-query {ok: true, data: {match: {states: [focused]}}} '')
    assert not (
        is-focused-query {ok: true, data: {match: {value: null, states: [focused]}}} calculator
    )
    assert not (is-focused-query {ok: true, data: {match: {value: null, states: []}}} '')
    assert not (
        is-focused-query {ok: false, data: {match: {value: null, states: [focused]}}} ''
    )
    assert not (is-focused-query {data: {match: {value: null, states: [focused]}}} '')
    assert not (is-focused-query {ok: true, data: {match: null}} '')
    assert not (is-focused-query {ok: true} '')
    print 'E2E query state tests passed'
}
