
## To start a project

    pipeawesome configuration_file

Will start pipeawesome. If either of `control_fifo` or `orphan_fifo` are not specified they will be created and reported.

Lines which are piped into `pipeawesome` will enter the `INPUT` stream, be processed by multiple `PROCESSORS` and then lines get to `OUTPUT` stream they will be piped out of `pipeawesome`.

```
INPUT -> FIFO -> PROCESSOR -> FIFO -> PROCESSOR -> FIFO -> OUTPUT
```

## Example

jo control=ADD stream=STDIN source=INPUT destination=PRE command=awk args="$(jo -a '{ printf "INPUT: "$1": 0" }')" > config.pa
jo control=ADD stream=STDIN source=PRE destination=MATHS command=awk args="$(jo -a 'BEGIN { FS=":" }{ printf "$1:"$2": "; print $2 | "bc" }')" > config.pa
jo control=ADD stream=STDIN source=MATHS destination=QUALITY_CONTROL command=awk args="$(jo -a 'BEGIN { FS=":" }{ if ($3 < 50) print "COLD:"$2":"$3; else if ($3 == 50) print "RIGHT:"$2": 50"; else print "HOT:"$2":"$3 }' )" > config.pa
jo control=ADD stream=STDIN source=QUALITY_CONTROL destination=FAIL_COLD command=grep args=$(jo -a '^COLD') > config.pa
jo control=ADD stream=STDIN source=QUALITY_CONTROL destination=FAIL_HOT command=grep args=$(jo -a '^HOT') > config.pa
jo control=ADD stream=STDIN source=QUALITY_CONTROL destination=JUST_RIGHT command=grep args=$(jo -a '^RIGHT') > config.pa
jo control=ADD stream=STDIN source=FAIL_HOT destination=MATHS command=awk args="$(jo -a 'BEGIN { FS=":" }{ print $1": "$2" - 1: 0" }')" > config.pa
jo control=ADD stream=STDIN source=FAIL_COLD destination=MATHS command=awk args="$(jo -a 'BEGIN { FS=":" }{ print $1": "$2" + 5: 0" }')" > config.pa
jo control=OUT stream=STDIN source=JUST_RIGHT pre='OUT: '

```
$ echo '3 + 4' | pipeawesome pipeawesome.conf

OUTPUT: STDOUT: JUST_RIGHT: 3 + 4 + 15 + 15 + 15 - 1 - 1: 50
```

BufReader / BufWriter both support `with_capacity`


    Control:
        Constructor(IN, CMD)
        AddOut(c: Control)
        BUFFER:
        POSITIONS: Int[]
        
    // ==================

    In
        reader: BufReader
        rx: [GET_LINES(Int)]
        tx: [LINE(String|null)]

    OUT
        writer: BufWriter
        rx: [GET_LINES(Int)]
        tx: [LINE(String|null)]


    Piper
        WATERMARK: Number
        MAX: Number
        BUFFER: Array[]
        IN: Thread<In>
        OUT: Map< Processing, Thread<Out> >
        DONE: boolean = false;
        REQUESTED = 0

        CONSTRUCT(InProc, OutProc[]):

        GET_DATA:
            Getting = true
            while Getting
                if IN.try_recv:
                    REQUESTED--
                    if NULL:
                        DONE = true
                    if BUFFER.length == MAX
                        Getting = FALSE
                else
                    Getting = false
                if !BUFFER.COUNT
                    sleep
                    Getting = true

        REQUEST_DATA:
            If !REQUESTED && BUFFER.COUNT < WATERMARK:
                ToRequest = ( MAX -  BUFFER.COUNT)
                IN.send GET_LINES ToRequest
                REQUESTED+=ToRequest


        SEND_DATA:
            IF !BUFFER.length:
                RETURN;
            For K,V of OUT:
                IF V.Processing:
                    RETURN
            line = BUFFER.shift()
            For K,V of OUT:
                V.thread.send line
                V.Processing = true

        SEND_DATA_RESP:
            For K,V of OUT:
                V.thread.try_recv:
                    V.processing = false;


        BEGIN:
            WHILE !DONE
                REQUEST_DATA()
                GET_DATA()
                SEND_DATA()
                SEND_DATA_RESP()

                    
