# Ilk

Ilk is a plain-text logical markup language.

## Language

An Ilk document is written in a similar fashion to other lightweight markup languages, but instead of targeting a structured output format, it produces a document of extracted text, alongside a database of facts.

By default, an Ilk document is just plain text, which extracts to an identical document with no facts:

```ilk
Hello World!
```

We can augment this document by asserting facts at specific points in the text:

```ilk
@{start}Hello World!
```

Assertions are inherently tied to their position in text:

```pl
region_start($a, 0). % the dollar sign ($) indicates a generated identifier
assertion(start, $a).
region_end($a, 0).
```

Facts are written in a Prolog-like style, as atoms, or compounds of a functor atom with an arbitrary number of argument terms. Multiple facts can be asserted by separating them with a semicolon (`;`). In addition to atoms (including quoted atoms) and compounds, integers and real numbers are available as terms:

Unquoted atom names have two forms: word names and symbol names. Word names (also used for region labels) are sequences of ASCII letters and numbers separated by underscores, and may optionally start/end with one of `$`, `?`, or `#`. Symbol names are sequences of `&*+-./:<=>@\\^~`.

```ilk
@{has(alice, item(lantern, 1)); name(alice, 'Alice')}Alice picked up the lantern.
```

```pl
region_start($a, 0).
assertion(has(alice, item(lantern, 1)), $a).
assertion(name(alice, 'Alice'), $a).
region_end($a, 0).
```

Facts may also be asserted over _regions_ of text. Facts asserted over regions may overlap, but overlapping regions must be disambiguated with labels. Note that labels are only significant to the parser, and do not become facts:

```ilk
@a<has(alice, lantern)|Alice and @<has(franklin, lantern); has(franklin, match)|Franklin picked up lanterns@a>, but only Franklin picked up an extra match.@>
```

```pl
region_start($a, 0).
assertion(has(alice, lantern), $a).
region_end($a, 37).

region_start($b, 10).
assertion(has(franklin, lantern), $b).
assertion(has(franklin, match), $b).
region_end($b, 82).
```

Because the regions above overlap without either completely containing the other, there is no parent or ancestor relationship between them. A region completely contained by other regions, however, is a child of each of them, even when those containing regions overlap one another:

```ilk
@a<area(a)|Alpha @<area(b)|Beta @<item|child@> gamma@a> delta@>
```

```pl
region_start($a, 0).
assertion(area(a), $a).
region_end($a, 22).

region_start($b, 6).
assertion(area(b), $b).
region_end($b, 28).

region_start($c, 11).
assertion(item, $c).
region_end($c, 16).

parent($a, $c).
parent($b, $c).
ancestor($a, $c).
ancestor($b, $c).
```

Note that naming a region moves it out of the normal open/close stack entirely, so in each overlapping example above, only one of the overlapping regions needed to be labeled. Labels may be omitted completely if there is no overlap:

```ilk
@<has(alice, lantern)|Alice picked up the lantern.@>
```

```pl
region_start($a, 0).
assertion(has(alice, lantern), $a).
region_end($a, 28).
```

So far, Ilk has tried to remain agnostic of any structure in the underlying content: it does not define paragraphs, runs, etc. In practice, however, complex documents often benefit from organizational structure, and when facts naturally assert that structure, it may be useful to have syntax for this purpose. These are blocks.

Block opening and closing markers must be the only non-whitespace content on their lines (any whitespace will be discarded). Furthermore, blocks are indentation-aware: the first non-empty line defines a prefix that will be stripped from that, and all successive, lines.

Blocks may also be nested without affecting the underlying content.

The following examples show how the whitespace used to define blocks do not affect the extracted text:

```ilk
Alice cleared her throat.
@[speech(alice)|
  Hello, Franklin.
  Have you seen my lantern?
@]
She waited.
```

```
Alice cleared her throat.
Hello, Franklin.
Have you seen my lantern?
She waited.
```

```ilk
@[outer|
  one
  @[inner|
    two
  @]
  three
@]
```

```
one
two
three
```

## AI Use

The code in this repository is, in part, AI-generated. All generated code is reviewed and revised by a human. All documentation and prose is written by a human.