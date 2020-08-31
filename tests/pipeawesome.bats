#!/usr/bin/env bats

# EXPECTED=$(cat ./tests/pipeawesome-terminator/expected.txt)

@test "pipeawesome with loops" {

    INPUT=$( cat ./tests/pipeawesome/pad_to_5.input.txt )
    EXPECTED=$( cat ./tests/pipeawesome/pad_to_5.expected.txt )
    RESULT=$( echo "$INPUT" | ./target/debug/pipeawesome  -p ./tests/pipeawesome/pad_to_5.json -t FAUCET=- -s OUTPUT=- )

    # shellcheck disable=SC2016,SC2002
    # RESULT=$(
    #     cat <(cat tests/pipeawesome-terminator/seen_all_closed_via_grace_period) \
    #         <(sleep 3 ; echo "SHOULD NOT BE SEEN") | \
    #     ./target/debug/pipeawesome-terminator -g 1 -r 'REQUIRE: ITEM: ' -l '^0*([^:]+).*' -p '$1' | \
    #     sort
    # )

    # echo "$RESULT"
    # [ "$RESULT" = "$EXPECTED" ]

    echo "$RESULT"
    echo "$EXPECTED"
    [ "$RESULT" = "$EXPECTED" ]
}


