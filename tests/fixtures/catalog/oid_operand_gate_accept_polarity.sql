-- The accept side of the declaration-only operand gate, so a fix that refuses too much
-- reddens here rather than passing the four `_is_loud` fixtures beside it. All three
-- answer on 18.6: a bare aggregate over an `oid` (declared 26 on both engines), the same
-- aggregate cast to `int4` and then shifted, and `count()`, whose result is `bigint`
-- however its argument is declared.
--
-- The values are compared rather than the OIDs themselves: a catalog OID never agrees
-- between two engines, so each column reduces to something that does.
-- @setup
CREATE TABLE oid_ops (a INT, b INT, CONSTRAINT oid_ops_ck CHECK (a > 0));

-- @test
SELECT count(conrelid) + 1 AS shifted_count,
       max(conrelid) > 0 AS aggregate_answers,
       max(conrelid)::int4 + 1 > 0 AS cast_then_shifted
FROM pg_constraint WHERE conname = 'oid_ops_ck'
