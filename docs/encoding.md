# Text entering the engine, in the server encoding

## The rule

> **Text entering the engine is validated in the server encoding wherever it enters.** A
> NUL byte and an illegal UTF-8 sequence are both
> `22021 invalid byte sequence for encoding "UTF8": <bytes>`.

That is the rule pglet is held to, and it has measured exceptions. pglet reaches one scan
from each door in § The doors that reach it, and 18.6 validates text at each of them it
has; the dump's export is pglet's own. § Measured doors that do not reach it lists the
doors measured to bypass the scan — SQL text handed to an in-process door among them,
whose NUL a dump can emit raw — and it is what was measured, not an enumeration of every
way text enters.

## The scan

`pglet_core::encoding` holds it, the leaf crate `pglet_sql`, `pglet` and both hosts
depend down on.

- `first_invalid_utf8_offset` returns the offset of the first NUL or the first byte that
  starts no legal UTF-8 sequence, scanning left to right. Rust's `str::from_utf8` refuses
  what `pg_utf8_islegal` refuses — overlongs, surrogate bytes, values above `U+10FFFF` —
  but it accepts a NUL, and `pg_utf8_verifystr` stops at one. So the scan looks for the
  NUL on its own and takes whichever failure comes first.
- `invalid_encoding_bytes(all, offset)` is `report_invalid_encoding`'s byte list:
  `min(pg_utf_mblen(lead), remaining)` bytes from the offset. `remaining` is counted in
  the buffer 18.6 was verifying, which is not always the slice the caller scanned, so the
  caller passes that buffer.
- `utf8_failure_bytes` and `text_from_utf8` compose the two over one buffer.

The sentence has one home, the `#[error]` attribute on `Error::InvalidEncoding`.

## The doors that reach it

| door | pglet | 18.6 | what is particular to the door |
| --- | --- | --- | --- |
| an `E'…'` literal's byte escapes | `lexer::escapes`'s `finish_literal`, from `decode_escape` | the scanner's `pg_verifymbstr` at the closing quote | `docs/parser.md` § A string literal is assembled in 18.6's units |
| COPY's raw input | `executor::copy`'s `execute_copy_from` | `CopyConvertBuf`'s `pg_encoding_verifymbstr`, over the input not yet verified | the bytes are named from the whole remaining input, and the error is raised once a line holding the failure is read, so a byte after the `\.` marker is not reported |
| a de-escaped COPY field | `copy_unescape_field` | `CopyReadAttributesText`'s `pg_verifymbstr` | the bytes are named from the field, and every field on the line is checked before the field count and before any value is typed |
| a bound `$N` in text format, native server | `pglet_server::encoding::decode_param` | `exec_bind_message`'s `pg_client_to_server`, ahead of the parameter's input function | whatever the parameter's type |
| a bound `$N` in binary format, native server | `recv_binary`'s `text`/`varchar` and `jsonb` arms, behind `value_from_binary` | `textrecv` and `jsonb_recv`, through `pq_getmsgtext` | an array's text elements reach the `text` arm one at a time, and the `jsonb` arm scans what stands **behind** the version byte its first byte always is, `jsonb_recv` reaching `pq_getmsgtext` only past that byte — so a leading `0x00` or high byte is `XX000 unsupported jsonb version number` rather than anything this scan gets to judge (`docs/param_binding.md`) |
| a parameter bound from JS, wasm host | `pglet_wasm`'s `bind_js_params` | no wire of its own; the text-format bind is the counterpart | a JS string arrives as a Rust `String`, so a NUL is the only failure; each parameter is checked before it is decoded, an array target's string items with it |
| the dump's COPY payload | `pglet::dump`'s `emit_copy` | no counterpart: the export is pglet's own | a NUL in the COPY text it would write refuses the export rather than being written raw, which a `psql -f` restore would drop along with the rest of its line |
| a stored jsonb string, at the dump | `pglet::dump`'s `refuse_unreadable_values` | no counterpart: the export is pglet's own | the row above cannot see this one — the renderer spells the NUL `\u0000`, so the payload carries no NUL — and the walk asks the value instead: § A stored `Value::Text` becomes a jsonb string |

## One byte, three SQLSTATEs

Each is 18.6's code, measured on both engines, and they are reproduced rather than unified:

- `22021` at the doors above.
- `22021` again from the **jsonb reader**, for a *raw* NUL inside a JSON string, through
  `json.rs`'s `raw_control_character` and this module's `invalid_encoding_bytes`. 18.6's
  JSON lexer has no counterpart arm — its encoding check answers ahead of the lexer at
  every route measured to carry a NUL — so this arm exists for the routes that reach the
  reader having skipped the scan, the in-process doors below among them. `22021` rather
  than the `22P02` the reader's other raw control characters take, because `22021` is
  what both engines were measured to answer for a NUL in jsonb text at a text-format
  `$N` and in a `COPY` field alike.
- `22P05 unsupported Unicode escape sequence` for a jsonb string's `\u0000` escape —
  `json_lex_string`'s `JSON_UNICODE_CODE_POINT_ZERO`, and pglet's
  `Error::UnsupportedUnicodeEscape` from `unescape_json_string`. On the wasm host a JS
  object or array bound to a jsonb parameter reaches this one, `JSON.stringify` having
  spelled its NUL as that escape. Where the escape stands inside a container or string
  the reader cannot close at all, or ahead of junk past a container it can, pglet answers
  `22P02` instead (`EDGES.md` row 131).
- `42601` for a string literal's `\u0000` or `U&'\0000'`, refused as an invalid Unicode
  escape value.

## Measured doors that do not reach it

- **Frontend message strings.** The SQL of a simple `Query` or a `Parse`, the statement
  and portal names a `Parse`, `Bind`, `Describe`, `Execute` or `Close` carries, and a
  `CopyFail` message are decoded by `pglet_wire`'s `Reader::read_cstring`, which
  substitutes `U+FFFD` for an illegal byte where 18.6 answers `22021` (`EDGES.md` row 130).
- **SQL text handed to an in-process door** — `Database::execute`, the SQL
  `Database::execute_bound` takes, and the wasm hosts' `exec`, `query` and `queryParams`.
  A Rust `&str` holds no illegal UTF-8 byte but can hold a NUL, and the lexer hands a
  literal's bytes to the scan from the `E'…'` decoder alone. A NUL inside a plain,
  dollar-quoted or `U&'…'` literal or a `"…"`-quoted identifier — the positions measured —
  passes the lexer, and one standing where a token would start is
  `42601 syntax error: unexpected character`. Past the lexer its cost is not decided by
  where it stood: a column's input can refuse it, and a stored one is written back out by
  the dump through paths measured to disagree — refusing the export with `22021` for a
  row value, and writing it raw in catalog text, where each `psql -f` restore into 18.6
  measured exited `0` having lost what the NUL's open quote ran over. `EDGES.md` row 133
  records each member measured. This door does not reach a NUL arriving as
  **jsonb text**: the reader refuses a raw one in a JSON string with its own `22021`
  (§ One byte, three SQLSTATEs), so no `::jsonb` cast, `INSERT` of a jsonb literal or
  other reader of jsonb text stores one. It does reach a NUL *inside a jsonb value*,
  by the route § A stored `Value::Text` becomes a jsonb string states — a route this
  door's own `Value::Text` feeds.
- **A `Value` bound through `Database::execute_bound`.** The bind callback hands the engine
  values rather than client bytes, so a `Value::Text` holding a NUL is written as it is
  (`length` `3`), and the dump then refuses the store. No wire message carries text this
  way, so there is no 18.6 answer to set beside it. A bound `Value::Jsonb` puts one
  straight inside a **jsonb** value, no jsonb text being read on the way — and so does the
  `Value::Text` this door binds, by the route below.
- **Startup parameter values**, which `decode_startup` decodes with `U+FFFD` substituted.
  18.6 does not refuse the one measured either, an `application_name` holding `0xE9`, so
  it is a difference in the value read back rather than a missing refusal (`EDGES.md`
  row 130's bullet).

## A stored `Value::Text` becomes a jsonb string

A NUL inside a jsonb value is a second question from a NUL in jsonb *text*, and the
reader's `22021` answers only the second. **A `Value::Text` holding a NUL becomes a jsonb
string where a jsonb-producing expression *wraps* it rather than parsing it as jsonb
text** — so what decides is the expression's own treatment of the text, not that the
expression produces a jsonb, and not which *door* delivered the `Value::Text`.

The wrapping expression here is `jsonb_build_object` (`executor::expr`). Its **key**
position takes a `Value::Text` straight, and its **value** position goes through
`value_to_jsonb` — `build_jsonb_object` is that function's only caller — whose `Text` arm
makes a JSON string of it and whose fallback arm renders another value's text output, so
a `text[]` element's NUL reaches it too (`jsonb_build_object('k', arr)` over
`ARRAY['p<NUL>q']` is the jsonb string `{p<NUL>q}`, measured).

The routes measured here that *parse* refuse instead, the reader standing in front of
each: the `::jsonb` cast (`executor::coerce`) and the text-left arms of `->`, `->>`, `#>`,
`#>>`, `?`, `?|` and `?&` (`executor::expr`), each of which calls `parse_jsonb`. So
producing a jsonb is not the predicate — `('{"a":1}'::text) -> 'a'` is the jsonb number
`1` here, a jsonb made from text, and over a column holding jsonb text with the NUL in it
that same `->` is `22021`, as are the other operators listed and the cast. (Over the bare
`x<NUL>y` those same routes are `22P02` at the `x`, the reader never reaching the byte —
which is why the text handed to them has to be jsonb text.) Over a **jsonb** left operand
they read what is stored, so they hand a NUL onward without introducing one (`v -> 'k'`
returns the jsonb string with the byte in it, measured).

Every other spelling probed is refused rather than evaluated, so none of them is a second
way in: `to_jsonb`, `to_json`, `jsonb_build_array`, `jsonb_object`, `jsonb_agg`,
`jsonb_set`, `jsonb_insert`, `jsonb_strip_nulls`, `jsonb_pretty`, `array_to_json`,
`row_to_json`, `json_build_object`, `json_object`, `jsonb_array_elements` and
`jsonb_path_query` answer `0A000 not implemented: function not supported: <name>`; the
`jsonb || jsonb` operator answers `0A000 not implemented: operator jsonb || jsonb is not
supported (only text concatenation)`, which is that SQLSTATE under a different message and
over an operator rather than a function; `jsonb - text` answers `42804` and the `#-`
operator `42883 operator does not exist` (`EDGES.md` row 184). That is a probe of the
spellings, so re-run it rather than inherit it. A bound `Value::Jsonb` is a jsonb string
already.

Measured through **`Database::execute`** alone, no bound `Value` anywhere — the NUL
arriving as SQL text (the in-process door above) into a `text` column, and the jsonb
built from the stored value:

```
INSERT INTO t VALUES (1, 'x<NUL>y');                           -- accepted, length 3
INSERT INTO j SELECT id, jsonb_build_object('k', a) FROM t;    -- accepted
INSERT INTO j SELECT id + 10, jsonb_build_object(a, 1) FROM t; -- accepted, the NUL in the KEY
SELECT v::text FROM j;  -->  {"k": "x<NUL>y"}  and  {"x<NUL>y": 1}
```

The wasm hosts do bind `Value`s: `pglet_wasm`'s one `execute_bound` call site
(`database.rs`) hands it `bind_js_params`, which returns an `Option<Value>` per JS
argument. What that path is built to refuse is a `Value::Text` carrying a NUL:
`refuse_text_outside_the_server_encoding` (`param.rs`) scans the JS strings that are
decoded as text without being re-spelled first — the parameter itself, and, for an array
target, the array's direct string items — and every other JS value is re-spelled on its way
to text, through a number's or a boolean's spelling or through `JSON.stringify`, which
writes a NUL as an escape. That pair is the bound: the scan is not over every JS string
reachable from an argument, it is over the ones that reach a `Value::Text` unrespelled.
`param.rs`'s own doc states the same reading. So the route a wasm host reaches here is the
SQL-text one above rather than a bound `Value::Text`.

**What the dump then does.** `pglet::dump`'s `refuse_unreadable_values` walks a stored
value's jsonb strings at every depth — an object's key as well as its value, through
arrays and nested containers, and through a `jsonb[]` column's elements — and hands each
to `utf8_failure_bytes`, so the export answers the same `22021` a NUL in a `text` column
earns. The renderer escapes the
NUL rather than writing it, so the COPY payload holds none for the payload gate to find;
without the value gate, each position measured — an object key, an object's string value,
an array element, a doubly-nested one and a `jsonb[]` element — dumps at exit `0`, and
each `psql -f` restore into a fresh 18.6 exits `0` with that table's `COPY` refused
`22P05 unsupported Unicode escape sequence` and `count(*)` `0`, its clean rows lost with
the offending one. The gate is asked of the **value** and not of the rendered text
because a jsonb string whose content is the six characters `\u0000` renders as
`\\u0000`, which holds the escape as a substring; that document restores into 18.6
whole, and refusing it would be the over-correction.

One field the scan's errors do not carry: 18.6 attaches a `CONTEXT` (`W`) to its COPY,
bound-parameter and jsonb errors — the COPY line, `unnamed portal parameter $1`,
`JSON data, line 1: …` — and pglet sends none.
