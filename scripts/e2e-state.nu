export def is-focused-query [reply: record, expected: string]: nothing -> bool {
    let match = $reply.data?.match?
    if $reply.ok? != true or $match == null { return false }
    if 'value' not-in ($match | columns) { return false }
    let states = $match.states? | default []
    'focused' in $states and ($match.value == $expected or ($expected == '' and $match.value == null))
}

export def has-match [reply: record]: nothing -> bool {
    $reply.ok and $reply.data?.match? != null
}

export def is-focused-match [reply: record]: nothing -> bool {
    $reply.ok and 'focused' in ($reply.data?.match?.states? | default [])
}
