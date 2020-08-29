#!/usr/bin/env bats
EXPECTED=$(cat ./pipeawesome-terminator-test-expected.txt)

@test "seen all closed via grace period" {

    # shellcheck disable=SC2016,SC2002
    RESULT=$(
        cat <(cat tests/pipeawesome-terminator/seen_all_closed_via_grace_period) \
            <(sleep 3 ; echo "SHOULD NOT BE SEEN") | \
        ./target/debug/pipeawesome-terminator -g 1 -r 'REQUIRE: ITEM: ' -l '^0*([^:]+).*' -p '$1' | \
        sort
    )

    echo "$RESULT"
    [ "$RESULT" = "$EXPECTED" ]

}

@test "seen all closed via end" {

    # shellcheck disable=SC2016,SC2002
    RESULT=$(
        cat tests/pipeawesome-terminator/seen_all_closed_via_end | while read -r LINE; do echo "$LINE"; sleep 0.1; done | \
        ./target/debug/pipeawesome-terminator -e 'REQUIRE: END' -r 'REQUIRE: ITEM: ' -l '^0*([^:]+).*' -p '$1' -g 1 |
        sort
    )

    echo "$RESULT"
    [ "$RESULT" = "$EXPECTED" ]

}
