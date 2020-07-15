SHELL := bash
.ONESHELL:
.SHELLFLAGS := -eu -o pipefail -c
.DELETE_ON_ERROR:
MAKEFLAGS += --warn-undefined-variables
MAKEFLAGS += --no-builtin-rules

ifeq ($(origin .RECIPEPREFIX), undefined)
  $(error This Make does not support .RECIPEPREFIX. Please use GNU Make 4.0 or later)
endif
.RECIPEPREFIX = >

# Default - top level rule is what gets ran when you run just `make`
README.md: README.src.md
> remark --use graphviz README.src.md > README.md
> git add *.svg README.md README.src.md

# Clean up the output directories; since all the sentinel files go under tmp, this will cause everything to get rebuilt
clean:
> git rm *.svg > /dev/null 2>&1 || true
> rm *.svg > /dev/null 2>&1 || true
> [ -f README.md ] && rm README.md
.PHONY: clean
