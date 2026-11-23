export def is-focused-query [reply: record, expected: string]: nothing -> bool {
    let match = $reply.data?.match?
    if $reply.ok? != true or $match == null { return false }
    if 'value' not-in ($match | columns) { return false }
    let states = $match.states? | default []
    'focused' in $states and ($match.value == $expected or ($expected == '' and $match.value == null))
}
