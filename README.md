# Pipeawesome

## As my mum would say... accusingly... "WHAT did YOU do?!?!?!"

I added loops, branches and joins to UNIX pipes.

## Why?

### My plan is

I have recently created two projects:

-   [eSQLate](https://github.com/forbesmyester/esqlate) - A relatively successful web front end for parameterized SQL queries.
-   [WV Linewise](https://github.com/forbesmyester/wv-linewise) - A pretty much ignored way to put a [web-view](https://github.com/Boscop/web-view) in the middle of a UNIX pipeline.

My plan is to join these two pieces of software together so eSQLate no longer has to run as a web service, but more like ordinary locally installed software.

### A more graphical / exciting explanation

A more exciting and / or exciting way to describe my idea is to thinking of a simple turn based strategy game like the below where the lines are actually UNIX pipes.

![](./af09533c5536a1ef6ba533be5202b9c1818c1b37.svg "`dot` image")

Of course, on a single machine this would practically be a turn based game as the display would be in different windows. But it would be trivial to run add SSH as an option for as part of this to make this into a real time game:

![](./5a41ae9cf241d17b63cf68da75f29a8d3981aac0.svg "`dot` image")

### The broader plan

Imagine we are writing server software. It seems we could use this to read from a distributed queue, do a series of steps and put them back onto some other queue... we'd in effect have microservices! To me this idea seems to have real benefits.

## More Detail

### UNIX pipes are wonderful.

When we can use UNIX pipes it often means we can write much less code and, if we're honest, the end result is much faster in both development time and execution speed.

Given an example command `aws sqs recieve-message-or-similar ... | jq ... | CMD1 ... | CMD2 | jq ... | aws sqs send-message-or-similar ...`we can immediately tell what it does. There's a kind of purity and ease of understanding that is wonderful...

### But they only go so far...

The above example raises the following questions:

-   What happens when we receive an invalid message?
-   If I have some restructuring to do, for example send some (or invalid) requests to a different queue for analysis, it is unclear how this should be achieved.

It seems to continue to develop this pipeline I will probably need to re-write it in a programming language because the following are difficult on the command line:

-   Branching
-   Joining
-   Loops

I find this a little sad because we're throwing away:

-   A really high performance method of pushing data which gives us back-pressure and buffering for free.
-   We're likely going to write one big binary and it's then much less obvious what it does and how it works. It will also likely have far more code and cost more produce.

### Are UNIX pipelines microservices?

There has been a big push towards microservices and these are often wired together using Queues. This got me thinking:

1.  Are UNIX pipes actually Queues?
2.  Can we view individual programs as microservices?

For me, while there are caveats, the answers to these questions is YES. I also believe that it would be cheaper, more reliable and faster to build (some) software in this way.

### That's great... I think... but how would we structure the command?

The first thing I started looking at was how would I want to structure the command. I came to the following conclusions:

-   It is pretty difficult to think of a way to express branching on a single line, let alone joins and loops.
-   Even if we could come up with some syntax, it would also have to be read by humans. More than one `if`/`loop` on a single line and it becomes really difficult.

It seems that when doing diagrams to describe what I want to achieve I usually use [Graphviz DOT](https://www.graphviz.org/doc/info/lang.html) and I even thought about using that as a file format for a while:

![](./02bc56e1a281dedb78be2c012ffd56b9791cb334.svg "`dot` image")

Thinking about Graphviz lead me to the revelation that we do actually want to use a directed graph to a key building block.  In the end I designed a JSON (groan) file format as it is somewhat easy to parse for both humans and computers.

### So what's in the configuration file and how do I run it?

For simple, and even at it's most complicated, the configuration looks like the following:

```json
{
  "commands": [
    {
      "name": "CAT",
      "src": [],
      "spec": {
        "command": "cat",
        "args": [ "tests/pipeawesome/soup_temperature.input.txt" ]
      }
    },
    {
      "name": "MATHS",
      "src": [ { "name": "CAT", "port": "OUT" } ],
      "spec": {
        "command": "gawk",
        "args": [ "{ cmd = \"echo \"$0\" | bc\" ; cmd | getline res ; close(cmd); print INPUT\": \"$0\": \"res; fflush() }"]
      }
    },
    {
      "name": "QUALITY_CONTROL",
      "src": [ { "name": "MATHS", "port": "OUT" } ],
      "spec": {
        "command": "awk",
        "args": [ "BEGIN { FS=\":\" }{ if ($3 < 88) print \"TOO_COLD:\"$2\":\"$3; else if ($3 > 93) print \"TOO_HOT:\"$2\":\"$3; else print \"JUST_RIGHT:\"$2\":\"$3; fflush() }" ]
      }
    }
  ],
  "outputs": { "OUTPUT": [ { "name": "QUALITY_CONTROL", "port": "OUT" } ] }
}
```

In this file format:

-   Outputs are listed in the `outputs` property of the JSON file.
-   Inputs are simply found by finding the "src"'s of the commands which are themselves not commands. If for example "CAT" did not exist in the example above it would become an input and would need to be specified.

To execute the file above you would use the following command:

```sh
pipeawesome --pipeline tests/pipeawesome/soup_temperature.paspec.json -output OUTPUT=- 
```

As this file format forms a directed graph. Therefore:

-   If you want to do a branch, you just list multiple commands coming from the same "src".
-   If you want to do a join you have one command with multiple "src".
-   Loops are achieved by a branch and a join back onto itself.

The only other thing to note is that commands have three outputs "OUT" "ERR" and "EXIT". Which are STDOUT, STDERR and the exit status of a command.

### Special note about loops

Pipeawesome will exit when all outputs (or commands which have no outbound connections) have been closed (or exited).

When a command (or input) finishes/exits (closes) it will pass this fact onto the downstream child commands (and outputs), but if a command has two or more inputs, it will only be notified when all of it's inputs are closed.

This means that a loop which has no commands that exit themselves will not finish because the bit that feeds back onto itself will never close.

To handle this situation, which you may not even want to do if writing server-like software, you'll need to add something past the join point which closes itself based on messages.

### 

If I wanted to add branching, joining and loops to this command, but all the solutions I know of add either a lot of ugliness or split the command over multiple lines. In all situations the complexity level jumps more than I feel it should.
