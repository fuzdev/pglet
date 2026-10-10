-- The `OFFSET` spelling of the no-FROM discard (`select/no_from_limit_zero.sql` is the
-- `LIMIT` one, and the two are separate members because `parse_limit_offset` returns them
-- together while `LIMIT`'s and `OFFSET`'s clause grammars are read independently). One row
-- here before, `(0 rows)` on 18.6, silent on both engines; its write polarity is
-- `select/no_from_offset_in_an_update.sql`.
-- @test
SELECT 1 AS q OFFSET 1;
