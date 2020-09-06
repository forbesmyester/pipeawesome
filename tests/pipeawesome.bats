#!/usr/bin/env bats

@test "pipeawesome simple" {

    EXPECTED=$( cat ./tests/pipeawesome/soup_temperature.expected.txt )
    RESULT=$( ./target/debug/pipeawesome -p ./tests/pipeawesome/soup_temperature.paspec.json -o OUTPUT=- )

    echo "RESULT = $RESULT"
    echo "EXPECTED = $EXPECTED"
    [ "$RESULT" = "$EXPECTED" ]
}


@test "pipeawesome with loops" {

    INPUT=$( cat ./tests/pipeawesome/pad_to_5.input.txt )
    EXPECTED=$( cat ./tests/pipeawesome/pad_to_5.expected.txt )
    RESULT=$( echo "$INPUT" | ./target/debug/pipeawesome  -p ./tests/pipeawesome/pad_to_5.paspec.json -i FAUCET=- -o OUTPUT=- )

    echo "RESULT = $RESULT"
    echo "EXPECTED = $EXPECTED"
    [ "$RESULT" = "$EXPECTED" ]
}


