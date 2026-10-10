-- Mixed-width integer arithmetic declares the **wider** operand's type, in either
-- order. Postgres registers a distinct operator per ordered pair and both directions
-- return the wider type — `int24pl(int2,int4)` and `int42pl(int4,int2)` are each
-- `prorettype => int4`; `int48*`/`int84*` and `int28*`/`int82*` are each `int8`. `%`
-- has no mixed-width operator and resolves through the implicit int2→int4→int8 casts
-- to the same answer.
--
-- pglet declared the **left** operand's type instead, so every narrower-on-the-left
-- pair was wrong: `int2 + int4` declared `int2` (OID 21, a two-byte type) while the
-- value path — which already widened correctly — sent four bytes, and `int4 + int8`
-- declared a four-byte type over an eight-byte value. That is the same
-- undecodable-to-a-binary-client shape already fixed for `float4` arithmetic
-- (`float4_same_width`), `sum(real)` (`sum_float4_accumulator`), and
-- `COALESCE(int4, int8)`. The evaluated values were never wrong; the recorded
-- expectation still moves with the declared type, because the harness decodes each
-- column against it and renders INT8 as a JSON string and INT4 as a number — so the
-- `.expected.json` below encodes the declaration in its value shapes, and a wrong
-- declaration either changes a shape or fails the decode outright. What the corpus
-- lacked was not an instrument but the query: no fixture did mixed-width integer
-- **column** arithmetic.
--
-- The wider-on-the-left direction is the control: it was right by accident (taking
-- the left operand happens to be correct there), so a fix that merely swapped which
-- side wins would redden it.
-- Magnitudes are chosen so every result stays in range while each still overflows the
-- type pglet used to declare: `s + i` is 90000 (past int2's 32767) under a declared
-- `int2`, and `i + b` is 5000060000 (past int4) under a declared `int4`.
-- @setup
CREATE TABLE mw (s SMALLINT, i INT, b BIGINT);
INSERT INTO mw VALUES (30000, 60000, 5000000000);
-- @test
SELECT s + i AS narrow_left_add,   -- int4 (was int2)
       i + b AS narrow_left_add8,  -- int8 (was int4)
       s + b AS narrow_left_28,    -- int8 (was int2)
       i + s AS wide_left_add,     -- int4 — control
       b + i AS wide_left_add8,    -- int8 — control
       i - b AS narrow_left_sub,   -- int8
       s * i AS narrow_left_mul,   -- int4
       b / i AS wide_left_div,     -- int8 — control
       i % b AS narrow_left_mod,   -- int8
       s % i AS narrow_left_mod2   -- int4
FROM mw;
