-- The **window node** builds its output descriptors on a third arm of the same walk.
-- Its bare-column path already read the input descriptors; its expression path did
-- not, so `m.b + 0` beside a window function declared `int4` over an eight-byte
-- value while `row_number()` — a fixed `int8` needing no resolution — stayed right.
-- A fix covering the projection and aggregate arms and not this one leaves it red.
--
-- `rn` is the polarity pin.
-- @setup
CREATE TABLE mw (s SMALLINT, i INT, b BIGINT);
INSERT INTO mw VALUES (30000, 60000, 5000000000);

-- @test
SELECT m.b + 0 AS win_expr,          -- int8 (was int4)
       row_number() OVER () AS rn    -- int8 — polarity pin
FROM mw m;
