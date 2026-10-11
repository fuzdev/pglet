# Text entering the engine, in the server encoding

## The rule

> **Text entering the engine is validated in the server encoding wherever it enters.** A
> NUL byte and an illegal UTF-8 sequence are both
> `22021 invalid byte sequence for encoding "UTF8": <bytes>`.

There are exceptions. Each entry point in § The doors that reach it runs one shared scan,
and 18.6 validates text at each of those it has (the dump's export is pglet's own).
§ Measured doors that do not reach it lists the entry points we found that skip the scan.
That list is what we measured, not every way text can get in. One of them is SQL text
passed to an in-process API, whose NUL can end up raw in a dump.

## The scan

It lives in `pglet_core::encoding`, the leaf crate that `pglet_sql`, `pglet` and both hosts
depend on.

- `first_invalid_utf8_offset` scans left to right and returns the offset of the first NUL,
  or of the first byte that doesn't start a legal UTF-8 sequence. Rust's `str::from_utf8`
  refuses what `pg_utf8_islegal` refuses (overlongs, surrogate bytes, values above
  `U+10FFFF`), but it accepts a NUL, and `pg_utf8_verifystr` stops at one. So the scan
  checks for NUL separately and reports whichever failure comes first.
- `invalid_encoding_bytes(all, offset)` builds `report_invalid_encoding`'s byte list:
  `min(pg_utf_mblen(lead), remaining)` bytes from the offset. `remaining` counts from the
  buffer 18.6 was verifying, which isn't always the slice the caller scanned, so the
  caller passes that buffer in.
- `utf8_failure_bytes` and `text_from_utf8` combine the two over one buffer.

The error message is defined in one place, the `#[error]` attribute on
`Error::InvalidEncoding`.

## The doors that reach it

| door | pglet | 18.6 | notes |
| --- | --- | --- | --- |
| an `E'…'` literal's byte escapes | `lexer::escapes`'s `finish_literal`, from `decode_escape` | the scanner's `pg_verifymbstr` at the closing quote | `docs/parser.md` § A string literal is assembled in 18.6's units |
| COPY's raw input | `executor::copy`'s `execute_copy_from` | `CopyConvertBuf`'s `pg_encoding_verifymbstr`, over the input not yet verified | the bytes are taken from the whole remaining input, and the error is raised once the line holding the failure is read, so a byte after the `\.` marker isn't reported |
| a de-escaped COPY field | `copy_unescape_field` | `CopyReadAttributesText`'s `pg_verifymbstr` | the bytes are taken from the field, and every field on the line is checked before the field count and before any value is typed |
| a bound `$N` in text format, native server | `pglet_server::encoding::decode_param` | `exec_bind_message`'s `pg_client_to_server`, before the parameter's input function | for every parameter type |
| a bound `$N` in binary format, native server | `recv_binary`'s `text`/`varchar` and `jsonb` arms, behind `value_from_binary` | `textrecv` and `jsonb_recv`, through `pq_getmsgtext` | an array's text elements reach the `text` arm one at a time. The `jsonb` arm scans what comes **after** the leading version byte, since `jsonb_recv` only reaches `pq_getmsgtext` past it, so a leading `0x00` or high byte is `XX000 unsupported jsonb version number`, and this scan never sees it (`docs/param_binding.md`) |
| a parameter bound from JS, wasm host | `pglet_wasm`'s `bind_js_params` | no wire of its own; the text-format bind is the counterpart | a JS string arrives as a Rust `String`, so NUL is the only possible failure. Each parameter is checked before it's decoded, along with an array target's string items |
| the dump's COPY payload | `pglet::dump`'s `emit_copy` | none: the export is pglet's own | a NUL in the COPY text refuses the export instead of being written raw, since a `psql -f` restore would drop it along with the rest of its line |
| a stored jsonb string, at the dump | `pglet::dump`'s `refuse_unreadable_values` | none: the export is pglet's own | the row above can't catch this one, because the renderer writes the NUL as `\u0000` and the payload has no NUL in it. So this check looks at the value instead: § A stored `Value::Text` becomes a jsonb string |

## One byte, three SQLSTATEs

Each is the code 18.6 gives, checked on both engines. We match them rather than merging
them into one:

- `22021` at the entry points above.
- `22021` again from the **jsonb reader**, for a *raw* NUL inside a JSON string, via
  `json.rs`'s `raw_control_character` and this module's `invalid_encoding_bytes`. 18.6's
  JSON lexer has no such check, because its encoding check runs before the lexer on every
  route we found that can carry a NUL. pglet needs it for the routes that reach the reader
  without going through the scan, like the in-process APIs below. It's `22021` and not the
  `22P02` other raw control characters get, because both engines answer `22021` for a NUL
  in jsonb text, both at a text-format `$N` and in a `COPY` field.
- `22P05 unsupported Unicode escape sequence` for a jsonb string's `\u0000` escape:
  `json_lex_string`'s `JSON_UNICODE_CODE_POINT_ZERO`, and pglet's
  `Error::UnsupportedUnicodeEscape` from `unescape_json_string`. On the wasm host, a JS
  object or array bound to a jsonb parameter hits this one, because `JSON.stringify` writes
  its NUL as that escape. When the escape is inside a container or string the reader can't
  close, or comes before junk after a container it can, pglet answers `22P02` instead
  (`EDGES.md` row 131).
- `42601` for a string literal's `\u0000` or `U&'\0000'`, refused as an invalid Unicode
  escape value.

## Measured doors that do not reach it

- **Frontend message strings.** `pglet_wire`'s `Reader::read_cstring` decodes the SQL of
  a simple `Query` or a `Parse`, the statement and portal names in a `Parse`, `Bind`,
  `Describe`, `Execute` or `Close`, and a `CopyFail` message. It substitutes `U+FFFD` for
  an illegal byte, where 18.6 answers `22021` (`EDGES.md` row 130).
- **SQL text passed to an in-process API**: `Database::execute`, the SQL that
  `Database::execute_bound` takes, and the wasm hosts' `exec`, `query` and `queryParams`.
  A Rust `&str` can't hold illegal UTF-8, but it can hold a NUL, and the lexer only passes
  a literal's bytes to the scan from the `E'…'` decoder. So a NUL inside a plain,
  dollar-quoted or `U&'…'` literal, or inside a `"…"`-quoted identifier, gets past the
  lexer (those are the positions we tested). A NUL where a token would start is
  `42601 syntax error: unexpected character`.

  What happens after the lexer doesn't depend on where the NUL was. A column's input
  function may refuse it. If it's stored, the dump handles it inconsistently: a row value
  refuses the export with `22021`, but catalog text gets written raw, and every `psql -f`
  restore into 18.6 we tried exited `0` after losing everything the NUL's open quote ran
  over. `EDGES.md` row 133 lists each case.

  This doesn't let a NUL in as **jsonb text**: the reader refuses a raw NUL in a JSON
  string with its own `22021` (§ One byte, three SQLSTATEs), so no `::jsonb` cast, jsonb
  literal `INSERT` or other jsonb text reader stores one. It does let a NUL get *inside a
  jsonb value*, by the route in § A stored `Value::Text` becomes a jsonb string, which
  this entry point's `Value::Text` feeds.
- **A `Value` bound through `Database::execute_bound`.** The bind callback passes values,
  not client bytes, so a `Value::Text` holding a NUL is stored as is (`length` `3`), and
  the dump then refuses the store. No wire message carries text this way, so there's no
  18.6 behavior to compare with. A bound `Value::Jsonb` puts a NUL straight into a
  **jsonb** value without reading jsonb text, and a bound `Value::Text` can too, by the
  route below.
- **Startup parameter values**, which `decode_startup` decodes with `U+FFFD` substituted.
  18.6 doesn't refuse the one we tested either (an `application_name` holding `0xE9`), so
  the difference is in the value read back, not a missing refusal (`EDGES.md` row 130's
  bullet).

## A stored `Value::Text` becomes a jsonb string

A NUL inside a jsonb value is a different problem from a NUL in jsonb *text*, and the
reader's `22021` only covers the second. **A `Value::Text` holding a NUL becomes a jsonb
string when a jsonb-producing expression *wraps* it instead of parsing it as jsonb text.**
What matters is how the expression treats the text. It doesn't matter that the result is
jsonb, or which entry point delivered the `Value::Text`.

The wrapping expression here is `jsonb_build_object` (`executor::expr`). Its **key**
position takes a `Value::Text` directly. Its **value** position goes through
`value_to_jsonb` (`build_jsonb_object` is its only caller), whose `Text` arm makes a JSON
string and whose fallback arm renders other values as text. So a NUL in a `text[]` element
gets through too: `jsonb_build_object('k', arr)` over `ARRAY['p<NUL>q']` is the jsonb
string `{p<NUL>q}`.

The routes we tested that *parse* refuse instead, since the reader runs first: the `::jsonb` cast
(`executor::coerce`) and the text-left arms of `->`, `->>`, `#>`, `#>>`, `?`, `?|` and
`?&` (`executor::expr`), which all call `parse_jsonb`. For example,
`('{"a":1}'::text) -> 'a'` is the jsonb number `1`, a jsonb built from text. Over a column
holding jsonb text with a NUL in it, the same `->` is `22021`, and so are the other
operators and the cast. (Over the bare `x<NUL>y` they're `22P02` at the `x`, before the
reader reaches the NUL, which is why the input has to be jsonb text to hit this.) With a
**jsonb** left operand they read what's stored, so they pass an existing NUL along without
adding one (`v -> 'k'` returns the jsonb string with the NUL in it).

Every other spelling we tried is refused, so none of them is another way
in: `to_jsonb`, `to_json`, `jsonb_build_array`, `jsonb_object`, `jsonb_agg`, `jsonb_set`,
`jsonb_insert`, `jsonb_strip_nulls`, `jsonb_pretty`, `array_to_json`, `row_to_json`,
`json_build_object`, `json_object`, `jsonb_array_elements` and `jsonb_path_query` answer
`0A000 not implemented: function not supported: <name>`. The `jsonb || jsonb` operator
answers `0A000 not implemented: operator jsonb || jsonb is not supported (only text
concatenation)`. `jsonb - text` answers `42804`, and `#-` answers
`42883 operator does not exist` (`EDGES.md` row 184). This list comes from probing, so
re-run the probe when it matters instead of trusting it. A bound `Value::Jsonb` is already
a jsonb string.

Here it is through **`Database::execute`** alone, with no bound `Value`: the NUL arrives
as SQL text (the in-process route above) into a `text` column, and the jsonb is built from
the stored value:

```
INSERT INTO t VALUES (1, 'x<NUL>y');                           -- accepted, length 3
INSERT INTO j SELECT id, jsonb_build_object('k', a) FROM t;    -- accepted
INSERT INTO j SELECT id + 10, jsonb_build_object(a, 1) FROM t; -- accepted, the NUL in the KEY
SELECT v::text FROM j;  -->  {"k": "x<NUL>y"}  and  {"x<NUL>y": 1}
```

The wasm hosts do bind `Value`s. `pglet_wasm`'s one `execute_bound` call site
(`database.rs`) passes it `bind_js_params`, which returns an `Option<Value>` per JS
argument. That path is built to refuse a `Value::Text` carrying a NUL:
`refuse_text_outside_the_server_encoding` (`param.rs`) scans the JS strings that become
text without being converted first. Those are the parameter itself and, for an array
target, the array's direct string items. Every other JS value is converted on its way to
text, either through a number's or boolean's text form or through `JSON.stringify`, which
writes a NUL as an escape. So the scan doesn't cover every JS string reachable from an
argument, only the ones that become a `Value::Text` without conversion, as `param.rs`'s
own doc says. That leaves wasm hosts with the SQL-text route above, not a bound
`Value::Text`.

**What the dump does.** `pglet::dump`'s `refuse_unreadable_values` walks every jsonb
string in a stored value at every depth: object keys and values, arrays, nested
containers, and the elements of a `jsonb[]` column. It passes each to
`utf8_failure_bytes`, so the export gives the same `22021` as a NUL in a `text` column.
The renderer escapes the NUL instead of writing it, so the COPY payload has no NUL for the
payload check to find. Without the value check, every position we tried (an object key, an
object's string value, an array element, a doubly nested one, a `jsonb[]` element) dumps
with exit `0`. Each `psql -f` restore into a fresh 18.6 then also exits `0`, but that
table's `COPY` is refused with `22P05 unsupported Unicode escape sequence` and `count(*)`
is `0`: the clean rows are lost along with the bad one.

The check looks at the **value** and not the rendered text because a jsonb string whose
content is the six characters `\u0000` renders as `\\u0000`, which contains the escape as
a substring. That document restores into 18.6 fine, and refusing it would be wrong.

One thing the scan's errors don't include: 18.6 attaches a `CONTEXT` (`W`) field to its
COPY, bound-parameter and jsonb errors (the COPY line, `unnamed portal parameter $1`,
`JSON data, line 1: …`), and pglet sends none.
