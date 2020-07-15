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

## Specifications

### Tap

```dot
digraph G {

    subgraph cluster_tap {
        label = "tap";
        color=lightgrey;

        subgraph cluster_tap_thread {
            label="thread"
            tap_handle [shape=Mdiamond, label=handle]
            tap_int_tx [label=int_tx]
        }
        tap_int_rx [label=int_rx]
        tap_tx [label=tx]
        tap_pending [label=pending]

        tap_handle -> tap_int_tx
        tap_int_tx -> tap_int_rx
        tap_int_rx -> tap_pending [style=dashed, label=2, arrowhead=box]
        tap_tx -> tap_pending [style=dashed, label=1, arrowhead=box]
        tap_pending -> tap_tx

    }

    tap_rx [shape=point]
    tap_tx -> tap_rx

}
```

#### 1

`tx` -> Err - OutputFull
`tx` -> Ok - If sent None - ExhaustedInput

#### 2 - Loop

`int_rx` Ok(line) - Puts data in `pending` and performs `1`
`int_rx` Err(_) - Waiting

### Sink

```dot
digraph G {

    subgraph cluster_sink {
        label = "sink";
        color=lightgrey;

        subgraph cluster_sink_thread {
            label="thread"
            sink_handle [shape=Mdiamond, label=handle]
            sink_int_rx [label=int_tx]
        }
        sink_int_tx [label=int_tx]
        sink_pending [label=pending]

        sink_pending -> sink_int_tx [style=dashed, arrowhead=box, label=1]
        sink_int_tx -> sink_int_rx -> sink_handle
        sink_rx -> sink_pending [style=dashed, arrowhead=box, label=2]

    }

    sink_tx [shape=point]
    sink_tx -> sink_rx

}

```

#### 1 - First step is to process pending

`int_tx` try_send -> Err(_): OutputFull
`int_tx` try_send -> Ok(sent): If sent None ExhaustedInput

#### 2 - If something has just moved out of pending

`rx` Ok(line) - Puts data in `pending` and performs `1`
`rx` Err(_) - Waiting

### Buffer

```dot
digraph G {

    subgraph cluster_buffer {
        label = "buffer";
        color=lightgrey;

        buffer_input_1 [label=input]
        buffer_input_2 [label=input]
        buffer_pending [label="pending(pos, line)"]
        buffer_output_1 [label=output]
        buffer_output_2 [label=output]

        buffer_input_1 -> buffer_pending
        buffer_input_2 -> buffer_pending
        buffer_pending -> buffer_output_1
        buffer_output_1 -> buffer_pending [style=dashed, arrowhead=box, label=1]
        buffer_pending -> buffer_output_2

    }

    buffer_input_1_in [shape=point]
    buffer_input_2_in [shape=point]
    buffer_output_1_out [shape=point]
    buffer_output_2_out [shape=point]

    buffer_input_1_in -> buffer_input_1 [style="dashed" label=2, arrowhead=box]
    buffer_input_2_in -> buffer_input_2 [style="dashed" label=2, arrowhead=box]
    buffer_output_1 -> buffer_output_1_out
    buffer_output_2 -> buffer_output_2_out

}


```

#### 1 - Sending data out, if possible

After send attempt, if something left in `partially_sent` then: OutputFull
Then if no inputs left then ExhaustedInput.

#### 2 - Pulling data in from externally

`input` try_recv -> Err(Empty) - Waiting
`input` try_recv -> Err(Disconnected) - panic!

### Command

```dot
digraph G {

    subgraph cluster_command {
        label = "command";
        color=lightgrey;

        subgraph cluster_command_stdin_thread {
            label="thread stdin"
            command_stdin [shape=Mdiamond, label=stdin]
            command_inner_stdin_rx [label=inner_stdin_rx]
            command_inner_stdin_rx -> command_stdin
        }

        command_inner_stdin_tx [label=inner_stdin_tx]
        command_pending_stdin [label=pending_stdin]
        command_stdin_rx [label=stdin_rx]

        command_pending_stdin -> command_inner_stdin_tx
        command_inner_stdin_tx -> command_pending_stdin [style="dashed", label=1, arrowhead=box]
        command_inner_stdin_tx -> command_inner_stdin_rx
        command_stdin_rx -> command_pending_stdin [style="dashed", label=2, arrowhead=box]

        subgraph cluster_command_stdout_thread {
            label="thread stdout"

            command_stdout [shape=Mdiamond, label=stdout]
            command_inner_stdout_tx [label=inner_stdout_tx]

            command_stdout -> command_inner_stdout_tx
        }

        command_stdout_tx [label=stdout_tx]
        command_pending_stdout [label=pending_stdout]
        command_inner_stdout_rx [label=inner_stdout_rx]

        command_pending_stdout -> command_stdout_tx
        command_stdout_tx -> command_pending_stdout [style="dashed", label=1, arrowhead=box]
        command_inner_stdout_rx -> command_pending_stdout [style="dashed", label=2, arrowhead=box]
        command_inner_stdout_tx -> command_inner_stdout_rx

        subgraph cluster_command_stderr_thread {
            label="thread stderr"

            command_stderr [shape=Mdiamond, label=stderr]
            command_inner_stderr_tx [label=inner_stderr_tx]

            command_stderr -> command_inner_stderr_tx
        }

        command_stderr_tx [label=stderr_tx]
        command_pending_stderr [label=pending_stderr]
        command_inner_stderr_rx [label=inner_stderr_rx]

        command_pending_stderr -> command_stderr_tx
        command_stderr_tx -> command_pending_stderr [style="dashed", label=1, arrowhead=box]
        command_inner_stderr_rx -> command_pending_stderr [style="dashed", label=2, arrowhead=box]
        command_inner_stderr_tx -> command_inner_stderr_rx

    }

    command_stdout_rx [shape=point]
    command_stderr_tx -> command_stderr_rx
    command_stderr_rx [shape=point]
    command_stdout_tx -> command_stdout_rx
    command_stdin_tx [shape=point]
    command_stdin_tx -> command_stdin_rx

}
```

#### 1 - Moving items to pending

If `stdin_rx.try_recv`, `inner_stdout_rx.try_recv()` and `inner_stderr_rx.try_recv()` all failed we will exit with Waiting, but only if nothing happens in 2

#### 2 - Processing Pending

If we could not write to `inner_stdin_tx` then we exit with `InternallyFull` unless we cannot write to `stdout_tx` or `stderr_tx` where we would exit with `OutputFull`

