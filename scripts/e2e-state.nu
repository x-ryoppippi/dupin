export def is-focused-query [reply: record, expected: string]: nothing -> bool {
    let value = $reply.data?.match?.value?
    let states = $reply.data?.match?.states? | default []
    $reply.ok and $value == $expected and 'focused' in $states
}
